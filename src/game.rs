use std::{collections::HashMap, sync::Arc};

use crate::map::{Face, Map, Vec3};
use crate::render::Camera;
use crate::sound::Wave;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Weapon {
    Hammer,
    Enforcer,
    Bio,
    Shock,
    Pulse,
    Ripper,
    Minigun,
    Flak,
    Rocket,
    Sniper,
    Redeemer,
}

impl Weapon {
    pub const ALL: [Self; 11] = [
        Self::Hammer,
        Self::Enforcer,
        Self::Bio,
        Self::Shock,
        Self::Pulse,
        Self::Ripper,
        Self::Minigun,
        Self::Flak,
        Self::Rocket,
        Self::Sniper,
        Self::Redeemer,
    ];

    pub fn class(self) -> &'static str {
        match self {
            Self::Hammer => "ImpactHammer",
            Self::Enforcer => "Enforcer",
            Self::Bio => "UT_BioRifle",
            Self::Shock => "ShockRifle",
            Self::Pulse => "PulseGun",
            Self::Ripper => "Ripper",
            Self::Minigun => "Minigun2",
            Self::Flak => "UT_FlakCannon",
            Self::Rocket => "UT_Eightball",
            Self::Sniper => "SniperRifle",
            Self::Redeemer => "WarheadLauncher",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hammer => "IMPACT HAMMER",
            Self::Enforcer => "ENFORCER",
            Self::Bio => "BIO RIFLE",
            Self::Shock => "SHOCK RIFLE",
            Self::Pulse => "PULSE GUN",
            Self::Ripper => "RIPPER",
            Self::Minigun => "MINIGUN",
            Self::Flak => "FLAK CANNON",
            Self::Rocket => "ROCKET LAUNCHER",
            Self::Sniper => "SNIPER RIFLE",
            Self::Redeemer => "REDEEMER",
        }
    }

    pub fn slot(self) -> u8 {
        if self == Self::Redeemer {
            0
        } else {
            ((self as u8) + 1) % 10
        }
    }

    fn initial(self) -> u16 {
        match self {
            Self::Hammer => 0,
            Self::Enforcer => 30,
            Self::Bio => 25,
            Self::Shock => 20,
            Self::Pulse => 60,
            Self::Ripper => 15,
            Self::Minigun => 60,
            Self::Flak => 10,
            Self::Rocket => 6,
            Self::Sniper => 8,
            Self::Redeemer => 1,
        }
    }

    fn maximum(self) -> u16 {
        match self {
            Self::Hammer => 0,
            Self::Enforcer | Self::Minigun => 199,
            Self::Bio => 100,
            Self::Shock => 50,
            Self::Pulse => 199,
            Self::Ripper => 75,
            Self::Flak | Self::Rocket => 50,
            Self::Sniper => 50,
            Self::Redeemer => 2,
        }
    }

    fn ammo_pool(self) -> usize {
        if self == Self::Minigun {
            Self::Enforcer as usize
        } else {
            self as usize
        }
    }

    fn behavior(self, alternate: bool) -> (f32, ShotKind) {
        let projectile = |speed, gravity, color| ShotKind::Projectile {
            speed,
            gravity,
            color,
        };
        match (self, alternate) {
            (Self::Hammer, _) => (
                0.55,
                ShotKind::Ray {
                    count: 1,
                    spread: 0.0,
                    range: 108.0,
                    color: 0xffbd78,
                },
            ),
            (Self::Enforcer, alt) => (
                if alt { 0.19 } else { 0.32 },
                ShotKind::Ray {
                    count: 1,
                    spread: if alt { 0.055 } else { 0.008 },
                    range: 12_000.0,
                    color: 0xffd294,
                },
            ),
            (Self::Bio, _) => (0.36, projectile(1050.0, 550.0, 0x8dff45)),
            (Self::Shock, false) => (
                0.68,
                ShotKind::Ray {
                    count: 1,
                    spread: 0.0,
                    range: 16_000.0,
                    color: 0xb37bff,
                },
            ),
            (Self::Shock, true) => (0.65, projectile(780.0, 0.0, 0xb37bff)),
            (Self::Pulse, _) => (0.13, projectile(1550.0, 0.0, 0x64ff91)),
            (Self::Ripper, _) => (0.42, projectile(1700.0, 0.0, 0xff9c4a)),
            (Self::Minigun, alt) => (
                if alt { 0.08 } else { 0.12 },
                ShotKind::Ray {
                    count: 1,
                    spread: if alt { 0.06 } else { 0.025 },
                    range: 11_000.0,
                    color: 0xffd294,
                },
            ),
            (Self::Flak, false) => (
                0.82,
                ShotKind::Ray {
                    count: 8,
                    spread: 0.09,
                    range: 3700.0,
                    color: 0xffb34e,
                },
            ),
            (Self::Flak, true) => (0.92, projectile(1120.0, 500.0, 0xffb34e)),
            (Self::Rocket, alt) => (
                0.9,
                projectile(1000.0, if alt { 480.0 } else { 0.0 }, 0xff993b),
            ),
            (Self::Sniper, _) => (
                0.85,
                ShotKind::Ray {
                    count: 1,
                    spread: 0.0,
                    range: 24_000.0,
                    color: 0xffd294,
                },
            ),
            (Self::Redeemer, _) => (1.3, projectile(650.0, 0.0, 0xff7229)),
        }
    }
}

