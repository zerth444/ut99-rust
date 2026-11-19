use std::{collections::HashMap, fs, io, path::Path, sync::Arc};

use crate::package::{Export, Package, invalid};

pub struct Texture {
    pub poly_flags: u32,
    frames: Vec<TextureImage>,
    rate: f32,
}

#[derive(Clone)]
pub struct TextureImage {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    palette: Vec<[f32; 3]>,
    opaque: bool,
}

impl Texture {
    pub(crate) fn read(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        if package.object_name(export.class) == Some("FireTexture") {
            return Self::read_fire(package, export);
        }
        let mut frames = Vec::new();
        let mut frame = export;
        let mut seen = std::collections::HashSet::new();
        let mut rate = 0.0;
        let mut poly_flags = 0;
        loop {
            if !seen.insert(frame.data.start) {
                break;
            }
            let image = match TextureImage::read(package, frame) {
                Ok(image) => image,
                Err(error) if frames.is_empty() => return Err(error),
                Err(_) => break,
            };
            let mut props = package.properties(frame)?;
            let mut next = 0;
            while let Some((name, mut value)) = props.next()? {
                match name {
                    "AnimNext" => next = value.compact()?,
                    "MaxFrameRate" if frames.is_empty() => rate = value.f32()?,
                    "PolyFlags" if frames.is_empty() => poly_flags = value.u32()?,
                    _ => {}
                }
            }
            frames.push(image);
            let Some(next) = usize::try_from(next)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| package.exports.get(i))
                .filter(|item| package.object_name(item.class) == Some("Texture"))
            else {
                break;
            };
            frame = next;
        }
        Ok(Self {
            poly_flags,
            frames,
            rate: if rate > 0.0 { rate } else { 10.0 },
        })
    }

    fn read_fire(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        let mut width = 0;
        let mut height = 0;
        let mut palette = 0;
        let mut heat = 230;
        let mut rising = false;
        let mut rate = 25.0_f32;
        let mut poly_flags = 0;
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "UClamp" => width = value.u32()? as usize,
                "VClamp" => height = value.u32()? as usize,
                "Palette" => palette = value.index()?,
                "RenderHeat" => heat = value.byte()?,
                "bRising" => rising = prop.boolean.unwrap_or(false),
                "MaxFrameRate" => rate = value.f32()?,
                "PolyFlags" => poly_flags = value.u32()?,
                _ => {}
            }
        }
        if !(1..=512).contains(&width) || !(1..=512).contains(&height) {
            return Err(invalid("Invalid fire texture dimensions"));
        }
        let palette = read_palette(package, palette)?;
        let mut reader = props.reader;
        let mipmaps = reader.byte()?;
        for _ in 0..mipmaps {
            if package.version >= 63 {
                reader.u32()?;
            }
            let size = reader.count(1)?;
            reader.take(size)?;
            reader.take(10)?; // Width, height and bit counts.
        }
        let count = reader.count(8)?;
        let sparks = reader.take(count * 8)?.as_chunks::<8>().0;
        let mut field = vec![0u8; width * height];
        let mut next = field.clone();
        let mut frames = Vec::with_capacity(32);
        let mut random = 0x5f4a_7c13_u32;
        let decay = u16::from(255 - heat) / 32;
        for tick in 0..64 {
            for &[kind, energy, x, y, a, b, _, speed] in sparks {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                let n = (random >> 24) as u8;
                let x = (usize::from(x) + usize::from(n) * usize::from(a) / 256) % width;
                let y = (usize::from(y) + usize::from(n.reverse_bits()) * usize::from(b) / 256)
                    % height;
                let energy = match kind {
                    0 => n,
                    2 => energy.wrapping_add(speed.wrapping_mul(tick)),
                    _ => energy.saturating_sub(n / 8),
                };
                field[y * width + x] = field[y * width + x].max(energy);
            }
            for y in 0..height {
                let row = ((y + usize::from(rising)) % height) * width;
                let below = ((y + 1 + usize::from(rising)) % height) * width;
                for x in 0..width {
                    let left = (x + width - 1) % width;
                    let right = (x + 1) % width;
                    let average = (u16::from(field[row + left])
                        + u16::from(field[row + x])
                        + u16::from(field[row + right])
                        + u16::from(field[below + x]))
                        / 4;
                    next[y * width + x] = average.saturating_sub(decay) as u8;
                }
            }
            std::mem::swap(&mut field, &mut next);
            if tick >= 32 {
                frames.push(TextureImage {
                    width,
                    height,
                    pixels: field.clone(),
                    palette: palette.clone(),
                    opaque: false,
                });
            }
        }
        Ok(Self {
            poly_flags,
            frames,
            rate: if rate > 0.0 { rate } else { 25.0 },
        })
    }

    fn read_wet(
        package: &Package<'_>,
        export: &Export,
        files: &HashMap<String, std::path::PathBuf>,
    ) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        let mut source = 0;
        let mut drops = Vec::new();
        let mut amplitude = 32;
        let mut rate = 20.0;
        let mut poly_flags = 0;
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "SourceTexture" => source = value.compact()?,
                "WaveAmp" => amplitude = value.byte()?,
                "MaxFrameRate" => rate = value.f32()?.max(1.0),
                "PolyFlags" => poly_flags = value.u32()?,
                "Drops" if value.remaining.len() == 8 => {
                    drops.push((value.byte()?, value.byte()?, value.byte()?, value.byte()?));
                }
                _ => {}
            }
        }
        let image = read_source_image(package, source, files)?;
        if drops.is_empty() {
            return Ok(Self {
                poly_flags,
                frames: vec![image],
                rate,
            });
        }

        const FRAMES: usize = 32;
        let strength = (f32::from(amplitude) / 12.0).clamp(1.0, 10.0);
        let width = image.width;
        let height = image.height;
        let mut frames = Vec::with_capacity(FRAMES);
        // Use the authored emitters and source image. The displacement is a
        // periodic ripple approximation, allowing deterministic frame capture.
        let drops: Vec<_> = drops
            .into_iter()
            .map(|(kind, depth, x, y)| {
                (
                    2.0 * f32::from(x) % width as f32,
                    2.0 * f32::from(y) % height as f32,
                    (f32::from(depth) / 255.0).max(0.2),
                    if (8..=15).contains(&kind) { 0.25 } else { 1.0 },
                )
            })
            .collect();
        let mut flow = vec![[0.0_f32; 4]; width * height];
        for y in 0..height {
            for x in 0..width {
                let mut coefficients = [0.0; 4];
                for &(px, py, depth, ratio) in &drops {
                    let dx = wrapped_distance(x as f32 - px, width);
                    let dy = wrapped_distance(y as f32 - py, height) * ratio;
                    let distance = (dx * dx + dy * dy).sqrt();
                    let weight = depth * strength / (1.0 + distance * 0.055);
                    let (sin, cos) = (distance * 0.23).sin_cos();
                    let dir_x = dx / distance.max(1.0);
                    let dir_y = dy / distance.max(1.0);
                    coefficients[0] += dir_x * weight * sin;
                    coefficients[1] += dir_x * weight * cos;
                    coefficients[2] += dir_y * weight * sin;
                    coefficients[3] += dir_y * weight * cos;
                }
                flow[y * width + x] = coefficients;
            }
        }
        for i in 0..FRAMES {
            let (sin, cos) = (i as f32 / FRAMES as f32 * std::f32::consts::TAU).sin_cos();
            let mut pixels = Vec::with_capacity(width * height);
            for (index, [ux, vx, uy, vy]) in flow.iter().copied().enumerate() {
                let x = (index % width) as isize;
                let y = (index / width) as isize;
                let sx = (x + (ux * cos - vx * sin).round() as isize).rem_euclid(width as isize)
                    as usize;
                let sy = (y + (uy * cos - vy * sin).round() as isize).rem_euclid(height as isize)
                    as usize;
                pixels.push(image.pixels[sy * width + sx]);
            }
            frames.push(TextureImage {
                width,
                height,
                pixels,
                palette: image.palette.clone(),
                opaque: image.opaque,
            });
        }
        Ok(Self {
            frames,
            rate,
            poly_flags,
        })
    }

    fn read_source_surface(
        package: &Package<'_>,
        export: &Export,
        files: &HashMap<String, std::path::PathBuf>,
        ice: bool,
    ) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        let mut source = 0;
        let mut glass = 0;
        let mut speed = [128; 2];
        let mut amplitude = 0;
        let mut poly_flags = 0;
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "SourceTexture" => source = value.compact()?,
                "GlassTexture" => glass = value.compact()?,
                "HorizPanSpeed" => speed[0] = value.byte()?,
                "VertPanSpeed" => speed[1] = value.byte()?,
                "Amplitude" => amplitude = value.byte()?,
                "PolyFlags" => poly_flags = value.u32()?,
                _ => {}
            }
        }
        let source = read_source_image(package, source, files)?;
        let mut frames = Vec::new();
        if ice && let Ok(glass) = read_source_image(package, glass, files) {
            for frame in 0..32 {
                let angle = frame as f32 * std::f32::consts::TAU / 32.0;
                let pan = [angle.sin(), angle.cos()].map(|component| component * 0.5);
                let [du, dv] =
                    std::array::from_fn(|axis| pan[axis] * (f32::from(speed[axis]) - 128.0));
                let mut image = source.clone();
                for y in 0..source.height {
                    for x in 0..source.width {
                        let mask = |u: f32, v: f32| glass.sample(u, v, false).unwrap()[0];
                        let (u, v) = (x as f32, y as f32);
                        let scale = f32::from(amplitude) / 255.0;
                        let sx = u + (mask(u + du, v + dv) - mask(u + du + 1.0, v + dv)) * scale;
                        let sy = v + (mask(u + du, v + dv) - mask(u + du, v + dv + 1.0)) * scale;
                        let sx = (sx.round() as isize).rem_euclid(source.width as isize) as usize;
                        let sy = (sy.round() as isize).rem_euclid(source.height as isize) as usize;
                        image.pixels[y * source.width + x] = source.pixels[sy * source.width + sx];
                    }
                }
                frames.push(image);
            }
        }
        if frames.is_empty() {
            frames.push(source);
        }
        Ok(Self {
            frames,
            rate: 20.0,
            poly_flags,
        })
    }

    fn read_wave(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        let (mut width, mut height, mut palette, mut poly_flags) = (0, 0, 0, 0);
        let mut drops = Vec::new();
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "UClamp" => width = value.u32()? as usize,
                "VClamp" => height = value.u32()? as usize,
                "Palette" => palette = value.index()?,
                "PolyFlags" => poly_flags = value.u32()?,
                "Drops" if value.remaining.len() == 8 => {
                    let bytes: [u8; 8] = value.take(8)?.try_into().unwrap();
                    drops.push(bytes);
                }
                _ => {}
            }
        }
        if !(1..=512).contains(&width) || !(1..=512).contains(&height) || drops.is_empty() {
            return Err(invalid("Wave texture needs dimensions and emitters"));
        }
        let palette = read_palette(package, palette)?;
        let mut frames = Vec::with_capacity(32);
        for frame in 0..32 {
            let phase = frame as f32 * std::f32::consts::TAU / 32.0;
            let mut field = vec![106.0f32; width * height];
            for &[_, strength, x, y, offset, _, _, period] in &drops {
                let radius = 12 + i32::from(period) / 24;
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let distance = (dx as f32).hypot(dy as f32);
                        let falloff = 1.0 - distance / radius as f32;
                        if falloff <= 0.0 {
                            continue;
                        }
                        let px = (2 * i32::from(x) + dx).rem_euclid(width as i32) as usize;
                        let py = (2 * i32::from(y) + dy).rem_euclid(height as i32) as usize;
                        field[py * width + px] +=
                            (distance * 0.78 - phase + f32::from(offset) / 40.0).sin()
                                * falloff
                                * f32::from(strength)
                                * 0.065;
                    }
                }
            }
            frames.push(TextureImage {
                width,
                height,
                pixels: field
                    .into_iter()
                    .map(|value| value.clamp(0.0, 255.0) as u8)
                    .collect(),
                palette: palette.clone(),
                opaque: false,
            });
        }
        Ok(Self {
            frames,
            rate: 20.0,
            poly_flags,
        })
    }

    pub fn size(&self) -> (f32, f32) {
        self.frames[0].size()
    }

    #[cfg(windows)]
    pub(crate) fn opaque(mut self) -> Self {
        for frame in &mut self.frames {
            frame.opaque = true;
        }
        self
    }

    pub fn image(&self, time: f32) -> &TextureImage {
        let frame = (time * self.rate).floor() as usize % self.frames.len();
        &self.frames[frame]
    }

    pub fn image_once(&self, progress: f32) -> &TextureImage {
        let frame = (progress * self.frames.len() as f32) as usize;
        &self.frames[frame.min(self.frames.len() - 1)]
    }

    pub(crate) fn animation_duration(&self) -> f32 {
        self.frames.len() as f32 / self.rate
    }
}

