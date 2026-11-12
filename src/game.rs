use std::{collections::HashMap, sync::Arc};

use crate::collision::Walker;
use crate::map::{Face, Map, Vec3};
use crate::render::Camera;
use crate::sound::Wave;

pub mod ballistics;
pub mod ordnance;
pub mod shock;

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
        if matches!(self, Self::Sniper | Self::Redeemer) {
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
    pub shock: Option<shock::Assets>,
    pub status_icon: Option<Arc<crate::texture::Texture>>,
    pub pulse_beam: Vec<Vec<Face>>,
    pub scope: Option<Arc<crate::texture::Texture>>,
    pub load: Motion,
    pub load_sound: Option<Arc<Wave>>,
    pub charge_loop: Option<Arc<Wave>>,
    pub guided_hud: [Option<Arc<crate::texture::Texture>>; 3],
    pub rocket_lock: [Option<Arc<Wave>>; 2],
    pub lock_reticle: Option<Arc<crate::texture::Texture>>,
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
    Armor(ArmorKind),
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
            "armor2" => Self::Armor(ArmorKind::Chest),
            "thighpads" => Self::Armor(ArmorKind::Thigh),
            "ut_shieldbelt" => Self::Armor(ArmorKind::Belt),
            "udamage" => Self::Power("DAMAGE AMPLIFIER"),
            "ut_jumpboots" => Self::Power("JUMP BOOTS"),
            "ut_invisibility" | "invisibility" => Self::Power("INVISIBILITY"),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
pub enum ArmorKind {
    Belt,
    Chest,
    Thigh,
}

impl ArmorKind {
    const ALL: [Self; 3] = [Self::Belt, Self::Chest, Self::Thigh];

    fn charge(self) -> u16 {
        match self {
            Self::Belt => 150,
            Self::Chest => 100,
            Self::Thigh => 50,
        }
    }

    fn absorption(self) -> u32 {
        match self {
            Self::Belt => 100,
            Self::Chest => 75,
            Self::Thigh => 50,
        }
    }
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

pub struct PulseBeam {
    pub start: Vec3,
    pub end: Vec3,
    pub started: f32,
}

pub struct Death {
    pub position: Vec3,
    pub yaw: f32,
    pub started: f32,
    pub animation: usize,
}

pub struct TrainingTarget {
    pub position: Vec3,
    pub yaw: f32,
    pub health: u16,
}

impl TrainingTarget {
    fn ray(&self, origin: Vec3, delta: Vec3) -> Option<f32> {
        if self.health == 0 {
            return None;
        }
        shock::cylinder_hit(
            origin - self.position,
            delta,
            crate::collision::PLAYER_RADIUS,
            39.0,
        )
    }

    fn damage(&mut self, damage: u16) {
        self.health = self.health.saturating_sub(damage);
    }
}

struct Charge {
    weapon: Weapon,
    alternate: bool,
    loaded: u8,
    since: f32,
    next_load: f32,
    tight: bool,
}

#[derive(Default)]
struct SniperZoom {
    level: f32,
    charging: bool,
    held: bool,
}

impl SniperZoom {
    fn update(&mut self, pressed: bool, dt: f32, ready: bool) {
        if !ready {
            *self = Self {
                held: pressed,
                ..Self::default()
            };
            return;
        }
        if pressed && !self.held {
            self.charging = self.level == 0.0;
            self.level = 0.0;
        }
        self.held = pressed;
        self.charging &= pressed;
        if self.charging {
            self.level = (self.level + dt).min(0.9);
        }
    }
}

pub struct Practice {
    pub health: u16,
    armor: [u16; 3],
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
    pub shock: shock::Shots,
    pub pulse_beam: Option<PulseBeam>,
    pub ordnance: ordnance::Shots,
    pub ballistics: ballistics::Effects,
    pub death: Option<Death>,
    charge: Option<Charge>,
    hit_taken: u16,
    pub hit_flash_until: f32,
    last_pain: f32,
    hammer_since: Option<f32>,
    was_guiding: bool,
    guide_trigger_held: bool,
    pub guidance_lost_until: f32,
    pub target: Option<TrainingTarget>,
    lock_since: Option<f32>,
    pub rocket_locked: bool,
    pub minigun_firing: bool,
    enforcer_alt_accuracy: f32,
    target_damage_fraction: f32,
    zoom: SniperZoom,
    next_shot: f32,
    rng: u32,
}

impl Practice {
    pub fn new(map: &Map) -> Self {
        let mut game = Self {
            health: 100,
            armor: [0; 3],
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
            shock: shock::Shots::default(),
            pulse_beam: None,
            ordnance: ordnance::Shots::default(),
            ballistics: ballistics::Effects::default(),
            death: None,
            charge: None,
            hit_taken: 0,
            hit_flash_until: 0.0,
            last_pain: -1.0,
            hammer_since: None,
            was_guiding: false,
            guide_trigger_held: false,
            guidance_lost_until: 0.0,
            target: None,
            lock_since: None,
            rocket_locked: false,
            minigun_firing: false,
            enforcer_alt_accuracy: 0.4,
            target_damage_fraction: 0.0,
            zoom: SniperZoom::default(),
            next_shot: 0.0,
            rng: 0x9916_cafe,
        };
        game.reset_player(&map.arsenal);
        game
    }

    pub fn reset_player(&mut self, arsenal: &HashMap<Weapon, WeaponAssets>) {
        self.health = 100;
        self.armor.fill(0);
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
        self.shock = shock::Shots::default();
        self.pulse_beam = None;
        self.ordnance = ordnance::Shots::default();
        self.ballistics = ballistics::Effects::default();
        self.death = None;
        self.charge = None;
        self.hit_taken = 0;
        self.hit_flash_until = self.time;
        self.last_pain = self.time - 1.0;
        self.hammer_since = None;
        self.was_guiding = false;
        self.guide_trigger_held = false;
        self.guidance_lost_until = self.time;
        self.target = None;
        self.lock_since = None;
        self.rocket_locked = false;
        self.minigun_firing = false;
        self.enforcer_alt_accuracy = 0.4;
        self.target_damage_fraction = 0.0;
        self.zoom = SniperZoom::default();
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
                let elapsed = self.time - self.hammer_since?;
                let pull = weapon.load.duration();
                if elapsed < pull {
                    return weapon.load.sample(elapsed);
                }
                let shake = weapon.alternate_motion.duration();
                (shake > 0.0)
                    .then(|| (elapsed - pull).rem_euclid(shake))
                    .and_then(|age| weapon.alternate_motion.sample(age))
            })
            .or_else(|| {
                let elapsed = self.time - self.charge.as_ref()?.since;
                let duration = weapon.load.duration();
                (duration > 0.0)
                    .then(|| elapsed.min((duration - 0.001).max(0.0)))
                    .and_then(|age| weapon.load.sample(age))
            })
            .or_else(|| {
                let beam = self.pulse_beam.as_ref()?;
                let duration = weapon.alternate_motion.duration();
                (duration > 0.0)
                    .then(|| (self.time - beam.started).rem_euclid(duration))
                    .and_then(|elapsed| weapon.alternate_motion.sample(elapsed))
            })
            .or_else(|| {
                let (_, alternate, at) = self.last_fire.filter(|(id, _, _)| *id == self.weapon)?;
                let motion = if alternate && !weapon.alternate_motion.frames.is_empty() {
                    if self.weapon == Weapon::Hammer {
                        &weapon.primary_motion
                    } else {
                        &weapon.alternate_motion
                    }
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

    pub fn pending_weapon(&self) -> Weapon {
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

    pub fn field_of_view(&self) -> f32 {
        90.0 - self.zoom.level * 88.0
    }

    pub fn scoped(&self) -> bool {
        self.zoom.level > 0.0
    }

    pub fn hammer_charging(&self) -> bool {
        self.hammer_since.is_some()
    }

    pub fn place_training_target(&mut self, map: &Map, camera: &Camera) {
        let forward = camera.forward();
        let distance = map
            .collision
            .ray(camera.position, forward * 640.0)
            .map_or(640.0, |point| (point - camera.position).dot(forward));
        for length in [
            (distance - 24.0).min(350.0),
            (distance - 24.0).min(220.0),
            96.0,
        ] {
            if length < 64.0 || length + 20.0 >= distance {
                continue;
            }
            let marker = camera.position + forward * length;
            if let Some((floor, normal)) = map.collision.ray_surface(marker, Vec3(0.0, 0.0, -512.0))
                && normal.2 >= 0.7
            {
                self.target = Some(TrainingTarget {
                    position: floor + Vec3(0.0, 0.0, 39.2),
                    yaw: camera.yaw + std::f32::consts::PI,
                    health: 100,
                });
                self.owned.fill(true);
                for weapon in Weapon::ALL {
                    let pool = weapon.ammo_pool();
                    self.ammo[pool] = self.ammo[pool].max(initial_ammo(&map.arsenal, weapon));
                }
                self.lock_since = None;
                self.rocket_locked = false;
                self.ordnance.clear_seeking();
                self.notice("TRAINING TARGET READY");
                return;
            }
        }
        self.notice("FIND OPEN FLOOR FOR TRAINING TARGET");
    }

    fn update_rocket_lock(
        &mut self,
        map: &Map,
        camera: &Camera,
        ready: bool,
        sounds: &mut Vec<Arc<Wave>>,
    ) {
        let visible = ready
            && self.weapon == Weapon::Rocket
            && self.target.as_ref().is_some_and(|target| {
                let offset = target.position - camera.position;
                let distance = offset.dot(offset).sqrt();
                target.health > 0
                    && distance <= 4000.0
                    && offset.dot(camera.look_direction()) > 0.93 * distance
                    && map.collision.ray(camera.position, offset).is_none()
            });
        let previous = self.rocket_locked;
        if visible {
            let since = *self.lock_since.get_or_insert(self.time);
            self.rocket_locked = self.time - since >= 1.25;
        } else {
            self.lock_since = None;
            self.rocket_locked = false;
        }
        if previous != self.rocket_locked
            && let Some(art) = map.arsenal.get(&Weapon::Rocket)
        {
            sounds.extend(art.rocket_lock[usize::from(previous)].iter().cloned());
        }
    }

    pub fn ammo_fraction(
        &self,
        weapon: Weapon,
        arsenal: &HashMap<Weapon, WeaponAssets>,
    ) -> Option<f32> {
        (weapon != Weapon::Hammer).then(|| {
            f32::from(self.ammo[weapon.ammo_pool()]) / f32::from(max_ammo(arsenal, weapon).max(1))
        })
    }

    pub fn armor(&self) -> u16 {
        self.armor.iter().sum::<u16>().min(150)
    }

    pub fn armor_charge(&self, kind: ArmorKind) -> u16 {
        self.armor[kind as usize]
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
        self.hit_taken = self.hit_taken.saturating_add(damage);
        self.hit_flash_until = self.time + 0.20;
        self.notice(format!("FALL DAMAGE -{damage}"));
    }

    pub fn begin_death(&mut self, position: Vec3, yaw: f32, map: &Map) -> Option<Arc<Wave>> {
        let choice = ((self.jitter() + 1.0) * 1.5) as usize % 3;
        self.death = Some(Death {
            position,
            yaw,
            started: self.time,
            animation: choice,
        });
        self.pulse_beam = None;
        self.charge = None;
        self.hammer_since = None;
        self.zoom = SniperZoom::default();
        self.hit_taken = 0;
        self.notice("PRESS FIRE TO RESPAWN");
        self.message_until = f32::INFINITY;
        map.player_audio.death[choice].clone()
    }

    pub fn pain_sound(&mut self, map: &Map) -> Option<Arc<Wave>> {
        let damage = std::mem::take(&mut self.hit_taken);
        if damage == 0 || self.health == 0 || self.time - self.last_pain < 0.3 {
            return None;
        }
        self.last_pain = self.time;
        let severity = f32::from(damage) * (self.jitter() + 1.0) * 0.5;
        let index = if severity < 8.0 {
            0
        } else if severity < 25.0 {
            if self.jitter() < 0.0 { 1 } else { 2 }
        } else {
            3
        };
        map.player_audio.pain[index].clone()
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
            Item::Armor(kind) => {
                let current = self.armor_charge(kind);
                let available = match kind {
                    ArmorKind::Belt => 150,
                    _ => 150 - self.armor_charge(ArmorKind::Belt),
                };
                let charge = available.min(kind.charge());
                if charge <= current {
                    return false;
                }
                if matches!(kind, ArmorKind::Belt) {
                    self.armor.fill(0);
                }
                self.armor[kind as usize] = charge;
                format!("ARMOR +{}", charge - current)
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
        player: Option<&mut Walker>,
    ) -> Vec<Arc<Wave>> {
        self.time += dt;
        self.effects.retain(|effect| effect.expires > self.time);
        let mut sounds = Vec::new();
        self.ballistics
            .update(map, self.target.as_ref(), self.time, dt, &mut sounds);
        for index in 0..map.props.len() {
            let prop = &mut map.props[index];
            if !prop.visible && self.time >= self.respawn[index] {
                prop.visible = true;
            }
            let Some(item) = prop.item else { continue };
            let offset = prop.position - camera.position;
            let horizontal = offset.0 * offset.0 + offset.1 * offset.1;
            if collect
                && self.health > 0
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
        let guiding = self.ordnance.guided_view().is_some();
        let ready = self.health > 0 && !guiding && matches!(self.weapon_state, WeaponState::Ready);
        let minigun = ready
            && self.weapon == Weapon::Minigun
            && (input.primary || input.alternate)
            && self.ammo().is_some_and(|ammo| ammo > 0);
        if self.minigun_firing && !minigun {
            sounds.extend(
                map.arsenal
                    .get(&Weapon::Minigun)
                    .and_then(|art| art.load_sound.clone()),
            );
        }
        self.minigun_firing = minigun;
        if self.weapon != Weapon::Enforcer || input.primary || !input.alternate {
            self.enforcer_alt_accuracy = 0.4;
        }
        self.update_rocket_lock(map, camera, ready, &mut sounds);
        self.zoom
            .update(input.alternate, dt, ready && self.weapon == Weapon::Sniper);
        self.update_pulse(
            map,
            camera,
            ready
                && self.weapon == Weapon::Pulse
                && input.alternate
                && (!input.primary || self.pulse_beam.is_some()),
        );
        if let Some(beam) = &self.pulse_beam
            && let Some(target) = &mut self.target
            && let Some(fraction) = target.ray(beam.start, beam.end - beam.start)
        {
            self.target_damage_fraction += 72.0 * dt;
            let damage = self.target_damage_fraction as u16;
            self.target_damage_fraction -= f32::from(damage);
            target.damage(damage);
            if let Some(beam) = &mut self.pulse_beam {
                beam.end = beam.start + (beam.end - beam.start) * fraction;
            }
        } else {
            self.target_damage_fraction = 0.0;
        }
        let mut blasts = self
            .shock
            .update(map, dt, self.time, &mut sounds, self.target.as_ref());
        let trigger = input.primary || input.alternate;
        blasts.extend(self.ordnance.guide(
            map,
            camera.look_direction(),
            dt,
            self.time,
            trigger && !self.guide_trigger_held,
            self.health > 0,
            &mut sounds,
        ));
        self.guide_trigger_held = trigger;
        self.ordnance.update_with_target(
            map,
            player.as_deref().map(|walker| walker.position),
            self.target.as_mut(),
            self.time,
            dt,
            &mut sounds,
            &mut blasts,
        );
        if self.was_guiding && self.ordnance.guided_view().is_none() {
            self.guidance_lost_until = self.time + 0.3;
            self.next_shot = self.time + 1.3;
        }
        let charging = self.update_charge(map, camera, &input, ready, &mut sounds, &mut blasts);
        let hammer_hit = self.update_hammer(map, camera, &input, ready, &mut sounds);
        let alternate_shot =
            input.alternate && !matches!(self.weapon, Weapon::Sniper | Weapon::Pulse);
        if !charging
            && self.weapon != Weapon::Hammer
            && self.pulse_beam.is_none()
            && (input.primary || alternate_shot)
            && self.time >= self.next_shot
            && ready
        {
            blasts.extend(self.fire(map, camera, !input.primary && alternate_shot, &mut sounds));
        }
        self.was_guiding = self.ordnance.guided_view().is_some();
        if let Some(target) = &mut self.target {
            for blast in &blasts {
                if let Some((damage, _)) = blast.hit_player(&map.collision, target.position) {
                    target.damage(damage);
                }
            }
            if target.health > 0
                && let Some((damage, _)) =
                    self.ordnance
                        .nuclear_wave(map, self.time, target.position, 1)
            {
                target.damage(damage);
            }
        }
        if let Some(player) = player {
            if let Some((damage, momentum)) = hammer_hit {
                player.apply_self_damage_momentum(momentum, map.collision.profile);
                self.splash_damage(damage);
            }
            for blast in blasts {
                if let Some((damage, momentum)) = blast.hit_player(&map.collision, player.position)
                {
                    player.apply_self_damage_momentum(momentum, map.collision.profile);
                    self.splash_damage(damage);
                }
            }
            if let Some((damage, impulse)) =
                self.ordnance
                    .nuclear_wave(map, self.time, player.position, 0)
            {
                player.add_velocity(impulse);
                self.splash_damage(damage);
            }
        }
        sounds
    }

    fn update_hammer(
        &mut self,
        map: &Map,
        camera: &Camera,
        input: &Input,
        ready: bool,
        sounds: &mut Vec<Arc<Wave>>,
    ) -> Option<(u16, Vec3)> {
        if !ready || self.weapon != Weapon::Hammer {
            self.hammer_since = None;
            return None;
        }
        let alternate = self.hammer_since.is_none() && input.alternate && !input.primary;
        let contact_trigger = self
            .hammer_since
            .is_some_and(|since| (self.time - since) * 0.75 > 1.0)
            && self.target.as_ref().is_some_and(|target| {
                let direction = camera.look_direction();
                let offset = direction * 25.0;
                map.collision.ray(camera.position, offset).is_none()
                    && target.ray(camera.position, offset).is_some()
            });
        if (input.primary && !contact_trigger) || self.hammer_since.is_none() && !alternate {
            if input.primary && self.hammer_since.is_none() && self.time >= self.next_shot {
                self.hammer_since = Some(self.time);
                sounds.extend(
                    map.arsenal
                        .get(&Weapon::Hammer)
                        .and_then(|art| art.load_sound.clone()),
                );
            }
            return None;
        }
        if self.time < self.next_shot {
            return None;
        }
        let charge = self
            .hammer_since
            .take()
            .map_or(0.0, |since| (self.time - since) * 0.75);
        let cooldown = map
            .arsenal
            .get(&Weapon::Hammer)
            .map_or(0.82, |art| art.primary_motion.duration().max(0.82));
        self.next_shot = self.time + cooldown;
        self.last_fire = Some((Weapon::Hammer, alternate, self.time));
        self.shots += 1;
        sounds.extend(shot_sound(&map.arsenal, Weapon::Hammer, alternate));
        if alternate {
            self.ordnance.deflect(map, camera);
            self.shock.deflect(map, camera);
        }
        let direction = camera.look_direction();
        let start = camera.position + camera.right() * 6.0 - direction.cross(camera.right()) * 5.0;
        let range = if alternate { 180.0 } else { 120.0 };
        let delta = direction * range;
        let wall = map.collision.ray_surface(start, delta);
        let visible = wall.map_or(delta, |(point, _)| point - start);
        if let Some(target) = &mut self.target
            && let Some(fraction) = target.ray(start, visible)
        {
            let point = start + visible * fraction;
            let distance = point - camera.position;
            let distance = distance.dot(distance).sqrt();
            if alternate {
                target.damage((20.0 * distance / 180.0) as u16);
            } else if distance < 90.0 {
                target.damage((60.0 * charge.min(1.5)) as u16);
            }
            return None;
        }
        let (point, normal) = wall?;
        self.effects.push(Effect {
            origin: point + normal * 2.0,
            end: point + normal * 2.0,
            color: 0xffbd78,
            expires: self.time + 0.16,
        });
        if alternate {
            let distance = point - camera.position;
            let scale = distance.dot(distance).sqrt() / 180.0;
            Some(((24.0 * scale) as u16, direction * (-40_000.0 * scale)))
        } else {
            Some((36, direction * (-69_000.0 * charge.clamp(1.0, 1.5))))
        }
    }

    fn update_pulse(&mut self, map: &Map, camera: &Camera, held: bool) {
        if !held {
            if self.pulse_beam.take().is_some() {
                self.next_shot = self.time;
            }
            return;
        }
        if self.time < self.next_shot && self.pulse_beam.is_none() {
            return;
        }
        if self.time >= self.next_shot {
            let ammo = &mut self.ammo[Weapon::Pulse.ammo_pool()];
            if *ammo == 0 {
                self.pulse_beam = None;
                self.next_shot = self.time + 0.24;
                return;
            }
            *ammo -= 1;
            self.next_shot += 0.24;
            if self.pulse_beam.is_none() || self.next_shot <= self.time {
                self.next_shot = self.time + 0.24;
            }
        }
        let direction = camera.look_direction();
        let right = camera.right();
        let up = direction.cross(right);
        let offset = direction * 16.0 + right * 14.0 - up * 8.0;
        let start = camera.position + offset;
        let end = if map.collision.ray(camera.position, offset).is_some() {
            start
        } else {
            map.collision
                .ray(start, direction * 810.0)
                .unwrap_or(start + direction * 810.0)
        };
        let started = self
            .pulse_beam
            .as_ref()
            .map_or(self.time, |beam| beam.started);
        self.pulse_beam = Some(PulseBeam {
            start,
            end,
            started,
        });
    }

    fn update_charge(
        &mut self,
        map: &Map,
        camera: &Camera,
        input: &Input,
        ready: bool,
        sounds: &mut Vec<Arc<Wave>>,
        blasts: &mut Vec<shock::Blast>,
    ) -> bool {
        let weapon = self.weapon;
        let rocket = weapon == Weapon::Rocket;
        if !ready || (!rocket && weapon != Weapon::Bio) {
            if let Some(charge) = self.charge.take()
                && self.health > 0
            {
                self.ammo[charge.weapon.ammo_pool()] += u16::from(charge.loaded);
            }
            return false;
        }
        let alternate = !input.primary && input.alternate;
        let trigger = if rocket {
            input.primary || input.alternate
        } else {
            alternate
        };
        let interval = if rocket { 0.9 } else { 0.5 };
        let limit = if rocket { 6 } else { 9 };
        let ammo = &mut self.ammo[weapon.ammo_pool()];
        let load_sound = || {
            map.arsenal
                .get(&weapon)
                .and_then(|art| art.load_sound.clone())
        };
        if self.charge.is_none() {
            if !trigger {
                return false;
            }
            if self.time < self.next_shot {
                return true;
            }
            if *ammo == 0 {
                self.notice("NO AMMO");
                self.next_shot = self.time + 0.35;
                return true;
            }
            *ammo -= 1;
            self.charge = Some(Charge {
                weapon,
                alternate,
                loaded: 1,
                since: self.time,
                next_load: self.time + interval,
                tight: rocket && !alternate && input.alternate,
            });
            sounds.extend(load_sound());
            return true;
        }
        let charge = self.charge.as_mut().unwrap();
        let held = if charge.alternate {
            input.alternate
        } else {
            input.primary
        };
        if rocket && !charge.alternate && input.alternate {
            charge.tight = true;
        }
        if held && self.time >= charge.next_load && charge.loaded < limit && *ammo > 0 {
            *ammo -= 1;
            charge.loaded += 1;
            charge.next_load += interval;
            if rocket {
                sounds.extend(load_sound());
            }
        }
        if held
            && (!rocket || (charge.loaded < limit && (*ammo > 0 || self.time < charge.next_load)))
        {
            return true;
        }
        let charge = self.charge.take().unwrap();
        let kind = if !rocket {
            ordnance::Kind::Glob
        } else if charge.alternate {
            ordnance::Kind::Grenade
        } else {
            ordnance::Kind::Rocket
        };
        let count = if rocket { charge.loaded } else { 1 };
        let scale = if rocket {
            1.0
        } else {
            (1.0 + 0.4 * f32::from(charge.loaded - 1)) * 0.5
        };
        let forward = camera.look_direction();
        let right = camera.right();
        for index in 0..count {
            let spread = f32::from(index) - (f32::from(count) - 1.0) * 0.5;
            let aim = if charge.tight {
                forward
            } else {
                crate::mover::unit(forward + right * (spread * 0.09))
            };
            let variation = (self.jitter() + 1.0) * 0.5;
            blasts.extend(self.ordnance.fire(
                map,
                camera,
                kind,
                aim,
                spread * 7.0,
                scale,
                self.time,
                variation,
                sounds,
            ));
            if kind == ordnance::Kind::Rocket && self.rocket_locked {
                self.ordnance.seek_latest(self.time);
            }
        }
        self.shots += u32::from(count);
        self.last_fire = Some((weapon, charge.alternate, self.time));
        self.next_shot = self.time + if rocket { 0.6 } else { 0.4 };
        sounds.extend(shot_sound(&map.arsenal, weapon, charge.alternate));
        true
    }

    fn splash_damage(&mut self, mut damage: u16) {
        for kind in ArmorKind::ALL {
            let charge = &mut self.armor[kind as usize];
            let absorbed = (*charge).min((u32::from(damage) * kind.absorption() / 100) as u16);
            *charge -= absorbed;
            damage -= absorbed;
        }
        if damage > 0 {
            self.health = self.health.saturating_sub(damage);
            self.hit_taken = self.hit_taken.saturating_add(damage);
            self.hit_flash_until = self.time + 0.20;
            self.notice(format!("DAMAGE -{damage}"));
        }
    }

    pub fn visible_effects(&self) -> impl Iterator<Item = Effect> + '_ {
        self.effects.iter().map(|effect| Effect { ..*effect })
    }

    fn fire(
        &mut self,
        map: &Map,
        camera: &Camera,
        alternate: bool,
        sounds: &mut Vec<Arc<Wave>>,
    ) -> Option<shock::Blast> {
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
        self.muzzle = self.time
            + map
                .arsenal
                .get(&weapon)
                .and_then(|asset| asset.flash.as_ref())
                .map_or(0.0, |flash| flash.length);
        self.last_fire = Some((weapon, alternate, self.time));
        self.shots += 1;
        if weapon != Weapon::Minigun {
            sounds.extend(shot_sound(&map.arsenal, weapon, alternate));
        }
        if weapon == Weapon::Shock {
            let motion = map.arsenal.get(&weapon).map(|asset| {
                if alternate {
                    &asset.alternate_motion
                } else {
                    &asset.primary_motion
                }
            });
            let cooldown = motion
                .filter(|motion| !motion.frames.is_empty())
                .map_or_else(|| shock::fallback_cooldown(alternate), Motion::duration);
            self.next_shot = self.time + cooldown;
            return self.shock.fire(
                map,
                camera,
                alternate,
                self.time,
                ammo,
                sounds,
                self.target.as_mut(),
            );
        }
        let kind = match weapon {
            Weapon::Pulse => Some(ordnance::Kind::Pulse),
            Weapon::Bio => Some(ordnance::Kind::Gel),
            Weapon::Flak if alternate => Some(ordnance::Kind::FlakShell),
            Weapon::Ripper if alternate => Some(ordnance::Kind::RazorAlt),
            Weapon::Ripper => Some(ordnance::Kind::Razor),
            Weapon::Redeemer if alternate => Some(ordnance::Kind::GuidedWarhead),
            Weapon::Redeemer => Some(ordnance::Kind::Warhead),
            _ => None,
        };
        if let Some(kind) = kind {
            self.next_shot = self.time
                + match weapon {
                    Weapon::Pulse => 0.18,
                    Weapon::Flak => 0.92,
                    Weapon::Ripper => 0.42,
                    Weapon::Redeemer => 1.3,
                    _ => 0.36,
                };
            return self.ordnance.fire(
                map,
                camera,
                kind,
                camera.look_direction(),
                0.0,
                1.0,
                self.time,
                0.0,
                sounds,
            );
        }
        if weapon == Weapon::Flak {
            self.next_shot = self.time + 0.82;
            let forward = camera.look_direction();
            let right = camera.right();
            let up = forward.cross(right);
            for _ in 0..8 {
                let direction = crate::mover::unit(
                    forward + right * (self.jitter() * 0.11) + up * (self.jitter() * 0.11),
                );
                let variation = (self.jitter() + 1.0) * 0.5;
                self.ordnance.fire(
                    map,
                    camera,
                    ordnance::Kind::Chunk,
                    direction,
                    0.0,
                    1.0,
                    self.time,
                    variation,
                    sounds,
                );
            }
            return None;
        }
        self.trace_bullet(map, camera, weapon, alternate, sounds);
        None
    }

    fn trace_bullet(
        &mut self,
        map: &Map,
        camera: &Camera,
        weapon: Weapon,
        alternate: bool,
        sounds: &mut Vec<Arc<Wave>>,
    ) {
        let (cooldown, accuracy, damage, side, height) = match (weapon, alternate) {
            (Weapon::Enforcer, false) => (0.32, 0.2, 17, 3.5, -1.4),
            (Weapon::Enforcer, true) => {
                let accuracy = self.enforcer_alt_accuracy;
                self.enforcer_alt_accuracy = (accuracy + 0.5).min(3.0);
                (0.19, accuracy, 17, 3.5, -1.4)
            }
            (Weapon::Minigun, alt) => {
                let random_damage = 9 + (((self.jitter() + 1.0) * 3.0) as i32).min(5);
                if alt {
                    (0.08, 0.95, random_damage, 5.0, -4.0)
                } else {
                    (0.13, 0.2, random_damage, 5.0, -4.0)
                }
            }
            (Weapon::Sniper, _) => (0.67, 0.0, 45, 0.0, 0.0),
            _ => return,
        };
        self.next_shot = self.time + cooldown;
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        let start = camera.position + right * side + up * height;
        let delta =
            forward * 10_000.0 + (right * self.jitter() + up * self.jitter()) * (500.0 * accuracy);
        let mut wall = map.collision.ray_surface(start, delta);
        let mut visible = wall.map_or(delta, |(point, _)| point - start);
        if let Some(target) = &mut self.target
            && let Some(fraction) = target.ray(start, visible)
        {
            let point = start + visible * fraction;
            let headshot = weapon == Weapon::Sniper && point.2 - target.position.2 > 0.62 * 39.0;
            target.damage(if headshot { 100 } else { damage as u16 });
            sounds.extend(map.ballistics.flesh.iter().cloned());
            visible = point - start;
            wall = None;
        }
        if weapon == Weapon::Minigun {
            self.ballistics.rounds = (self.ballistics.rounds + 1) % 4;
            if self.ballistics.rounds == 0 && visible.dot(visible) > 250.0_f32.powi(2) {
                self.ballistics.tracer.push(ballistics::Moving {
                    position: start + forward * 96.0,
                    velocity: crate::mover::unit(delta) * 4000.0,
                    born: self.time,
                    style: 0,
                    scale: 1.0,
                    bounces: 0,
                });
            }
        }
        if weapon != Weapon::Minigun || self.time - self.ballistics.last_case > 0.125 {
            let (ahead, side, height) = match weapon {
                Weapon::Enforcer => (20.0, 3.5, 1.0),
                Weapon::Minigun => (30.0, 5.0, -5.0),
                _ => (30.0, 5.0, -1.0),
            };
            let offset = forward * ahead + right * side + up * height;
            if map.collision.ray(camera.position, offset).is_none() {
                let velocity = (forward * (0.55 + 0.15 * self.jitter())
                    + right * (0.3 + 0.1 * self.jitter())
                    + up * (1.15 + 0.15 * self.jitter()))
                    * 160.0;
                self.ballistics.cases.push(ballistics::Moving {
                    position: camera.position + offset,
                    velocity,
                    born: self.time,
                    style: usize::from(weapon == Weapon::Minigun),
                    scale: if weapon == Weapon::Sniper { 2.0 } else { 1.0 },
                    bounces: 0,
                });
            }
            if weapon == Weapon::Minigun {
                self.ballistics.last_case = self.time;
            }
        }
        if let Some((point, normal)) = wall {
            let sound = (((self.jitter() + 1.0) * 2.0) as usize).min(3);
            if let Some(sound) = map.ballistics.wall_sounds.get(sound) {
                sounds.extend(sound.iter().cloned());
            }
            let origin = point + normal * if weapon == Weapon::Enforcer { 1.0 } else { 9.0 };
            let style = (((self.jitter() + 1.0) * 2.0) as usize).min(3);
            self.ballistics.smoke.push((origin, self.time, style));
            let count = if weapon == Weapon::Minigun { 2 } else { 3 };
            let sparks = ((self.jitter() + 1.0) * 0.5 * count as f32) as u8;
            for _ in 0..sparks {
                let direction =
                    crate::mover::unit(Vec3(self.jitter(), self.jitter(), self.jitter()));
                let velocity = (normal + direction) * (100.0 * (self.jitter() + 1.0));
                self.ballistics.sparks.push(ballistics::Moving {
                    position: point + normal * 9.0,
                    velocity,
                    born: self.time,
                    style: 0,
                    scale: 1.0,
                    bounces: 0,
                });
            }
        }
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

fn hammer_deflection(map: &Map, camera: &Camera, position: Vec3, velocity: Vec3) -> Option<Vec3> {
    let offset = position - camera.position;
    let distance = offset.dot(offset).sqrt();
    if !(0.01..=550.0).contains(&distance)
        || offset.dot(camera.look_direction()) <= 0.9 * distance
        || map.collision.ray(camera.position, offset).is_some()
    {
        return None;
    }
    let side = camera.right();
    let sign = if velocity.dot(side) > 0.0 { 1.0 } else { -1.0 };
    let speed = velocity.dot(velocity).sqrt();
    Some(crate::mover::unit(velocity + side * ((750.0 - distance) * sign)) * speed)
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

    fn fire_input(primary: bool, alternate: bool) -> Input {
        Input {
            primary,
            alternate,
            slot: None,
            cycle: 0,
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn enforcer_and_minigun_use_stock_impact_and_tracer_assets() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        assert!(map.ballistics.tracer.sample(0.0).is_some());
        assert!(
            map.ballistics
                .cases
                .iter()
                .all(|motion| motion.sample(0.0).is_some())
        );
        assert!(map.ballistics.smoke.iter().all(Option::is_some));
        assert!(map.ballistics.spark.is_some());
        assert!(map.ballistics.wall_sounds.iter().all(Option::is_some));
        let camera = Camera {
            position: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
        };
        for (distance, tracers) in [(200.0, 0), (400.0, 1)] {
            map.collision =
                crate::collision::Collision::read(&[], std::iter::empty(), &[]).unwrap();
            map.collision.add_box(
                Vec3(distance, -100.0, -100.0),
                Vec3(distance + 10.0, 100.0, 100.0),
            );
            let mut game = Practice::new(&map);
            let mut sounds = Vec::new();
            for _ in 0..4 {
                game.trace_bullet(&map, &camera, Weapon::Enforcer, false, &mut sounds);
            }
            assert!(game.ballistics.tracer.is_empty() && game.effects.is_empty());
            assert_eq!(game.ballistics.smoke.len(), 4);
            for _ in 0..3 {
                game.trace_bullet(&map, &camera, Weapon::Minigun, false, &mut sounds);
                assert!(game.ballistics.tracer.is_empty());
            }
            game.trace_bullet(&map, &camera, Weapon::Minigun, false, &mut sounds);
            assert_eq!(game.ballistics.tracer.len(), tracers);
            if let Some(shot) = game.ballistics.tracer.first() {
                assert_eq!(shot.position.0, 96.0);
                assert!((shot.velocity.dot(shot.velocity).sqrt() - 4000.0).abs() < 0.01);
                game.ballistics.update(&map, None, 0.1, 0.1, &mut sounds);
                assert!(game.ballistics.tracer.is_empty());
            }
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn deck16_all_weapon_inputs_produce_the_expected_action() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let camera = Camera::from_start(map.starts[11]);
        for weapon in Weapon::ALL {
            let art = &map.arsenal[&weapon];
            assert!(
                !art.primary_motion.frames.is_empty(),
                "{weapon:?} fire animation"
            );
            for alternate in [false, true] {
                let mut game = Practice::new(&map);
                game.weapon = weapon;
                game.weapon_state = WeaponState::Ready;
                game.ammo[weapon.ammo_pool()] = 10;
                let sounds = game.update(
                    &mut map,
                    &camera,
                    0.01,
                    fire_input(!alternate, alternate),
                    false,
                    None,
                );
                match (weapon, alternate) {
                    (Weapon::Sniper, true) => {
                        assert!(game.scoped());
                        assert_eq!(game.ammo(), Some(10));
                    }
                    (Weapon::Pulse, true) => {
                        assert!(game.pulse_beam.is_some());
                        assert_eq!(game.ammo(), Some(9));
                    }
                    (Weapon::Hammer, false) | (Weapon::Rocket, _) | (Weapon::Bio, true) => {
                        assert_eq!(game.shots, 0, "{weapon:?} charges before release");
                        game.update(
                            &mut map,
                            &camera,
                            0.01,
                            fire_input(false, false),
                            false,
                            None,
                        );
                        assert_eq!(game.shots, 1, "{weapon:?} fires on release");
                    }
                    _ => {
                        assert!(game.shots > 0, "{weapon:?}, alternate={alternate}");
                        assert_eq!(game.ammo(), (weapon != Weapon::Hammer).then_some(9));
                        if weapon != Weapon::Minigun {
                            assert!(
                                !sounds.is_empty(),
                                "{weapon:?}, alternate={alternate} sound"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn hammer_jumps_and_rocket_lock_and_redeemer_guidance_work_in_practice() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let mut collision =
            crate::collision::Collision::read(&[], std::iter::empty(), &[]).unwrap();
        collision.add_box(Vec3(-4000.0, -4000.0, -100.0), Vec3(4000.0, 4000.0, 0.0));
        map.collision = collision;
        let mut game = Practice::new(&map);
        let mut walker = Walker::spawn(&map.collision, Vec3(0.0, 0.0, 39.0));
        let camera = Camera {
            position: walker.position + Vec3(0.0, 0.0, 27.0),
            yaw: 0.0,
            pitch: -1.5,
        };
        game.weapon = Weapon::Hammer;
        game.weapon_state = WeaponState::Ready;
        for _ in 0..140 {
            game.update(
                &mut map,
                &camera,
                0.01,
                fire_input(true, false),
                false,
                Some(&mut walker),
            );
        }
        assert!(game.hammer_charging() && walker.grounded);
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(false, false),
            false,
            Some(&mut walker),
        );
        assert_eq!(game.health, 64);
        walker.advance(&map.collision, Vec3::default(), None, 0.05);
        assert!(walker.position.2 > 45.0);
        assert!(map.arsenal[&Weapon::Hammer].charge_loop.is_some());

        let mut camera = Camera {
            position: Vec3(0.0, 0.0, 66.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        game.reset_player(&map.arsenal);
        game.place_training_target(&map, &camera);
        assert!(game.target.is_some() && game.owned().count() == Weapon::ALL.len());
        game.weapon = Weapon::Rocket;
        game.weapon_state = WeaponState::Ready;
        for _ in 0..130 {
            game.update(
                &mut map,
                &camera,
                0.01,
                fire_input(false, false),
                false,
                None,
            );
        }
        assert!(game.rocket_locked);
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(true, false),
            false,
            None,
        );
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(false, false),
            false,
            None,
        );
        game.target.as_mut().unwrap().position.1 = 44.0;
        for _ in 0..55 {
            game.update(
                &mut map,
                &camera,
                0.01,
                fire_input(false, false),
                false,
                None,
            );
        }
        assert!(
            game.target.as_ref().unwrap().health < 100,
            "seeking rocket should reach shifted target"
        );

        game.reset_player(&map.arsenal);
        game.weapon = Weapon::Redeemer;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Redeemer.ammo_pool()] = 1;
        let art = &map.ordnance[ordnance::Kind::GuidedWarhead as usize];
        assert!(
            !art.flight.frames.is_empty() && art.explosion.is_some() && !art.wavefront.is_empty()
        );
        assert!(map.arsenal[&Weapon::Redeemer].guided_hud[0].is_some());
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(false, true),
            false,
            None,
        );
        let start = game
            .ordnance
            .guided_view()
            .expect("missile camera")
            .position;
        camera.yaw = 0.5;
        for _ in 0..125 {
            game.update(
                &mut map,
                &camera,
                0.01,
                fire_input(false, true),
                false,
                None,
            );
        }
        let missile = game
            .ordnance
            .guided_view()
            .expect("holding launch does not detonate");
        assert!(missile.yaw > 0.3 && missile.position.1 > start.1 + 50.0);
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(false, false),
            false,
            None,
        );
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(true, false),
            false,
            None,
        );
        assert!(game.ordnance.guided_view().is_none() && game.guidance_lost_until > game.time);
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn deck16_rocket_loads_a_salvo_and_player_death_uses_stock_art() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let camera = Camera::from_start(map.starts[11]);
        let walker = Walker::spawn(&map.collision, map.starts[11].position);
        assert!(map.player_audio.pain.iter().all(Option::is_some));
        assert!(map.player_audio.death.iter().all(Option::is_some));
        assert!(
            map.player_deaths
                .iter()
                .all(|motion| motion.frames.len() > 5 && !motion.frames[0].is_empty())
        );
        if path
            .with_file_name("../Textures/SoldierSkins.utx")
            .is_file()
        {
            assert!(
                map.player_deaths[0].frames[0]
                    .iter()
                    .any(|face| face.texture.is_some())
            );
        }
        let input = |primary| Input {
            primary,
            alternate: false,
            cycle: 0,
            slot: None,
        };
        let mut game = Practice::new(&map);
        game.weapon = Weapon::Rocket;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Rocket.ammo_pool()] = 6;
        assert!(map.arsenal[&Weapon::Rocket].load_sound.is_some());
        assert!(map.arsenal[&Weapon::Bio].fire.is_some());
        for _ in 0..95 {
            game.update(&mut map, &camera, 0.01, input(true), false, None);
        }
        assert_eq!(game.ammo(), Some(4));
        assert_eq!(game.ordnance.active_count(), 0);
        game.update(&mut map, &camera, 0.01, input(false), false, None);
        assert_eq!(game.ordnance.active_count(), 2);
        assert_eq!(game.ammo(), Some(4));
        game.splash_damage(27);
        assert!(game.pain_sound(&map).is_some());
        assert!(game.pain_sound(&map).is_none());
        assert!(game.hit_flash_until > game.time);
        game.splash_damage(1000);
        assert!(
            game.begin_death(walker.position, camera.yaw, &map)
                .is_some()
        );
        assert_eq!(game.health, 0);
        let mut frame = crate::render::Frame::new();
        frame.draw_practice(&map, &camera, game.time, &game);
        let first_pose = frame.pixels.clone();
        game.update(&mut map, &camera, 0.36, input(true), true, None);
        frame.draw_practice(&map, &camera, game.time, &game);
        assert_ne!(frame.pixels, first_pose);
        assert_eq!(game.health, 0);
        game.reset_player(&map.arsenal);
        assert!(game.death.is_none());
        assert_eq!(game.health, 100);
    }

    #[test]
    #[ignore = "requires the local GOTY map and game packages"]
    fn deck16_pulse_beam_and_sniper_zoom_use_their_own_secondary_modes() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        for weapon in Weapon::ALL {
            let model = &map.arsenal[&weapon];
            assert!(
                !model.view.is_empty(),
                "{} first-person model",
                weapon.label()
            );
            assert!(model.status_icon.is_some(), "{} HUD icon", weapon.label());
        }
        let pulse = &map.arsenal[&Weapon::Pulse];
        assert_eq!(pulse.pulse_beam.len(), 5);
        assert!(pulse.pulse_beam.iter().all(|faces| !faces.is_empty()));
        assert!(pulse.alternate.is_some() && !pulse.alternate_motion.frames.is_empty());
        assert!(map.arsenal[&Weapon::Sniper].scope.is_some());
        assert_eq!(Weapon::Sniper.slot(), 0);

        let mut game = Practice::new(&map);
        let camera = Camera::from_start(map.starts[11]);
        let input = |primary, alternate| Input {
            primary,
            alternate,
            cycle: 0,
            slot: None,
        };
        game.weapon = Weapon::Pulse;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Pulse.ammo_pool()] = 6;
        game.update(&mut map, &camera, 0.01, input(false, true), false, None);
        assert!(game.pulse_beam.is_some());
        assert!(
            (game.pulse_beam.as_ref().unwrap().start - camera.position).dot(camera.right()) > 0.0
        );
        assert_eq!(game.ammo(), Some(5));
        for _ in 0..25 {
            game.update(&mut map, &camera, 0.01, input(false, true), false, None);
        }
        assert_eq!(game.ammo(), Some(4));
        game.update(&mut map, &camera, 0.01, input(true, false), false, None);
        assert!(game.pulse_beam.is_none());
        assert_eq!(game.ordnance.active_count(), 1);
        let pulse = &map.ordnance[ordnance::Kind::Pulse as usize];
        assert!(pulse.sprite.is_some() && pulse.explosion.is_some() && pulse.sound.is_some());
        assert_eq!(pulse.speed.unwrap_or(1450.0), 1450.0);

        game.weapon = Weapon::Sniper;
        game.ammo[Weapon::Sniper.ammo_pool()] = 8;
        for _ in 0..20 {
            game.update(&mut map, &camera, 0.02, input(false, true), false, None);
        }
        assert!(game.field_of_view() < 60.0);
        let scoped_fov = game.field_of_view();
        game.update(&mut map, &camera, 0.01, input(false, false), false, None);
        assert_eq!(game.field_of_view(), scoped_fov);
        game.update(&mut map, &camera, 0.01, input(false, true), false, None);
        assert!(!game.scoped());
        assert_eq!(game.ammo(), Some(8));
    }

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
        practice.update(&mut map, &camera, 0.016, shoot(), false, None);
        assert_eq!(
            practice.ammo(),
            Some(30),
            "firing waits until Select finishes"
        );
        let sounds = practice.update(&mut map, &camera, opening, shoot(), false, None);
        assert_eq!(practice.ammo(), Some(29));
        assert_eq!(practice.shots, 1);
        assert!(!sounds.is_empty() && !practice.ballistics.cases.is_empty());
        assert!(practice.ballistics.tracer.is_empty() && practice.effects.is_empty());
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
        let sounds = practice.update(&mut map, &camera, 0.016, idle(), true, None);
        assert_eq!(
            practice.weapon,
            Weapon::Enforcer,
            "Down precedes the weapon swap"
        );
        let lower = map.arsenal[&Weapon::Enforcer].down_motion.duration();
        practice.update(&mut map, &camera, lower + 0.01, shoot(), false, None);
        assert_eq!(practice.weapon, Weapon::Shock);
        assert!(matches!(practice.weapon_state, WeaponState::Raising { .. }));
        assert_eq!(practice.ammo(), Some(20));
        assert!(!map.props[pickup].visible);
        assert!(!sounds.is_empty(), "Original pickup sound");
        let mut switch = idle();
        switch.slot = Some(2);
        let wait = map.props[pickup].respawn_time.unwrap_or(25.0) + 1.0;
        practice.update(&mut map, &camera, wait, switch, false, None);
        practice.update(&mut map, &camera, 1.0, idle(), false, None);
        assert!(map.props[pickup].visible);
        assert_eq!(practice.weapon, Weapon::Enforcer);
        assert_eq!(practice.ammo(), Some(29));
    }
}