#[derive(Default)]
pub struct WeaponAssets {
    pub view: Vec<Face>,
    pub pickup_ammo: Option<u16>,
    pub max_ammo: Option<u16>,
    pub primary_motion: Motion,
    pub alternate_motion: Motion,
    pub select_motion: Motion,
    pub down_motion: Motion,
    pub select_sound: Option<Arc<Wave>>,
    pub flash: Option<Flash>,
    pub fire: Option<Arc<Wave>>,
    pub alternate: Option<Arc<Wave>>,
}

pub struct Flash {
    pub textures: Vec<Arc<crate::texture::Texture>>,
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub length: f32,
}

#[derive(Default)]
pub struct Motion {
    pub frames: Vec<Vec<Face>>,
    pub rate: f32,
}

#[derive(Clone, Copy)]
pub struct Pose<'a> {
    pub current: &'a [Face],
    pub next: &'a [Face],
    pub fraction: f32,
}

impl Motion {
    pub fn duration(&self) -> f32 {
        if self.frames.is_empty() {
            0.0
        } else {
            self.frames.len() as f32 / self.rate
        }
    }

    pub fn sample(&self, elapsed: f32) -> Option<Pose<'_>> {
        if elapsed < 0.0 || !elapsed.is_finite() {
            return None;
        }
        let position = elapsed * self.rate;
        let index = position as usize;
        let current = self.frames.get(index)?;
        let next = self
            .frames
            .get(index + 1)
            .map_or(current.as_slice(), Vec::as_slice);
        Some(Pose {
            current,
            next,
            fraction: position.fract(),
        })
    }
}

#[derive(Clone, Copy)]
enum WeaponState {
    Ready,
    Lowering { target: Weapon, since: f32 },
    Raising { since: f32 },
}

