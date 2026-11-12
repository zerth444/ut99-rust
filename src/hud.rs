use std::ptr::null_mut;
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

use crate::{game::Practice, render::Frame, texture::TextureImage};

const CELL_W: usize = 12;
const CELL_H: usize = 20;
const COLUMNS: usize = 16;
const ROWS: usize = 6;

pub struct Hud {
    glyphs: Option<TextureImage>,
}

impl Hud {
    pub fn new() -> Self {
        Self {
            glyphs: glyph_atlas(),
        }
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        game: &Practice,
        captured: bool,
        has_art: bool,
        music: Option<&str>,
        stats: Option<&str>,
    ) {
        let scale = (frame.width as f32 / 1280.0).clamp(0.7, 2.0);
        let width = frame.width as i32;
        let height = frame.height as i32;
        if !has_art {
            let ammo = game
                .ammo()
                .map_or_else(|| "MELEE".to_string(), |ammo| ammo.to_string());
            self.text(
                frame,
                12,
                height - 30,
                scale,
                &format!(
                    "HEALTH {}  ARMOR {}  {}  {}",
                    game.health,
                    game.armor(),
                    game.weapon.label(),
                    ammo
                ),
                [0.8, 0.9, 1.0],
            );
        }
        if game.time < game.message_until {
            self.centered(
                frame,
                (50.0 * scale) as i32,
                scale,
                &game.message,
                [1.0, 0.79, 0.32],
            );
        }
        if game.scoped() {
            let magnification = 90.0 / game.field_of_view();
            self.text(
                frame,
                width / 2 + width / 10,
                height / 2 + (height * 3 / 20),
                scale,
                &format!("X{magnification:.1}"),
                [0.20, 1.0, 0.20],
            );
        }
        if !captured && game.death.is_none() {
            let w = (350.0 * scale).round() as i32;
            let h = (54.0 * scale).round() as i32;
            frame.solid(
                [(width - w) / 2, (height - h) / 2, w, h],
                [0.07, 0.09, 0.12],
            );
            self.centered(
                frame,
                (height - h) / 2 + (7.0 * scale) as i32,
                scale,
                "CLICK TO CAPTURE MOUSE",
                [0.86, 0.94, 0.98],
            );
            self.centered(
                frame,
                height / 2,
                scale,
                "WASD / SPACE / LMB / RMB",
                [0.60, 0.77, 0.84],
            );
        }
        let mut y = (10.0 * scale) as i32;
        if let Some(stats) = stats {
            self.text(frame, 12, y, scale, stats, [0.72, 0.95, 0.78]);
            y += (CELL_H as f32 * scale) as i32;
        }
        if let Some(track) = music {
            self.text(
                frame,
                12,
                y,
                scale,
                &format!("NOW PLAYING  {track}"),
                [0.77, 0.88, 0.93],
            );
        }
    }

    fn centered(&self, frame: &mut Frame, y: i32, scale: f32, text: &str, color: [f32; 3]) {
        let width = text.chars().count() as f32 * CELL_W as f32 * scale;
        self.text(
            frame,
            (frame.width as f32 * 0.5 - width * 0.5).round() as i32,
            y,
            scale,
            text,
            color,
        );
    }

    fn text(&self, frame: &mut Frame, x: i32, y: i32, scale: f32, text: &str, color: [f32; 3]) {
        let Some(glyphs) = &self.glyphs else {
            return;
        };
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let index = if ch.is_ascii_graphic() {
                ch as usize - 32
            } else {
                usize::from(b'?' - 32)
            };
            let dx = (i as f32 * CELL_W as f32 * scale).round() as i32;
            frame.translucent_tile(
                glyphs,
                [
                    x + dx,
                    y,
                    (CELL_W as f32 * scale).round() as i32,
                    (CELL_H as f32 * scale).round() as i32,
                ],
                [
                    (index % COLUMNS * CELL_W) as f32,
                    (index / COLUMNS * CELL_H) as f32,
                    CELL_W as f32,
                    CELL_H as f32,
                ],
                color,
            );
        }
    }
}

fn glyph_atlas() -> Option<TextureImage> {
    let (width, height) = (CELL_W * COLUMNS, CELL_H * ROWS);
    let bitmap = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits = null_mut();
        let dib = CreateDIBSection(dc, &bitmap, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous_bitmap = SelectObject(dc, dib);
        let face: Vec<_> = "Consolas\0".encode_utf16().collect();
        let font = CreateFontW(
            -18,
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_DEFAULT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            ANTIALIASED_QUALITY.into(),
            (FF_MODERN | FIXED_PITCH).into(),
            face.as_ptr(),
        );
        let previous_font = SelectObject(dc, font);
        std::ptr::write_bytes(bits.cast::<u32>(), 0, width * height);
        SetTextColor(dc, 0xffffff);
        SetBkMode(dc, TRANSPARENT as i32);
        for (index, character) in (32u16..=126).enumerate() {
            let x = (index % COLUMNS * CELL_W) as i32;
            let y = (index / COLUMNS * CELL_H) as i32;
            let cell = RECT {
                left: x,
                top: y,
                right: x + CELL_W as i32,
                bottom: y + CELL_H as i32,
            };
            ExtTextOutW(
                dc,
                x,
                y,
                ETO_CLIPPED,
                &cell,
                &character,
                1,
                std::ptr::null(),
            );
        }
        GdiFlush();
        let pixels = std::slice::from_raw_parts(bits.cast::<u32>(), width * height)
            .iter()
            .map(|pixel| (pixel & 255) as u8)
            .collect();
        SelectObject(dc, previous_font);
        SelectObject(dc, previous_bitmap);
        DeleteObject(font);
        DeleteObject(dib);
        DeleteDC(dc);
        Some(TextureImage::glyph_mask(width, height, pixels))
    }
}
