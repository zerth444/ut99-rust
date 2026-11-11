use std::sync::Arc;

use crate::game::Practice;
use crate::render::{Frame, HEIGHT, WIDTH};
use crate::texture::Texture;

#[derive(Default)]
pub struct Art {
    pub elements: Option<Arc<Texture>>,
    pub weapons: Option<Arc<Texture>>,
    pub crosshair: Option<Arc<Texture>>,
    pub doll: Option<Arc<Texture>>,
}

impl Art {
    pub fn ready(&self) -> bool {
        self.elements.is_some() && self.weapons.is_some()
    }

    pub fn draw(&self, frame: &mut Frame, game: &Practice) {
        let (Some(elements), Some(weapons)) = (&self.elements, &self.weapons) else {
            return;
        };
        let elements = elements.image(0.0);
        let weapons = weapons.image(0.0);
        let scale = WIDTH as f32 / 1280.0;
        let panel = (128.0 * scale).round() as i32;
        let row = (64.0 * scale).round() as i32;
        let right = WIDTH as i32;
        let bottom = HEIGHT as i32;
        let color = [0.69, 0.78, 1.0];
        let digits = |frame: &mut Frame, value: u16, x: i32, y: i32| {
            let step = (16.0 * scale).round();
            let digits = value.min(9999).to_string();
            let mut cursor = x as f32 + (3 - digits.len() as i32) as f32 * step;
            for digit in digits.bytes() {
                cursor -= step * if digit == b'1' { 0.625 } else { 0.25 };
                frame.tile(
                    elements,
                    [cursor.round() as i32, y, (25.0 * scale).round() as i32, row],
                    [f32::from(digit - b'0') * 25.0, 0.0, 25.0, 64.0],
                    [1.0; 3],
                );
                cursor += 32.0 * scale;
            }
        };
        if let Some(doll) = &self.doll {
            frame.tile(
                doll.image(0.0),
                [right - panel, 0, panel, row * 4],
                [0.0, 0.0, 128.0, 256.0],
                color,
            );
        }
        let status = right - panel - (140.0 * scale).round() as i32;
        frame.tile(
            elements,
            [status, 0, panel, row],
            [0.0, 192.0, 128.0, 64.0],
            color,
        );
        frame.tile(
            elements,
            [status, row, panel, row],
            [128.0, 128.0, 128.0, 64.0],
            color,
        );
        digits(frame, game.armor, status + 2, row / 4);
        digits(frame, game.health, status + 2, row + row / 4);

        let ammo = right - panel;
        let ammo_y = bottom - row * 2;
        frame.tile(
            elements,
            [ammo, ammo_y, panel, row],
            [128.0, 192.0, 128.0, 64.0],
            color,
        );
        if let Some(count) = game.ammo() {
            digits(frame, count, ammo + 2, ammo_y + row / 4);
        }

        let scale = scale * 0.8;
        let slot_width = (128.0 * scale).round() as i32;
        let slot_height = (64.0 * scale).round() as i32;
        let slot_offset = (102.0 * scale).round() as i32;
        let base = (right - slot_offset * 9 - slot_width) / 2;
        let y = bottom - slot_height;
        let mut slots = [false; 10];
        let active = slot_index(game.weapon.slot());
        for weapon in game.owned() {
            slots[slot_index(weapon.slot())] = true;
        }
        for (index, &owned) in slots.iter().enumerate() {
            let tint = if active == index {
                [1.0, 0.82, 0.4]
            } else if owned {
                color
            } else {
                [0.29, 0.35, 0.48]
            };
            let x = base + index as i32 * slot_offset;
            frame.tile(
                weapons,
                [x, y, slot_width, slot_height],
                [
                    (index % 4) as f32 * 64.0,
                    (index / 4) as f32 * 32.0,
                    64.0,
                    32.0,
                ],
                tint,
            );
            if owned {
                let digit = (index + 1) % 10;
                frame.tile(
                    elements,
                    [
                        x + 3,
                        y + 1,
                        (19.0 * scale).round() as i32,
                        (48.0 * scale).round() as i32,
                    ],
                    [digit as f32 * 25.0, 0.0, 25.0, 64.0],
                    [1.0, 0.82, 0.4],
                );
            }
        }

        if let Some(crosshair) = &self.crosshair {
            let (w, h) = crosshair.size();
            frame.tile(
                crosshair.image(0.0),
                [
                    (right - w as i32) / 2,
                    (bottom - h as i32) / 2,
                    w as i32,
                    h as i32,
                ],
                [0.0, 0.0, w, h],
                [0.5, 1.0, 0.5],
            );
        }
    }
}

fn slot_index(slot: u8) -> usize {
    if slot == 0 { 9 } else { (slot - 1) as usize }
}