#[derive(Clone, Copy)]
pub enum Item {
    Gun(Weapon),
    Ammo(Weapon, u16),
    Health(u16, u16),
    Armor(u16),
    Power(&'static str),
}

impl Item {
    pub fn from_class(class: &str) -> Option<Self> {
        if let Some(weapon) = Weapon::ALL
            .iter()
            .copied()
            .find(|weapon| weapon.class().eq_ignore_ascii_case(class))
        {
            return Some(Self::Gun(weapon));
        }
        Some(match class.to_ascii_lowercase().as_str() {
            "enforcerammo" | "miniammo" | "eclip" => Self::Ammo(Weapon::Enforcer, 50),
            "bioammo" => Self::Ammo(Weapon::Bio, 25),
            "shockcore" => Self::Ammo(Weapon::Shock, 10),
            "pammo" => Self::Ammo(Weapon::Pulse, 25),
            "bladehopper" => Self::Ammo(Weapon::Ripper, 25),
            "flakammo" => Self::Ammo(Weapon::Flak, 10),
            "rocketpack" => Self::Ammo(Weapon::Rocket, 12),
            "bulletbox" => Self::Ammo(Weapon::Sniper, 10),
            "medbox" => Self::Health(20, 100),
            "healthvial" => Self::Health(5, 199),
            "healthpack" | "superhealthpack" => Self::Health(100, 199),
            "armor2" => Self::Armor(100),
            "thighpads" => Self::Armor(50),
            "ut_shieldbelt" => Self::Armor(150),
            "udamage" => Self::Power("DAMAGE AMPLIFIER"),
            "ut_jumpboots" => Self::Power("JUMP BOOTS"),
            "ut_invisibility" | "invisibility" => Self::Power("INVISIBILITY"),
            _ => return None,
        })
    }
}

enum ShotKind {
    Ray {
        count: u8,
        spread: f32,
        range: f32,
        color: u32,
    },
    Projectile {
        speed: f32,
        gravity: f32,
        color: u32,
    },
}

struct Projectile {
    position: Vec3,
    velocity: Vec3,
    gravity: f32,
    splash: Option<(f32, f32)>,
    expires: f32,
    color: u32,
}

pub struct Effect {
    pub origin: Vec3,
    pub end: Vec3,
    pub color: u32,
    pub expires: f32,
}

pub struct Input {
    pub primary: bool,
    pub alternate: bool,
    pub slot: Option<u8>,
    pub cycle: i8,
}

pub struct Practice {
    pub health: u16,
    pub armor: u16,
    pub weapon: Weapon,
    pub effects: Vec<Effect>,
    pub muzzle: f32,
    last_fire: Option<(Weapon, bool, f32)>,
    weapon_state: WeaponState,
    pub time: f32,
    pub message: String,
    pub message_until: f32,
    pub shots: u32,
    pub jump_boost: f32,
    owned: [bool; 11],
    ammo: [u16; 11],
    respawn: Vec<f32>,
    projectiles: Vec<Projectile>,
    next_shot: f32,
    rng: u32,
}

impl Practice {
    pub fn new(map: &Map) -> Self {
        let mut game = Self {
            health: 100,
            armor: 0,
            weapon: Weapon::Enforcer,
            effects: Vec::new(),
            muzzle: 0.0,
            last_fire: None,
            weapon_state: WeaponState::Ready,
            time: 0.0,
            message: String::new(),
            message_until: 0.0,
            shots: 0,
            jump_boost: 0.0,
            owned: [false; 11],
            ammo: [0; 11],
            respawn: vec![0.0; map.props.len()],
            projectiles: Vec::new(),
            next_shot: 0.0,
            rng: 0x9916_cafe,
        };
        game.reset_player(&map.arsenal);
        game
    }

    pub fn reset_player(&mut self, arsenal: &HashMap<Weapon, WeaponAssets>) {
        self.health = 100;
        self.armor = 0;
        self.weapon = Weapon::Enforcer;
        self.jump_boost = 0.0;
        self.owned.fill(false);
        self.ammo.fill(0);
        self.owned[Weapon::Hammer as usize] = true;
        self.owned[Weapon::Enforcer as usize] = true;
        self.ammo[Weapon::Enforcer.ammo_pool()] = initial_ammo(arsenal, Weapon::Enforcer);
        self.next_shot = self.time;
        self.muzzle = 0.0;
        self.last_fire = None;
        self.weapon_state = WeaponState::Raising { since: self.time };
        self.effects.clear();
        self.projectiles.clear();
    }

    pub fn alternate_flash(&self) -> bool {
        self.last_fire
            .is_some_and(|(id, alt, _)| id == self.weapon && alt)
    }

