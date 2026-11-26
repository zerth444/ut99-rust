use std::{fs, io, path::Path};

use crate::{
    package::{Package, invalid},
    render::Frame,
    texture::Texture,
};

pub struct MenuFont {
    pages: Vec<Page>,
}

pub const FONT_SIZES: [u32; 9] = [10, 12, 15, 17, 20, 22, 25, 27, 30];

struct Page {
    texture: Texture,
    glyphs: Vec<[u32; 4]>,
}

impl MenuFont {
    pub fn load_sizes(path: &Path) -> [Option<Self>; FONT_SIZES.len()] {
        let Ok(bytes) = fs::read(path) else {
            return Default::default();
        };
        let Ok(package) = Package::read(&bytes) else {
            return Default::default();
        };
        FONT_SIZES.map(|size| Self::load(&package, &format!("Tahoma{size}")).ok())
    }

    fn load(package: &Package<'_>, name: &str) -> io::Result<Self> {
        let font = package
            .exports
            .iter()
            .find(|font| {
                package.names[font.name] == name && package.object_name(font.class) == Some("Font")
            })
            .ok_or_else(|| invalid("Tahoma font is missing"))?;
        let mut data = crate::map::native_data(package, font)?;
        let mut pages = Vec::new();
        for _ in 0..data.count(2)? {
            let texture = data
                .index()?
                .checked_sub(1)
                .and_then(|index| package.exports.get(index))
                .ok_or_else(|| invalid("Font texture is missing"))?;
            let texture = Texture::read(package, texture)?;
            let mut glyphs = Vec::new();
            for _ in 0..data.count(16)? {
                glyphs.push([data.u32()?, data.u32()?, data.u32()?, data.u32()?]);
            }
            pages.push(Page { texture, glyphs });
        }
        Ok(Self { pages })
    }

    fn glyphs<'a>(&'a self, label: &'a str) -> impl Iterator<Item = (&'a Texture, [u32; 4])> + 'a {
        label.chars().filter_map(|ch| {
            let ch = if ch as u32 <= 255 {
                ch as u32
            } else {
                u32::from(b'?')
            };
            let mut index = ch as usize;
            for page in &self.pages {
                if let Some(&rect) = page.glyphs.get(index) {
                    return Some((&page.texture, rect));
                }
                index -= page.glyphs.len();
            }
            None
        })
    }

    pub fn draw(&self, frame: &mut Frame, x: f32, y: f32, scale: f32, label: &str) {
        self.draw_glyphs(frame, x, y, scale, label, None);
    }

    pub fn draw_glyphs(
        &self,
        frame: &mut Frame,
        mut x: f32,
        y: f32,
        scale: f32,
        label: &str,
        tint: Option<[f32; 3]>,
    ) {
        for (texture, [u, v, w, h]) in self.glyphs(label) {
            if w != 0 && h != 0 {
                let rect = [
                    x.round() as i32,
                    y.round() as i32,
                    (x + w as f32 * scale).round() as i32 - x.round() as i32,
                    (y + h as f32 * scale).round() as i32 - y.round() as i32,
                ];
                let source = [u as f32, v as f32, w as f32, h as f32];
                if let Some(tint) = tint {
                    frame.translucent_tile(texture.image(0.0), rect, source, tint);
                } else {
                    frame.ink(texture.image(0.0), rect, source);
                }
            }
            x += (w.max(3) as f32 + 1.0) * scale;
        }
    }
}
