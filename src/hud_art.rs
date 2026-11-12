use std::{collections::HashMap, sync::Arc};

use crate::game::{ArmorKind, Practice, Weapon, WeaponAssets};
use crate::render::Frame;
use crate::texture::{Texture, TextureImage};

const BLUE: [f32; 3] = [0.63, 0.79, 1.0];
const WHITE: [f32; 3] = [1.0; 3];
const GOLD: [f32; 3] = [1.0, 0.78, 0.28];

#[derive(Default)]
pub struct Art {
    pub elements: Option<Arc<Texture>>,
    pub weapons: Option<Arc<Texture>>,
    pub crosshair: Option<Arc<Texture>>,
    pub doll: Option<Arc<Texture>>,
    pub belt: Option<Arc<Texture>>,
}

impl Art {
    pub fn ready(&self) -> bool {
        self.elements.is_some() && self.weapons.is_some()
    }

    pub fn draw(
        &self,
        frame: &mut Frame,
        game: &Practice,
        arsenal: &HashMap<Weapon, WeaponAssets>,
    ) {
        let width = frame.width as f32;
        let height = frame.height as f32;
        let scale = (width / 1280.0).min(height / 480.0);
        if let (Some(elements), Some(weapons)) = (&self.elements, &self.weapons) {
            self.status(frame, game, elements.image(0.0), width, scale);
            let weapon_scale = 0.8 * scale;
            let row_width = 1024.0 * scale;
            let ammo_y = height
                - if row_width <= width - 255.0 * scale {
                    63.5
                } else {
                    127.5
                } * scale;
            let ammo_x = width - 128.0 * scale;
            let elements = elements.image(0.0);
            art(
                frame,
                elements,
                [ammo_x, ammo_y, 128.0 * scale, 64.0 * scale],
                [128.0, 192.0, 128.0, 64.0],
                BLUE,
            );
            if let Some(ammo) = game.ammo() {
                number(
                    frame,
                    elements,
                    ammo,
                    ammo_x + 4.0 * scale,
                    ammo_y + 16.0 * scale,
                    scale,
                    WHITE,
                );
            }
            weapon_bar(
                frame,
                game,
                arsenal,
                elements,
                weapons.image(0.0),
                row_width,
                weapon_scale,
            );
        }

        let crosshair = if game.rocket_locked {
            arsenal
                .get(&Weapon::Rocket)
                .and_then(|weapon| weapon.lock_reticle.as_ref())
                .or(self.crosshair.as_ref())
        } else {
            self.crosshair.as_ref()
        };
        if game.death.is_none()
            && !game.scoped()
            && let Some(crosshair) = crosshair
        {
            let (w, h) = crosshair.size();
            let crosshair_scale = if width < 512.0 {
                0.5
            } else {
                (0.1 + width / 640.0).floor().clamp(1.0, 2.0)
            };
            let (dw, dh) = (w * crosshair_scale, h * crosshair_scale);
            art(
                frame,
                crosshair.image(0.0),
                [(width - dw) * 0.5, (height - dh) * 0.5, dw, dh],
                [0.0, 0.0, w, h],
                [0.5, 1.0, 0.5],
            );
        }
    }

    fn status(
        &self,
        frame: &mut Frame,
        game: &Practice,
        elements: &TextureImage,
        width: f32,
        scale: f32,
    ) {
        let doll_x = width - 128.0 * scale;
        if let Some(doll) = &self.doll {
            let doll = doll.image(0.0);
            art(
                frame,
                doll,
                [doll_x, 0.0, 128.0 * scale, 256.0 * scale],
                [0.0, 0.0, 128.0, 256.0],
                BLUE,
            );
            for (y, amount) in [
                (0.0, f32::from(game.armor_charge(ArmorKind::Chest)) / 100.0),
                (64.0, f32::from(game.armor_charge(ArmorKind::Thigh)) / 50.0),
                (128.0, f32::from(game.jump_boost > game.time)),
            ] {
                if amount > 0.0 {
                    art(
                        frame,
                        doll,
                        [doll_x, y * scale, 128.0 * scale, 64.0 * scale],
                        [128.0, y, 128.0, 64.0],
                        BLUE.map(|channel| channel * amount),
                    );
                }
            }
        }
        if game.armor_charge(ArmorKind::Belt) > 0
            && let Some(belt) = &self.belt
        {
            let (w, h) = belt.size();
            art(
                frame,
                belt.image(0.0),
                [doll_x, 0.0, w * scale, h * scale],
                [0.0, 0.0, w, h],
                GOLD,
            );
        }
        let x = doll_x - 140.0 * scale;
        let health_y = 64.0 * scale;
        let health_tint = if game.health < 50 {
            let pulse = (game.time * 8.0).sin() * 0.20 + 0.8;
            [1.0, pulse, pulse]
        } else {
            WHITE
        };
        art(
            frame,
            elements,
            [x, 0.0, 128.0 * scale, 64.0 * scale],
            [0.0, 192.0, 128.0, 64.0],
            BLUE,
        );
        art(
            frame,
            elements,
            [x, health_y, 128.0 * scale, 64.0 * scale],
            [128.0, 128.0, 128.0, 64.0],
            BLUE,
        );
        number(
            frame,
            elements,
            game.armor(),
            x + 4.0 * scale,
            16.0 * scale,
            scale,
            WHITE,
        );
        number(
            frame,
            elements,
            game.health,
            x + 4.0 * scale,
            health_y + 16.0 * scale,
            scale,
            health_tint,
        );
    }
}