    pub fn pose<'a>(&self, weapon: &'a WeaponAssets) -> Pose<'a> {
        let rest = || Pose {
            current: &weapon.view,
            next: &weapon.view,
            fraction: 0.0,
        };
        let transition = match self.weapon_state {
            WeaponState::Lowering { since, .. } => weapon.down_motion.sample(self.time - since),
            WeaponState::Raising { since } => weapon.select_motion.sample(self.time - since),
            WeaponState::Ready => None,
        };
        transition
            .or_else(|| {
                let (_, alternate, at) = self.last_fire.filter(|(id, _, _)| *id == self.weapon)?;
                let motion = if alternate && !weapon.alternate_motion.frames.is_empty() {
                    &weapon.alternate_motion
                } else {
                    &weapon.primary_motion
                };
                motion.sample(self.time - at)
            })
            .unwrap_or_else(rest)
    }

    fn request_weapon(&mut self, target: Weapon) {
        self.weapon_state = match self.weapon_state {
            WeaponState::Lowering { since, .. } if target != self.weapon => {
                WeaponState::Lowering { target, since }
            }
            WeaponState::Lowering { .. } => WeaponState::Raising { since: self.time },
            state if target == self.weapon => state,
            _ => WeaponState::Lowering {
                target,
                since: self.time,
            },
        };
    }

    fn pending_weapon(&self) -> Weapon {
        match self.weapon_state {
            WeaponState::Lowering { target, .. } => target,
            _ => self.weapon,
        }
    }

    fn update_weapon(&mut self, arsenal: &HashMap<Weapon, WeaponAssets>) -> Option<Arc<Wave>> {
        let mut sound = None;
        if let WeaponState::Lowering { target, since } = self.weapon_state {
            let duration = arsenal
                .get(&self.weapon)
                .map_or(0.0, |asset| asset.down_motion.duration());
            if self.time - since >= duration {
                self.weapon = target;
                self.last_fire = None;
                self.muzzle = 0.0;
                self.weapon_state = WeaponState::Raising {
                    since: since + duration,
                };
                sound = arsenal
                    .get(&target)
                    .and_then(|asset| asset.select_sound.clone());
            }
        }
        if let WeaponState::Raising { since } = self.weapon_state {
            let duration = arsenal
                .get(&self.weapon)
                .map_or(0.0, |asset| asset.select_motion.duration());
            if self.time - since >= duration {
                self.weapon_state = WeaponState::Ready;
            }
        }
        sound
    }

    pub fn ammo(&self) -> Option<u16> {
        (self.weapon != Weapon::Hammer).then_some(self.ammo[self.weapon.ammo_pool()])
    }

    pub fn owned(&self) -> impl Iterator<Item = Weapon> + '_ {
        Weapon::ALL
            .into_iter()
            .filter(|weapon| self.owned[*weapon as usize])
    }

    pub fn jump_speed(&self, base: f32) -> f32 {
        if self.jump_boost > self.time {
            510.0
        } else {
            base
        }
    }

    pub fn fall_damage(&mut self, speed: f32, jump_speed: f32) {
        let damage = if speed > 1650.0 + jump_speed {
            1000
        } else if speed >= 750.0 + jump_speed {
            (0.15 * (speed - 700.0 - jump_speed)) as u16
        } else {
            return;
        };
        self.health = self.health.saturating_sub(damage);
        self.notice(format!("FALL DAMAGE -{damage}"));
    }

    fn notice(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.message_until = self.time + 2.4;
    }

    fn pickup(&mut self, item: Item, arsenal: &HashMap<Weapon, WeaponAssets>) -> bool {
        let message = match item {
            Item::Gun(weapon) => {
                let index = weapon.ammo_pool();
                let was_owned = self.owned[weapon as usize];
                let add = if was_owned {
                    initial_ammo(arsenal, weapon).div_ceil(2)
                } else {
                    initial_ammo(arsenal, weapon)
                };
                if was_owned && self.ammo[index] >= max_ammo(arsenal, weapon) {
                    return false;
                }
                self.owned[weapon as usize] = true;
                self.ammo[index] = (self.ammo[index] + add).min(max_ammo(arsenal, weapon));
                if !was_owned {
                    self.request_weapon(weapon);
                }
                weapon.label().to_owned()
            }
            Item::Ammo(weapon, count) => {
                let ammo = &mut self.ammo[weapon.ammo_pool()];
                if *ammo >= max_ammo(arsenal, weapon) {
                    return false;
                }
                let previous = *ammo;
                *ammo = (*ammo + count).min(max_ammo(arsenal, weapon));
                format!("{} AMMO +{}", weapon.label(), *ammo - previous)
            }
            Item::Health(amount, cap) => {
                if self.health >= cap {
                    return false;
                }
                let previous = self.health;
                self.health = (self.health + amount).min(cap);
                format!("HEALTH +{}", self.health - previous)
            }
            Item::Armor(amount) => {
                if self.armor >= 150 {
                    return false;
                }
                let previous = self.armor;
                self.armor = (self.armor + amount).min(150);
                format!("ARMOR +{}", self.armor - previous)
            }
            Item::Power(label) => {
                if label == "JUMP BOOTS" {
                    self.jump_boost = self.time + 30.0;
                }
                label.to_owned()
            }
        };
        self.notice(message);
        true
    }

    pub fn update(
        &mut self,
        map: &mut Map,
        camera: &Camera,
        dt: f32,
        input: Input,
        collect: bool,
    ) -> Vec<Arc<Wave>> {
        self.time += dt;
        self.effects.retain(|effect| effect.expires > self.time);
        let mut sounds = Vec::new();
        for index in 0..map.props.len() {
            let prop = &mut map.props[index];
            if !prop.visible && self.time >= self.respawn[index] {
                prop.visible = true;
            }
            let Some(item) = prop.item else { continue };
            let offset = prop.position - camera.position;
            let horizontal = offset.0 * offset.0 + offset.1 * offset.1;
            if collect
                && prop.visible
                && horizontal < 45.0 * 45.0
                && (offset.2 + 23.0).abs() < 55.0
                && self.pickup(item, &map.arsenal)
            {
                prop.visible = false;
                self.respawn[index] = self.time + prop.respawn_time.unwrap_or(25.0);
                if let Some(sound) = &prop.pickup_sound {
                    sounds.push(Arc::clone(sound));
                }
            }
        }
        if let Some(slot) = input.slot {
            let choices: Vec<_> = self
                .owned()
                .filter(|weapon| weapon.slot() == slot)
                .collect();
            if !choices.is_empty() {
                let next = choices
                    .iter()
                    .position(|weapon| *weapon == self.pending_weapon())
                    .map_or(0, |i| (i + 1) % choices.len());
                self.request_weapon(choices[next]);
            }
        }
        if input.cycle != 0 {
            let owned: Vec<_> = self.owned().collect();
            if let Some(current) = owned
                .iter()
                .position(|weapon| *weapon == self.pending_weapon())
            {
                self.request_weapon(
                    owned[(current as isize + input.cycle as isize).rem_euclid(owned.len() as isize)
                        as usize],
                );
            }
        }
        sounds.extend(self.update_weapon(&map.arsenal));
        if (input.primary || input.alternate)
            && self.time >= self.next_shot
            && matches!(self.weapon_state, WeaponState::Ready)
        {
            let sound = self.fire(map, camera, !input.primary && input.alternate);
            sounds.extend(sound);
        }
        let mut damage = 0;
        self.projectiles.retain_mut(|shot| {
            if shot.expires <= self.time {
                return false;
            }
            shot.velocity.2 -= shot.gravity * dt;
            let delta = shot.velocity * dt;
            if let Some(point) = map.collision.ray(shot.position, delta) {
                if let Some((radius, strength)) = shot.splash {
                    let offset = point - camera.position;
                    let distance = offset.dot(offset).sqrt();
                    if map.collision.ray(camera.position, offset * 0.98).is_none() {
                        damage += ((1.0 - distance / radius).max(0.0) * strength).round() as u16;
                    }
                }
                self.effects.push(Effect {
                    origin: point,
                    end: point,
                    color: shot.color,
                    expires: self.time + 0.18,
                });
                false
            } else {
                shot.position = shot.position + delta;
                true
            }
        });
        if damage > 0 {
            let absorbed = self.armor.min(damage.saturating_mul(2) / 3);
            self.armor -= absorbed;
            self.health = self.health.saturating_sub(damage - absorbed);
            self.notice(format!("DAMAGE -{}", damage - absorbed));
        }
        sounds
    }

    pub fn visible_effects(&self) -> impl Iterator<Item = Effect> + '_ {
        self.effects
            .iter()
            .map(|effect| Effect { ..*effect })
            .chain(self.projectiles.iter().map(|shot| Effect {
                origin: shot.position - shot.velocity * 0.025,
                end: shot.position,
                color: shot.color,
                expires: shot.expires,
            }))
    }

    fn fire(&mut self, map: &Map, camera: &Camera, alternate: bool) -> Option<Arc<Wave>> {
        let weapon = self.weapon;
        let ammo = &mut self.ammo[weapon.ammo_pool()];
        if weapon != Weapon::Hammer {
            if *ammo == 0 {
                self.notice("NO AMMO");
                self.next_shot = self.time + 0.35;
                return None;
            }
            *ammo -= 1;
        }
        let (cooldown, shot) = weapon.behavior(alternate);
        self.next_shot = self.time + cooldown;
        self.muzzle = self.time
            + map
                .arsenal
                .get(&weapon)
                .and_then(|asset| asset.flash.as_ref())
                .map_or(0.0, |flash| flash.length);
        self.last_fire = Some((weapon, alternate, self.time));
        self.shots += 1;
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        let muzzle = camera.position + forward * 32.0 + right * 13.0 - up * 9.0;
        match shot {
            ShotKind::Ray {
                count,
                spread,
                range,
                color,
            } => {
                for _ in 0..count {
                    let direction =
                        forward + right * (self.jitter() * spread) + up * (self.jitter() * spread);
                    let delta = direction * range;
                    let hit = map.collision.ray(camera.position, delta);
                    let end = hit.unwrap_or(camera.position + delta);
                    self.effects.push(Effect {
                        origin: muzzle,
                        end,
                        color,
                        expires: self.time + 0.085,
                    });
                    if let Some(point) = hit {
                        self.effects.push(Effect {
                            origin: point - forward * 3.0,
                            end: point - forward * 3.0,
                            color,
                            expires: self.time + 0.14,
                        });
                    }
                }
            }
            ShotKind::Projectile {
                speed,
                gravity,
                color,
            } => {
                let start = camera.position + forward * 20.0;
                if let Some(point) = map.collision.ray(camera.position, forward * 20.0) {
                    self.effects.push(Effect {
                        origin: point - forward * 3.0,
                        end: point - forward * 3.0,
                        color,
                        expires: self.time + 0.17,
                    });
                } else {
                    self.projectiles.push(Projectile {
                        position: start,
                        velocity: forward * speed,
                        gravity,
                        splash: match weapon {
                            Weapon::Rocket => Some((180.0, 100.0)),
                            Weapon::Redeemer => Some((420.0, 170.0)),
                            Weapon::Flak | Weapon::Bio => Some((95.0, 40.0)),
                            _ => None,
                        },
                        color,
                        expires: self.time + 5.0,
                    });
                }
            }
        }
        shot_sound(&map.arsenal, weapon, alternate)
    }

    fn jitter(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 16) as f32 / 32768.0) - 1.0
    }
}

