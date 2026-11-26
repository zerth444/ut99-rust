use crate::game::{Weapon, WeaponAssets, cheats, initial_ammo, max_ammo, ordnance::Kind};
use crate::map::Map;
use std::collections::HashMap;

pub(super) struct Inventory {
    owned: [bool; Weapon::ALL.len()],
    ammo: [u16; Weapon::ALL.len()],
    capacity_override: [Option<u16>; Weapon::ALL.len()],
}

impl Inventory {
    pub fn empty() -> Self {
        Self {
            owned: [false; Weapon::ALL.len()],
            ammo: [0; Weapon::ALL.len()],
            capacity_override: [None; Weapon::ALL.len()],
        }
    }

    pub fn new(map: &Map) -> Self {
        let mut inventory = Self::empty();
        inventory.pickup(&map.arsenal, Weapon::Hammer);
        inventory.pickup(&map.arsenal, Weapon::Enforcer);
        inventory
    }

    pub fn grant_training_weapon(&mut self, weapon: Weapon) {
        self.owned[weapon as usize] = true;
        if weapon.uses_ammo() {
            let pool = weapon.ammo_pool();
            self.capacity_override[pool] = Some(199);
            self.ammo[pool] = 199;
        }
    }

    pub fn cheat_refill(&mut self) {
        for weapon in Weapon::ALL {
            if weapon.uses_ammo() && self.owns(weapon) {
                let pool = weapon.ammo_pool();
                self.capacity_override[pool] = Some(cheats::AMMO);
                self.ammo[pool] = cheats::AMMO;
            }
        }
    }

    pub fn owns(&self, weapon: Weapon) -> bool {
        self.owned[weapon as usize]
    }

    pub fn ammo(&self, weapon: Weapon) -> u16 {
        self.ammo[weapon.ammo_pool()]
    }

    pub fn usable(&self, weapon: Weapon) -> bool {
        self.owns(weapon) && (!weapon.uses_ammo() || self.ammo(weapon) > 0)
    }

    pub fn grant_ammo(
        &mut self,
        art: &HashMap<Weapon, WeaponAssets>,
        weapon: Weapon,
        quantity: u16,
    ) -> bool {
        let cap = self.capacity(art, weapon);
        let ammo = &mut self.ammo[weapon.ammo_pool()];
        let next = ammo.saturating_add(quantity).min(cap);
        if next <= *ammo {
            return false;
        }
        *ammo = next;
        true
    }

    pub fn capacity(&self, art: &HashMap<Weapon, WeaponAssets>, weapon: Weapon) -> u16 {
        self.capacity_override[weapon.ammo_pool()].unwrap_or_else(|| max_ammo(art, weapon))
    }

    pub fn pickup(&mut self, art: &HashMap<Weapon, WeaponAssets>, weapon: Weapon) -> bool {
        self.pickup_with_ammo(art, weapon, initial_ammo(art, weapon))
    }

    pub fn pickup_with_ammo(
        &mut self,
        art: &HashMap<Weapon, WeaponAssets>,
        weapon: Weapon,
        quantity: u16,
    ) -> bool {
        let new = !self.owns(weapon);
        self.owned[weapon as usize] = true;
        let ammo = self.grant_ammo(art, weapon, quantity);
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
        self.preferred_with_favorite(distance, None)
    }

    pub fn preferred_with_favorite(&self, distance: f32, favorite: Option<Weapon>) -> Weapon {
        // Stock Bot.SwitchToBestWeapon adds 0.22 on a 0–1 rating scale.
        let score =
            |weapon| rating(weapon, distance) + if Some(weapon) == favorite { 22.0 } else { 0.0 };
        Weapon::ALL
            .into_iter()
            .filter(|&weapon| weapon != Weapon::Translocator && self.usable(weapon))
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
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
    pub max_units: u16,
    pub shape: ChargeShape,
}

#[derive(Clone, Copy)]
pub(super) enum ChargeShape {
    Volley,
    Grow,
}

impl ChargeProfile {
    const BIO_LOAD_PERIOD: f32 = 0.52;