fn weapon_bar(
    frame: &mut Frame,
    game: &Practice,
    arsenal: &HashMap<Weapon, WeaponAssets>,
    elements: &TextureImage,
    atlas: &TextureImage,
    row_width: f32,
    scale: f32,
) {
    let base = (frame.width as f32 - row_width) * 0.5;
    let y = frame.height as f32 - 63.5 * scale;
    let current = game.weapon;
    let pending = game.pending_weapon();
    let mut slots = [None; 10];
    for weapon in game.owned() {
        let slot = &mut slots[slot_index(weapon)];
        if slot.is_none() || weapon == current || (weapon == pending && *slot != Some(current)) {
            *slot = Some(weapon);
        }
    }
    if pending != current {
        let x = base + slot_index(pending) as f32 * row_width * 0.1;
        art(
            frame,
            atlas,
            [
                x - 64.0 * scale,
                frame.height as f32 - 96.0 * scale,
                256.0 * scale,
                128.0 * scale,
            ],
            [0.0, 128.0, 256.0, 128.0],
            GOLD,
        );
    }
    for (index, weapon) in slots.into_iter().enumerate() {
        let x = base + index as f32 * row_width * 0.1;
        let active = weapon == Some(current);
        let tint = if active || weapon == Some(pending) {
            BLUE
        } else {
            BLUE.map(|c| c * 0.5)
        };
        let icon = weapon.and_then(|weapon| arsenal.get(&weapon)?.status_icon.as_deref());
        if (active || index == 9)
            && let Some(icon) = icon
        {
            let (w, h) = icon.size();
            frame.tile(
                icon.image(0.0),
                rect([x, y, w * scale, h * scale]),
                [0.0, 0.0, w, h],
                tint,
            );
        } else {
            art(
                frame,
                atlas,
                [x, y, 128.0 * scale, 64.0 * scale],
                [
                    (index % 4) as f32 * 64.0,
                    (index / 4) as f32 * 32.0,
                    64.0,
                    32.0,
                ],
                tint,
            );
        }
        let Some(weapon) = weapon else {
            continue;
        };
        if active {
            art(
                frame,
                atlas,
                [x, y, 128.0 * scale, 64.0 * scale],
                [128.0, 64.0, 128.0, 64.0],
                GOLD,
            );
        }
        let digit = (index + 1) % 10;
        art(
            frame,
            elements,
            [
                x + 4.0 * scale,
                y + 4.0 * scale,
                18.75 * scale,
                48.0 * scale,
            ],
            [digit as f32 * 25.0, 0.0, 25.0, 64.0],
            GOLD,
        );
        if let Some(fraction) = game.ammo_fraction(weapon, arsenal) {
            art(
                frame,
                elements,
                [
                    x + 4.0 * scale,
                    y + 52.0 * scale,
                    88.0 * scale * fraction.clamp(0.0, 1.0),
                    8.0 * scale,
                ],
                [64.0, 64.0, 128.0, 8.0],
                WHITE,
            );
        }
    }
}

fn number(
    frame: &mut Frame,
    texture: &TextureImage,
    value: u16,
    x: f32,
    y: f32,
    scale: f32,
    tint: [f32; 3],
) {
    let digits = value.min(9999).to_string();
    let step = 16.0 * scale;
    let mut x = x + (3 - digits.len() as i32) as f32 * step;
    for digit in digits.bytes() {
        x -= step * if digit == b'1' { 0.625 } else { 0.25 };
        art(
            frame,
            texture,
            [x, y, 25.0 * scale, 64.0 * scale],
            [f32::from(digit - b'0') * 25.0, 0.0, 25.0, 64.0],
            tint,
        );
        x += 32.0 * scale;
    }
}

fn art(
    frame: &mut Frame,
    texture: &TextureImage,
    dest: [f32; 4],
    source: [f32; 4],
    tint: [f32; 3],
) {
    frame.translucent_tile(texture, rect(dest), source, tint);
}

fn rect([x, y, w, h]: [f32; 4]) -> [i32; 4] {
    let left = x.round() as i32;
    let top = y.round() as i32;
    [
        left,
        top,
        (x + w).round() as i32 - left,
        (y + h).round() as i32 - top,
    ]
}

fn slot_index(weapon: Weapon) -> usize {
    match weapon.slot() {
        0 => 9,
        slot => usize::from(slot - 1),
    }
}