fn initial_ammo(arsenal: &HashMap<Weapon, WeaponAssets>, weapon: Weapon) -> u16 {
    arsenal
        .get(&weapon)
        .and_then(|asset| asset.pickup_ammo)
        .unwrap_or_else(|| weapon.initial())
}

fn max_ammo(arsenal: &HashMap<Weapon, WeaponAssets>, weapon: Weapon) -> u16 {
    arsenal
        .get(&weapon)
        .and_then(|asset| asset.max_ammo)
        .unwrap_or_else(|| weapon.maximum())
}

fn shot_sound(
    arsenal: &HashMap<Weapon, WeaponAssets>,
    weapon: Weapon,
    alt: bool,
) -> Option<Arc<Wave>> {
    let assets = arsenal.get(&weapon)?;
    if alt && assets.alternate.is_some() {
        return assets.alternate.clone();
    }
    assets.fire.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;

    #[test]
    #[ignore = "requires the local GOTY map and game packages"]
    fn deck16_practice_can_fire_pick_up_switch_and_respawn() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        let profile = map.collision.profile;
        assert_eq!(profile.ground_speed, 400.0);
        assert_eq!(profile.acceleration, 2048.0);
        assert_eq!(profile.air_control, 0.35);
        assert_eq!(profile.eye_height, 27.0);
        assert!(map.player_audio.steps.iter().all(Option::is_some));
        assert!(map.player_audio.jump.is_some());
        assert!(map.player_audio.land.is_some());
        let mut camera = Camera::from_start(map.starts[6]);
        let mut practice = Practice::new(&map);
        let idle = || Input {
            primary: false,
            alternate: false,
            slot: None,
            cycle: 0,
        };
        let shoot = || Input {
            primary: true,
            ..idle()
        };
        let model = map
            .arsenal
            .get(&Weapon::Enforcer)
            .expect("Enforcer defaults");
        assert!(!model.view.is_empty(), "Enforcer first-person mesh");
        assert_eq!(model.pickup_ammo, Some(30));
        assert!(model.max_ammo.is_some());
        assert!(map.hud.ready() && map.hud.crosshair.is_some() && map.hud.doll.is_some());
        assert_eq!(model.primary_motion.frames.len(), 10);
        assert_eq!(model.alternate_motion.frames.len(), 8);
        let first = model.primary_motion.frames[0][0].vertices[0];
        assert!(model.primary_motion.frames.iter().skip(1).any(|frame| {
            let delta = frame[0].vertices[0] - first;
            delta.dot(delta) > 0.01
        }));
        assert!(model.fire.is_some(), "Enforcer original sound");
        assert_eq!(model.select_motion.frames.len(), 28);
        assert_eq!(model.down_motion.frames.len(), 12);
        assert!(model.select_sound.is_some());
        let flash = model
            .flash
            .as_ref()
            .expect("Original Enforcer muzzle flash");
        assert_eq!(flash.textures.len(), 5);
        assert!((flash.length - 0.02).abs() < 0.001);
        let pose = model
            .primary_motion
            .sample(0.5 / model.primary_motion.rate)
            .unwrap();
        assert!((pose.fraction - 0.5).abs() < 0.001);
        assert!(!std::ptr::eq(pose.current, pose.next));
        let opening = model.select_motion.duration();
        assert_eq!(practice.ammo(), Some(30));
        practice.update(&mut map, &camera, 0.016, shoot(), false);
        assert_eq!(
            practice.ammo(),
            Some(30),
            "firing waits until Select finishes"
        );
        let sounds = practice.update(&mut map, &camera, opening, shoot(), false);
        assert_eq!(practice.ammo(), Some(29));
        assert_eq!(practice.shots, 1);
        assert!(!sounds.is_empty() && !practice.effects.is_empty());
        assert!(practice.muzzle > practice.time);
        let mut with_flash = crate::render::Frame::new();
        with_flash.draw_practice(&map, &camera, practice.time, &practice);
        practice.muzzle = 0.0;
        let mut without_flash = crate::render::Frame::new();
        without_flash.draw_practice(&map, &camera, practice.time, &practice);
        assert_ne!(with_flash.pixels, without_flash.pixels);
        let pickup = map
            .props
            .iter()
            .position(|prop| matches!(prop.item, Some(Item::Gun(Weapon::Shock))))
            .unwrap();
        camera.position = map.props[pickup].position + Vec3(0.0, 0.0, 23.0);
        let sounds = practice.update(&mut map, &camera, 0.016, idle(), true);
        assert_eq!(
            practice.weapon,
            Weapon::Enforcer,
            "Down precedes the weapon swap"
        );
        let lower = map.arsenal[&Weapon::Enforcer].down_motion.duration();
        practice.update(&mut map, &camera, lower + 0.01, shoot(), false);
        assert_eq!(practice.weapon, Weapon::Shock);
        assert!(matches!(practice.weapon_state, WeaponState::Raising { .. }));
        assert_eq!(practice.ammo(), Some(20));
        assert!(!map.props[pickup].visible);
        assert!(!sounds.is_empty(), "Original pickup sound");
        let mut switch = idle();
        switch.slot = Some(2);
        let wait = map.props[pickup].respawn_time.unwrap_or(25.0) + 1.0;
        practice.update(&mut map, &camera, wait, switch, false);
        practice.update(&mut map, &camera, 1.0, idle(), false);
        assert!(map.props[pickup].visible);
        assert_eq!(practice.weapon, Weapon::Enforcer);
        assert_eq!(practice.ammo(), Some(29));
    }
}
