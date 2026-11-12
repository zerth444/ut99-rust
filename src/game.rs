use std::{collections::HashMap, sync::Arc};

use crate::collision::Walker;
use crate::map::{Face, Map, Vec3};
use crate::render::Camera;
use crate::sound::Wave;

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
            (
                Self::Bio | Self::Shock | Self::Pulse | Self::Rocket | Self::Ripper | Self::Flak,
                _,
            ) => {
                unreachable!("weapon has dedicated firing logic")
            }
            (Self::Minigun, alt) => (
                if alt { 0.08 } else { 0.12 },
                ShotKind::Ray {
                    count: 1,
                    spread: if alt { 0.06 } else { 0.025 },
                    range: 11_000.0,
                    color: 0xffd294,
                },
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
    pub shock: Option<shock::Assets>,
    pub status_icon: Option<Arc<crate::texture::Texture>>,
    pub pulse_beam: Vec<Vec<Face>>,
    pub scope: Option<Arc<crate::texture::Texture>>,
    pub load: Motion,
    pub load_sound: Option<Arc<Wave>>,
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
    projectiles: Vec<Projectile>,
    pub shock: shock::Shots,
    pub pulse_beam: Option<PulseBeam>,
    pub ordnance: ordnance::Shots,
    pub death: Option<Death>,
    charge: Option<Charge>,
    hit_taken: u16,
    pub hit_flash_until: f32,
    last_pain: f32,
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
            projectiles: Vec::new(),
            shock: shock::Shots::default(),
            pulse_beam: None,
            ordnance: ordnance::Shots::default(),
            death: None,
            charge: None,
            hit_taken: 0,
            hit_flash_until: 0.0,
            last_pain: -1.0,
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
        self.projectiles.clear();
        self.shock = shock::Shots::default();
        self.pulse_beam = None;
        self.ordnance = ordnance::Shots::default();
        self.death = None;
        self.charge = None;
        self.hit_taken = 0;
        self.hit_flash_until = self.time;
        self.last_pain = self.time - 1.0;
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
        let ready = self.health > 0 && matches!(self.weapon_state, WeaponState::Ready);
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
        let mut blasts = self.shock.update(map, dt, self.time, &mut sounds);
        self.ordnance.update(
            map,
            player.as_deref().map(|walker| walker.position),
            self.time,
            dt,
            &mut sounds,
            &mut blasts,
        );
        let charging = self.update_charge(map, camera, &input, ready, &mut sounds, &mut blasts);
        let alternate_shot =
            input.alternate && !matches!(self.weapon, Weapon::Sniper | Weapon::Pulse);
        if !charging
            && self.pulse_beam.is_none()
            && (input.primary || alternate_shot)
            && self.time >= self.next_shot
            && ready
        {
            blasts.extend(self.fire(map, camera, !input.primary && alternate_shot, &mut sounds));
        }
        if let Some(player) = player {
            for blast in blasts {
                if let Some((damage, momentum)) = blast.hit_player(&map.collision, player.position)
                {
                    player.apply_self_damage_momentum(momentum, map.collision.profile);
                    self.splash_damage(damage);
                }
            }
        }
        let mut damage = 0u16;
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
                        damage = damage.saturating_add(
                            ((1.0 - distance / radius).max(0.0) * strength).round() as u16,
                        );
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
        self.splash_damage(damage);
        sounds
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
        sounds.extend(shot_sound(&map.arsenal, weapon, alternate));
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
            return self
                .shock
                .fire(map, camera, alternate, self.time, ammo, sounds);
        }
        let kind = match weapon {
            Weapon::Pulse => Some(ordnance::Kind::Pulse),
            Weapon::Bio => Some(ordnance::Kind::Gel),
            Weapon::Flak if alternate => Some(ordnance::Kind::FlakShell),
            Weapon::Ripper if alternate => Some(ordnance::Kind::RazorAlt),
            Weapon::Ripper => Some(ordnance::Kind::Razor),
            _ => None,
        };
        if let Some(kind) = kind {
            self.next_shot = self.time
                + match weapon {
                    Weapon::Pulse => 0.18,
                    Weapon::Flak => 0.92,
                    Weapon::Ripper => 0.42,
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
        let (cooldown, shot) = weapon.behavior(alternate);
        self.next_shot = self.time + cooldown;
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
                            Weapon::Redeemer => Some((420.0, 170.0)),
                            _ => None,
                        },
                        color,
                        expires: self.time + 5.0,
                    });
                }
            }
        }
        None
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
        assert!(game.projectiles.is_empty());
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