    pub fn units(
        self,
        rocket: Option<&super::super::RocketAnimations>,
        elapsed: f32,
        cap: u16,
    ) -> u16 {
        match self.shape {
            ChargeShape::Volley => {
                u16::from(super::super::RocketAnimations::load_phase(rocket, elapsed, cap as u8).0)
            }
            ChargeShape::Grow => 1u16
                .saturating_add((elapsed.max(0.0) / Self::BIO_LOAD_PERIOD) as u16)
                .min(cap),
        }
    }

    pub fn time_for(self, rocket: Option<&super::super::RocketAnimations>, units: u16) -> f32 {
        match self.shape {
            ChargeShape::Volley => super::super::RocketAnimations::load_time(rocket, units as u8),
            ChargeShape::Grow => Self::BIO_LOAD_PERIOD * f32::from(units.saturating_sub(1)),
        }
    }
}

pub(super) fn charge_profile(weapon: Weapon, alt: bool) -> Option<ChargeProfile> {
    match (weapon, alt) {
        (Weapon::Rocket, _) => Some(ChargeProfile {
            max_units: u16::from(super::super::ROCKET_CAPACITY),
            shape: ChargeShape::Volley,
        }),
        (Weapon::Bio, true) => Some(ChargeProfile {
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
    },
    Missile {
        kind: Kind,
        blast: f32,
        gravity: f32,
        rebounds: u8,
    },
    Shock,
    Zoom,
}

pub(super) fn shot(weapon: Weapon, alt: bool) -> Shot {
    let bullet = || trace(0, 0.0, 1, super::ballistics::BulletProfile::RANGE);
    fn trace(damage: u16, interval: f32, ammo: u16, range: f32) -> Shot {
        Shot {
            damage,
            interval,
            ammo,
            guided: false,
            path: Path::Trace { range },
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
        (Weapon::Translocator, _) => trace(0, 0.0, 0, 0.0),
        (Weapon::Hammer, false) => trace(44, 0.58, 0, 96.0),
        (Weapon::Hammer, true) => trace(28, 0.34, 0, 78.0),
        (Weapon::Enforcer, _) => bullet(),
        (Weapon::Bio, false) => missile(20, 0.36, 1, Gel, 150.0, 1.0, 0),
        (Weapon::Bio, true) => missile(75, 0.88, 1, Glob, 75.0, 1.0, 0),
        (Weapon::Shock, _) => Shot {
            damage: 40,
            interval: super::super::shock::fallback_cooldown(alt),
            ammo: 1,
            guided: false,
            path: Path::Shock,
        },
        (Weapon::Pulse, false) => missile(20, 0.15, 1, Pulse, 0.0, 0.0, 0),
        (Weapon::Pulse, true) => trace(12, 0.24, 1, super::PulseBeam::RANGE),
        (Weapon::Ripper, false) => missile(30, 0.41, 1, Razor, 0.0, 0.0, 3),
        (Weapon::Ripper, true) => missile(34, 0.58, 1, RazorAlt, 180.0, 0.0, 0),
        (Weapon::Minigun, _) => bullet(),
        (Weapon::Flak, false) => missile(
            16,
            0.82,
            1,
            Chunk,
            0.0,
            0.0,
            super::super::ordnance::FLAK_CHUNK_REBOUNDS,
        ),
        (Weapon::Flak, true) => {
            let (radius, damage, _) = super::super::ordnance::FLAK_SHELL_BLAST;
            missile(damage as u16, 0.92, 1, FlakShell, radius, 1.0, 0)
        }
        (Weapon::Rocket, false) => missile(75, 0.92, 1, Rocket, 220.0, 0.0, 0),
        (Weapon::Rocket, true) => missile(80, 0.98, 1, Grenade, 200.0, 1.0, 2),
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
    kind.launch_speed(&map.ordnance[kind as usize], 0.5)
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
