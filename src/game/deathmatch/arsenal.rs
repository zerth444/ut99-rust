use crate::game::{Weapon, WeaponAssets, initial_ammo, max_ammo, ordnance::Kind};
use crate::map::Map;
use std::collections::HashMap;

pub(super) struct Inventory {
    owned: [bool; Weapon::ALL.len()],
    ammo: [u16; Weapon::ALL.len()],
}

impl Inventory {
    pub fn new(map: &Map) -> Self {
        let mut inventory = Self {
            owned: [false; Weapon::ALL.len()],
            ammo: [0; Weapon::ALL.len()],
        };
        inventory.pickup(&map.arsenal, Weapon::Hammer);
        inventory.pickup(&map.arsenal, Weapon::Enforcer);
        inventory
    }

    pub fn owns(&self, weapon: Weapon) -> bool {
        self.owned[weapon as usize]
    }

    pub fn ammo(&self, weapon: Weapon) -> u16 {
        self.ammo[weapon.ammo_pool()]
    }

    pub fn usable(&self, weapon: Weapon) -> bool {
        self.owns(weapon) && (weapon == Weapon::Hammer || self.ammo(weapon) > 0)
    }

    pub fn grant_ammo(
        &mut self,
        art: &HashMap<Weapon, WeaponAssets>,
        weapon: Weapon,
        quantity: u16,
    ) -> bool {
        let ammo = &mut self.ammo[weapon.ammo_pool()];
        let next = ammo.saturating_add(quantity).min(max_ammo(art, weapon));
        if next <= *ammo {
            return false;
        }
        *ammo = next;
        true
    }

    pub fn pickup(&mut self, art: &HashMap<Weapon, WeaponAssets>, weapon: Weapon) -> bool {
        let new = !self.owns(weapon);
        self.owned[weapon as usize] = true;
        let ammo = self.grant_ammo(art, weapon, initial_ammo(art, weapon));
        new || ammo
    }

    pub fn spend(&mut self, weapon: Weapon, quantity: u16) -> bool {
        let ammo = &mut self.ammo[weapon.ammo_pool()];
        let Some(remaining) = ammo.checked_sub(quantity) else {
            return false;
        };
        *ammo = remaining;
        true
    }

    pub fn select(&self, current: Weapon, slot: Option<u8>, cycle: i8) -> Option<Weapon> {
        if slot.is_none() && cycle == 0 {
            return None;
        }
        let mut candidates = Weapon::ALL
            .into_iter()
            .filter(|&weapon| self.owns(weapon) && slot.is_none_or(|slot| weapon.slot() == slot))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        // A second press cycles guns sharing the same number, including slot zero.
        if let Some(index) = candidates.iter().position(|&weapon| weapon == current) {
            let offset = if slot.is_some() { 1 } else { i32::from(cycle) };
            return Some(
                candidates[(index as i32 + offset).rem_euclid(candidates.len() as i32) as usize],
            );
        }
        candidates.pop()
    }