fn read_source_image(
    package: &Package<'_>,
    reference: i32,
    files: &HashMap<String, std::path::PathBuf>,
) -> io::Result<TextureImage> {
    if reference > 0 {
        let export = package
            .exports
            .get(reference as usize - 1)
            .ok_or_else(|| invalid("Invalid texture source"))?;
        return TextureImage::read(package, export);
    }
    let (package_name, path) =
        import_path(package, reference).ok_or_else(|| invalid("Missing texture source"))?;
    let bytes = fs::read(
        files
            .get(&package_name)
            .ok_or_else(|| invalid("Missing source texture package"))?,
    )?;
    let package = Package::read(&bytes)?;
    let export = package
        .exports
        .iter()
        .find(|export| {
            package.object_name(export.class) == Some("Texture")
                && export_path(&package, export).as_ref() == Some(&path)
        })
        .ok_or_else(|| invalid("Missing source texture image"))?;
    TextureImage::read(&package, export)
}

fn wrapped_distance(delta: f32, period: usize) -> f32 {
    let period = period as f32;
    (delta + period * 0.5).rem_euclid(period) - period * 0.5
}

impl TextureImage {
    #[cfg(windows)]
    pub(crate) fn glyph_mask(width: usize, height: usize, pixels: Vec<u8>) -> Self {
        let palette = (0..=255).map(|value| [value as f32; 3]).collect();
        Self {
            width,
            height,
            pixels,
            palette,
            opaque: false,
        }
    }
    pub(crate) fn bgra(&self) -> Vec<u32> {
        let palette: Vec<_> = self
            .palette
            .iter()
            .enumerate()
            .map(|(i, color)| {
                let [r, g, b] = color.map(|c| c.clamp(0.0, 255.0) as u32);
                (u32::from(self.opaque || i != 0) * 255 << 24) | (r << 16) | (g << 8) | b
            })
            .collect();
        self.pixels
            .iter()
            .map(|&index| palette[index as usize])
            .collect()
    }
    pub fn size(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }

