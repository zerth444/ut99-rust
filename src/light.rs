use std::collections::HashMap;
use std::io;

use crate::map::{Face, Vec3};
use crate::package::{Package, Reader};

pub struct LightIndex {
    offset: i32,
    pan: [f32; 2],
    width: usize,
    height: usize,
    scale: [f32; 2],
    actors: i32,
}

impl LightIndex {
    pub fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        let offset = reader.u32()? as i32;
        let pan = [reader.f32()?, reader.f32()?];
        reader.f32()?; // Reserved Z component.
        Ok(Self {
            offset,
            pan,
            width: reader.index()?,
            height: reader.index()?,
            scale: [reader.f32()?, reader.f32()?],
            actors: reader.u32()? as i32,
        })
    }
}

pub struct Light {
    position: Vec3,
    color: [f32; 3],
    radius: f32,
}

pub fn read_lights(package: &Package<'_>, refs: &[i32]) -> io::Result<HashMap<i32, Light>> {
    let mut lights = HashMap::new();
    for &id in refs {
        if id <= 0 || lights.contains_key(&id) {
            continue;
        }
        let Some(actor) = package.exports.get(id as usize - 1) else {
            continue;
        };
        let mut props = package.properties(actor)?;
        let mut position = Vec3::default();
        let mut hue = 0;
        let mut saturation = 255;
        let mut brightness = 64;
        let mut radius = 64;
        let mut active = true;
        while let Some((name, mut value)) = props.next()? {
            match name {
                "Location" => position = Vec3::read(&mut value)?,
                "LightHue" => hue = value.byte()?,
                "LightSaturation" => saturation = value.byte()?,
                "LightBrightness" => brightness = value.byte()?,
                "LightRadius" => radius = value.byte()?,
                "LightType" => active = value.byte()? != 0,
                _ => {}
            }
        }
        let color = hue_color(hue, saturation, brightness);
        lights.insert(
            id,
            Light {
                position,
                color,
                radius: if active {
                    (f32::from(radius) + 1.0) * 25.0
                } else {
                    0.0
                },
            },
        );
    }
    Ok(lights)
}

fn hue_color(hue: u8, saturation: u8, brightness: u8) -> [f32; 3] {
    let hue = f32::from(hue) / 85.0;
    let fraction = hue.fract();
    let gray = f32::from(saturation) / 255.0;
    let falling = 1.0 - fraction * (1.0 - gray);
    let rising = gray + fraction * (1.0 - gray);
    let value = (f32::from(brightness) / 255.0).sqrt() * 1.85;
    let color = match hue as u8 {
        0 => [falling, rising, gray],
        1 => [gray, falling, rising],
        _ => [rising, gray, falling],
    };
    color.map(|channel| channel * value)
}

pub struct Lightmap {
    width: usize,
    height: usize,
    pan: [f32; 2],
    scale: [f32; 2],
    colors: Vec<[f32; 3]>,
}

impl Lightmap {
    pub(crate) fn gpu_data(&self) -> (usize, usize, [f32; 2], [f32; 2], &[[f32; 3]]) {
        (self.width, self.height, self.pan, self.scale, &self.colors)
    }
    pub fn bake(
        face: &Face,
        index: &LightIndex,
        masks: &[u8],
        refs: &[i32],
        lights: &HashMap<i32, Light>,
    ) -> Option<Self> {
        let width = index.width;
        let height = index.height;
        if width == 0 || height == 0 || index.scale.iter().any(|&scale| scale <= 0.0) {
            return None;
        }
        let u_cross = face.v_axis.cross(face.normal);
        let determinant = face.u_axis.dot(u_cross);
        if determinant.abs() < 0.00001 {
            return None;
        }
        let pitch = width.div_ceil(8);
        let stride = pitch.checked_mul(height)?;
        let offset = usize::try_from(index.offset).ok()?;
        let actors = refs.get(usize::try_from(index.actors).ok()?..)?;
        let end = actors.iter().position(|&id| id == 0)?;
        let masks = masks.get(offset..offset.checked_add(stride.checked_mul(end)?)?)?;
        if end == 0 {
            return None;
        }

        let step_u = u_cross * (index.scale[0] / determinant);
        let step_v = face.normal.cross(face.u_axis) * (index.scale[1] / determinant);
        let origin = face.origin
            + u_cross * (index.pan[0] / determinant)
            + face.normal.cross(face.u_axis) * (index.pan[1] / determinant);
        let mut colors = vec![[0.30; 3]; width.checked_mul(height)?];
        for (slot, id) in actors[..end].iter().enumerate() {
            let Some(light) = lights.get(id).filter(|light| light.radius > 0.0) else {
                continue;
            };
            let mask = &masks[slot * stride..(slot + 1) * stride];
            for y in 0..height {
                for x in 0..width {
                    let point = origin + step_u * x as f32 + step_v * y as f32;
                    let to_light = light.position - point;
                    let distance = to_light.dot(to_light).sqrt();
                    if distance >= light.radius {
                        continue;
                    }
                    let fraction = (distance / light.radius).max(0.01);
                    let attenuation =
                        ((1.0 - fraction).powi(2) * (1.0 + 2.0 * fraction) / fraction).min(1.0);
                    let angle = (to_light.dot(face.normal).abs() / distance.max(1.0)).min(1.0);
                    let mut visibility = 0.0;
                    for (dy, wy) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                        let row = y.saturating_add_signed(dy).min(height - 1) * pitch;
                        for (dx, wx) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                            let col = x.saturating_add_signed(dx).min(width - 1);
                            if mask[row + col / 8] & (1 << (col % 8)) != 0 {
                                visibility += wx * wy;
                            }
                        }
                    }
                    let amount = visibility * (1.0 / 16.0) * angle * attenuation;
                    for (channel, light) in colors[y * width + x].iter_mut().zip(light.color) {
                        *channel += light * amount;
                    }
                }
            }
        }
        Some(Self {
            width,
            height,
            pan: index.pan,
            scale: index.scale,
            colors,
        })
    }

    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let x = ((u - self.pan[0]) / self.scale[0]).clamp(0.0, (self.width - 1) as f32);
        let y = ((v - self.pan[1]) / self.scale[1]).clamp(0.0, (self.height - 1) as f32);
        let x0 = x as usize;
        let y0 = y as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let top = self.colors[y0 * self.width + x0];
        let right = self.colors[y0 * self.width + x1];
        let bottom = self.colors[y1 * self.width + x0];
        let corner = self.colors[y1 * self.width + x1];
        std::array::from_fn(|i| {
            let a = top[i] + (right[i] - top[i]) * x.fract();
            let b = bottom[i] + (corner[i] - bottom[i]) * x.fract();
            a + (b - a) * y.fract()
        })
    }
}
