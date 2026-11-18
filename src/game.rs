use std::{collections::HashMap, sync::Arc};

use crate::collision::Walker;
use crate::map::{Face, Map, Vec3};
use crate::render::Camera;
use crate::sound::{CONTACT_SOUND_RADIUS, SoundEvents, Wave};

pub mod ballistics;
pub mod deathmatch;
pub mod decals;
mod liquid;
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

    fn switch_priority(self) -> u8 {
        // PlayerPawn.WeaponPriority lists these from lowest to highest.
        match self {
            Self::Hammer => 2,
            Self::Enforcer => 3,
            Self::Shock => 5,
            Self::Bio => 6,
            Self::Pulse => 7,
            Self::Sniper => 8,
            Self::Ripper => 9,
            Self::Minigun => 10,
            Self::Flak => 11,
            Self::Rocket => 12,
            Self::Redeemer => 13,
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
    pub muzzle_triangle: Option<usize>,
    pub case_triangle: Option<usize>,
    pub fire: Option<Arc<Wave>>,
    pub alternate: Option<Arc<Wave>>,
    pub shock: Option<shock::Assets>,
    pub status_icon: Option<Arc<crate::texture::Texture>>,
    pub pulse_beam: Vec<Vec<Face>>,
    pub pulse_starter: Vec<Vec<Face>>,
    pub pulse_tip: [Option<Arc<crate::texture::Texture>>; 2],
    pub pulse_scorch: Option<(Arc<crate::texture::Texture>, f32)>,
    pub attack_start: Motion,
    pub attack_end: Motion,
    pub spin_down: Motion,
    pub scope: Option<Arc<crate::texture::Texture>>,
    pub load: Motion,
    pub load_sound: Option<Arc<Wave>>,
    pub charge_loop: Option<Arc<Wave>>,
    pub guided_hud: [Option<Arc<crate::texture::Texture>>; 3],
    pub rocket_lock: [Option<Arc<Wave>>; 2],
    pub lock_reticle: Option<Arc<crate::texture::Texture>>,
    pub rocket: Option<RocketAnimations>,
    pub loaded: Motion,
    pub walking: Motion,
}

#[derive(Default)]
pub struct RocketAnimations {
    pub loads: [Motion; 6],
    pub rotations: [Motion; 6],
    pub fires: [Motion; 6],
    pub rotate_sound: Option<Arc<Wave>>,
}

impl WeaponAssets {
    pub fn muzzle_position(&self, pose: Pose<'_>) -> Option<Vec3> {
        pose.edge_center(self.muzzle_triangle?)
    }

    fn fire_motion(&self, weapon: Weapon, alternate: bool) -> &Motion {
        if alternate && weapon != Weapon::Hammer && !self.alternate_motion.frames.is_empty() {
            &self.alternate_motion
        } else {
            &self.primary_motion
        }
    }

    fn charge_pose(&self, elapsed: f32) -> Option<Pose<'_>> {
        if elapsed >= self.load.duration() && !self.loaded.frames.is_empty() {
            return self.loaded.sample(0.0);
        }
        self.load.hold_pose(elapsed)
    }

    fn pulse_pose(&self, alternate: bool, age: f32) -> Option<Pose<'_>> {
        if !alternate {
            return self.primary_motion.sample_loop(age);
        }
        let windup = self.attack_start.duration();
        if age < windup {
            self.attack_start.sample(age)
        } else {
            self.alternate_motion.sample_loop(age - windup)
        }
    }

    fn pulse_end(&self, alternate: bool) -> &Motion {
        if alternate {
            &self.attack_end
        } else {
            &self.spin_down
        }
    }

    fn rocket_fire_pose(&self, loaded: u8, age: f32, has_ammo: bool) -> Option<Pose<'_>> {
        let rocket = self.rocket.as_ref()?;
        let fire = &rocket.fires[usize::from(loaded.checked_sub(1)?)];
        if age < fire.duration() {
            fire.sample(age)
        } else if has_ammo {
            rocket.loads[0].sample(age - fire.duration())
        } else {
            None
        }
    }
}

impl Pose<'_> {
    fn edge_center(self, triangle: usize) -> Option<Vec3> {
        let triangle = Some(triangle);
        let center = |faces: &[Face]| {
            let index = faces
                .binary_search_by_key(&triangle, |face| face.mesh_triangle)
                .ok()?;
            let vertices = &faces[index].vertices;
            Some((vertices[0] + vertices[2]) * 0.5)
        };
        let current = center(self.current)?;
        let next = center(self.next).unwrap_or(current);
        Some(current + (next - current) * self.fraction)
    }
}

pub struct Flash {
    pub textures: Vec<Arc<crate::texture::Texture>>,
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub length: f32,
    pub half_offset: f32,
}

#[derive(Default)]
pub struct Motion {
    pub frames: Vec<Vec<Face>>,
    pub rate: f32,
}

#[derive(Default)]
pub struct ActorClips {
    pub walk: Motion,
    pub run: Motion,
    pub fire_run: Motion,
    pub strafe: [Motion; 2],
    pub backpedal: Motion,
    pub fire: Motion,
    pub air: Motion,
    pub land: Motion,
    pub hurt: Motion,
    pub swim: Motion,
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

    fn sample_loop(&self, elapsed: f32) -> Option<Pose<'_>> {
        let duration = self.duration();
        if duration <= 0.0 {
            return None;
        }
        let time = elapsed.rem_euclid(duration);
        let mut pose = self.sample(time)?;
        if time * self.rate >= (self.frames.len() - 1) as f32 {
            pose.next = &self.frames[0];
        }
        Some(pose)
    }

    fn hold_pose(&self, elapsed: f32) -> Option<Pose<'_>> {
        self.sample(elapsed.min((self.duration() - 0.001).max(0.0)))
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
    Power(Powerup, u16),
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
            "udamage" => Self::Power(Powerup::Damage, 300),
            "ut_jumpboots" => Self::Power(Powerup::Boots, 3),
            "ut_invisibility" => Self::Power(Powerup::Invisibility, 100),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
pub enum Powerup {
    Boots,
    Damage,
    Invisibility,
}

#[derive(Default)]
pub struct PowerupArt {
    pub boots_on: Option<Arc<Wave>>,
    pub boots_jump: Option<Arc<Wave>>,
    pub amp_fire: [Option<Arc<Wave>>; 2],
    pub amp_out: Option<Arc<Wave>>,
    pub amp_skin: Option<Arc<crate::texture::Texture>>,
    pub invis_skin: Option<Arc<crate::texture::Texture>>,
    pub shield_hit: Option<Arc<Wave>>,
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

    fn pickup_charge(self, armor: &[u16; 3]) -> u16 {
        let available = if matches!(self, Self::Belt) {
            150
        } else {
            150 - armor[Self::Belt as usize]
        };
        available.min(self.charge())
    }

    fn pickup(self, armor: &mut [u16; 3]) -> Option<u16> {
        let charge = self.pickup_charge(armor);
        let gained = charge
            .checked_sub(armor[self as usize])
            .filter(|&gain| gain != 0)?;
        if matches!(self, Self::Belt) {
            armor.fill(0);
        }
        armor[self as usize] = charge;
        Some(gained)
    }
}