    pub fn preferred(&self, distance: f32) -> Weapon {
        Weapon::ALL
            .into_iter()
            .filter(|&weapon| self.usable(weapon))
            .max_by(|&a, &b| rating(a, distance).total_cmp(&rating(b, distance)))
            .unwrap_or(Weapon::Hammer)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Shot {
    pub damage: u16,
    pub interval: f32,
    pub ammo: u16,
    pub path: Path,
    pub guided: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ChargeProfile {
    pub period: f32,
    pub max_units: u16,
    pub shape: ChargeShape,
}

#[derive(Clone, Copy)]
pub(super) enum ChargeShape {
    Volley,
    Grow,
}

impl ChargeProfile {
    pub fn units(self, elapsed: f32, ammo: u16, cost: u16) -> u16 {
        let available = ammo / cost.max(1);
        let built = 1u16.saturating_add((elapsed.max(0.0) / self.period) as u16);
        built.min(self.max_units).min(available)
    }
}

pub(super) fn charge_profile(weapon: Weapon, alt: bool) -> Option<ChargeProfile> {
    match (weapon, alt) {
        (Weapon::Rocket, _) => Some(ChargeProfile {
            period: 0.35,
            max_units: 4,
            shape: ChargeShape::Volley,
        }),
        (Weapon::Bio, true) => Some(ChargeProfile {
            period: 0.52,
            max_units: 5,
            shape: ChargeShape::Grow,
        }),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) enum Path {
    Trace {
        range: f32,
        rays: u8,
        spread: f32,
    },
    Missile {
        kind: Kind,
        blast: f32,
        gravity: f32,
        rebounds: u8,
    },
    Zoom,
}

pub(super) fn shot(weapon: Weapon, alt: bool) -> Shot {
    let bullet = || trace(0, 0.0, 1, super::ballistics::BulletProfile::RANGE, 1, 0.0);
    fn trace(damage: u16, interval: f32, ammo: u16, range: f32, rays: u8, spread: f32) -> Shot {
        Shot {
            damage,
            interval,
            ammo,
            guided: false,
            path: Path::Trace {
                range,
                rays,
                spread,
            },
        }
    }
    fn missile(
        damage: u16,
        interval: f32,
        ammo: u16,
        kind: Kind,
        blast: f32,
        gravity: f32,
        rebounds: u8,
    ) -> Shot {
        Shot {
            damage,
            interval,
            ammo,
            guided: false,
            path: Path::Missile {
                kind,
                blast,
                gravity,
                rebounds,
            },
        }
    }
    use Kind::*;
    match (weapon, alt) {
        (Weapon::Hammer, false) => trace(44, 0.58, 0, 96.0, 1, 0.0),
        (Weapon::Hammer, true) => trace(28, 0.34, 0, 78.0, 1, 0.0),
        (Weapon::Enforcer, _) => bullet(),
        (Weapon::Bio, false) => missile(28, 0.36, 1, Gel, 44.0, 0.72, 0),
        (Weapon::Bio, true) => missile(42, 0.88, 1, Glob, 76.0, 0.8, 0),
        (Weapon::Shock, false) => trace(39, 0.59, 1, 6500.0, 1, 0.0),
        (Weapon::Shock, true) => missile(58, 0.71, 1, Pulse, 145.0, 0.0, 0),
        (Weapon::Pulse, false) => missile(16, 0.15, 1, Pulse, 0.0, 0.0, 0),
        (Weapon::Pulse, true) => trace(12, 0.13, 1, 690.0, 1, 0.0),
        (Weapon::Ripper, false) => missile(33, 0.41, 1, Razor, 0.0, 0.0, 3),
        (Weapon::Ripper, true) => missile(42, 0.58, 1, RazorAlt, 115.0, 0.0, 0),
        (Weapon::Minigun, _) => bullet(),
        (Weapon::Flak, false) => trace(12, 0.66, 1, 1100.0, 8, 0.16),
        (Weapon::Flak, true) => missile(86, 0.85, 1, FlakShell, 170.0, 0.8, 0),
        (Weapon::Rocket, false) => missile(83, 0.92, 1, Rocket, 207.0, 0.0, 0),
        (Weapon::Rocket, true) => missile(80, 0.98, 1, Grenade, 190.0, 0.87, 2),
        (Weapon::Sniper, false) => bullet(),
        (Weapon::Sniper, true) => Shot {
            damage: 0,
            interval: 0.0,
            ammo: 0,
            guided: false,
            path: Path::Zoom,
        },
        (Weapon::Redeemer, false) => missile(220, 1.3, 1, Warhead, 460.0, 0.0, 0),
        (Weapon::Redeemer, true) => Shot {
            guided: true,
            ..missile(220, 1.3, 1, GuidedWarhead, 460.0, 0.0, 0)
        },
    }
}

pub(super) fn velocity(map: &Map, kind: Kind) -> f32 {
    map.ordnance[kind as usize]
        .speed
        .filter(|&speed| speed.is_finite() && speed > 0.0)
        .unwrap_or(1050.0)
}

pub(super) fn bot_shot(weapon: Weapon, ammo: u16, range: f32) -> (bool, Shot) {
    let alt = match weapon {
        Weapon::Enforcer => range < 115.0,
        Weapon::Hammer => range < 63.0,
        Weapon::Bio => (210.0..490.0).contains(&range),
        Weapon::Shock => (245.0..680.0).contains(&range),
        Weapon::Pulse => range < 570.0,
        Weapon::Ripper => (240.0..560.0).contains(&range),
        Weapon::Minigun => range < 280.0,
        Weapon::Flak => (365.0..710.0).contains(&range),
        _ => false,
    };
    let alt = alt && ammo >= shot(weapon, true).ammo;
    (alt, shot(weapon, alt))
}

fn rating(weapon: Weapon, distance: f32) -> f32 {
    match weapon {
        Weapon::Hammer if distance < 88.0 => 51.0,
        Weapon::Hammer => 0.0,
        Weapon::Flak if distance < 500.0 => 91.0,
        Weapon::Bio if distance < 450.0 => 70.0,
        Weapon::Pulse if distance < 620.0 => 75.0,
        Weapon::Minigun => 67.0,
        Weapon::Rocket if distance > 265.0 => 85.0,
        Weapon::Redeemer if distance > 585.0 => 96.0,
        Weapon::Sniper if distance > 680.0 => 90.0,
        Weapon::Shock => 72.0,
        Weapon::Enforcer => 36.0,
        Weapon::Ripper => 61.0,
        _ => 12.0,
    }
}
