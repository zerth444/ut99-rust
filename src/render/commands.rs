use std::collections::{HashMap, HashSet};

use crate::{light::Lightmap, map::Map, texture::TextureImage};

pub const WHITE: usize = 0;
pub const LIGHTMAPS: usize = 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    World,
    SkyMask,
    Modulated,
    #[cfg(windows)]
    ModulatedScreen,
    ScreenDepth,
    Screen,
    Ui,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Material {
    pub texture: usize,
    pub mode: Mode,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub light_uv: [f32; 2],
    pub bounds: [f32; 4],
    pub color: [f32; 4],
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

struct Atlas {
    width: usize,
    height: usize,
    entries: HashMap<usize, [usize; 2]>,
}

#[derive(Clone, Copy)]
pub enum Layer {
    Sky,
    World,
    Effects,
    Weapon,
    Hud,
}

#[derive(Default)]
pub struct Pass {
    pub geometry: HashMap<Material, Vec<Vertex>>,
    pub ordered: Vec<(Material, Vec<Vertex>)>,
    used: usize,
}

impl Pass {
    pub fn batches(&self) -> impl Iterator<Item = (Material, &[Vertex])> {
        self.geometry
            .iter()
            .filter(|(m, _)| m.mode == Mode::SkyMask)
            .chain(
                self.geometry
                    .iter()
                    .filter(|(m, _)| m.mode != Mode::SkyMask),
            )
            .map(|(material, vertices)| (*material, vertices.as_slice()))
            .filter(|(_, vertices)| !vertices.is_empty())
            .chain(
                self.ordered[..self.used]
                    .iter()
                    .map(|(material, vertices)| (*material, vertices.as_slice())),
            )
    }

    fn clear(&mut self) {
        for batch in self.geometry.values_mut() {
            batch.clear();
        }
        self.used = 0;
    }

    fn push(&mut self, material: Material, vertices: &[Vertex]) {
        if matches!(material.mode, Mode::World | Mode::SkyMask) {
            self.geometry
                .entry(material)
                .or_default()
                .extend_from_slice(vertices);
        } else if self.used > 0 && self.ordered[self.used - 1].0 == material {
            self.ordered[self.used - 1].1.extend_from_slice(vertices);
        } else {
            if self.used == self.ordered.len() {
                self.ordered.push((material, Vec::new()));
            }
            let batch = &mut self.ordered[self.used];
            batch.0 = material;
            batch.1.clear();
            batch.1.extend_from_slice(vertices);
            self.used += 1;
        }
    }
}

pub struct DrawList {
    pub passes: [Pass; 5],
    pub uploads: Vec<(usize, Image, bool)>,
    seen: HashSet<usize>,
    atlas: Atlas,
    pub(super) layer: Layer,
}

impl DrawList {
    pub(super) fn new(map: &Map) -> Self {
        let (atlas, image) = lightmaps(map);
        Self {
            passes: Default::default(),
            uploads: vec![
                (
                    WHITE,
                    Image {
                        width: 1,
                        height: 1,
                        pixels: vec![u32::MAX],
                    },
                    false,
                ),
                (LIGHTMAPS, image, false),
            ],
            seen: HashSet::new(),
            atlas,
            layer: Layer::World,
        }
    }

    pub(super) fn clear(&mut self) {
        for pass in &mut self.passes {
            pass.clear();
        }
    }

    pub(super) fn texture(&mut self, image: Option<&TextureImage>) -> (usize, [f32; 2]) {
        let Some(image) = image else {
            return (WHITE, [1.0; 2]);
        };
        let id = image as *const TextureImage as usize;
        let (w, h) = image.size();
        if self.seen.insert(id) {
            self.uploads.push((
                id,
                Image {
                    width: w as u32,
                    height: h as u32,
                    pixels: image.bgra(),
                },
                true,
            ));
        }
        (id, [w, h])
    }

    pub(super) fn push(&mut self, material: Material, vertices: &[Vertex]) {
        self.passes[self.layer as usize].push(material, vertices);
    }

    pub(super) fn lighting(&self, lightmap: &Lightmap) -> Option<([f32; 4], [f32; 4])> {
        let [x, y] = *self
            .atlas
            .entries
            .get(&(lightmap as *const Lightmap as usize))?;
        let (w, h, pan, scale, _) = lightmap.gpu_data();
        let (aw, ah) = (self.atlas.width as f32, self.atlas.height as f32);
        let first = [(x as f32 + 0.5) / aw, (y as f32 + 0.5) / ah];
        let last = [
            first[0] + (w - 1) as f32 / aw,
            first[1] + (h - 1) as f32 / ah,
        ];
        let step = [(scale[0] * aw).recip(), (scale[1] * ah).recip()];
        Some((
            [
                step[0],
                step[1],
                first[0] - pan[0] * step[0],
                first[1] - pan[1] * step[1],
            ],
            [first[0], first[1], last[0], last[1]],
        ))
    }
}

fn lightmaps(map: &Map) -> (Atlas, Image) {
    let mut maps = HashMap::new();
    for face in map
        .faces
        .iter()
        .chain(map.movers.iter().flat_map(|mover| &mover.faces))
    {
        if let Some(lightmap) = face.lightmap.as_deref() {
            maps.insert(lightmap as *const Lightmap as usize, lightmap);
        }
    }
    let mut maps: Vec<_> = maps.into_iter().collect();
    maps.sort_unstable_by_key(|(_, map)| std::cmp::Reverse(map.gpu_data().1));
    let width = 2048;
    let mut x = 0;
    let mut y = 0;
    let mut shelf = 0;
    let mut entries = HashMap::new();
    for &(id, map) in &maps {
        let (w, h, ..) = map.gpu_data();
        if w > width || h > 8192 {
            continue;
        }
        if x + w > width {
            y += shelf;
            x = 0;
            shelf = 0;
        }
        if y + h > 8192 {
            break;
        }
        entries.insert(id, [x, y]);
        x += w;
        shelf = shelf.max(h);
    }
    let height = (y + shelf).clamp(1, 8192);
    let mut pixels = vec![0; width * height];
    for (id, map) in maps {
        let Some(&[x, y]) = entries.get(&id) else {
            continue;
        };
        let (w, h, _, _, colors) = map.gpu_data();
        for row in 0..h {
            let target = (y + row) * width + x;
            for (pixel, color) in pixels[target..target + w]
                .iter_mut()
                .zip(&colors[row * w..][..w])
            {
                let [r, g, b] = color.map(|c| (c * (255.0 / 4.0)).round().clamp(0.0, 255.0) as u32);
                *pixel = (r << 16) | (g << 8) | b;
            }
        }
    }
    (
        Atlas {
            width,
            height,
            entries,
        },
        Image {
            width: width as u32,
            height: height as u32,
            pixels,
        },
    )
}