fn absorb_damage(armor: &mut [u16; 3], mut damage: u16) -> (u16, bool) {
    let mut shield = false;
    for kind in ArmorKind::ALL {
        let charge = &mut armor[kind as usize];
        let absorbed = (*charge).min((u32::from(damage) * kind.absorption() / 100) as u16);
        *charge -= absorbed;
        damage -= absorbed;
        shield |= matches!(kind, ArmorKind::Belt) && absorbed > 0;
    }
    (damage, shield)
}

fn landing_damage(speed: f32, jump_speed: f32) -> u16 {
    if speed > 1650.0 + jump_speed {
        1000
    } else if speed >= 750.0 + jump_speed {
        (0.15 * (speed - 700.0 - jump_speed)) as u16
    } else {
        0
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
    pub wall: Option<Vec3>,
    pub blocked: bool,
    pub hit_player: bool,
}

impl PulseBeam {
    const RANGE: f32 = 810.0;

    fn trace(map: &Map, camera: &Camera, started: f32) -> Self {
        let direction = camera.look_direction();
        let right = camera.right();
        let up = direction.cross(right);
        let offset = direction * 17.5 + right * 14.0 - up * 10.0;
        let start = camera.position + offset;
        let blocked = map.collision.ray(camera.position, offset).is_some();
        let delta = direction * Self::RANGE;
        let wall = (!blocked)
            .then(|| map.collision.ray_surface(start, delta))
            .flatten();
        Self {
            start,
            end: if blocked {
                start
            } else {
                wall.map_or(start + delta, |(point, _)| point)
            },
            started,
            wall: wall.map(|(_, normal)| normal),
            blocked,
            hit_player: false,
        }
    }
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
    outgoing_scale: u16,
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
        self.health = self
            .health
            .saturating_sub(damage.saturating_mul(self.outgoing_scale));
    }
}

struct Charge {
    weapon: Weapon,
    alternate: bool,
    loaded: u8,
    since: f32,
    next_load: f32,
    tight: bool,
    rotating: bool,
    phase_since: f32,
    bio_size: f32,
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
    pub base_fov: f32,
    pub hud_scale: f32,
    pub show_decals: bool,
    pub score: i16,
    pub pickup_time: f32,
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
    pub boot_charges: u16,
    boots_in_flight: bool,
    amp_until: f32,
    amp_warning: f32,
    invis_until: f32,
    owned: [bool; 11],
    ammo: [u16; 11],
    respawn: Vec<Option<f32>>,
    pub shock: shock::Shots,
    pub pulse_beam: Option<PulseBeam>,
    pub pulse_primary_since: Option<f32>,
    pulse_angle: f32,
    enforcer_windup: Option<f32>,
    attack_end: Option<(Weapon, bool, f32)>,
    pub ordnance: ordnance::Shots,
    pub ballistics: ballistics::Effects,
    pub decals: decals::Marks,
    pub death: Option<Death>,
    charge: Option<Charge>,
    rocket_fired: Option<(u8, f32)>,
    walking_since: Option<f32>,
    hit_taken: u16,
    pub hit_flash_until: f32,
    pub shield_flash_until: f32,
    shield_hit_pending: bool,
    last_pain: f32,
    liquid: liquid::LiquidState,
    hammer_since: Option<f32>,
    was_guiding: bool,
    guide_trigger_held: bool,
    pub guidance_lost_until: f32,
    pub target: Option<TrainingTarget>,
    lock_since: Option<f32>,
    pub rocket_locked: bool,
    pub minigun_firing: bool,
    enforcer_alt_accuracy: f32,
    bullet_cadence: ballistics::Cadence,
    pulse_contact_time: f32,
    pulse_last_contact: f32,
    pulse_contact: bool,
    zoom: SniperZoom,
    next_shot: f32,
    rng: u32,
}