    pub fn sample(&self, u: f32, v: f32, masked: bool) -> Option<[f32; 3]> {
        let x = (u.floor() as isize).rem_euclid(self.width as isize) as usize;
        let y = (v.floor() as isize).rem_euclid(self.height as isize) as usize;
        let color = self.pixels[y * self.width + x] as usize;
        if masked && !self.opaque && color == 0 {
            None
        } else {
            Some(self.palette[color])
        }
    }

    fn read(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        let mut props = package.properties(export)?;
        let mut palette = None;
        let mut format = 0;
        while let Some((name, mut value)) = props.next()? {
            match name {
                "Palette" => palette = Some(value.index()?),
                "Format" => format = value.byte()?,
                _ => {}
            }
        }
        if format != 0 {
            return Err(invalid("Expected a P8 texture"));
        }
        let palette = read_palette(package, palette.ok_or_else(|| invalid("Missing palette"))?)?;

        let mut reader = props.reader;
        if reader.byte()? == 0 {
            return Err(invalid("Texture has no mipmaps"));
        }
        if package.version >= 63 {
            reader.u32()?;
        }
        let size = reader.count(1)?;
        let pixels = reader.take(size)?;
        let width = reader.u32()? as usize;
        let height = reader.u32()? as usize;
        if width == 0 || height == 0 || width.checked_mul(height) != Some(size) {
            return Err(invalid("Invalid P8 texture dimensions"));
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.to_vec(),
            palette,
            opaque: false,
        })
    }
}

