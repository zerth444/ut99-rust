use std::{fs, io, path::Path};

use crate::{
    package::{Package, invalid},
    render::Frame,
    texture::Texture,
};

pub struct MenuFont {
    pages: Vec<Page>,
}

struct Page {
    texture: Texture,
    glyphs: Vec<[u32; 4]>,
}

impl MenuFont {
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let package = Package::read(&bytes)?;
        let font = package
            .exports
            .iter()
            .find(|font| {
                package.names[font.name] == "Tahoma12"
                    && package.object_name(font.class) == Some("Font")
            })
            .ok_or_else(|| invalid("Tahoma12 font is missing"))?;
        let mut data = crate::map::native_data(&package, font)?;
        let mut pages = Vec::new();
        for _ in 0..data.count(2)? {
            let texture = data
                .index()?
                .checked_sub(1)
                .and_then(|index| package.exports.get(index))
                .ok_or_else(|| invalid("Font texture is missing"))?;
            let texture = Texture::read(&package, texture)?;
            let mut glyphs = Vec::new();
            for _ in 0..data.count(16)? {
                glyphs.push([data.u32()?, data.u32()?, data.u32()?, data.u32()?]);
            }
            pages.push(Page { texture, glyphs });
        }
        Ok(Self { pages })
    }

    pub fn draw(&self, frame: &mut Frame, mut x: f32, y: f32, scale: f32, label: &str) {
        for ch in label.bytes() {
            let mut index = usize::from(ch);
            for page in &self.pages {
                if let Some(&[u, v, w, h]) = page.glyphs.get(index) {
                    if w != 0 && h != 0 {
                        frame.tile(
                            page.texture.image(0.0),
                            [
                                x.round() as i32,
                                y.round() as i32,
                                (w as f32 * scale).round() as i32,
                                (h as f32 * scale).round() as i32,
                            ],
                            [u as f32, v as f32, w as f32, h as f32],
                            [0.03; 3],
                        );
                    }
                    x += (w.max(3) as f32 + 1.0) * scale;
                    break;
                }
                index -= page.glyphs.len();
            }
        }
    }
}