impl Practice {
    pub fn new(map: &Map) -> Self {
        let mut game = Self {
            health: 100,
            base_fov: 90.0,
            hud_scale: 1.0,
            show_decals: true,
            score: 0,
            pickup_time: -1.0,
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
            boot_charges: 0,
            boots_in_flight: false,
            amp_until: 0.0,
            amp_warning: 0.0,
            invis_until: 0.0,
            owned: [false; 11],
            ammo: [0; 11],
            respawn: vec![None; map.props.len()],
            shock: shock::Shots::default(),
            pulse_beam: None,
            pulse_primary_since: None,
            pulse_angle: 0.0,
            enforcer_windup: None,
            attack_end: None,
            ordnance: ordnance::Shots::default(),
            ballistics: ballistics::Effects::default(),
            decals: decals::Marks::default(),
            death: None,
            charge: None,
            rocket_fired: None,
            walking_since: None,
            hit_taken: 0,
            hit_flash_until: 0.0,
            shield_flash_until: 0.0,
            shield_hit_pending: false,
            last_pain: -1.0,
            liquid: liquid::LiquidState::default(),
            hammer_since: None,
            was_guiding: false,
            guide_trigger_held: false,
            guidance_lost_until: 0.0,
            target: None,
            lock_since: None,
            rocket_locked: false,
            minigun_firing: false,
            enforcer_alt_accuracy: 0.4,
            bullet_cadence: ballistics::Cadence::default(),
            pulse_contact_time: 0.0,
            pulse_last_contact: -1.0,
            pulse_contact: false,
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
        self.clear_powerups();
        self.liquid = liquid::LiquidState::default();
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
        self.pulse_primary_since = None;
        self.pulse_angle = 0.0;
        self.enforcer_windup = None;
        self.attack_end = None;
        self.ordnance = ordnance::Shots::default();
        self.ballistics = ballistics::Effects::default();
        self.death = None;
        self.charge = None;
        self.rocket_fired = None;
        self.walking_since = None;
        self.hit_taken = 0;
        self.hit_flash_until = self.time;
        self.shield_flash_until = self.time;
        self.shield_hit_pending = false;
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
        self.pulse_contact_time = 0.0;
        self.pulse_last_contact = -1.0;
        self.pulse_contact = false;
        self.zoom = SniperZoom::default();
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
                let charge = self.charge.as_ref()?;
                if charge.weapon == Weapon::Rocket {
                    let rocket = weapon.rocket.as_ref()?;
                    let motion = if charge.rotating {
                        &rocket.rotations[usize::from(charge.loaded - 1)]
                    } else {
                        &rocket.loads[usize::from(charge.loaded)]
                    };
                    return motion.sample(
                        (self.time - charge.phase_since).min((motion.duration() - 0.001).max(0.0)),
                    );
                }
                weapon.charge_pose(self.time - charge.since)
            })
            .or_else(|| {
                if self.weapon != Weapon::Rocket {
                    return None;
                }
                let (loaded, since) = self.rocket_fired?;
                weapon.rocket_fire_pose(
                    loaded,
                    self.time - since,
                    self.ammo().is_some_and(|ammo| ammo > 0),
                )
            })
            .or_else(|| {
                let beam = self.pulse_beam.as_ref()?;
                weapon.pulse_pose(true, self.time - beam.started)
            })
            .or_else(|| {
                weapon
                    .attack_start
                    .sample(self.time - self.enforcer_windup?)
            })
            .or_else(|| weapon.pulse_pose(false, self.time - self.pulse_primary_since?))
            .or_else(|| {
                let (id, alt, started) = self.attack_end.filter(|(id, _, _)| *id == self.weapon)?;
                let motion = if id == Weapon::Pulse {
                    weapon.pulse_end(alt)
                } else {
                    &weapon.attack_end
                };
                motion.sample(self.time - started)
            })
            .or_else(|| {
                let (_, alternate, at) = self.last_fire.filter(|(id, _, _)| *id == self.weapon)?;
                weapon
                    .fire_motion(self.weapon, alternate)
                    .sample(self.time - at)
            })
            .or_else(|| {
                if self.weapon != Weapon::Bio {
                    return None;
                }
                let duration = weapon.walking.duration();
                let age = self.time - self.walking_since?;
                (duration > 0.0)
                    .then(|| age.rem_euclid(duration))
                    .and_then(|age| weapon.walking.sample(age))
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

    fn switch_priority(&self, weapon: Weapon) -> u8 {
        if weapon == Weapon::Hammer || self.ammo[weapon.ammo_pool()] > 0 {
            weapon.switch_priority()
        } else {
            0
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
        self.base_fov - self.zoom.level * (self.base_fov - 2.0)
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
                    outgoing_scale: self.damage_scale(),
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
        sounds: &mut SoundEvents,
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
        if self.boot_charges != 0 {
            base * 3.0
        } else {
            base
        }
    }

    pub fn damage_amp_active(&self) -> bool {
        self.time < self.amp_until
    }

    pub fn has_jump_boots(&self) -> bool {
        self.boot_charges > 0 || self.boots_in_flight
    }

    pub fn landed(&mut self, speed: f32, base_jump: f32) {
        let jump_speed = if self.has_jump_boots() {
            base_jump * 3.0
        } else {
            base_jump
        };
        self.fall_damage(speed, jump_speed);
        self.boots_in_flight = false;
    }

    pub fn invisible(&self) -> bool {
        self.time < self.invis_until
    }

    fn damage_scale(&self) -> u16 {
        if self.damage_amp_active() { 3 } else { 1 }
    }

    fn clear_powerups(&mut self) {
        self.boot_charges = 0;
        self.boots_in_flight = false;
        self.amp_until = 0.0;
        self.amp_warning = 0.0;
        self.invis_until = 0.0;
    }

    pub fn used_boot_jump(&mut self) -> bool {
        let had_charge = self.boot_charges != 0;
        self.boot_charges = self.boot_charges.saturating_sub(1);
        self.boots_in_flight |= had_charge;
        had_charge
    }

    pub fn fall_damage(&mut self, speed: f32, jump_speed: f32) {
        let damage = landing_damage(speed, jump_speed);
        if damage > 0 {
            self.splash_damage(damage);
        }
    }

    pub fn begin_death(&mut self, position: Vec3, yaw: f32, map: &Map) -> Option<Arc<Wave>> {
        let choice = ((self.jitter() + 1.0) * 1.5) as usize % 3;
        self.score = self.score.saturating_sub(1);
        self.clear_powerups();
        self.death = Some(Death {
            position,
            yaw,
            started: self.time,
            animation: choice,
        });
        self.pulse_beam = None;
        self.pulse_primary_since = None;
        self.enforcer_windup = None;
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
        if std::mem::take(&mut self.shield_hit_pending)
            && let Some(sound) = &map.powerups.shield_hit
        {
            return Some(sound.clone());
        }
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
                let was_empty = self.ammo[index] == 0;
                let add = initial_ammo(arsenal, weapon);
                self.owned[weapon as usize] = true;
                self.ammo[index] = (self.ammo[index] + add).min(max_ammo(arsenal, weapon));
                if (!was_owned || was_empty)
                    && self.switch_priority(weapon) > self.switch_priority(self.pending_weapon())
                {
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
                let Some(gained) = kind.pickup(&mut self.armor) else {
                    return false;
                };
                format!("ARMOR +{gained}")
            }
            Item::Power(Powerup::Boots, charge) => {
                self.boot_charges = self.boot_charges.max(charge);
                "ANTIGRAV BOOTS".into()
            }
            Item::Power(Powerup::Damage, charge) => {
                self.amp_until = self.time + f32::from(charge) * 0.1;
                self.amp_warning = (self.amp_until - 5.0).max(self.time);
                "DAMAGE AMPLIFIER".into()
            }
            Item::Power(Powerup::Invisibility, charge) => {
                self.invis_until = self.time + (f32::from(charge) + 1.0) * 0.5;
                "INVISIBILITY".into()
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
    ) -> SoundEvents {
        self.time += dt;
        if self.weapon == Weapon::Bio
            && player
                .as_deref()
                .is_some_and(|walker| walker.speed() > 10.0)
        {
            self.walking_since.get_or_insert(self.time);
        } else {
            self.walking_since = None;
        }
        self.effects.retain(|effect| effect.expires > self.time);
        self.decals.update(self.time);
        self.decals.follow_movers(&map.movers);
        self.ordnance.follow_movers(&map.movers);
        let mut sounds = SoundEvents::default();
        if self.damage_amp_active() && self.time >= self.amp_warning {
            self.amp_warning += 1.0;
            sounds.extend(map.powerups.amp_out.iter().cloned());
        }
        self.ballistics
            .update(map, self.target.as_ref(), self.time, dt, &mut sounds);
        let pickup_center = player.as_deref().map_or(
            camera.position - Vec3(0.0, 0.0, map.collision.profile.eye_height),
            |walker| walker.position,
        );
        for index in 0..map.props.len() {
            let prop = &mut map.props[index];
            if self.respawn[index].is_some_and(|time| self.time >= time) {
                prop.visible = true;
                self.respawn[index] = None;
            }
            let Some(item) = prop.item else { continue };
            let offset = prop.position - pickup_center;
            let horizontal = offset.0 * offset.0 + offset.1 * offset.1;
            let radius = prop.pickup_radius + crate::collision::PLAYER_RADIUS;
            if collect
                && self.health > 0
                && prop.visible
                && horizontal <= radius * radius
                && offset.2.abs() <= prop.pickup_height + crate::collision::PLAYER_HEIGHT
                && self.pickup(item, &map.arsenal)
            {
                prop.visible = false;
                self.pickup_time = self.time;
                self.respawn[index] = Some(self.time + prop.respawn_time.unwrap_or(25.0));
                if let Some(sound) = &prop.pickup_sound {
                    sounds.push(Arc::clone(sound));
                }
                if matches!(item, Item::Power(Powerup::Boots, _)) {
                    sounds.extend(map.powerups.boots_on.iter().cloned());
                }
            }
        }
        let damage_scale = self.damage_scale();
        if let Some(target) = &mut self.target {
            target.outgoing_scale = damage_scale;
        }
        let shots_before = self.shots;
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
        if self.weapon == Weapon::Rocket
            && self.charge.is_none()
            && matches!(self.weapon_state, WeaponState::Ready)
            && self.ammo().is_some_and(|ammo| ammo > 0)
            && let Some((loaded, since)) = self.rocket_fired
            && let Some(art) = map.arsenal.get(&Weapon::Rocket)
            && let Some(rocket) = &art.rocket
        {
            let reload_at = since + rocket.fires[usize::from(loaded - 1)].duration();
            if self.time - dt < reload_at && self.time >= reload_at {
                sounds.extend(art.load_sound.clone());
                self.next_shot = self.next_shot.max(reload_at + rocket.loads[0].duration());
            }
        }
        let guiding = self.ordnance.guided_view().is_some();
        if self.health > 0
            && !guiding
            && self.charge.is_none()
            && matches!(self.weapon_state, WeaponState::Ready)
            && self.time >= self.next_shot
            && self.switch_priority(self.weapon) == 0
            && let Some(replacement) = self
                .owned()
                .max_by_key(|&weapon| self.switch_priority(weapon))
        {
            self.request_weapon(replacement);
        }
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
        self.update_pulse_primary(
            map,
            ready && input.primary && self.pulse_beam.is_none(),
            &mut sounds,
        );
        let pulse_blast = self.update_pulse_contact(map, dt, &mut sounds);
        if let Some(beam) = &self.pulse_beam
            && let Some(normal) = beam.wall
            && let Some((texture, scale)) = map
                .arsenal
                .get(&Weapon::Pulse)
                .and_then(|art| art.pulse_scorch.as_ref())
        {
            self.decals.project(
                &map.faces,
                beam.end,
                normal,
                texture,
                *scale,
                self.time + 5.0,
            );
        }
        let mut blasts = self.shock.update(
            map,
            dt,
            self.time,
            &mut sounds,
            self.target.as_ref(),
            &mut self.ordnance,
        );
        blasts.extend(pulse_blast);
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
        self.ordnance
            .project_marks(map, &mut self.decals, self.time);
        let charging = self.update_charge(map, camera, &input, ready, &mut sounds, &mut blasts);
        let hammer_hit = self.update_hammer(map, camera, &input, ready, &mut sounds);
        let mut alternate_shot =
            input.alternate && !matches!(self.weapon, Weapon::Sniper | Weapon::Pulse);
        let mut primary_shot = input.primary;
        if self.weapon == Weapon::Enforcer {
            let firing = self.update_enforcer_alt(map, ready, input);
            alternate_shot = firing;
            primary_shot &= !firing && self.enforcer_windup.is_none();
        }
        if !charging
            && self.weapon != Weapon::Hammer
            && self.pulse_beam.is_none()
            && (primary_shot || alternate_shot)
            && self.time >= self.next_shot
            && ready
        {
            blasts.extend(self.fire(map, camera, !primary_shot && alternate_shot, &mut sounds));
        }
        self.ordnance
            .damage_blasts(map, &mut blasts, self.time, &mut sounds);
        if self.was_guiding && self.ordnance.guided_view().is_none() {
            self.guidance_lost_until = self.time + 0.3;
            self.next_shot = self.time + 1.3;
        }
        self.was_guiding = self.ordnance.guided_view().is_some();
        if self.shots != shots_before && damage_scale > 1 {
            let warning = usize::from(self.amp_until - self.time < 5.0);
            sounds.extend(map.powerups.amp_fire[warning].iter().cloned());
        }
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
        let self_hits = self.ordnance.take_self_hits();
        if let Some(player) = player {
            self.update_liquid(map, player.position, camera.position, &mut sounds);
            for (damage, momentum) in self_hits {
                player.apply_self_damage_momentum(momentum, map.collision.profile);
                self.splash_damage(damage.saturating_mul(damage_scale));
            }
            if let Some((damage, momentum)) = hammer_hit {
                player.apply_self_damage_momentum(momentum, map.collision.profile);
                self.splash_damage(damage.saturating_mul(damage_scale));
            }
            for blast in blasts {
                if let Some((damage, momentum)) = blast.hit_player(&map.collision, player.position)
                {
                    player.apply_self_damage_momentum(momentum, map.collision.profile);
                    self.splash_damage(damage.saturating_mul(damage_scale));
                }
            }
            if let Some((damage, impulse)) =
                self.ordnance
                    .nuclear_wave(map, self.time, player.position, 0)
            {
                player.add_velocity(impulse);
                self.splash_damage(damage.saturating_mul(damage_scale));
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
        sounds: &mut SoundEvents,
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
        emit_shot_sound(map, Weapon::Hammer, alternate, None, sounds);
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
                self.attack_end = Some((Weapon::Pulse, true, self.time));
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
                self.attack_end = Some((Weapon::Pulse, true, self.time));
                self.next_shot = self.time + 0.24;
                return;
            }
            *ammo -= 1;
            self.next_shot += 0.24;
            if self.pulse_beam.is_none() || self.next_shot <= self.time {
                self.next_shot = self.time + 0.24;
            }
        }
        let started = self
            .pulse_beam
            .as_ref()
            .map_or(self.time, |beam| beam.started);
        self.pulse_beam = Some(PulseBeam::trace(map, camera, started));
    }

    fn update_pulse_contact(
        &mut self,
        map: &Map,
        dt: f32,
        sounds: &mut SoundEvents,
    ) -> Option<shock::Blast> {
        let Some(beam) = &mut self.pulse_beam else {
            self.pulse_contact_time = 0.0;
            self.pulse_last_contact = -1.0;
            self.pulse_contact = false;
            return None;
        };
        let delta = beam.end - beam.start;
        let pawn = self
            .target
            .as_ref()
            .and_then(|target| target.ray(beam.start, delta));
        let projectile = self
            .ordnance
            .trace_target(&map.ordnance, beam.start, delta)
            .filter(|(_, fraction)| pawn.is_none_or(|pawn| *fraction < pawn));
        let hit = projectile.map(|(_, fraction)| fraction).or(pawn);
        if !beam.blocked
            && let Some(fraction) = hit
        {
            beam.end = beam.start + delta * fraction;
            beam.wall = projectile.map(|_| crate::mover::unit(delta) * -1.0);
            beam.hit_player = projectile.is_none();
            let mut damage = 0;
            if !self.pulse_contact {
                let initial = (0.5 * (self.time - self.pulse_last_contact)).min(0.1);
                damage = (72.0 * initial) as u16;
            }
            self.pulse_contact = true;
            self.pulse_last_contact = self.time;
            self.pulse_contact_time += dt;
            if self.pulse_contact_time > 0.22 {
                damage += (72.0 * self.pulse_contact_time) as u16;
                self.pulse_contact_time = 0.0;
            }
            if let Some((index, _)) = projectile {
                return self.ordnance.damage_target(
                    &map.ordnance,
                    index,
                    damage,
                    self.time,
                    sounds,
                );
            }
            if let Some(target) = &mut self.target {
                target.damage(damage);
            }
        } else {
            if self.pulse_contact_time > 0.0 {
                if let Some(target) = &mut self.target {
                    target.damage((72.0 * self.pulse_contact_time) as u16);
                }
                self.pulse_contact_time = 0.0;
            }
            self.pulse_contact = false;
        }
        None
    }

    fn update_pulse_primary(&mut self, map: &Map, held: bool, sounds: &mut SoundEvents) {
        if held && self.weapon == Weapon::Pulse && self.ammo().is_some_and(|ammo| ammo > 0) {
            if self.pulse_primary_since.is_none() && self.time >= self.next_shot {
                self.pulse_primary_since = Some(self.time);
                self.pulse_angle = 0.0;
                self.attack_end = None;
            }
        } else if self.pulse_primary_since.take().is_some() {
            self.attack_end = Some((Weapon::Pulse, false, self.time));
            if self.last_fire.is_some_and(|(id, _, _)| id == Weapon::Pulse) {
                self.last_fire = None;
            }
            sounds.extend(
                map.arsenal
                    .get(&Weapon::Pulse)
                    .and_then(|art| art.load_sound.clone()),
            );
        }
    }

    fn update_enforcer_alt(&mut self, map: &Map, ready: bool, input: Input) -> bool {
        let art = map.arsenal.get(&Weapon::Enforcer);
        let held = input.alternate && !input.primary;
        let repeating = self
            .last_fire
            .is_some_and(|(id, alt, _)| id == Weapon::Enforcer && alt);
        if !ready {
            self.enforcer_windup = None;
            self.enforcer_alt_accuracy = 0.4;
            return false;
        }
        if self.enforcer_windup.is_some() {
            if self.time < self.next_shot {
                return false;
            }
            self.enforcer_windup = None;
            return true;
        }
        if repeating {
            if self.time < self.next_shot || (held && self.ammo().is_some_and(|ammo| ammo > 0)) {
                return true;
            }
            self.attack_end = Some((Weapon::Enforcer, true, self.time));
            self.next_shot = self.time + art.map_or(0.0, |art| art.attack_end.duration());
            self.last_fire = None;
            self.enforcer_alt_accuracy = 0.4;
        } else if held && self.time >= self.next_shot && self.ammo().is_some_and(|ammo| ammo > 0) {
            self.attack_end = None;
            let duration = art.map_or(0.0, |art| art.attack_start.duration());
            if duration == 0.0 {
                return true;
            }
            self.enforcer_windup = Some(self.time);
            self.next_shot = self.time + duration;
        }
        false
    }

    fn update_charge(
        &mut self,
        map: &Map,
        camera: &Camera,
        input: &Input,
        ready: bool,
        sounds: &mut SoundEvents,
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
        let animations = map.arsenal.get(&weapon).and_then(|art| art.rocket.as_ref());
        let rotate_duration = |index: u8| {
            animations
                .map_or(7.0 / 15.0, |art| {
                    art.rotations[usize::from(index)].duration()
                })
                .max(0.001)
        };
        let load_duration = |index: u8| {
            animations
                .map_or(7.0 / 15.0, |art| art.loads[usize::from(index)].duration())
                .max(0.001)
        };
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
                next_load: self.time + if rocket { rotate_duration(0) } else { 0.5 },
                tight: rocket && !alternate && input.alternate,
                rotating: true,
                phase_since: self.time,
                bio_size: 0.0,
            });
            if rocket {
                self.rocket_fired = None;
                if *ammo > 0 {
                    sounds.extend(animations.and_then(|art| art.rotate_sound.clone()));
                }
            } else {
                sounds.extend(load_sound());
                return true;
            }
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
        if rocket {
            while held && self.time >= charge.next_load && charge.loaded < 6 && *ammo > 0 {
                charge.phase_since = charge.next_load;
                if charge.rotating {
                    charge.rotating = false;
                    charge.next_load += load_duration(charge.loaded);
                    sounds.extend(load_sound());
                } else {
                    *ammo -= 1;
                    charge.loaded += 1;
                    charge.rotating = true;
                    charge.next_load += rotate_duration(charge.loaded - 1);
                    if *ammo > 0 {
                        sounds.extend(animations.and_then(|art| art.rotate_sound.clone()));
                    }
                }
            }
        } else if self.time > charge.next_load && charge.bio_size < 4.1 && *ammo > 0 {
            *ammo -= 1;
            charge.loaded += 1;
            charge.bio_size = (charge.bio_size + self.time - charge.phase_since).min(4.1);
            charge.phase_since = self.time;
            charge.next_load = self.time + 0.5;
        }
        if held && (!rocket || (charge.loaded < 6 && *ammo > 0)) {
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
            (1.0 + 0.8 * charge.bio_size) * 0.5
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
        if rocket {
            self.rocket_fired = Some((charge.loaded, self.time));
        }
        self.next_shot = self.time
            + if rocket {
                animations.map_or(0.6, |art| {
                    art.fires[usize::from(charge.loaded - 1)].duration()
                }) + if self.ammo[weapon.ammo_pool()] > 0 {
                    load_duration(0)
                } else {
                    0.0
                }
            } else {
                map.arsenal
                    .get(&Weapon::Bio)
                    .map_or(0.75, |art| art.alternate_motion.duration())
            };
        emit_shot_sound(map, weapon, charge.alternate, None, sounds);
        true
    }

    fn splash_damage(&mut self, damage: u16) {
        let (damage, shield) = absorb_damage(&mut self.armor, damage);
        if shield {
            self.shield_flash_until = self.time + 0.16;
            self.shield_hit_pending = true;
        }
        self.health_damage(damage);
    }

    fn health_damage(&mut self, damage: u16) {
        if damage > 0 {
            self.health = self.health.saturating_sub(damage);
            self.hit_taken = self.hit_taken.saturating_add(damage);
            self.hit_flash_until = self.time + 0.20;
            self.notice(format!("DAMAGE -{damage}"));
        }
    }

    fn update_liquid(&mut self, map: &Map, center: Vec3, eye: Vec3, sounds: &mut SoundEvents) {
        if self.health == 0 {
            return;
        }
        match self.liquid.step(map, center, eye, self.time, true, sounds) {
            Some(liquid::Impact::Splash(damage)) => self.splash_damage(damage),
            Some(liquid::Impact::Drown(damage)) => self.health_damage(damage),
            Some(liquid::Impact::Heal(amount)) if self.health < 100 => {
                self.health = self.health.saturating_add(amount).min(100);
            }
            _ => {}
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
        sounds: &mut SoundEvents,
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
        emit_shot_sound(map, weapon, alternate, None, sounds);
        if weapon == Weapon::Shock {
            self.next_shot = self.time + shot_interval(map, weapon, alternate).unwrap();
            return self.shock.fire(
                map,
                camera,
                alternate,
                self.time,
                ammo,
                sounds,
                self.target.as_mut(),
                &mut self.ordnance,
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
            let variation = if weapon == Weapon::Pulse {
                let angle = self.pulse_angle;
                self.pulse_angle += 1.8;
                angle
            } else {
                0.0
            };
            self.next_shot = self.time + shot_interval(map, weapon, alternate).unwrap_or(0.36);
            return self.ordnance.fire(
                map,
                camera,
                kind,
                camera.look_direction(),
                0.0,
                1.0,
                self.time,
                variation,
                sounds,
            );
        }
        if weapon == Weapon::Flak {
            self.next_shot = self.time + shot_interval(map, weapon, alternate).unwrap();
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
        self.trace_bullet(map, camera, weapon, alternate, sounds)
    }

    fn trace_bullet(
        &mut self,
        map: &Map,
        camera: &Camera,
        weapon: Weapon,
        alternate: bool,
        sounds: &mut SoundEvents,
    ) -> Option<shock::Blast> {
        let profile = ballistics::BulletProfile::for_weapon(
            map,
            weapon,
            alternate,
            &mut self.enforcer_alt_accuracy,
            || ballistics::jitter(&mut self.rng),
        )?;
        self.next_shot = self.time + profile.interval;
        let damage = profile.damage;
        let (start, delta) = profile.ray(camera, || self.jitter());
        let mut wall = map.collision.ray_surface(start, delta);
        let mut visible = wall.map_or(delta, |(point, _)| point - start);
        let projectile = self.ordnance.trace_target(&map.ordnance, start, visible);
        let pawn_fraction = self
            .target
            .as_ref()
            .and_then(|target| target.ray(start, visible));
        let projectile_hit = projectile
            .is_some_and(|(_, fraction)| pawn_fraction.is_none_or(|pawn| fraction < pawn));
        let blast = if let Some((index, fraction)) = projectile
            && pawn_fraction.is_none_or(|pawn| fraction < pawn)
        {
            visible = visible * fraction;
            wall = None;
            self.ordnance.damage_target(
                &map.ordnance,
                index,
                damage * self.damage_scale(),
                self.time,
                sounds,
            )
        } else {
            None
        };
        if !projectile_hit
            && let Some(target) = &mut self.target
            && let Some(fraction) = target.ray(start, visible)
        {
            let point = start + visible * fraction;
            let headshot = weapon == Weapon::Sniper && point.2 - target.position.2 > 0.62 * 39.0;
            target.damage(if headshot { 100 } else { damage });
            sounds.extend_at(
                map.ballistics.flesh.iter().cloned(),
                point,
                CONTACT_SOUND_RADIUS,
                1.0,
            );
            visible = point - start;
            wall = None;
        }
        let case_origin = map
            .arsenal
            .get(&weapon)
            .and_then(|art| self.pose(art).edge_center(art.case_triangle?))
            .filter(|point| point.0 > 0.0);
        let seed = &mut self.rng;
        self.ballistics.bullet(
            map,
            &mut self.decals,
            camera,
            weapon,
            self.time,
            ballistics::Bullet {
                start,
                delta,
                visible,
                wall,
                case_origin,
            },
            &mut self.bullet_cadence,
            sounds,
            || ballistics::jitter(seed),
        );
        blast
    }

    fn jitter(&mut self) -> f32 {
        ballistics::jitter(&mut self.rng)
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
    if weapon == Weapon::Bio {
        return if alt {
            assets.fire.clone()
        } else {
            assets.alternate.clone()
        };
    }
    if alt && weapon != Weapon::Enforcer && assets.alternate.is_some() {
        return assets.alternate.clone();
    }
    assets.fire.clone()
}

fn emit_shot_sound(
    map: &Map,
    weapon: Weapon,
    alternate: bool,
    source: Option<Vec3>,
    sounds: &mut SoundEvents,
) {
    let wave = shot_sound(&map.arsenal, weapon, alternate);
    if let Some(position) = source {
        sounds.extend_at(wave, position, CONTACT_SOUND_RADIUS, 1.0);
    } else if !matches!(weapon, Weapon::Minigun | Weapon::Pulse) {
        sounds.extend(wave);
    }
}

fn shot_interval(map: &Map, weapon: Weapon, alternate: bool) -> Option<f32> {
    let duration = || {
        map.arsenal
            .get(&weapon)
            .map(|art| art.fire_motion(weapon, alternate).duration())
            .filter(|&duration| duration > 0.0)
    };
    Some(match (weapon, alternate) {
        (Weapon::Pulse, false) => 0.18,
        (Weapon::Pulse, true) => 0.24,
        (Weapon::Flak, false) => 0.82,
        (Weapon::Flak, true) => 0.92,
        (Weapon::Ripper, _) => 0.42,
        (Weapon::Redeemer, _) => 1.3,
        (Weapon::Bio, false) => duration().unwrap_or(0.3 / 1.05),
        (Weapon::Bio, true) => duration().unwrap_or(0.75),
        (Weapon::Shock, _) => duration().unwrap_or_else(|| shock::fallback_cooldown(alternate)),
        _ => return None,
    })
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
    fn deck16_rocket_and_bio_animation_and_sound_sequences() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let camera = Camera::from_start(map.starts[11]);
        let rocket = map.arsenal[&Weapon::Rocket].rocket.as_ref().unwrap();
        assert_eq!(
            rocket.loads.each_ref().map(|clip| clip.frames.len()),
            [7, 7, 7, 7, 7, 6]
        );
        assert_eq!(
            rocket.rotations.each_ref().map(|clip| clip.frames.len()),
            [7; 6]
        );
        // The class defaults deliberately reuse Fire2/Fire3 for five/six rounds.
        assert_eq!(
            rocket.fires.each_ref().map(|clip| clip.frames.len()),
            [8, 11, 10, 11, 11, 10]
        );
        assert!(
            rocket
                .loads
                .iter()
                .chain(&rocket.rotations)
                .all(|clip| (clip.rate - 15.0).abs() < 0.001)
        );
        assert!(
            rocket
                .fires
                .iter()
                .all(|clip| (clip.rate - 18.0).abs() < 0.001)
        );
        assert!(rocket.rotate_sound.is_some());
        for alternate in [false, true] {
            for count in 1..=6 {
                let mut game = Practice::new(&map);
                game.weapon = Weapon::Rocket;
                game.weapon_state = WeaponState::Ready;
                game.ammo[Weapon::Rocket.ammo_pool()] = 20;
                let mut sounds = SoundEvents::default();
                let mut blasts = Vec::new();
                let held = fire_input(!alternate, alternate);
                game.update_charge(&map, &camera, &held, true, &mut sounds, &mut blasts);
                assert!(Arc::ptr_eq(
                    sounds.waves().next_back().unwrap(),
                    rocket.rotate_sound.as_ref().unwrap()
                ));
                while game
                    .charge
                    .as_ref()
                    .is_some_and(|charge| charge.loaded < count)
                {
                    let charge = game.charge.as_ref().unwrap();
                    let rotating = charge.rotating;
                    game.time = charge.next_load + 0.0001;
                    sounds.clear();
                    game.update_charge(&map, &camera, &held, true, &mut sounds, &mut blasts);
                    if rotating {
                        let charge = game.charge.as_ref().unwrap();
                        assert!(!charge.rotating);
                        assert!(std::ptr::eq(
                            game.pose(&map.arsenal[&Weapon::Rocket]).current,
                            rocket.loads[usize::from(charge.loaded)].frames[0].as_slice()
                        ));
                        assert!(Arc::ptr_eq(
                            sounds.waves().next_back().unwrap(),
                            map.arsenal[&Weapon::Rocket].load_sound.as_ref().unwrap()
                        ));
                    }
                }
                if count < 6 {
                    sounds.clear();
                    game.update_charge(
                        &map,
                        &camera,
                        &fire_input(false, false),
                        true,
                        &mut sounds,
                        &mut blasts,
                    );
                }
                assert!(game.charge.is_none());
                assert_eq!(game.shots, u32::from(count));
                assert_eq!(game.ammo(), Some(20 - u16::from(count)));
                assert_eq!(game.ordnance.active_count(), usize::from(count));
                assert!(sounds.waves().any(|sound| {
                    Arc::ptr_eq(
                        sound,
                        shot_sound(&map.arsenal, Weapon::Rocket, alternate)
                            .as_ref()
                            .unwrap(),
                    )
                }));
                let fire = &rocket.fires[usize::from(count - 1)];
                assert!(std::ptr::eq(
                    game.pose(&map.arsenal[&Weapon::Rocket]).current,
                    fire.frames[0].as_slice()
                ));
                assert!(
                    (game.next_shot - game.time - fire.duration() - rocket.loads[0].duration())
                        .abs()
                        < 0.001
                );
                game.time += fire.duration() + 0.001;
                assert!(std::ptr::eq(
                    game.pose(&map.arsenal[&Weapon::Rocket]).current,
                    rocket.loads[0].frames[0].as_slice()
                ));
            }
        }
        let bio = &map.arsenal[&Weapon::Bio];
        assert_eq!(bio.load.frames.len(), 30);
        assert!((bio.load.duration() - 30.0 / 7.2).abs() < 0.001);
        assert_eq!(bio.loaded.frames.len(), 1);
        assert_eq!(bio.walking.frames.len(), 18);
        assert!((bio.primary_motion.duration() - 9.0 / 31.5).abs() < 0.001);
        assert!((bio.alternate_motion.duration() - 0.75).abs() < 0.001);
        assert!(Arc::ptr_eq(
            bio.fire.as_ref().unwrap(),
            bio.alternate.as_ref().unwrap()
        ));
        let mut game = Practice::new(&map);
        game.weapon = Weapon::Bio;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Bio.ammo_pool()] = 20;
        let mut sounds = SoundEvents::default();
        let mut blasts = Vec::new();
        game.update_charge(
            &map,
            &camera,
            &fire_input(false, true),
            true,
            &mut sounds,
            &mut blasts,
        );
        assert_eq!(sounds.len(), 1);
        assert!(Arc::ptr_eq(
            sounds.waves().next().unwrap(),
            bio.load_sound.as_ref().unwrap()
        ));
        for _ in 0..10 {
            game.time += 0.51;
            sounds.clear();
            game.update_charge(
                &map,
                &camera,
                &fire_input(false, true),
                true,
                &mut sounds,
                &mut blasts,
            );
            assert!(
                sounds.is_empty(),
                "BioAltRep plays once rather than restarting per ammo"
            );
        }
        let charge = game.charge.as_ref().unwrap();
        assert!((charge.bio_size - 4.1).abs() < 0.001);
        assert_eq!(charge.loaded, 10);
        assert_eq!(game.ammo(), Some(10));
        assert!(std::ptr::eq(
            game.pose(bio).current,
            bio.loaded.frames[0].as_slice()
        ));
        game.update_charge(
            &map,
            &camera,
            &fire_input(false, false),
            true,
            &mut sounds,
            &mut blasts,
        );
        assert_eq!(game.ordnance.active_count(), 1);
        assert_eq!(sounds.len(), 1);
        assert!(Arc::ptr_eq(
            sounds.waves().next().unwrap(),
            bio.fire.as_ref().unwrap()
        ));
        assert!((game.next_shot - game.time - 0.75).abs() < 0.001);

        // The last rocket fires immediately; there is no empty reload animation or sound.
        let mut game = Practice::new(&map);
        game.weapon = Weapon::Rocket;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Rocket.ammo_pool()] = 1;
        let sounds = game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(true, false),
            false,
            None,
        );
        assert_eq!(game.shots, 1);
        assert!(game.charge.is_none());
        assert_eq!(sounds.len(), 1);
        assert!(
            (game.next_shot
                - game.time
                - map.arsenal[&Weapon::Rocket].rocket.as_ref().unwrap().fires[0].duration())
            .abs()
                < 0.001
        );

        let mut game = Practice::new(&map);
        game.weapon = Weapon::Rocket;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Rocket.ammo_pool()] = 8;
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(true, false),
            false,
            None,
        );
        // Releasing during Load2 must fire one rocket, not grant a second round early.
        let rotate = map.arsenal[&Weapon::Rocket]
            .rocket
            .as_ref()
            .unwrap()
            .rotations[0]
            .duration();
        let sounds = game.update(
            &mut map,
            &camera,
            rotate + 0.01,
            fire_input(true, false),
            false,
            None,
        );
        assert!(!game.charge.as_ref().unwrap().rotating);
        assert!(sounds.waves().any(|sound| Arc::ptr_eq(
            sound,
            map.arsenal[&Weapon::Rocket].load_sound.as_ref().unwrap()
        )));
        game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(false, false),
            false,
            None,
        );
        assert_eq!(game.shots, 1);
        assert_eq!(game.ammo(), Some(7));
        let fire = map.arsenal[&Weapon::Rocket].rocket.as_ref().unwrap().fires[0].duration();
        let sounds = game.update(
            &mut map,
            &camera,
            fire + 0.001,
            fire_input(true, false),
            false,
            None,
        );
        assert!(
            game.charge.is_none(),
            "the firing/reload sequence blocks another shot"
        );
        assert!(sounds.waves().any(|sound| Arc::ptr_eq(
            sound,
            map.arsenal[&Weapon::Rocket].load_sound.as_ref().unwrap()
        )));
        let load = map.arsenal[&Weapon::Rocket].rocket.as_ref().unwrap().loads[0].duration();
        game.update(
            &mut map,
            &camera,
            load,
            fire_input(true, false),
            false,
            None,
        );
        assert!(game.charge.is_some());
        assert_eq!(game.ammo(), Some(6));

        let mut game = Practice::new(&map);
        game.weapon = Weapon::Bio;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Bio.ammo_pool()] = 20;
        let sounds = game.update(
            &mut map,
            &camera,
            0.01,
            fire_input(true, false),
            false,
            None,
        );
        assert_eq!(game.shots, 1);
        assert!(sounds.waves().any(|sound| Arc::ptr_eq(
            sound,
            map.arsenal[&Weapon::Bio].alternate.as_ref().unwrap()
        )));
        let cooldown = map.arsenal[&Weapon::Bio].primary_motion.duration();
        game.update(
            &mut map,
            &camera,
            cooldown * 0.5,
            fire_input(true, false),
            false,
            None,
        );
        assert_eq!(game.shots, 1);
        game.update(
            &mut map,
            &camera,
            cooldown * 0.5 + 0.001,
            fire_input(true, false),
            false,
            None,
        );
        assert_eq!(game.shots, 2);
        let mut walker = Walker::spawn(&map.collision, map.starts[11].position);
        walker.add_velocity(Vec3(40.0, 0.0, 0.0));
        game.update(
            &mut map,
            &camera,
            cooldown + 0.01,
            fire_input(false, false),
            false,
            Some(&mut walker),
        );
        assert!(std::ptr::eq(
            game.pose(&map.arsenal[&Weapon::Bio]).current,
            map.arsenal[&Weapon::Bio].walking.frames[0].as_slice()
        ));
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn deck16_bullet_holes_attach_to_walls_and_survive_respawn() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        assert!(map.ballistics.pock.iter().all(Option::is_some));
        assert!((map.ballistics.pock_scale - 0.19).abs() < 0.001);
        let mut game = Practice::new(&map);
        let mut camera = Camera::from_start(map.starts[11]);
        for _ in 0..8 {
            game.trace_bullet(
                &map,
                &camera,
                Weapon::Enforcer,
                false,
                &mut SoundEvents::default(),
            );
            if game.decals.visible().next().is_some() {
                break;
            }
            camera.yaw += std::f32::consts::FRAC_PI_4;
        }
        assert!(
            game.decals.visible().next().is_some(),
            "a bullet should mark the Deck16 BSP"
        );
        let mut frame = crate::render::Frame::hardware(&map);
        frame.draw_practice(&map, &camera, game.time, &game);
        use crate::render::commands::{Layer, Mode};
        assert!(
            frame.commands.as_ref().unwrap().passes[Layer::Effects as usize]
                .batches()
                .any(|(material, _)| material.mode == Mode::Modulated)
        );
        game.reset_player(&map.arsenal);
        assert!(game.decals.visible().next().is_some());
        game.decals.update(24.0);
        assert!(game.decals.visible().next().is_none());
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
            let mut sounds = SoundEvents::default();
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
                    (Weapon::Enforcer, true) => {
                        let enforcer = &map.arsenal[&Weapon::Enforcer];
                        let windup = enforcer.attack_start.duration();
                        assert_eq!(enforcer.attack_start.frames.len(), 6);
                        assert_eq!(enforcer.attack_end.frames.len(), 7);
                        assert_eq!(enforcer.flash.as_ref().unwrap().half_offset, 64.0);
                        assert_eq!(game.shots, 0);
                        assert_eq!(game.ammo(), Some(10));
                        let sounds = game.update(
                            &mut map,
                            &camera,
                            windup + 0.01,
                            fire_input(false, false),
                            false,
                            None,
                        );
                        assert_eq!(game.shots, 1);
                        assert_eq!(game.ammo(), Some(9));
                        assert!(
                            game.last_fire
                                .is_some_and(|(id, alt, _)| id == Weapon::Enforcer && alt)
                        );
                        assert!(Arc::ptr_eq(
                            sounds.waves().next().unwrap(),
                            map.arsenal[&Weapon::Enforcer].fire.as_ref().unwrap()
                        ));
                        game.update(
                            &mut map,
                            &camera,
                            1.0,
                            fire_input(false, false),
                            false,
                            None,
                        );
                        assert_eq!(game.attack_end, Some((Weapon::Enforcer, true, game.time)));
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
                        if !matches!(weapon, Weapon::Minigun | Weapon::Pulse) {
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
        assert_eq!(pulse.pulse_starter.len(), 5);
        assert!(pulse.pulse_starter.iter().all(|faces| !faces.is_empty()));
        assert!(pulse.pulse_tip.iter().all(Option::is_some));
        assert!((pulse.pulse_scorch.as_ref().unwrap().1 - 0.2).abs() < 0.001);
        assert_eq!(pulse.attack_start.frames.len(), 15);
        assert_eq!(pulse.attack_end.frames.len(), 5);
        assert_eq!(pulse.primary_motion.frames.len(), 40);
        assert_eq!(pulse.spin_down.frames.len(), 25);
        assert!(pulse.fire.is_some() && pulse.load_sound.is_some());
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
        let barrel = |motion: &Motion, frame: f32| {
            model
                .muzzle_position(motion.sample(frame / motion.rate).unwrap())
                .unwrap()
        };
        let normal = barrel(&model.primary_motion, 0.0);
        let sideways = barrel(&model.alternate_motion, 0.0);
        assert!(sideways.1 > normal.1 + 30.0 && sideways.2 < normal.2 - 30.0);
        let recoiled = barrel(&model.alternate_motion, 1.0);
        let between = barrel(&model.alternate_motion, 0.5);
        assert!((recoiled - sideways).dot(recoiled - sideways) > 1.0);
        assert!(
            (between - (sideways + recoiled) * 0.5).dot(between - (sideways + recoiled) * 0.5)
                < 0.001
        );
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

    #[test]
    #[ignore = "requires the local GOTY map and game packages"]
    fn deck16_powerups_use_stock_charges_and_scale_combat() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        let mut practice = Practice::new(&map);
        let mut camera = Camera::from_start(map.starts[6]);
        let idle = || Input {
            primary: false,
            alternate: false,
            slot: None,
            cycle: 0,
        };
        let boots = map
            .props
            .iter()
            .position(|prop| matches!(prop.item, Some(Item::Power(Powerup::Boots, 3))))
            .unwrap();
        let bounds = map.props[boots].pickup_radius + crate::collision::PLAYER_RADIUS;
        camera.position = map.props[boots].position + Vec3(bounds + 1.0, 0.0, 27.0);
        practice.update(&mut map, &camera, 0.016, idle(), true, None);
        assert!(
            map.props[boots].visible,
            "outside the authored pickup cylinder"
        );
        camera.position = map.props[boots].position + Vec3(0.0, 0.0, 27.0);
        practice.update(&mut map, &camera, 0.016, idle(), true, None);
        assert!(!map.props[boots].visible);
        assert_eq!(practice.boot_charges, 3);
        practice.update(&mut map, &camera, 35.0, idle(), false, None);
        assert_eq!(practice.boot_charges, 3, "charges are not a timed buff");
        assert!(map.props[boots].visible, "map pickup respawned");
        let jump = map.collision.profile.jump_speed;
        for _ in 0..3 {
            assert_eq!(practice.jump_speed(jump), jump * 3.0);
            assert!(practice.used_boot_jump());
            practice.landed(0.0, jump);
        }
        assert_eq!(practice.jump_speed(jump), jump);
        assert!(!practice.has_jump_boots());

        let amp = map
            .props
            .iter()
            .position(|prop| matches!(prop.item, Some(Item::Power(Powerup::Damage, 300))))
            .unwrap();
        camera.position = map.props[amp].position + Vec3(0.0, 0.0, 27.0);
        practice.update(&mut map, &camera, 0.016, idle(), true, None);
        assert!(!map.props[amp].visible);
        assert!((practice.amp_until - practice.time - 30.0).abs() < 0.001);
        assert!(map.powerups.amp_skin.is_some() && map.hud.amp.is_some());
        assert!(map.powerups.amp_fire.iter().all(Option::is_some));
        assert!(map.powerups.boots_on.is_some());
        assert!(map.powerups.boots_jump.is_some());
        assert!(map.powerups.shield_hit.is_some());
        let camera = map
            .starts
            .iter()
            .find_map(|&start| {
                let camera = Camera::from_start(start);
                practice.place_training_target(&map, &camera);
                practice.target.as_ref().map(|_| camera)
            })
            .expect("unobstructed Deck16 spawn");
        practice.weapon = Weapon::Sniper;
        practice.weapon_state = WeaponState::Ready;
        practice.next_shot = practice.time;
        let mut shoot = idle();
        shoot.primary = true;
        practice.update(&mut map, &camera, 0.016, shoot, false, None);
        assert_eq!(
            practice.target.as_ref().unwrap().health,
            0,
            "one amplified rifle shot kills"
        );
        practice.update(&mut map, &camera, 30.0, idle(), false, None);
        assert!(!practice.damage_amp_active());
        assert!(!map.props[amp].visible);
        practice.ammo.fill(0);
        practice.ammo[Weapon::Pulse.ammo_pool()] = 1;
        practice.ammo[Weapon::Ripper.ammo_pool()] = 1;
        practice.update(&mut map, &camera, 0.016, idle(), false, None);
        assert_eq!(practice.pending_weapon(), Weapon::Ripper);
        assert!(practice.pickup(Item::Gun(Weapon::Shock), &map.arsenal));
        assert_eq!(
            practice.pending_weapon(),
            Weapon::Ripper,
            "lower ranked pickup cannot steal focus"
        );
    }
}