fn read_palette(package: &Package<'_>, id: usize) -> io::Result<Vec<[f32; 3]>> {
    let palette = id
        .checked_sub(1)
        .and_then(|index| package.exports.get(index))
        .filter(|export| package.object_name(export.class) == Some("Palette"))
        .ok_or_else(|| invalid("Texture has no local palette"))?;
    let mut colors = package.properties(palette)?;
    while colors.next()?.is_some() {}
    if colors.reader.count(4)? != 256 {
        return Err(invalid("Expected a 256-color palette"));
    }
    Ok(colors
        .reader
        .take(256 * 4)?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|rgba| [rgba[0] as f32, rgba[1] as f32, rgba[2] as f32])
        .collect())
}

pub fn load_materials(
    map: &Package<'_>,
    path: &Path,
    references: impl Iterator<Item = i32>,
) -> HashMap<i32, Arc<Texture>> {
    let search_paths = path.parent().into_iter().flat_map(|folder| {
        [
            folder.to_path_buf(),
            folder.join("Textures"),
            folder.join("../Textures"),
        ]
    });
    let mut files: HashMap<_, _> = search_paths
        .filter_map(|folder| fs::read_dir(folder).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            extension.eq_ignore_ascii_case("utx").then_some(())?;
            Some((path.file_stem()?.to_str()?.to_ascii_lowercase(), path))
        })
        .collect();
    if let Some(folder) = path.parent()
        && let Ok(entries) = fs::read_dir(folder.join("../System"))
    {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("u"))
                && let Some(name) = path.file_stem().and_then(|name| name.to_str())
            {
                files.entry(name.to_ascii_lowercase()).or_insert(path);
            }
        }
    }

    let read = |package: &Package<'_>, item: &Export| {
        let result = match package.object_name(item.class) {
            Some("Texture" | "FireTexture") => Texture::read(package, item),
            Some("WetTexture") => Texture::read_wet(package, item, &files),
            Some("WaveTexture") => Texture::read_wave(package, item),
            Some("IceTexture") => Texture::read_source_surface(package, item, &files, true),
            Some("ScriptedTexture") => Texture::read_source_surface(package, item, &files, false),
            _ => return None,
        };
        result.ok().map(Arc::new)
    };
    let mut textures = HashMap::new();
    let mut imports = HashMap::<String, HashMap<i32, Vec<String>>>::new();
    for reference in references {
        if reference > 0 {
            if let Some(export) = map.exports.get(reference as usize - 1) {
                textures
                    .entry(reference)
                    .or_insert_with(|| read(map, export));
            }
        } else if let Some((package, path)) = import_path(map, reference) {
            imports.entry(package).or_default().insert(reference, path);
        }
    }

    for (name, materials) in imports {
        let Some(bytes) = files.get(&name).and_then(|file| fs::read(file).ok()) else {
            continue;
        };
        let Ok(package) = Package::read(&bytes) else {
            continue;
        };
        for (reference, path) in materials {
            let matching = package.exports.iter().filter(|export| {
                package
                    .name(export.name)
                    .ok()
                    .zip(path.first())
                    .is_some_and(|(name, leaf)| name.eq_ignore_ascii_case(leaf))
                    && export_path(&package, export).as_ref() == Some(&path)
            });
            if let Some(texture) = matching.filter_map(|item| read(&package, item)).next() {
                textures.insert(reference, Some(texture));
            }
        }
    }
    textures
        .into_iter()
        .filter_map(|(key, texture)| texture.map(|value| (key, value)))
        .collect()
}

