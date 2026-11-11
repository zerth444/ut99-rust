use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;

use crate::game::Practice;

pub struct Hud {
    title: HFONT,
    small: HFONT,
    panel: HBRUSH,
}

impl Hud {
    pub fn new() -> Self {
        let face: Vec<_> = "Consolas".encode_utf16().chain([0]).collect();
        let font = |height| unsafe {
            CreateFontW(
                height,
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
                CLEARTYPE_QUALITY.into(),
                (FF_MODERN | FIXED_PITCH).into(),
                face.as_ptr(),
            )
        };
        Self {
            title: font(-25),
            small: font(-16),
            panel: unsafe { CreateSolidBrush(rgb(19, 26, 31)) },
        }
    }

    pub fn draw(
        &self,
        dc: HDC,
        viewport: RECT,
        game: &Practice,
        captured: bool,
        has_art: bool,
        music: Option<&str>,
    ) {
        let width = viewport.right;
        let height = viewport.bottom;
        if width < 450 || height < 180 {
            return;
        }
        let bottom = height - 15;
        let top = bottom - 57;
        let left = 15;
        let right = width - 15;
        unsafe {
            let old_font = SelectObject(dc, self.small);
            let old_mode = SetBkMode(dc, TRANSPARENT as i32);
            if !has_art {
                self.box_at(dc, left, top, left + 208, bottom);
                self.box_at(dc, right - 224, top, right, bottom);
                self.text(dc, left + 11, top + 5, "HEALTH", rgb(161, 199, 202));
                self.text(dc, left + 111, top + 5, "ARMOR", rgb(161, 199, 202));
                self.text(
                    dc,
                    right - 213,
                    top + 5,
                    game.weapon.label(),
                    rgb(161, 199, 202),
                );
                SelectObject(dc, self.title);
                self.text(
                    dc,
                    left + 11,
                    top + 22,
                    &game.health.to_string(),
                    rgb(211, 244, 236),
                );
                self.text(
                    dc,
                    left + 111,
                    top + 22,
                    &game.armor.to_string(),
                    rgb(211, 244, 236),
                );
                self.text(
                    dc,
                    right - 213,
                    top + 22,
                    &game
                        .ammo()
                        .map_or_else(|| "MELEE".into(), |ammo| format!("{ammo} AMMO")),
                    rgb(255, 202, 109),
                );
                SelectObject(dc, self.small);

                if width >= 780 {
                    let slots: Vec<_> = game.owned().map(|weapon| weapon.slot()).collect();
                    let start = width / 2 - slots.len() as i32 * 12;
                    for (i, slot) in slots.into_iter().enumerate() {
                        let x = start + i as i32 * 24;
                        self.box_at(dc, x, bottom - 26, x + 21, bottom);
                        let color = if slot == game.weapon.slot() {
                            rgb(255, 202, 109)
                        } else {
                            rgb(161, 199, 202)
                        };
                        self.text(dc, x + 5, bottom - 22, &slot.to_string(), color);
                    }
                }
            }
            if game.time < game.message_until {
                self.centered(dc, width, 46, &game.message, rgb(255, 202, 109));
            }
            if !captured {
                self.box_at(
                    dc,
                    width / 2 - 175,
                    height / 2 - 26,
                    width / 2 + 175,
                    height / 2 + 38,
                );
                self.centered(
                    dc,
                    width,
                    height / 2 - 17,
                    "CLICK TO CAPTURE MOUSE",
                    rgb(223, 240, 240),
                );
                self.centered(
                    dc,
                    width,
                    height / 2 + 8,
                    "WASD / SPACE / LMB / RMB",
                    rgb(161, 199, 202),
                );
            }
            if let Some(track) = music {
                self.text(
                    dc,
                    left,
                    12,
                    &format!("NOW PLAYING  {track}"),
                    rgb(198, 224, 225),
                );
            }
            SetBkMode(dc, old_mode);
            SelectObject(dc, old_font);
        }
    }

    fn box_at(&self, dc: HDC, left: i32, top: i32, right: i32, bottom: i32) {
        unsafe {
            FillRect(
                dc,
                &RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
                self.panel,
            );
        }
    }

    fn text(&self, dc: HDC, x: i32, y: i32, text: &str, color: u32) {
        let text: Vec<_> = text.encode_utf16().collect();
        unsafe {
            SetTextColor(dc, color);
            TextOutW(dc, x, y, text.as_ptr(), text.len() as i32);
        }
    }

    fn centered(&self, dc: HDC, width: i32, y: i32, text: &str, color: u32) {
        let x = (width - text.chars().count() as i32 * 9) / 2;
        self.text(dc, x, y, text, color);
    }
}

impl Drop for Hud {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.title);
            DeleteObject(self.small);
            DeleteObject(self.panel);
        }
    }
}

fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