pub(crate) fn import_path(
    package: &Package<'_>,
    mut reference: i32,
) -> Option<(String, Vec<String>)> {
    let mut path = Vec::new();
    while reference < 0 && path.len() < package.imports.len() {
        let item = package.imports.get(reference.unsigned_abs() as usize - 1)?;
        let name = package.name(item.name).ok()?.to_ascii_lowercase();
        if item.outer == 0 {
            return Some((name, path));
        }
        path.push(name);
        reference = item.outer;
    }
    None
}

pub(crate) fn export_path<'p>(
    package: &'p Package<'_>,
    mut export: &'p Export,
) -> Option<Vec<String>> {
    let mut path = Vec::new();
    while path.len() < package.exports.len() {
        path.push(package.name(export.name).ok()?.to_ascii_lowercase());
        if export.outer == 0 {
            return Some(path);
        }
        export = package
            .exports
            .get(usize::try_from(export.outer).ok()?.checked_sub(1)?)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Map;

    #[test]
    #[ignore = "requires the local GOTY textures"]
    fn deck16_and_liandri_wet_textures_change_pixels() {
        for name in ["DM-Deck16][.unr", "DM-Liandri.unr"] {
            let path = Path::new("local/goty/Maps").join(name);
            let data = fs::read(&path).unwrap();
            let package = Package::read(&data).unwrap();
            let map = Map::load(&package, Some(&path)).unwrap();
            assert!(
                map.faces
                    .iter()
                    .filter_map(|face| face.texture.as_ref())
                    .any(|texture| {
                        texture.frames.len() == 32
                            && texture.image(0.0).pixels != texture.image(0.4).pixels
                    }),
                "{name}"
            );
        }
    }
}
