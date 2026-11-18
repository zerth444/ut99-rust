//! Offline arena simulation. The window supplies input and displays snapshots.

mod ai;
mod arsenal;
mod guidance;
mod navigation;
mod timestep;

use super::{
    Item, Pose, Powerup, Weapon, WeaponAssets, ballistics, decals,
    liquid::{Impact, LiquidState},
    shock::cylinder_hit,
};
use crate::{
    collision::{PLAYER_HEIGHT, PLAYER_RADIUS, Walker},
    map::{Map, Vec3},
    mover::{LiftStop, unit},
    render::Camera,
    sound::{BLAST_SOUND_RADIUS, CONTACT_SOUND_RADIUS, SoundEvents},
};
pub use ai::Difficulty;
use arsenal::{ChargeProfile, ChargeShape, Inventory, Path};
use guidance::Pilot;
use navigation::{JumpControl, JumpProgress, LiftRide, Navigation, Step, WAYPOINT_RADIUS};
pub use timestep::Timestep;

const RESPAWN_DELAY: f32 = 1.25;
const SPAWN_PROTECTION: f32 = 0.75;
const ENVIRONMENT_CREDIT_WINDOW: f32 = 3.0;

#[derive(Clone, Copy)]
pub struct Rules {
    pub bots: usize,
    pub frag_limit: i16,
    pub seconds: u16,
    pub difficulty: Difficulty,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            bots: 3,
            frag_limit: 10,
            seconds: 300,
            difficulty: Difficulty::default(),
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct Command {
    pub movement: [f32; 2],
    pub walk: bool,
    pub dodge: Option<[f32; 2]>,
    pub jump: bool,
    pub normal_jump: bool,
    pub primary: bool,
    pub alternate: bool,
    pub restart: bool,
    pub look: [f32; 2],
    pub slot: Option<u8>,
    pub cycle: i8,
    pub preferred: Option<Weapon>,
    pub release_charge: bool,
}

pub struct Fighter {
    pub walker: Walker,
    pub yaw: f32,
    pitch: f32,
    pub health: u16,
    armor: [u16; 3],
    power: ActivePowerups,
    inventory: Inventory,
    pub weapon: Weapon,
    pub selected_at: f32,
    pickup_time: f32,
    pub last_alt: bool,
    pub zoomed: bool,
    alt_held: bool,
    charge: Option<Charge>,
    charge_locked: Option<bool>,
    wait_for_fire_release: bool,
    pub score: i16,
    pub deaths: u16,
    pub fired_at: f32,
    pub(crate) fired_count: u32,
    enforcer_accuracy: f32,
    bullet_cadence: ballistics::Cadence,
    pub fell_at: Option<f32>,
    pub(crate) hurt_at: f32,
    pub(crate) shield_at: f32,
    landed_at: f32,
    airborne_since: Option<f32>,
    stride_time: f32,
    gait: Gait,
    previous_body: (Vec3, f32),
    protected_until: f32,
    ready_at: f32,
    liquid: LiquidState,
    last_hostile_impulse: Option<(usize, f32)>,
    portal: Option<usize>,
    spawn: usize,
    bot: Bot,
}

impl Fighter {
    fn spawn(map: &Map, spawn: usize, time: f32) -> Self {
        let start = map.starts[spawn];
        let walker = Walker::spawn(&map.collision, start.position);
        Self {
            previous_body: (walker.position, start.yaw),
            walker,
            yaw: start.yaw,
            pitch: 0.0,
            health: 100,
            armor: [0; 3],
            power: ActivePowerups::default(),
            inventory: Inventory::new(map),
            weapon: Weapon::Enforcer,
            selected_at: f32::NEG_INFINITY,
            pickup_time: f32::NEG_INFINITY,
            last_alt: false,
            zoomed: false,
            alt_held: false,
            charge: None,
            charge_locked: None,
            wait_for_fire_release: false,
            score: 0,
            deaths: 0,
            fired_at: f32::NEG_INFINITY,
            fired_count: 0,
            enforcer_accuracy: 0.4,
            bullet_cadence: ballistics::Cadence::default(),
            fell_at: None,
            hurt_at: f32::NEG_INFINITY,
            shield_at: f32::NEG_INFINITY,
            landed_at: f32::NEG_INFINITY,
            airborne_since: None,
            stride_time: 0.0,
            gait: Gait::Idle,
            protected_until: time + SPAWN_PROTECTION,
            ready_at: time,
            liquid: LiquidState::default(),
            last_hostile_impulse: None,
            portal: None,
            spawn,
            bot: Bot::default(),
        }
    }

    pub fn camera(&self, map: &Map) -> Camera {
        let mut camera = Camera {
            position: self.walker.position,
            yaw: self.yaw,
            pitch: self.pitch,
        };
        camera.position = self.walker.eye(map.collision.profile, camera.right());
        camera
    }

    pub fn ammo(&self) -> Option<u16> {
        (self.weapon != Weapon::Hammer).then(|| self.inventory.ammo(self.weapon))
    }

    pub fn charge_status(&self, time: f32) -> Option<(u16, f32)> {
        let charge = self.charge?;
        let elapsed = time - charge.since;
        let cost = arsenal::shot(self.weapon, charge.alternate).ammo;
        let units = charge
            .profile
            .units(elapsed, self.inventory.ammo(self.weapon), cost);
        Some((
            units,
            (elapsed / (charge.profile.period * f32::from(charge.profile.max_units - 1)))
                .clamp(0.0, 1.0),
        ))
    }

    pub(crate) fn weapon_pose<'a>(&self, art: &'a WeaponAssets, time: f32) -> Pose<'a> {
        let fire = if self.last_alt && !art.alternate_motion.frames.is_empty() {
            &art.alternate_motion
        } else {
            &art.primary_motion
        };
        self.charge
            .and_then(|charge| {
                let age = (time - charge.since).min((art.load.duration() - 0.001).max(0.0));
                art.load.sample(age)
            })
            .or_else(|| fire.sample(time - self.fired_at))
            .or_else(|| art.select_motion.sample(time - self.selected_at))
            .unwrap_or(Pose {
                current: &art.view,
                next: &art.view,
                fraction: 0.0,
            })
    }

    pub fn armor(&self) -> u16 {
        self.armor.iter().sum::<u16>().min(150)
    }

    pub fn amp_left(&self, time: f32) -> f32 {
        (self.power.amp_until - time).max(0.0)
    }

    pub fn invis_left(&self, time: f32) -> f32 {
        (self.power.invis_until - time).max(0.0)
    }

    pub fn boot_charges(&self) -> u16 {
        self.power.boot_charges
    }

    fn equip(
        &mut self,
        art: Option<&WeaponAssets>,
        weapon: Weapon,
        time: f32,
        sounds: &mut SoundEvents,
        local: bool,
    ) {
        if self.weapon == weapon || !self.inventory.owns(weapon) {
            return;
        }
        self.weapon = weapon;
        self.charge = None;
        self.charge_locked = None;
        self.zoomed = false;
        self.fired_at = f32::NEG_INFINITY;
        self.selected_at = time;
        self.ready_at =
            time + art.map_or(0.27, |art| art.select_motion.duration().clamp(0.15, 0.55));
        let sound = art.and_then(|art| art.select_sound.as_ref()).cloned();
        if local {
            sounds.extend(sound);
        } else {
            sounds.extend_at(sound, self.walker.position, 500.0, 0.6);
        }
    }

    fn capture_body(&mut self) {
        self.previous_body = (self.walker.position, self.yaw);
    }

    pub(crate) fn body_transform(&self, blend: f32) -> (Vec3, f32) {
        let (position, yaw) = self.previous_body;
        (
            position + (self.walker.position - position) * blend,
            yaw + angle_delta(self.yaw, yaw) * blend,
        )
    }

    fn update_gait(&mut self, dt: f32) {
        let velocity = self.walker.velocity();
        let speed = velocity.0.hypot(velocity.1);
        self.stride_time += dt * (speed / 285.0).clamp(0.0, 2.0);
        let (sin, cos) = self.yaw.sin_cos();
        let forward = velocity.dot(Vec3(cos, sin, 0.0));
        let side = velocity.dot(Vec3(-sin, cos, 0.0));
        let strafe = Gait::Strafe(usize::from(side > 0.0));
        self.gait = if speed < if self.gait == Gait::Idle { 47.0 } else { 27.0 } {
            Gait::Idle
        } else if side.abs() > forward.abs() * if self.gait == strafe { 0.92 } else { 1.32 } {
            strafe
        } else if forward
            < if self.gait == Gait::Backpedal {
                -15.0
            } else {
                -35.0
            }
        {
            Gait::Backpedal
        } else if speed < if self.gait == Gait::Run { 145.0 } else { 185.0 } {
            Gait::Walk
        } else {
            Gait::Run
        };
    }

    pub(crate) fn pose<'a>(&self, map: &'a Map, id: usize, time: f32) -> Option<Pose<'a>> {
        if let Some(death) = self.fell_at {
            let clip = &map.player_deaths[id % map.player_deaths.len()];
            return clip.sample((time - death).min((clip.duration() - 0.001).max(0.0)));
        }
        let clips = &map.player_clips;
        let firing = time - self.fired_at;
        let hurt = time - self.hurt_at;
        let landed = time - self.landed_at;
        let (clip, age) = if map.collision.regions.at(self.walker.position).water {
            (&clips.swim, time)
        } else if hurt < 0.12 && clips.hurt.duration() > 0.0 {
            return clips.hurt.sample(hurt.min(clips.hurt.duration() - 0.001));
        } else if !self.walker.grounded {
            let age = self.airborne_since.map_or(0.0, |started| time - started);
            return clips
                .air
                .sample(age.min((clips.air.duration() - 0.001).max(0.0)))
                .or_else(|| map.player_idle.sample(0.0));
        } else if landed < 0.13 && clips.land.duration() > 0.0 {
            return clips.land.sample(landed.min(clips.land.duration() - 0.001));
        } else {
            match self.gait {
                Gait::Idle if firing < clips.fire.duration() => return clips.fire.sample(firing),
                Gait::Idle => (&map.player_idle, time),
                Gait::Strafe(side) => (&clips.strafe[side], self.stride_time),
                Gait::Backpedal => (&clips.backpedal, self.stride_time),
                Gait::Walk | Gait::Run if firing < 0.27 && !clips.fire_run.frames.is_empty() => {
                    (&clips.fire_run, self.stride_time)
                }
                Gait::Walk => (&clips.walk, self.stride_time),
                Gait::Run => (&clips.run, self.stride_time),
            }
        };
        clip.sample_loop(age)
            .or_else(|| map.player_idle.sample(0.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gait {
    Idle,
    Walk,
    Run,
    Backpedal,
    Strafe(usize),
}

#[derive(Clone, Copy)]
struct Charge {
    since: f32,
    alternate: bool,
    profile: ChargeProfile,
}

#[derive(Default)]
struct ActivePowerups {
    boot_charges: u16,
    boosted_airborne: bool,
    amp_until: f32,
    invis_until: f32,
}

impl ActivePowerups {
    fn wants(&self, power: Powerup, charge: u16, now: f32) -> bool {
        match power {
            Powerup::Boots => charge > self.boot_charges,
            Powerup::Damage => now + f32::from(charge) * 0.1 > self.amp_until,
            Powerup::Invisibility => now + (f32::from(charge) + 1.0) * 0.5 > self.invis_until,
        }
    }
}

#[derive(Default)]
struct Bot {
    target: Option<usize>,
    react_at: f32,
    combat_move: Vec3,
    aim_bias: Option<[f32; 2]>,
    weapon_mode: Option<(Weapon, bool, f32)>,
    goal: Option<Step>,
    rethink_at: f32,
    last_seen: Option<(Vec3, f32)>,
    lift: Option<ActiveLift>,
    plan_origin: Option<Vec3>,
    retreat: Option<Retreat>,
    retreat_retry_at: f32,
    jump: JumpProgress,
    jump_until: Option<f32>,
    evasion: Option<Evasion>,
    evade_again_at: f32,
}

struct Evasion {
    direction: Vec3,
    until: f32,
}

struct Retreat {
    pickup: usize,
    until: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LiftPhase {
    Board,
    Ride,
    Leave,
}

struct ActiveLift {
    ride: LiftRide,
    phase: LiftPhase,
    expires: f32,
}

struct Bolt {
    owner: usize,
    position: Vec3,
    velocity: Vec3,
    expires: f32,
    born: f32,
    payload: ProjectilePayload,
    pilot: Option<Pilot>,
}

#[derive(Clone, Copy)]
struct ProjectilePayload {
    kind: super::ordnance::Kind,
    damage: u16,
    blast: f32,
    gravity: f32,
    rebounds: u8,
    scale: f32,
}

struct Detonation {
    position: Vec3,
    owner: usize,
    payload: ProjectilePayload,
}

pub struct Trace {
    pub start: Vec3,
    pub end: Vec3,
    pub explosion: Option<super::ordnance::Kind>,
    pub born: f32,
    pub scale: f32,
    expires: f32,
}

struct Hit {
    bypass_armor: bool,
    victim: usize,
    owner: Option<usize>,
    damage: u16,
    impulse: Vec3,
}

pub struct Match {
    rules: Rules,
    preferred_start: usize,
    pub time: f32,
    pub fighters: Vec<Fighter>,
    pub finished: bool,
    pub hud_scale: f32,
    pub show_decals: bool,
    pub decals: decals::Marks,
    pub ballistics: ballistics::Effects,
    pub traces: Vec<Trace>,
    bolts: Vec<Bolt>,
    initial_props: Vec<bool>,
    pickups: Vec<Option<f32>>,
    kill_z: f32,
    navigation: Navigation,
    recent_spawns: Vec<Option<u64>>,
    spawn_sequence: u64,
    message: String,
    message_until: f32,
}

impl Match {
    pub fn new(map: &Map, start: usize, rules: Rules) -> Self {
        let mut game = Self {
            rules,
            preferred_start: start,
            time: 0.0,
            fighters: Vec::with_capacity(rules.bots + 1),
            finished: false,
            hud_scale: 1.0,
            show_decals: true,
            decals: decals::Marks::default(),
            ballistics: ballistics::Effects::default(),
            traces: Vec::new(),
            bolts: Vec::new(),
            initial_props: map.props.iter().map(|prop| prop.visible).collect(),
            pickups: vec![None; map.props.len()],
            kill_z: map
                .starts
                .iter()
                .map(|start| start.position.2)
                .fold(f32::INFINITY, f32::min)
                - 4096.0,
            navigation: Navigation::new(map),
            recent_spawns: vec![None; map.starts.len()],
            spawn_sequence: 0,
            message: String::new(),
            message_until: 0.0,
        };
        game.spawn_all(map);
        game
    }

    fn spawn_all(&mut self, map: &Map) {
        self.fighters.clear();
        for id in 0..=self.rules.bots {
            let spawn = if id == 0 && usable_start(map, self.preferred_start).is_some() {
                self.preferred_start
            } else {
                self.choose_spawn(map, id, None)
            };
            let fighter = self.spawn_at(map, spawn);
            self.fighters.push(fighter);
        }
    }

    fn spawn_at(&mut self, map: &Map, start: usize) -> Fighter {
        self.recent_spawns[start] = Some(self.spawn_sequence);
        self.spawn_sequence += 1;
        Fighter::spawn(map, start, self.time)
    }

    fn restart(&mut self, map: &mut Map) {
        self.time = 0.0;
        self.finished = false;
        self.traces.clear();
        self.decals = decals::Marks::default();
        self.ballistics = ballistics::Effects::default();
        self.bolts.clear();
        self.pickups.fill(None);
        for (prop, &visible) in map.props.iter_mut().zip(&self.initial_props) {
            prop.visible = visible;
        }
        for mover in &mut map.movers {
            mover.reset_motion(&mut map.collision);
        }
        self.message_until = 0.0;
        self.spawn_all(map);
    }

    fn choose_spawn(&self, map: &Map, fighter: usize, previous: Option<usize>) -> usize {
        map.starts
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                let start = usable_start(map, index)?;
                let mut nearest = 850.0_f32;
                let mut exposed = 0.0_f32;
                let mut occupied = false;
                for (id, other) in self.fighters.iter().enumerate() {
                    if id == fighter || other.health == 0 {
                        continue;
                    }
                    let offset = start - other.walker.position;
                    occupied |= offset.0.hypot(offset.1) < 2.0 * PLAYER_RADIUS + 8.0
                        && offset.2.abs() < 2.0 * PLAYER_HEIGHT;
                    let distance = offset.dot(offset).sqrt();
                    nearest = nearest.min(distance);
                    let eye = other.camera(map).position;
                    let sightline = start + Vec3(0.0, 0.0, 12.0) - eye;
                    if distance < 1050.0 && map.collision.ray(eye, sightline).is_none() {
                        exposed = exposed.max(290.0 * (1.0 - distance / 1050.0));
                    }
                }
                let last = self.recent_spawns[index];
                let recency = last.map_or(0.0, |last| {
                    45.0 * (5_u64.saturating_sub(self.spawn_sequence - last)) as f32
                });
                let repeat = if previous == Some(index) { 190.0 } else { 0.0 };
                let score = nearest - exposed - recency - repeat;
                Some((index, !occupied, score, last))
            })
            .max_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| a.2.total_cmp(&b.2))
                    .then_with(|| b.3.cmp(&a.3))
            })
            .map_or(self.preferred_start, |(spawn, _, _, _)| spawn)
    }

    pub fn remaining(&self) -> u16 {
        (f32::from(self.rules.seconds) - self.time).ceil().max(0.0) as u16
    }

    pub(crate) fn hud_state(&self, map: &Map, scale: f32) -> crate::hud_art::State {
        let player = &self.fighters[0];
        crate::hud_art::State {
            hud_scale: scale,
            time: self.time,
            pickup_time: player.pickup_time,
            score: player.score,
            health: player.health,
            armor: player.armor,
            boots: player.boot_charges() > 0 || player.power.boosted_airborne,
            amplified: player.amp_left(self.time) > 0.0,
            rocket_locked: false,
            crosshair_visible: player.health > 0
                && !player.zoomed
                && !self.finished
                && self.pilot_index().is_none(),
            weapon: player.weapon,
            pending: player.weapon,
            ammo: player.ammo(),
            owned: Weapon::ALL.map(|weapon| player.inventory.owns(weapon)),
            fractions: Weapon::ALL.map(|weapon| {
                (weapon != Weapon::Hammer).then(|| {
                    f32::from(player.inventory.ammo(weapon))
                        / f32::from(super::max_ammo(&map.arsenal, weapon).max(1))
                })
            }),
        }
    }

    pub fn difficulty(&self) -> Difficulty {
        self.rules.difficulty
    }

    pub fn standings(&self) -> Vec<usize> {
        let mut ids: Vec<_> = (0..self.fighters.len()).collect();
        ids.sort_by_key(|&id| (std::cmp::Reverse(self.fighters[id].score), id));
        ids
    }

    pub fn label(id: usize) -> String {
        if id == 0 {
            "PLAYER".into()
        } else {
            format!("BOT {id}")
        }
    }

    pub fn notice(&self) -> Option<&str> {
        (self.time < self.message_until).then_some(self.message.as_str())
    }

    fn pilot_index(&self) -> Option<usize> {
        if self.finished || self.fighters[0].health == 0 {
            return None;
        }
        self.bolts
            .iter()
            .position(|bolt| bolt.owner == 0 && bolt.pilot.is_some())
    }

    pub fn guidance_left(&self) -> Option<f32> {
        self.pilot_index()
            .map(|index| self.bolts[index].expires - self.time)
    }

    pub fn player_view(&self, map: &Map) -> Camera {
        if let Some(index) = self.pilot_index() {
            let bolt = &self.bolts[index];
            return bolt.pilot.as_ref().unwrap().view(bolt.position);
        }
        self.fighters[0].camera(map)
    }

    pub(crate) fn bolts(
        &self,
    ) -> impl Iterator<Item = (super::ordnance::Kind, Vec3, Vec3, f32, f32)> + '_ {
        let camera = self.pilot_index();
        self.bolts
            .iter()
            .enumerate()
            .filter(move |&(index, _)| Some(index) != camera)
            .map(|(_, bolt)| {
                (
                    bolt.payload.kind,
                    bolt.position,
                    unit(bolt.velocity),
                    self.time - bolt.born,
                    bolt.payload.scale,
                )
            })
    }

    pub fn step(&mut self, map: &mut Map, command: Command, dt: f32) -> SoundEvents {
        let mut sounds = SoundEvents::default();
        if self.finished {
            if command.restart {
                self.restart(map);
            }
            return sounds;
        }
        self.time += dt;
        for fighter in &mut self.fighters {
            fighter.capture_body();
        }
        self.traces.retain(|trace| trace.expires > self.time);
        let flight = self.pilot_index();
        let manual_detonation = flight.and_then(|index| {
            let bolt = &mut self.bolts[index];
            let pilot = bolt.pilot.as_mut().unwrap();
            let speed = bolt.velocity.dot(bolt.velocity).sqrt();
            let explode = pilot.update(command.look, command.primary || command.alternate, dt);
            if explode {
                let bolt = self.bolts.swap_remove(index);
                return Some(Detonation {
                    position: bolt.position,
                    owner: bolt.owner,
                    payload: bolt.payload,
                });
            }
            bolt.velocity = pilot.view(bolt.position).look_direction() * speed;
            None
        });
        let mut inputs = Vec::with_capacity(self.fighters.len());
        inputs.push(if flight.is_some() {
            Command::default()
        } else {
            command
        });
        for id in 1..self.fighters.len() {
            inputs.push(self.bot_command(map, id, dt));
        }
        {
            let mut requests = vec![None::<(bool, LiftStop)>; map.movers.len()];
            for fighter in self.fighters.iter().filter(|fighter| fighter.health > 0) {
                if let Some(lift) = &fighter.bot.lift {
                    let riding = lift.phase != LiftPhase::Board;
                    let stop = if riding {
                        lift.ride.from.opposite()
                    } else {
                        lift.ride.from
                    };
                    let request = &mut requests[lift.ride.mover];
                    if request.is_none_or(|(previous_riding, _)| riding && !previous_riding) {
                        *request = Some((riding, stop));
                    }
                }
            }
            let mut walkers: Vec<_> = self
                .fighters
                .iter_mut()
                .filter(|fighter| fighter.health > 0)
                .map(|fighter| &mut fighter.walker)
                .collect();
            for (mover, request) in map.movers.iter_mut().zip(requests) {
                mover.tick_many(
                    &mut map.collision,
                    &mut walkers,
                    dt,
                    request.map(|(_, stop)| stop),
                );
            }
        }
        let mut hits = Vec::new();
        for (id, input) in inputs.iter().enumerate() {
            if let Some(fell) = self.fighters[id].fell_at {
                let extra = if id == 0 {
                    0.0
                } else {
                    let [bias, _] = ai::shot_bias(id, u32::from(self.fighters[id].deaths));
                    (bias + 1.0) * 0.22
                };
                if self.time - fell >= RESPAWN_DELAY + extra {
                    let old = self.fighters[id].spawn;
                    let spawn = self.choose_spawn(map, id, Some(old));
                    let mut respawned = self.spawn_at(map, spawn);
                    respawned.score = self.fighters[id].score;
                    respawned.deaths = self.fighters[id].deaths;
                    self.fighters[id] = respawned;
                }
                continue;
            }
            let fighter = &mut self.fighters[id];
            fighter.yaw = (fighter.yaw + input.look[0]).rem_euclid(std::f32::consts::TAU);
            fighter.pitch = (fighter.pitch + input.look[1]).clamp(-1.48, 1.48);
            let aim = fighter.camera(map);
            let [forward, strafe] = input.movement;
            let walk = aim.forward() * forward + aim.right() * strafe;
            let profile = map.collision.profile;
            let speed = profile.ground_speed * if input.walk { 0.3 } else { 1.0 };
            let speed = speed / walk.dot(walk).sqrt().max(1.0);
            let dodged = !input.walk
                && !map.collision.regions.at(fighter.walker.position).water
                && input.dodge.is_some_and(|[forward, strafe]| {
                    let direction = aim.forward() * forward + aim.right() * strafe;
                    direction.dot(direction) > 0.01
                        && fighter.walker.dodge(profile, unit(direction))
                });
            let swim = aim.look_direction() * forward
                + aim.right() * strafe
                + Vec3(0.0, 0.0, f32::from(input.jump));
            let base_jump = map.collision.profile.jump_speed;
            let use_boots = !input.walk && !input.normal_jump && fighter.power.boot_charges > 0;
            let jump_speed = if use_boots {
                base_jump * 3.0
            } else {
                base_jump
            };
            fighter.walker.air_control_override = (!input.normal_jump
                && (use_boots || fighter.power.boosted_airborne))
                .then_some(1.0);
            let motion = fighter.walker.advance_with_swim(
                &map.collision,
                walk * speed,
                swim,
                input.jump.then_some(jump_speed),
                dt,
            );
            fighter.update_gait(dt);
            let swimming = map.collision.regions.at(fighter.walker.position).water;
            if fighter.walker.grounded || swimming {
                fighter.airborne_since = None;
            } else {
                fighter.airborne_since.get_or_insert(self.time);
            }
            if motion.jump || dodged {
                let boost = use_boots && motion.jump;
                if boost {
                    fighter.power.boot_charges -= 1;
                    fighter.power.boosted_airborne = true;
                }
                let sound = if boost {
                    map.powerups
                        .boots_jump
                        .as_ref()
                        .or(map.player_audio.jump.as_ref())
                } else {
                    map.player_audio.jump.as_ref()
                };
                sounds.extend_at(sound.cloned(), fighter.walker.position, 580.0, 0.8);
            }
            if let Some(speed) = motion.landing.filter(|&speed| !swimming && speed > 105.0) {
                fighter.landed_at = self.time;
                let fall = super::landing_damage(
                    speed,
                    if fighter.power.boosted_airborne {
                        base_jump * 3.0
                    } else {
                        base_jump
                    },
                );
                if fall > 0 {
                    hits.push(Hit {
                        bypass_armor: false,
                        victim: id,
                        owner: None,
                        damage: fall,
                        impulse: Vec3::default(),
                    });
                }
                let gain = (speed / map.collision.profile.jump_speed.max(1.0))
                    .powi(2)
                    .min(1.0);
                sounds.extend_at(
                    map.player_audio.land.iter().cloned(),
                    fighter.walker.position,
                    620.0,
                    gain,
                );
            }
            if motion.landing.is_some() || swimming {
                fighter.power.boosted_airborne = false;
            }
            if let Some(step) = motion.footstep {
                let sound = if swimming {
                    &map.player_audio.water_step
                } else {
                    &map.player_audio.steps[step]
                };
                sounds.extend_at(sound.iter().cloned(), fighter.walker.position, 500.0, 0.7);
            }
            if let Some((position, yaw)) = crate::teleport::touched(
                &map.teleporters,
                fighter.walker.position,
                &mut fighter.portal,
            ) {
                fighter.walker = Walker::spawn(&map.collision, position);
                fighter.yaw = yaw.unwrap_or(fighter.yaw);
                fighter.capture_body();
                fighter.airborne_since = None;
                fighter.power.boosted_airborne = false;
                fighter.bot.goal = None;
                fighter.bot.lift = None;
                fighter.bot.jump_until = None;
                fighter.bot.rethink_at = self.time;
            }
            if fighter.walker.position.2 < self.kill_z {
                hits.push(Hit {
                    bypass_armor: false,
                    victim: id,
                    owner: None,
                    damage: u16::MAX,
                    impulse: Vec3::default(),
                });
                continue;
            }
            let center = fighter.walker.position;
            let eye = fighter.camera(map).position;
            match fighter
                .liquid
                .step(map, center, eye, self.time, id == 0, &mut sounds)
            {
                Some(impact @ (Impact::Splash(_) | Impact::Drown(_))) => {
                    let (damage, bypass_armor) = match impact {
                        Impact::Splash(damage) => (damage, false),
                        Impact::Drown(damage) => (damage, true),
                        Impact::Heal(_) => unreachable!(),
                    };
                    hits.push(Hit {
                        victim: id,
                        owner: None,
                        damage,
                        bypass_armor,
                        impulse: Vec3::default(),
                    });
                }
                Some(Impact::Heal(amount)) if fighter.health < 100 => {
                    fighter.health = fighter.health.saturating_add(amount).min(100);
                }
                _ => {}
            }
        }
        self.separate_fighters(map);
        self.collect(map, &mut sounds);
        if let Some(detonation) = manual_detonation {
            self.detonate(map, vec![detonation], &mut hits, &mut sounds);
        }
        self.move_bolts(map, dt, &mut hits, &mut sounds);
        for (id, input) in inputs.into_iter().enumerate() {
            if !hits
                .iter()
                .any(|hit| hit.victim == id && hit.damage == u16::MAX)
            {
                self.fire(map, id, input, &mut hits, &mut sounds);
            }
        }
        for hit in hits {
            self.apply_hit(map, hit, &mut sounds);
        }
        self.decals.update(self.time);
        self.decals.follow_movers(&map.movers);
        self.ballistics
            .update(map, None, self.time, dt, &mut sounds);
        self.finished = self.remaining() == 0
            || self
                .fighters
                .iter()
                .any(|fighter| fighter.score >= self.rules.frag_limit);
        if flight.is_some() && self.pilot_index().is_none() {
            self.fighters[0].wait_for_fire_release = command.primary || command.alternate;
        }
        sounds
    }

    fn separate_fighters(&mut self, map: &Map) {
        for first in 0..self.fighters.len() {
            let (before, after) = self.fighters.split_at_mut(first + 1);
            let a = &mut before[first];
            if a.health == 0 {
                continue;
            }
            for b in after.iter_mut().filter(|fighter| fighter.health > 0) {
                let delta = b.walker.position - a.walker.position;
                let distance = delta.0.hypot(delta.1);
                let overlap = 2.0 * PLAYER_RADIUS - distance;
                if overlap <= 0.0 || delta.2.abs() >= 2.0 * PLAYER_HEIGHT {
                    continue;
                }
                let direction = if distance > 0.001 {
                    Vec3(delta.0 / distance, delta.1 / distance, 0.0)
                } else {
                    Vec3(1.0, 0.0, 0.0)
                };
                a.walker.carry(&map.collision, direction * (-0.5 * overlap));
                b.walker.carry(&map.collision, direction * (0.5 * overlap));
            }
        }
    }

    fn collect(&mut self, map: &mut Map, sounds: &mut SoundEvents) {
        let mut nearby = Vec::with_capacity(self.fighters.len());
        for (index, prop) in map.props.iter_mut().enumerate() {
            if self.pickups[index].is_some_and(|due| self.time >= due) {
                self.pickups[index] = None;
                prop.visible = true;
            }
            let Some(item) = prop.item.filter(|_| prop.visible) else {
                continue;
            };
            nearby.clear();
            nearby.extend(
                self.fighters
                    .iter()
                    .enumerate()
                    .filter_map(|(id, fighter)| {
                        let delta = fighter.walker.position - prop.position;
                        (fighter.health > 0
                            && delta.2.abs() <= prop.pickup_height + PLAYER_HEIGHT
                            && delta.0.hypot(delta.1) <= prop.pickup_radius + PLAYER_RADIUS)
                            .then_some((delta.dot(delta), id))
                    }),
            );
            nearby.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, id) in &nearby {
                let fighter = &mut self.fighters[id];
                if map
                    .collision
                    .ray(prop.position, fighter.walker.position - prop.position)
                    .is_some()
                {
                    continue;
                }
                let new_weapon = match item {
                    Item::Gun(weapon) if !fighter.inventory.owns(weapon) => Some(weapon),
                    _ => None,
                };
                if !take_item(&map.arsenal, fighter, item, self.time) {
                    continue;
                }
                prop.visible = false;
                fighter.pickup_time = self.time;
                self.pickups[index] = Some(self.time + prop.respawn_time.unwrap_or(25.0));
                sounds.extend_at(
                    prop.pickup_sound.iter().cloned(),
                    prop.position,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
                if matches!(item, Item::Power(Powerup::Boots, _)) {
                    sounds.extend_at(
                        map.powerups.boots_on.iter().cloned(),
                        prop.position,
                        CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                if let Some(weapon) = new_weapon
                    && (fighter.ammo() == Some(0)
                        || weapon.switch_priority() > fighter.weapon.switch_priority())
                {
                    fighter.equip(map.arsenal.get(&weapon), weapon, self.time, sounds, id == 0);
                }
                break;
            }
        }
    }

    fn fire(
        &mut self,
        map: &Map,
        id: usize,
        input: Command,
        hits: &mut Vec<Hit>,
        sounds: &mut SoundEvents,
    ) {
        let fighter = &mut self.fighters[id];
        if fighter.health == 0 {
            return;
        }
        let pressed_alt = input.alternate && !fighter.alt_held;
        fighter.alt_held = input.alternate;
        if fighter.weapon != Weapon::Enforcer || !input.alternate || input.primary {
            fighter.enforcer_accuracy = 0.4;
        }
        let requested = input.preferred.or_else(|| {
            fighter
                .inventory
                .select(fighter.weapon, input.slot, input.cycle)
        });
        if let Some(weapon) = requested.filter(|&weapon| weapon != fighter.weapon) {
            fighter.equip(map.arsenal.get(&weapon), weapon, self.time, sounds, id == 0);
            return;
        }
        if fighter.wait_for_fire_release {
            if input.primary || input.alternate {
                return;
            }
            fighter.wait_for_fire_release = false;
        }
        let weapon = fighter.weapon;
        let input_holds = |alt: bool| {
            if alt {
                input.alternate && !input.primary
            } else {
                input.primary
            }
        };
        if fighter.charge_locked.is_some_and(|alt| !input_holds(alt)) {
            fighter.charge_locked = None;
        }
        let ammo = fighter.inventory.ammo(weapon);
        let (alt, mut shot, units, shape) = if let Some(charge) = fighter.charge {
            let alt = charge.alternate;
            let shot = arsenal::shot(weapon, alt);
            let held = input_holds(alt);
            let units = charge
                .profile
                .units(self.time - charge.since, ammo, shot.ammo);
            let cap = charge.profile.max_units.min(ammo / shot.ammo.max(1));
            if held && units < cap {
                return;
            }
            fighter.charge = None;
            if id != 0 && !held && !input.release_charge {
                return; // The bot lost its shot and deliberately aborted.
            }
            if units == 0 {
                return;
            }
            fighter.charge_locked = held.then_some(alt);
            (alt, shot, units, Some(charge.profile.shape))
        } else {
            if self.time < fighter.ready_at || !(input.primary || input.alternate) {
                return;
            }
            let mut alt = input.alternate && !input.primary;
            let mut shot = arsenal::shot(weapon, alt);
            if matches!(shot.path, Path::Zoom) {
                if pressed_alt {
                    fighter.zoomed = !fighter.zoomed;
                }
                return;
            }
            if ammo < shot.ammo {
                alt = false;
                shot = arsenal::shot(weapon, false);
            }
            if ammo >= shot.ammo
                && let Some(profile) = arsenal::charge_profile(weapon, alt)
            {
                if fighter.charge_locked == Some(alt) {
                    return;
                }
                fighter.charge = Some(Charge {
                    since: self.time,
                    alternate: alt,
                    profile,
                });
                let wave = map
                    .arsenal
                    .get(&weapon)
                    .and_then(|art| art.load_sound.clone());
                if id == 0 {
                    sounds.extend(wave);
                } else {
                    sounds.extend_at(wave, fighter.walker.position, CONTACT_SOUND_RADIUS, 0.75);
                }
                return;
            }
            (alt, shot, 1, None)
        };
        shot.ammo = shot.ammo.saturating_mul(units);
        if !fighter.inventory.spend(weapon, shot.ammo) {
            let fallback = fighter.inventory.preferred(560.0);
            fighter.equip(
                map.arsenal.get(&fallback),
                fallback,
                self.time,
                sounds,
                id == 0,
            );
            return;
        }
        fighter.fired_at = self.time;
        if fighter.protected_until.is_finite() {
            fighter.protected_until = fighter.protected_until.min(self.time);
        }
        fighter.fired_count = fighter.fired_count.wrapping_add(1);
        fighter.last_alt = alt;
        let mut seed = self.time.to_bits().rotate_left(5) ^ (id as u32).wrapping_mul(0x9e37_79b9);
        let mut jitter = || ballistics::jitter(&mut seed);
        let bullet = ballistics::BulletProfile::for_weapon(
            map,
            weapon,
            alt,
            &mut fighter.enforcer_accuracy,
            &mut jitter,
        );
        let pause = if id == 0 {
            0.0
        } else {
            self.rules.difficulty.combat().fire_pause
                * (0.8 + 0.2 * ai::shot_bias(id, fighter.fired_count)[0])
        };
        let interval = bullet
            .as_ref()
            .map_or(shot.interval, |bullet| bullet.interval);
        fighter.ready_at = self.time + interval + pause;
        let aim = fighter.camera(map);
        let case_origin = bullet
            .is_some()
            .then(|| map.arsenal.get(&weapon))
            .flatten()
            .and_then(|art| {
                fighter
                    .weapon_pose(art, self.time)
                    .edge_center(art.case_triangle?)
            })
            .filter(|point| point.0 > 0.0);
        let start = aim.position;
        let direction = aim.look_direction();
        let boosted = fighter.amp_left(self.time) > 0.0;
        let extra = f32::from(units.saturating_sub(1));
        let grow = matches!(shape, Some(ChargeShape::Grow));
        let damage = if grow {
            (f32::from(shot.damage) * (1.0 + 0.38 * extra)).round() as u16
        } else {
            bullet.as_ref().map_or(shot.damage, |bullet| bullet.damage)
        };
        let base_damage = damage;
        let damage = base_damage.saturating_mul(if boosted { 3 } else { 1 });
        let wave = super::shot_sound(&map.arsenal, weapon, alt);
        if id == 0 {
            sounds.extend(wave);
        } else {
            sounds.extend_at(wave, start, CONTACT_SOUND_RADIUS, 1.0);
        }
        if boosted {
            let warning = usize::from(fighter.amp_left(self.time) < 5.0);
            sounds.extend_at(
                map.powerups.amp_fire[warning].iter().cloned(),
                start,
                CONTACT_SOUND_RADIUS,
                0.9,
            );
        }

        match shot.path {
            Path::Missile {
                kind,
                blast,
                gravity,
                rebounds,
            } => {
                let speed = arsenal::velocity(map, kind);
                let guided = id == 0 && shot.guided;
                let count = if matches!(shape, Some(ChargeShape::Volley)) {
                    units
                } else {
                    1
                };
                for index in 0..count {
                    let spread = (f32::from(index) - f32::from(count - 1) * 0.5) * 0.055;
                    let aim_direction = unit(direction + aim.right() * spread);
                    self.bolts.push(Bolt {
                        owner: id,
                        position: start,
                        velocity: aim_direction * speed + Vec3(0.0, 0.0, gravity * 115.0),
                        born: self.time,
                        expires: self.time
                            + if guided {
                                guidance::FLIGHT_TIME
                            } else if rebounds > 0 && gravity > 0.0 {
                                2.35
                            } else {
                                4.0
                            },
                        payload: ProjectilePayload {
                            kind,
                            damage,
                            blast: if grow {
                                blast * (1.0 + 0.17 * extra)
                            } else {
                                blast
                            },
                            gravity,
                            rebounds,
                            scale: if grow { 1.0 + 0.20 * extra } else { 1.0 },
                        },
                        pilot: guided.then(|| Pilot::new(&aim)),
                    });
                }
            }
            Path::Trace {
                range,
                rays,
                spread,
            } => {
                let right = aim.right();
                let up = direction.cross(right);
                for ray in 0..rays {
                    let spread = if ray == 0 && rays > 1 { 0.0 } else { spread };
                    let (start, delta) = match &bullet {
                        Some(profile) => profile.ray(&aim, &mut jitter),
                        None => (
                            start,
                            unit(direction + (right * jitter() + up * jitter()) * spread) * range,
                        ),
                    };
                    let wall = map.collision.ray_surface(start, delta);
                    let end = wall.map_or(start + delta, |(point, _)| point);
                    let clipped = end - start;
                    let target = nearest_hit(&self.fighters, id, start, clipped, [0.0, 0.0]);
                    let projectile = self.closest_explosive(map, start, clipped);
                    let intercepted = projectile
                        .filter(|&(_, at)| target.is_none_or(|(_, victim_at)| at < victim_at));
                    let fraction = if let Some((projectile, at)) = intercepted {
                        let bolt = self.bolts.swap_remove(projectile);
                        self.detonate(
                            map,
                            vec![Detonation {
                                position: bolt.position,
                                owner: id,
                                payload: bolt.payload,
                            }],
                            hits,
                            sounds,
                        );
                        at
                    } else if let Some((victim, at)) = target {
                        let point = start + clipped * at;
                        if bullet.is_some() {
                            sounds.extend_at(
                                map.ballistics.flesh.iter().cloned(),
                                point,
                                CONTACT_SOUND_RADIUS,
                                1.0,
                            );
                        }
                        let headshot = weapon == Weapon::Sniper
                            && point.2 - self.fighters[victim].walker.position.2
                                > 0.62 * PLAYER_HEIGHT;
                        hits.push(Hit {
                            bypass_armor: false,
                            victim,
                            owner: Some(id),
                            damage: if headshot {
                                if boosted { 300 } else { 100 }
                            } else {
                                damage
                            },
                            impulse: unit(delta) * (f32::from(base_damage) * 3.4),
                        });
                        at
                    } else {
                        1.0
                    };
                    if ray == 0 && bullet.is_some() {
                        self.ballistics.bullet(
                            map,
                            &mut self.decals,
                            &aim,
                            weapon,
                            self.time,
                            ballistics::Bullet {
                                start,
                                delta,
                                visible: clipped * fraction,
                                wall: wall.filter(|_| target.is_none() && intercepted.is_none()),
                                case_origin,
                            },
                            &mut self.fighters[id].bullet_cadence,
                            sounds,
                            &mut jitter,
                        );
                    } else if ray == 0 && weapon != Weapon::Hammer {
                        self.traces.push(Trace {
                            start,
                            end: start + clipped * fraction,
                            explosion: None,
                            born: self.time,
                            scale: 1.0,
                            expires: self.time + 0.065,
                        });
                    }
                }
            }
            Path::Zoom => unreachable!(),
        }
    }

    fn closest_explosive(&self, map: &Map, start: Vec3, delta: Vec3) -> Option<(usize, f32)> {
        self.bolts
            .iter()
            .enumerate()
            .filter(|(_, bolt)| bolt.payload.blast > 0.0)
            .filter_map(|(index, bolt)| {
                let art = &map.ordnance[bolt.payload.kind as usize];
                let at = cylinder_hit(
                    start - bolt.position,
                    delta,
                    art.collision_radius.max(4.0) * bolt.payload.scale,
                    art.collision_height.max(4.0) * bolt.payload.scale,
                )?;
                (at < 1.0).then_some((index, at))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    fn move_bolts(&mut self, map: &Map, dt: f32, hits: &mut Vec<Hit>, sounds: &mut SoundEvents) {
        let mut detonations = Vec::new();
        self.bolts.retain_mut(|bolt| {
            let payload = &mut bolt.payload;
            let art = &map.ordnance[payload.kind as usize];
            let zone = map.collision.regions.at(bolt.position);
            bolt.velocity = bolt.velocity + zone.gravity * (payload.gravity * dt);
            let delta = bolt.velocity * dt;
            let radius = art.collision_radius.max(0.0) * payload.scale;
            let height = art.collision_height.max(0.0) * payload.scale;
            let wall = map
                .collision
                .projectile_surface(bolt.position, delta, radius, height);
            let end = wall.map_or(bolt.position + delta, |(point, _)| point);
            let target = nearest_hit(
                &self.fighters,
                bolt.owner,
                bolt.position,
                end - bolt.position,
                [radius, height],
            );
            let expired = self.time >= bolt.expires;
            if target.is_none() && !expired {
                if let Some((contact, normal)) = wall {
                    if payload.rebounds > 0 {
                        payload.rebounds -= 1;
                        bolt.position = contact + normal * 0.7;
                        let retain = if payload.gravity > 0.0 { 0.64 } else { 0.95 };
                        bolt.velocity =
                            (bolt.velocity - normal * (2.0 * bolt.velocity.dot(normal))) * retain;
                        sounds.extend_at(
                            art.contact.iter().cloned(),
                            contact,
                            CONTACT_SOUND_RADIUS,
                            1.0,
                        );
                        return true;
                    }
                } else {
                    bolt.position = end;
                    return true;
                }
            }
            let position = target.map_or(end, |(_, fraction)| {
                bolt.position + (end - bolt.position) * fraction
            }) - unit(bolt.velocity) * 0.5;
            if payload.blast > 0.0 {
                detonations.push(Detonation {
                    position,
                    owner: bolt.owner,
                    payload: *payload,
                });
            } else if let Some((victim, _)) = target {
                hits.push(Hit {
                    bypass_armor: false,
                    victim,
                    owner: Some(bolt.owner),
                    damage: payload.damage,
                    impulse: unit(bolt.velocity) * 90.0,
                });
            } else if wall.is_some() {
                sounds.extend_at(
                    art.contact.iter().chain(&art.sound).take(1).cloned(),
                    position,
                    CONTACT_SOUND_RADIUS,
                    0.9,
                );
            }
            false
        });
        self.detonate(map, detonations, hits, sounds);
    }

    fn detonate(
        &mut self,
        map: &Map,
        mut queue: Vec<Detonation>,
        hits: &mut Vec<Hit>,
        sounds: &mut SoundEvents,
    ) {
        let mut next = 0;
        while next < queue.len() {
            let Detonation {
                position,
                owner,
                payload,
            } = queue[next];
            next += 1;
            let art = &map.ordnance[payload.kind as usize];
            for (victim, fighter) in self
                .fighters
                .iter()
                .enumerate()
                .filter(|(_, f)| f.health > 0)
            {
                let delta = fighter.walker.position - position;
                let distance = (delta.dot(delta).sqrt() - PLAYER_RADIUS).max(0.0);
                if distance >= payload.blast || map.collision.ray(position, delta).is_some() {
                    continue;
                }
                let strength = 1.0 - distance / payload.blast;
                hits.push(Hit {
                    bypass_armor: false,
                    victim,
                    owner: Some(owner),
                    damage: (f32::from(payload.damage) * strength) as u16,
                    impulse: unit(delta) * (370.0 * strength),
                });
            }
            self.traces.push(Trace {
                start: position,
                end: position,
                explosion: Some(payload.kind),
                born: self.time,
                scale: payload.scale,
                expires: self.time + 0.48,
            });
            sounds.extend_at(art.sound.iter().cloned(), position, BLAST_SOUND_RADIUS, 1.0);
            self.bolts.retain(|bolt| {
                let delta = bolt.position - position;
                if bolt.payload.blast > 0.0
                    && delta.dot(delta) < payload.blast.powi(2)
                    && map.collision.ray(position, delta).is_none()
                {
                    queue.push(Detonation {
                        position: bolt.position,
                        owner,
                        payload: bolt.payload,
                    });
                    false
                } else {
                    true
                }
            });
        }
    }

    fn apply_hit(&mut self, map: &Map, hit: Hit, sounds: &mut SoundEvents) {
        let victim = &mut self.fighters[hit.victim];
        if victim.health == 0
            || hit.damage == 0
            || (hit.owner.is_some() && self.time < victim.protected_until)
        {
            return;
        }
        let (damage, shield) = if hit.bypass_armor {
            (hit.damage, false)
        } else {
            super::absorb_damage(&mut victim.armor, hit.damage)
        };
        if shield {
            victim.shield_at = self.time;
            sounds.extend_at(
                map.powerups.shield_hit.iter().cloned(),
                victim.walker.position,
                CONTACT_SOUND_RADIUS,
                0.85,
            );
        }
        victim.health = victim.health.saturating_sub(damage);
        victim.walker.add_velocity(hit.impulse);
        if let Some(owner) = hit.owner.filter(|&owner| owner != hit.victim)
            && hit.impulse.dot(hit.impulse) > 0.0
        {
            victim.last_hostile_impulse = Some((owner, self.time));
        }
        if damage == 0 {
            return;
        }
        let since_pain = self.time - victim.hurt_at;
        victim.hurt_at = self.time;
        if victim.health > 0 {
            if since_pain > 0.37 {
                let sound = &map.player_audio.pain[(usize::from(hit.damage) / 12).min(3)];
                sounds.extend_at(
                    sound.iter().cloned(),
                    victim.walker.position,
                    CONTACT_SOUND_RADIUS,
                    0.8,
                );
            }
            return;
        }
        victim.deaths += 1;
        victim.charge = None;
        victim.fell_at = Some(self.time);
        for bolt in &mut self.bolts {
            if bolt.owner == hit.victim {
                bolt.pilot = None;
            }
        }
        sounds.extend_at(
            map.player_audio.death.iter().flatten().take(1).cloned(),
            victim.walker.position,
            CONTACT_SOUND_RADIUS,
            1.0,
        );
        let owner = hit.owner.or_else(|| {
            victim
                .last_hostile_impulse
                .filter(|&(_, at)| self.time - at <= ENVIRONMENT_CREDIT_WINDOW)
                .map(|(owner, _)| owner)
        });
        if let Some(owner) = owner.filter(|&owner| owner != hit.victim) {
            self.fighters[owner].score += 1;
            self.message = format!("{} FRAGGED {}", Self::label(owner), Self::label(hit.victim));
        } else {
            self.fighters[hit.victim].score -= 1;
            self.message = format!("{} WENT DOWN", Self::label(hit.victim));
        }
        self.message_until = self.time + 2.5;
    }

    fn bot_command(&mut self, map: &Map, id: usize, dt: f32) -> Command {
        let fighter = &self.fighters[id];
        if fighter.health == 0 {
            return Command::default();
        }
        if let Some(command) = follow_transfer(map, &mut self.fighters[id], self.time, dt) {
            return command;
        }
        let skill = self.rules.difficulty.combat();
        let fighter = &self.fighters[id];
        if let Some(evading) = &fighter.bot.evasion
            && self.time < evading.until
        {
            return evade(fighter, evading.direction, dt);
        }
        if self.time >= fighter.bot.evade_again_at
            && let Some(direction) = self.evade_direction(map, id, &skill)
        {
            let fighter = &mut self.fighters[id];
            fighter.bot.evasion = Some(Evasion {
                direction,
                until: self.time + 0.32,
            });
            fighter.bot.evade_again_at = self.time + 0.9;
            let mut command = evade(fighter, direction, dt);
            if skill.dodge {
                command.dodge = Some(command.movement);
            }
            return command;
        }
        let fighter = &self.fighters[id];
        let eye = fighter.camera(map);
        let continuing = fighter.bot.target;
        let tracked_alive = continuing.is_some_and(|target| self.fighters[target].health > 0);
        let attacker = fighter
            .last_hostile_impulse
            .filter(|&(_, at)| self.time - at < 0.7)
            .map(|(id, _)| id);
        let target = self
            .fighters
            .iter()
            .enumerate()
            .filter(|(other, fighter)| {
                *other != id && fighter.health > 0 && self.time >= fighter.protected_until
            })
            .filter_map(|(other, fighter)| {
                let delta = fighter.walker.position + Vec3(0.0, 0.0, 12.0) - eye.position;
                let distance = delta.dot(delta);
                let known = continuing == Some(other) || attacker == Some(other);
                if !known
                    && distance > 180.0_f32.powi(2)
                    && delta.dot(eye.forward()) < 0.15 * delta.0.hypot(delta.1)
                {
                    return None;
                }
                if fighter.invis_left(self.time) > 0.0
                    && self.time - fighter.fired_at > 0.42
                    && distance > 145.0_f32.powi(2)
                {
                    return None;
                }
                (distance < 1450.0_f32.powi(2) && map.collision.ray(eye.position, delta).is_none())
                    .then_some((
                        other,
                        fighter.walker.position,
                        delta,
                        distance,
                        fighter.walker.velocity(),
                    ))
            })
            .min_by(|a, b| {
                let score = |id, distance| {
                    if Some(id) == continuing {
                        distance * 0.55
                    } else {
                        distance
                    }
                };
                f32::total_cmp(&score(a.0, a.3), &score(b.0, b.3))
            })
            .filter(|&(next, _, _, distance, _)| {
                !tracked_alive
                    || continuing == Some(next)
                    || attacker == Some(next)
                    || fighter.bot.last_seen.is_none_or(|(point, at)| {
                        self.time - at >= 0.28
                            || distance < distance_squared(point, eye.position) * 0.44
                    })
            });
        let fighter = &mut self.fighters[id];
        if let Some(command) = try_retreat(
            &self.navigation,
            map,
            fighter,
            &skill,
            target.is_some(),
            self.time,
            dt,
        ) {
            return command;
        }
        let bot = &mut fighter.bot;
        if let Some((other, target_position, mut delta, distance, velocity)) = target {
            bot.last_seen = Some((target_position, self.time));
            bot.plan_origin = None;
            if bot.target != Some(other) {
                bot.target = Some(other);
                bot.react_at = self.time + skill.reaction + (id % 3) as f32 * 0.07;
                bot.weapon_mode = None;
            }
            let range = distance.sqrt();
            let charge = fighter.charge;
            let weapon = if charge.is_some()
                || (self.time - fighter.selected_at < 1.1
                    && fighter.inventory.usable(fighter.weapon))
            {
                fighter.weapon
            } else {
                fighter.inventory.preferred(range)
            };
            let (alt, shot) = charge.map_or_else(
                || {
                    let ammo = fighter.inventory.ammo(weapon);
                    if let Some((chosen, alt, until)) = bot.weapon_mode
                        && chosen == weapon
                        && self.time < until
                        && ammo >= arsenal::shot(weapon, alt).ammo
                    {
                        return (alt, arsenal::shot(weapon, alt));
                    }
                    let result = arsenal::bot_shot(weapon, ammo, range);
                    bot.weapon_mode = Some((weapon, result.0, self.time + 0.55));
                    result
                },
                |charge| (charge.alternate, arsenal::shot(weapon, charge.alternate)),
            );
            let mut clear = true;
            let attack_range = match shot.path {
                Path::Missile {
                    kind,
                    blast,
                    gravity,
                    ..
                } => {
                    let speed = arsenal::velocity(map, kind);
                    let lead = ai::intercept(delta, velocity, speed);
                    let travel = lead.dot(lead).sqrt() / speed;
                    let fall = -map.collision.regions.at(eye.position).gravity.2;
                    let lift = gravity * (0.5 * fall * travel * travel - 115.0 * travel);
                    let aim = delta + (lead - delta) * skill.lead + Vec3(0.0, 0.0, lift);
                    clear = range > blast * 1.08;
                    if map.collision.ray(eye.position, aim).is_none() {
                        delta = aim;
                    }
                    speed * if gravity > 0.0 { 0.95 } else { 1.45 }
                }
                Path::Trace { range, .. } => range,
                Path::Zoom => 0.0,
            };
            let strafe = (self.time * 1.75 + id as f32 * 1.7).sin();
            let error = range.max(370.0) * skill.error;
            let desired_bias = ai::shot_bias(id, fighter.fired_count);
            let bias = bot.aim_bias.get_or_insert(desired_bias);
            for (axis, desired) in bias.iter_mut().zip(desired_bias) {
                *axis += (desired - *axis) * (1.0 - (-12.0 * dt).exp());
            }
            let [side_error, height_error] = *bias;
            delta = delta
                + eye.right() * (side_error * error)
                + Vec3(0.0, 0.0, height_error * error * 0.55);
            let (turn, pitch) = aim_error(delta, fighter.yaw, fighter.pitch);
            let yaw = turn.clamp(-skill.turn * dt, skill.turn * dt);
            let tilt = pitch.clamp(-skill.turn * dt * 0.75, skill.turn * dt * 0.75);
            let firing = self.time >= bot.react_at
                && (turn - yaw).abs() < 0.045
                && (pitch - tilt).abs() < 0.045
                && range < attack_range
                && clear;
            let release_charge = firing
                && charge.is_some_and(|charge| {
                    let wanted = match self.rules.difficulty {
                        Difficulty::Easy => 1.0,
                        Difficulty::Standard => 2.0,
                        Difficulty::Hard => 3.0,
                    };
                    self.time - charge.since >= charge.profile.period * (wanted - 0.85)
                });
            let hold_trigger = firing && !release_charge && fighter.charge_locked != Some(alt);
            let advance = if weapon == Weapon::Hammer {
                ((range - 49.0) / 120.0).clamp(0.0, 0.85)
            } else {
                ((range - (attack_range * 0.58).min(360.0)) / 450.0).clamp(-0.28, 0.65)
            };
            let offset = target_position - fighter.walker.position;
            let forward = unit(Vec3(offset.0, offset.1, 0.0));
            let right = Vec3(-forward.1, forward.0, 0.0);
            let desired = forward * advance + right * (strafe * skill.strafe);
            bot.combat_move =
                bot.combat_move + (desired - bot.combat_move) * (1.0 - (-8.0 * dt).exp());
            let movement = safe_combat_move(map, &fighter.walker, bot.combat_move);
            return Command {
                movement: movement_input(movement, fighter.yaw + yaw),
                look: [yaw, tilt],
                primary: hold_trigger && !alt,
                alternate: hold_trigger && alt,
                release_charge,
                preferred: (weapon != fighter.weapon).then_some(weapon),
                ..Command::default()
            };
        }
        if let Some((point, seen)) = bot.last_seen
            && tracked_alive
            && self.time - seen < 0.28
        {
            let (turn, pitch) = aim_error(point - eye.position, fighter.yaw, fighter.pitch);
            let yaw = turn.clamp(-skill.turn * dt, skill.turn * dt);
            bot.combat_move = bot.combat_move * (-7.0 * dt).exp();
            let movement = safe_combat_move(map, &fighter.walker, bot.combat_move);
            return Command {
                movement: movement_input(movement, fighter.yaw + yaw),
                look: [yaw, pitch.clamp(-skill.turn * dt, skill.turn * dt)],
                ..Command::default()
            };
        }
        if bot.target.take().is_some() {
            bot.goal = None;
            bot.rethink_at = self.time;
        }
        bot.combat_move = Vec3::default();
        let position = fighter.walker.position;
        let check_progress = self.time >= bot.rethink_at;
        let arrived = matches!(bot.goal, Some(Step::Walk(point)) if near_waypoint(point, position));
        let stalled = check_progress
            && !arrived
            && bot.plan_origin.is_some_and(|origin| {
                matches!(bot.goal, Some(Step::Walk(_)))
                    && distance_squared(origin, position) < 24.0_f32.powi(2)
            });
        if arrived || stalled || (check_progress && bot.goal.is_none()) {
            bot.plan_origin = Some(position);
            let mut supplies: Vec<_> = map
                .props
                .iter()
                .filter(|prop| {
                    prop.visible
                        && distance_squared(prop.position, position) > 42.0_f32.powi(2)
                        && match prop.item {
                            Some(Item::Health(_, max)) => fighter.health < max.min(80),
                            Some(Item::Armor(kind)) => {
                                fighter.armor[kind as usize] < kind.pickup_charge(&fighter.armor)
                            }
                            Some(Item::Power(power, charge)) => {
                                fighter.power.wants(power, charge, self.time)
                            }
                            Some(Item::Gun(weapon)) => !fighter.inventory.owns(weapon),
                            Some(Item::Ammo(weapon, _)) => {
                                fighter.inventory.owns(weapon)
                                    && fighter.inventory.ammo(weapon)
                                        < super::max_ammo(&map.arsenal, weapon) / 3
                            }
                            _ => false,
                        }
                })
                .map(|prop| prop.position)
                .collect();
            supplies.sort_by(|a, b| {
                distance_squared(*a, position).total_cmp(&distance_squared(*b, position))
            });
            let chase = bot
                .last_seen
                .filter(|&(point, time)| {
                    self.time - time < 3.0 && distance_squared(point, position) > 90.0_f32.powi(2)
                })
                .map(|(point, _)| point);
            let supply = supplies
                .into_iter()
                .take(if stalled { 0 } else { 2 })
                .find_map(|point| self.navigation.next_step(&map.collision, position, point));
            let pursue = (!stalled)
                .then_some(chase)
                .flatten()
                .and_then(|point| self.navigation.next_step(&map.collision, position, point));
            let stocked = fighter.inventory.preferred(560.0).switch_priority()
                > Weapon::Enforcer.switch_priority();
            bot.goal = if stocked {
                pursue.or(supply)
            } else {
                supply.or(pursue)
            };
            if bot.goal.is_none() {
                bot.goal = (0..12).find_map(|offset| {
                    let yaw = fighter.yaw
                        + (offset as f32 + id as f32 + self.time.floor()) * std::f32::consts::TAU
                            / 12.0;
                    let direction = Vec3(yaw.cos(), yaw.sin(), 0.0);
                    [280.0, 140.0].into_iter().find_map(|distance| {
                        let point = position + direction * distance;
                        navigation::walk_link(&map.collision, position, point)
                            .then_some(Step::Walk(point))
                    })
                });
            }
            bot.rethink_at = self.time + 1.35;
        } else if check_progress {
            bot.plan_origin = Some(position);
            bot.rethink_at = self.time + 1.35;
        }
        if let Some(command) = follow_transfer(map, fighter, self.time, dt) {
            return command;
        }
        if let Some(Step::Walk(goal)) = fighter.bot.goal {
            return steer(fighter, goal, dt);
        }
        Command {
            look: [(1.8 * dt).min(1.5), -fighter.pitch * (dt * 4.0).min(1.0)],
            ..Command::default()
        }
    }

    fn evade_direction(&self, map: &Map, id: usize, skill: &ai::Combat) -> Option<Vec3> {
        let fighter = &self.fighters[id];
        let position = fighter.walker.position;
        if !fighter.walker.grounded || map.collision.regions.at(position).water {
            return None;
        }
        let threat = self
            .bolts
            .iter()
            .filter_map(|bolt| {
                if bolt.owner == id || self.time - bolt.born < skill.reaction.min(0.16) {
                    return None;
                }
                let relative = bolt.velocity - fighter.walker.velocity();
                let speed_squared = relative.dot(relative);
                if speed_squared < 100.0 {
                    return None;
                }
                let offset = bolt.position - position;
                let closest = -offset.dot(relative) / speed_squared;
                if !(0.02..=skill.evade_window).contains(&closest) {
                    return None;
                }
                let pass = offset + relative * closest;
                let clearance = 43.0 + bolt.payload.blast.min(150.0) * 0.53;
                if pass.dot(pass) >= clearance * clearance
                    || map.collision.ray(position, offset).is_some()
                    || map
                        .collision
                        .ray(bolt.position, bolt.velocity * closest)
                        .is_some()
                {
                    return None;
                }
                let side = unit(Vec3(-relative.1, relative.0, 0.0));
                (side.dot(side) > 0.5).then_some((closest, pass, side))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let (_, pass, side) = threat;
        [side, side * -1.0]
            .into_iter()
            .filter(|&direction| {
                let step = direction * if skill.dodge { 160.0 } else { 95.0 };
                if map
                    .collision
                    .projectile_surface(position, step, PLAYER_RADIUS, PLAYER_HEIGHT)
                    .is_some()
                {
                    return false;
                }
                // Probe both the middle and the end of the escape so the bot
                // doesn't dodge across a hole toward an otherwise safe platform.
                [0.5, 1.0].into_iter().all(|offset| {
                    let point = position + step * offset;
                    let ground = map.collision.ray(point, Vec3(0.0, 0.0, -75.0));
                    let zone = map.collision.regions.at(point);
                    ground.is_some() && !zone.water && !zone.pain
                })
            })
            .min_by(|a, b| a.dot(pass).total_cmp(&b.dot(pass)))
    }
}

fn try_retreat(
    navigation: &Navigation,
    map: &Map,
    fighter: &mut Fighter,
    skill: &ai::Combat,
    under_fire: bool,
    time: f32,
    dt: f32,
) -> Option<Command> {
    if fighter.bot.retreat.as_ref().is_some_and(|plan| {
        time >= plan.until || !map.props[plan.pickup].visible || fighter.health >= 63
    }) {
        fighter.bot.retreat = None;
        fighter.bot.goal = None;
        fighter.bot.rethink_at = time;
        fighter.bot.retreat_retry_at = time + 1.0;
    }
    let position = fighter.walker.position;
    if fighter.bot.retreat.is_none()
        && under_fire
        && fighter.health <= skill.rush_health
        && time >= fighter.bot.retreat_retry_at
    {
        fighter.bot.retreat_retry_at = time + 1.1;
        let mut supplies: Vec<_> = map
            .props
            .iter()
            .enumerate()
            .filter(|(_, prop)| {
                prop.visible
                    && matches!(prop.item, Some(Item::Health(_, cap)) if cap > fighter.health)
            })
            .map(|(id, prop)| (id, distance_squared(prop.position, position)))
            .filter(|(_, distance)| *distance < 1750.0_f32.powi(2))
            .collect();
        supplies.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        if let Some((pickup, step)) = supplies.into_iter().take(3).find_map(|(id, _)| {
            navigation
                .next_step(&map.collision, position, map.props[id].position)
                .map(|step| (id, step))
        }) {
            fighter.bot.retreat = Some(Retreat {
                pickup,
                until: time + 9.0,
            });
            fighter.bot.rethink_at = time + 1.0;
            fighter.bot.goal = Some(step);
            fighter.bot.target = None;
        }
    }
    let pickup = fighter.bot.retreat.as_ref()?.pickup;
    if time >= fighter.bot.rethink_at
        || fighter
            .bot
            .goal
            .is_none_or(|goal| matches!(goal, Step::Walk(point) if near_waypoint(point, position)))
    {
        fighter.bot.goal =
            navigation.next_step(&map.collision, position, map.props[pickup].position);
        fighter.bot.rethink_at = time + 1.0;
        if fighter.bot.goal.is_none() {
            fighter.bot.retreat = None;
            fighter.bot.retreat_retry_at = time + 1.0;
            return None;
        }
    }
    follow_transfer(map, fighter, time, dt).or_else(|| {
        let Step::Walk(point) = fighter.bot.goal? else {
            return None;
        };
        Some(steer(fighter, point, dt))
    })
}

fn follow_transfer(map: &Map, fighter: &mut Fighter, time: f32, dt: f32) -> Option<Command> {
    if fighter.bot.lift.is_none() {
        match fighter.bot.goal? {
            Step::Walk(_) => return None,
            Step::Jump(maneuver) => {
                if fighter.bot.jump_until.is_none() {
                    fighter.bot.jump_until = Some(time + 2.8);
                    fighter.bot.jump = JumpProgress::default();
                }
                let landed = if fighter.bot.jump_until.is_some_and(|until| time < until) {
                    match fighter.bot.jump.command(maneuver, &fighter.walker) {
                        JumpControl::Drive { movement, jump } => {
                            return Some(Command {
                                jump,
                                normal_jump: true,
                                ..drive(fighter, movement, dt)
                            });
                        }
                        JumpControl::Landed => true,
                        JumpControl::Failed => false,
                    }
                } else {
                    false
                };
                fighter.bot.jump_until = None;
                fighter.bot.goal = None;
                fighter.bot.rethink_at = time + if landed { 0.0 } else { 0.4 };
                return Some(Command::default());
            }
            Step::Teleport(position) => {
                if time >= fighter.bot.rethink_at {
                    fighter.bot.goal = None;
                    return None;
                }
                return Some(steer(fighter, position, dt));
            }
            Step::Lift(ride) => {
                let travel = map.movers[ride.mover]
                    .lift_ride_seconds()
                    .unwrap_or_default();
                fighter.bot.lift = Some(ActiveLift {
                    ride,
                    phase: LiftPhase::Board,
                    expires: time + 2.0 * travel + 8.0,
                });
            }
        }
    }
    let lift = fighter.bot.lift.as_mut().unwrap();
    let mover = &map.movers[lift.ride.mover];
    let destination = lift.ride.from.opposite();
    let position = fighter.walker.position;
    let reached_exit = lift.phase == LiftPhase::Leave
        && fighter.walker.grounded
        && !mover.has_rider(&fighter.walker)
        && near_waypoint(lift.ride.exit, position);
    if time >= lift.expires || reached_exit {
        fighter.bot.lift = None;
        fighter.bot.goal = None;
        fighter.bot.rethink_at = time;
        return Some(Command::default());
    }
    let target = match lift.phase {
        LiftPhase::Board => {
            if !mover.lift_at(lift.ride.from) {
                lift.ride.entry
            } else {
                let board = mover.lift_board_position(lift.ride.from);
                if mover.has_rider(&fighter.walker)
                    && (board.0 - position.0).hypot(board.1 - position.1) < 34.0
                {
                    lift.phase = LiftPhase::Ride;
                    return Some(Command::default());
                }
                board
            }
        }
        LiftPhase::Ride if mover.lift_at(destination) => {
            lift.phase = LiftPhase::Leave;
            lift.ride.exit
        }
        LiftPhase::Ride => return Some(Command::default()),
        LiftPhase::Leave => lift.ride.exit,
    };
    let jump = lift.phase == LiftPhase::Leave
        && target.2 - position.2 > map.collision.profile.step_height
        && (target.0 - position.0).hypot(target.1 - position.1) < 130.0;
    Some(Command {
        jump,
        ..steer(fighter, target, dt)
    })
}

fn near_waypoint(waypoint: Vec3, position: Vec3) -> bool {
    let delta = waypoint - position;
    delta.0.hypot(delta.1) < WAYPOINT_RADIUS && delta.2.abs() < 56.0
}

fn safe_combat_move(map: &Map, walker: &Walker, desired: Vec3) -> Vec3 {
    let origin = walker.position;
    if !walker.grounded || map.collision.regions.at(origin).water {
        return desired;
    }
    let safe = |movement: Vec3| {
        let speed = movement.0.hypot(movement.1);
        if speed < 0.01 {
            return true;
        }
        let step = movement * ((PLAYER_RADIUS + 52.0 * speed.min(1.0)) / speed);
        let point = origin + step;
        let ground = map.collision.ray(point, Vec3(0.0, 0.0, -95.0));
        map.collision.ray(origin, step).is_none()
            && ground.is_some_and(|ground| {
                let zone = map.collision.regions.at(ground + Vec3(0.0, 0.0, 4.0));
                !zone.pain && !zone.water
            })
    };
    if safe(desired) {
        desired
    } else {
        let [x, y] = [Vec3(desired.0, 0.0, 0.0), Vec3(0.0, desired.1, 0.0)];
        [x, y]
            .into_iter()
            .filter(|&movement| safe(movement))
            .max_by(|a, b| a.dot(*a).total_cmp(&b.dot(*b)))
            .unwrap_or_default()
    }
}

fn movement_input(movement: Vec3, yaw: f32) -> [f32; 2] {
    let (sin, cos) = yaw.sin_cos();
    [
        movement.dot(Vec3(cos, sin, 0.0)),
        movement.dot(Vec3(-sin, cos, 0.0)),
    ]
}

fn evade(fighter: &Fighter, direction: Vec3, dt: f32) -> Command {
    if fighter.bot.target.is_some() {
        Command {
            movement: movement_input(direction, fighter.yaw),
            ..Command::default()
        }
    } else {
        drive(fighter, direction, dt)
    }
}

fn steer(fighter: &Fighter, point: Vec3, dt: f32) -> Command {
    let delta = point - fighter.walker.position;
    let distance = delta.0.hypot(delta.1);
    if distance < 10.0 {
        return Command::default();
    }
    let speed = (distance / 145.0).clamp(0.19, 0.8);
    drive(
        fighter,
        Vec3(delta.0, delta.1, 0.0) * (speed / distance),
        dt,
    )
}

fn drive(fighter: &Fighter, movement: Vec3, dt: f32) -> Command {
    let speed = movement.0.hypot(movement.1);
    if speed < 0.001 {
        return Command::default();
    }
    let (turn, _) = aim_error(movement, fighter.yaw, fighter.pitch);
    let yaw_change = turn.clamp(-4.6 * dt, 4.6 * dt);
    Command {
        movement: movement_input(movement, fighter.yaw + yaw_change),
        look: [yaw_change, -fighter.pitch * (dt * 4.0).min(1.0)],
        ..Command::default()
    }
}

fn take_item(
    arsenal: &std::collections::HashMap<Weapon, WeaponAssets>,
    fighter: &mut Fighter,
    item: Item,
    time: f32,
) -> bool {
    match item {
        Item::Health(amount, limit) if fighter.health < limit => {
            fighter.health = fighter.health.saturating_add(amount).min(limit);
        }
        Item::Armor(kind) => return kind.pickup(&mut fighter.armor).is_some(),
        Item::Power(power, charge) if fighter.power.wants(power, charge, time) => match power {
            Powerup::Boots => fighter.power.boot_charges = charge,
            Powerup::Damage => fighter.power.amp_until = time + f32::from(charge) * 0.1,
            Powerup::Invisibility => {
                fighter.power.invis_until = time + (f32::from(charge) + 1.0) * 0.5
            }
        },
        Item::Gun(weapon) => return fighter.inventory.pickup(arsenal, weapon),
        Item::Ammo(weapon, amount) => return fighter.inventory.grant_ammo(arsenal, weapon, amount),
        _ => return false,
    }
    true
}

fn nearest_hit(
    fighters: &[Fighter],
    owner: usize,
    start: Vec3,
    delta: Vec3,
    extent: [f32; 2],
) -> Option<(usize, f32)> {
    fighters
        .iter()
        .enumerate()
        .filter(|(id, fighter)| *id != owner && fighter.health > 0)
        .filter_map(|(id, fighter)| {
            let fraction = cylinder_hit(
                start - fighter.walker.position,
                delta,
                PLAYER_RADIUS + extent[0],
                PLAYER_HEIGHT + extent[1],
            )?;
            (fraction < 1.0).then_some((id, fraction))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

fn distance_squared(a: Vec3, b: Vec3) -> f32 {
    let delta = a - b;
    delta.dot(delta)
}

fn usable_start(map: &Map, index: usize) -> Option<Vec3> {
    let start = map.starts[index];
    let walker = Walker::spawn(&map.collision, start.position);
    let zone = map.collision.regions.at(walker.position);
    (start.enabled && walker.grounded && !zone.water && !zone.pain).then_some(walker.position)
}

fn aim_error(delta: Vec3, yaw: f32, pitch: f32) -> (f32, f32) {
    (
        angle_delta(delta.1.atan2(delta.0), yaw),
        delta.2.atan2(delta.0.hypot(delta.1)) - pitch,
    )
}

fn angle_delta(target: f32, current: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;

    fn arena() -> Map {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap()
    }

    fn flight_fixture() -> (Map, Match) {
        let mut map = arena();
        map.collision = crate::collision::Collision::test_world();
        map.collision
            .add_box(Vec3(-900.0, -900.0, -96.0), Vec3(900.0, 900.0, 0.0));
        map.starts = vec![
            crate::map::Start {
                position: Vec3(0.0, 0.0, 39.2),
                yaw: 0.0,
                enabled: true,
            },
            crate::map::Start {
                position: Vec3(520.0, 325.0, 39.2),
                yaw: std::f32::consts::PI,
                enabled: true,
            },
        ];
        map.navigation_points.clear();
        map.movers.clear();
        map.lift_exits.clear();
        map.teleporters.clear();
        map.props.clear();
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let player = &mut game.fighters[0];
        player.inventory.pickup(&map.arsenal, Weapon::Redeemer);
        player
            .inventory
            .grant_ammo(&map.arsenal, Weapon::Redeemer, 1);
        player.weapon = Weapon::Redeemer;
        (map, game)
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn render_rate_does_not_change_match_steps_or_lose_short_presses() {
        let (mut map, mut game) = flight_fixture();
        let mut baseline = None;
        for hz in [60, 144, 240] {
            game.restart(&mut map);
            let mut clock = Timestep::default();
            game.fighters[0].protected_until = f32::INFINITY;
            for _ in 0..hz {
                clock.advance(
                    &mut game,
                    &mut map,
                    Command {
                        movement: [0.68, -0.12],
                        primary: true,
                        look: [0.52 / hz as f32, 0.0],
                        ..Command::default()
                    },
                    [false; 2],
                    false,
                    1.0 / f64::from(hz),
                );
            }
            let snapshot = (
                game.time,
                game.fighters[0].walker.position,
                game.fighters[0].ammo(),
            );
            if let Some((time, position, ammo)) = baseline {
                assert_eq!(snapshot.0, time);
                assert_eq!(snapshot.2, ammo);
                assert!(
                    distance_squared(snapshot.1, position) < 0.0025,
                    "{hz}Hz diverged"
                );
            } else {
                baseline = Some(snapshot);
            }
        }
        game.restart(&mut map);
        let mut clock = Timestep::default();
        let ammo = game.fighters[0].ammo().unwrap();
        clock.advance(
            &mut game,
            &mut map,
            Command::default(),
            [true, false],
            true,
            1.0 / 240.0,
        );
        assert_eq!(game.time, 0.0);
        clock.advance(
            &mut game,
            &mut map,
            Command::default(),
            [false; 2],
            false,
            1.0 / 240.0,
        );
        assert_eq!(game.fighters[0].ammo(), Some(ammo - 1));
        assert!(game.fighters[0].walker.velocity().2 > 0.0);
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn nearest_eligible_fighter_gets_a_contested_pickup() {
        let (mut map, _) = flight_fixture();
        map.props.push(crate::props::Prop {
            tag: None,
            faces: Vec::new(),
            position: Vec3(75.0, 0.0, 39.2),
            radius: 20.0,
            pickup_radius: 30.0,
            pickup_height: 25.0,
            item: Some(Item::Health(20, 100)),
            visible: true,
            pickup_sound: None,
            respawn_time: Some(0.5),
            cinematic: None,
        });
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        for (id, x) in [(0, 44.0), (1, 82.0)] {
            game.fighters[id].walker = Walker::spawn(&map.collision, Vec3(x, 0.0, 39.2));
            game.fighters[id].health = 80;
        }
        let mut sounds = SoundEvents::default();
        game.collect(&mut map, &mut sounds);
        assert_eq!(
            (game.fighters[0].health, game.fighters[1].health),
            (80, 100)
        );
        assert!(!map.props[0].visible);
        game.time = 0.6;
        game.collect(&mut map, &mut sounds);
        assert_eq!(
            (game.fighters[0].health, game.fighters[1].health),
            (100, 100)
        );
        assert!(!map.props[0].visible);
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn drowning_ignores_armor_while_shallow_hazard_damage_uses_it() {
        let (mut map, mut game) = flight_fixture();
        game.fighters[0].protected_until = f32::INFINITY;
        let belt = super::super::ArmorKind::Belt;
        assert!(belt.pickup(&mut game.fighters[0].armor).is_some());
        map.collision.regions.level.water = true;
        map.collision.profile.underwater_time = 0.12;
        for _ in 0..16 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert_eq!(
            (game.fighters[0].health, game.fighters[0].armor()),
            (95, 150)
        );
        map.collision.regions.level.water = false;
        map.collision.regions.zones = vec![crate::movement::Zone {
            pain: true,
            water: true,
            damage_per_second: 20,
            ..crate::movement::Zone::default()
        }];
        map.collision.regions.nodes = vec![crate::movement::RegionNode {
            plane: crate::collision::Plane {
                normal: Vec3(0.0, 0.0, 1.0),
                distance: 20.0,
            },
            children: [-1, -1],
            zones: [0, -1],
        }];
        for _ in 0..10 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert_eq!(
            (game.fighters[0].health, game.fighters[0].armor()),
            (95, 142)
        );
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn bot_dodges_a_visible_projectile_but_not_one_behind_cover() {
        let (mut map, mut game) = flight_fixture();
        game.fighters[0].protected_until = f32::INFINITY;
        let start = game.fighters[1].walker.position;
        let payload = ProjectilePayload {
            kind: super::super::ordnance::Kind::Pulse,
            damage: 70,
            blast: 0.0,
            gravity: 0.0,
            rebounds: 0,
            scale: 1.0,
        };
        let bolt_start = start - Vec3(220.0, 0.0, 0.0);
        game.bolts.push(Bolt {
            owner: 0,
            position: bolt_start,
            velocity: Vec3(700.0, 0.0, 0.0),
            expires: 2.0,
            born: -0.30,
            payload,
            pilot: None,
        });
        for _ in 0..40 {
            game.step(&mut map, Command::default(), 1.0 / 120.0);
        }
        assert_eq!(game.fighters[1].health, 100);
        assert!((game.fighters[1].walker.position.1 - start.1).abs() > 70.0);
        game.fighters[1].walker = Walker::spawn(&map.collision, start);
        game.bolts[0].position = bolt_start;
        let wall = start - Vec3(95.0, 0.0, 0.0);
        map.collision
            .add_box(wall - Vec3(7.0, 95.0, 50.0), wall + Vec3(7.0, 95.0, 70.0));
        assert!(
            game.evade_direction(&map, 1, &Difficulty::Standard.combat())
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn piloting_shifts_input_and_camera_until_an_armed_manual_detonation() {
        let (mut map, mut game) = flight_fixture();
        game.fighters[0].protected_until = f32::INFINITY;
        let dt = 1.0 / 60.0;
        let ammo = game.fighters[0].ammo().unwrap();
        let launch = Command {
            alternate: true,
            ..Command::default()
        };
        game.step(&mut map, launch, dt);
        assert!(game.guidance_left().is_some());
        assert_eq!(game.fighters[0].ammo(), Some(ammo - 1));
        let old_body = game.fighters[0].walker.position;
        let old_bot = game.fighters[1].walker.position;
        for _ in 0..16 {
            game.step(
                &mut map,
                Command {
                    look: [0.035, 0.0],
                    movement: [1.0, 0.0],
                    jump: true,
                    ..launch
                },
                dt,
            );
        }
        assert!(distance_squared(game.fighters[0].walker.position, old_body) < 1.0);
        assert!(distance_squared(game.fighters[1].walker.position, old_bot) > 10.0);
        assert!(game.player_view(&map).yaw > 0.45);
        assert!(distance_squared(game.player_view(&map).position, old_body) > 5_000.0);
        assert_eq!(
            game.bolts().count(),
            0,
            "hide only the player's guided projectile"
        );

        game.step(&mut map, Command::default(), dt);
        let detonate = Command {
            primary: true,
            ..Command::default()
        };
        game.step(&mut map, detonate, dt);
        assert!(game.guidance_left().is_none() && game.bolts.is_empty());
        assert!(game.traces.iter().any(|trace| trace.explosion.is_some()));
        assert!(
            distance_squared(
                game.player_view(&map).position,
                game.fighters[0].camera(&map).position
            ) < 0.01
        );
        game.fighters[0].ready_at = 0.0;
        game.step(&mut map, detonate, dt);
        assert!(
            game.bolts.is_empty(),
            "the detonation press must not fire another shot"
        );
        game.step(&mut map, Command::default(), dt);
        game.step(&mut map, detonate, dt);
        assert_eq!(game.bolts.len(), 1);
        assert!(game.bolts[0].pilot.is_none());
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn guidance_ends_on_impact_interception_expiry_and_owner_death() {
        let (mut map, mut game) = flight_fixture();
        let dt = 1.0 / 60.0;
        let launch = Command {
            alternate: true,
            ..Command::default()
        };
        for cause in ["expiry", "death", "interception", "wall"] {
            game.restart(&mut map);
            let owner = &mut game.fighters[0];
            owner.inventory.pickup(&map.arsenal, Weapon::Redeemer);
            owner.weapon = Weapon::Redeemer;
            game.step(&mut map, launch, dt);
            assert!(game.guidance_left().is_some());
            let mut sounds = SoundEvents::default();
            match cause {
                "expiry" => {
                    game.bolts[0].expires = game.time;
                    game.step(&mut map, Command::default(), dt);
                    assert!(game.bolts.is_empty());
                }
                "death" => {
                    game.fighters[0].protected_until = 0.0;
                    game.apply_hit(
                        &map,
                        Hit {
                            bypass_armor: false,
                            victim: 0,
                            owner: Some(1),
                            damage: 1000,
                            impulse: Vec3::default(),
                        },
                        &mut sounds,
                    );
                    assert_eq!(game.fighters[1].score, 1);
                    assert_eq!(game.bolts[0].owner, 0);
                    assert!(game.bolts[0].pilot.is_none());
                }
                "interception" => {
                    let bot = &mut game.fighters[1];
                    bot.inventory.pickup(&map.arsenal, Weapon::Shock);
                    bot.weapon = Weapon::Shock;
                    bot.yaw = std::f32::consts::PI;
                    let eye = bot.camera(&map).position;
                    game.bolts[0].position = eye - Vec3(190.0, 0.0, 0.0);
                    game.fire(
                        &map,
                        1,
                        Command {
                            primary: true,
                            ..Command::default()
                        },
                        &mut Vec::new(),
                        &mut sounds,
                    );
                    assert!(game.bolts.is_empty());
                    assert!(game.traces.iter().any(|trace| trace.explosion.is_some()));
                }
                "wall" => {
                    map.collision
                        .add_box(Vec3(122.0, -90.0, 0.0), Vec3(194.0, 90.0, 145.0));
                    for _ in 0..60 {
                        game.step(&mut map, launch, dt);
                        if game.guidance_left().is_none() {
                            break;
                        }
                    }
                    assert!(game.bolts.is_empty());
                    assert!(game.traces.iter().any(|trace| trace.explosion.is_some()));
                }
                _ => unreachable!(),
            }
            assert!(game.guidance_left().is_none(), "{cause}");
            assert!(
                distance_squared(
                    game.player_view(&map).position,
                    game.fighters[0].camera(&map).position
                ) < 0.01
            );
        }
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn rocket_releases_a_spread_and_does_not_rearm_until_the_trigger_is_released() {
        let map = arena();
        let mut game = Match::new(&map, 6, Rules::default());
        let mut sounds = SoundEvents::default();
        let mut hits = Vec::new();
        let held = Command {
            primary: true,
            ..Command::default()
        };
        let fighter = &mut game.fighters[0];
        fighter.inventory.pickup(&map.arsenal, Weapon::Rocket);
        fighter
            .inventory
            .grant_ammo(&map.arsenal, Weapon::Rocket, 20);
        fighter.weapon = Weapon::Rocket;
        let starting_ammo = fighter.ammo().unwrap();
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        game.time = 0.74;
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        assert!(game.bolts.is_empty());
        assert_eq!(game.fighters[0].ammo(), Some(starting_ammo));
        assert_eq!(game.fighters[0].charge_status(game.time).unwrap().0, 3);
        game.fire(&map, 0, Command::default(), &mut hits, &mut sounds);
        assert_eq!(game.bolts.len(), 3);
        assert!(game.bolts.iter().all(|bolt| bolt.owner == 0));
        let spread = game.bolts[0].velocity - game.bolts[2].velocity;
        assert!(spread.dot(spread) > 500.0);
        assert_eq!(game.fighters[0].ammo(), Some(starting_ammo - 3));

        game.time = 2.0;
        let remainder = game.fighters[0].ammo().unwrap() - 2;
        assert!(game.fighters[0].inventory.spend(Weapon::Rocket, remainder));
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        game.time = 2.4;
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        assert_eq!(game.bolts.len(), 5);
        assert_eq!(game.fighters[0].ammo(), Some(0));
        game.fighters[0]
            .inventory
            .grant_ammo(&map.arsenal, Weapon::Rocket, 4);
        game.time = 4.0;
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        assert!(game.fighters[0].charge.is_none());
        game.fire(&map, 0, Command::default(), &mut hits, &mut sounds);
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        assert!(game.fighters[0].charge.is_some());
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn bio_charge_grows_one_projectile_and_switching_or_bot_abort_cancels_it() {
        let map = arena();
        let mut game = Match::new(&map, 6, Rules::default());
        let mut sounds = SoundEvents::default();
        let mut hits = Vec::new();
        let held = Command {
            alternate: true,
            ..Command::default()
        };
        for fighter in &mut game.fighters[..2] {
            fighter.inventory.pickup(&map.arsenal, Weapon::Bio);
            fighter.weapon = Weapon::Bio;
        }
        let starting_ammo = game.fighters[0].ammo();
        game.fire(&map, 0, held, &mut hits, &mut sounds);
        game.time = 1.20;
        game.fire(
            &map,
            0,
            Command {
                slot: Some(2),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert!(game.bolts.is_empty());
        assert_eq!(
            Some(game.fighters[0].inventory.ammo(Weapon::Bio)),
            starting_ammo
        );
        assert!(game.fighters[0].charge.is_none());

        let bot_ammo = game.fighters[1].ammo().unwrap();
        game.fire(&map, 1, held, &mut hits, &mut sounds);
        game.time = 1.8;
        game.fire(&map, 1, Command::default(), &mut hits, &mut sounds);
        assert_eq!(game.fighters[1].ammo(), Some(bot_ammo));
        assert!(game.bolts.is_empty());
        game.fire(&map, 1, held, &mut hits, &mut sounds);
        game.time = 2.91;
        game.fire(
            &map,
            1,
            Command {
                release_charge: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        let base = arsenal::shot(Weapon::Bio, true);
        assert_eq!(game.bolts.len(), 1);
        assert_eq!(game.bolts[0].owner, 1);
        assert!(game.bolts[0].payload.damage > base.damage);
        assert!(game.bolts[0].payload.scale > 1.0);
        assert_eq!(game.fighters[1].ammo(), Some(bot_ammo - 3));
    }

    #[test]
    #[ignore = "requires local GOTY assets"]
    fn bot_combat_holds_and_releases_its_own_rocket_charge() {
        let mut map = arena();
        map.collision = crate::collision::Collision::test_world();
        map.collision
            .add_box(Vec3(-900.0, -900.0, -96.0), Vec3(900.0, 900.0, 0.0));
        map.starts = vec![
            crate::map::Start {
                position: Vec3(440.0, 0.0, 39.2),
                yaw: std::f32::consts::PI,
                enabled: true,
            },
            crate::map::Start {
                position: Vec3(0.0, 0.0, 39.2),
                yaw: 0.0,
                enabled: true,
            },
        ];
        map.navigation_points.clear();
        map.movers.clear();
        map.lift_exits.clear();
        map.teleporters.clear();
        map.props.clear();
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        game.fighters[0].protected_until = 0.0;
        let bot = &mut game.fighters[1];
        bot.inventory.pickup(&map.arsenal, Weapon::Rocket);
        bot.weapon = Weapon::Rocket;
        let ammo_before = bot.ammo().unwrap();
        let mut charging = false;
        for _ in 0..150 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            charging |= game.fighters[1].charge.is_some();
            if !game.bolts.is_empty() {
                break;
            }
        }
        assert!(charging);
        assert_eq!(game.bolts.len(), 2);
        assert!(game.bolts.iter().all(|bolt| bolt.owner == 1));
        assert_eq!(game.fighters[1].ammo(), Some(ammo_before - 2));
    }

    #[test]
    #[ignore = "requires local GOTY map"]
    fn deck16_bot_executes_a_planned_jump() {
        let mut map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let route = game
            .navigation
            .jumps()
            .filter(|jump| {
                jump.end.2 >= jump.start.2 - 12.0
                    && distance_squared(jump.start, game.fighters[0].walker.position)
                        > 220.0_f32.powi(2)
            })
            .find_map(|jump| {
                match game
                    .navigation
                    .next_step(&map.collision, jump.start, jump.end)
                {
                    Some(Step::Jump(first)) if distance_squared(first.end, jump.end) < 1.0 => {
                        Some(first)
                    }
                    _ => None,
                }
            })
            .expect("the stock map has a usable directed jump");
        game.fighters[0].protected_until = f32::INFINITY;
        let bot = &mut game.fighters[1];
        bot.walker = Walker::spawn(&map.collision, route.start);
        bot.bot.goal = Some(Step::Jump(route));
        let mut airborne = false;
        for _ in 0..360 {
            game.step(&mut map, Command::default(), 1.0 / 144.0);
            airborne |= !game.fighters[1].walker.grounded;
            if game.fighters[1].bot.goal.is_none() {
                break;
            }
        }
        let bot = &game.fighters[1];
        assert!(airborne && bot.walker.grounded);
        assert!(
            bot.bot.goal.is_none()
                && distance_squared(bot.walker.position, route.end) < 56.0_f32.powi(2),
            "bot stopped at {:?}, expected {:?}",
            bot.walker.position,
            route.end
        );
    }

    #[test]
    #[ignore = "requires local GOTY asset fixtures"]
    fn bot_runs_up_jumps_and_lands_across_a_gap_without_spending_boots() {
        let mut map = arena();
        map.collision = crate::collision::Collision::test_world();
        map.movers.clear();
        map.teleporters.clear();
        map.lift_exits.clear();
        map.props.clear();
        let entry = Vec3(0.0, 0.0, 39.2);
        let exit = Vec3(231.0, 0.0, 39.2);
        map.starts = vec![crate::map::Start {
            position: Vec3(-175.0, 0.0, 39.2),
            yaw: 0.0,
            enabled: true,
        }];
        map.navigation_points = vec![entry, exit];
        map.collision
            .add_box(Vec3(-270.0, -105.0, -95.0), Vec3(73.0, 105.0, 0.0));
        map.collision
            .add_box(Vec3(160.0, -105.0, -95.0), Vec3(400.0, 105.0, 0.0));
        for dt in [1.0 / 60.0, 1.0 / 240.0] {
            let mut game = Match::new(
                &map,
                0,
                Rules {
                    bots: 1,
                    ..Rules::default()
                },
            );
            game.fighters[0].protected_until = f32::INFINITY;
            let bot = &mut game.fighters[1];
            bot.walker = Walker::spawn(&map.collision, entry);
            bot.yaw = 1.0;
            bot.power.boot_charges = 3;
            let from = bot.walker.position;
            let route = game.navigation.next_step(&map.collision, from, exit);
            assert!(matches!(route, Some(Step::Jump(_))), "{route:?}");
            bot.bot.goal = route;
            let mut rose = false;
            for _ in 0..(2.7 / dt) as usize {
                game.step(&mut map, Command::default(), dt);
                let bot = &game.fighters[1];
                rose |= bot.walker.position.2 > from.2 + 28.0;
                if bot.bot.goal.is_none() {
                    break;
                }
            }
            let bot = &game.fighters[1];
            assert!(rose, "crossing must leave the platform with an upward jump");
            assert!(
                bot.walker.grounded && bot.walker.position.0 > 178.0,
                "failed to land at {dt}s: {:?}",
                bot.walker.position
            );
            assert_eq!(bot.boot_charges(), 3);
        }
        map.collision
            .add_box(Vec3(117.0, -115.0, -100.0), Vec3(128.0, 115.0, 180.0));
        let navigation = Navigation::new(&map);
        assert!(
            navigation.next_step(&map.collision, entry, exit).is_none(),
            "a solid wall must invalidate the edge"
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn an_intercepted_projectile_credits_its_attacker_and_respects_cover() {
        let mut map = arena();
        let mut orientation = None;
        for start in 0..map.starts.len() {
            let pawn = Fighter::spawn(&map, start, 0.0);
            let eye = pawn.camera(&map).position;
            for index in 0..4 {
                let yaw = (index as f32) * std::f32::consts::FRAC_PI_2;
                let direction = Vec3(yaw.cos(), yaw.sin(), 0.0);
                if pawn.walker.grounded && map.collision.ray(eye, direction * 435.0).is_none() {
                    orientation = Some((start, eye, yaw, direction));
                    break;
                }
            }
            if orientation.is_some() {
                break;
            }
        }
        let (start, eye, yaw, direction) = orientation.expect("clear lane for the interception");
        for cover in [false, true] {
            let mut game = Match::new(
                &map,
                start,
                Rules {
                    bots: 1,
                    ..Rules::default()
                },
            );
            game.time = 2.0;
            game.fighters[0].yaw = yaw;
            game.fighters[0]
                .inventory
                .pickup(&map.arsenal, Weapon::Shock);
            game.fighters[0].weapon = Weapon::Shock;
            game.fighters[1].walker.position = eye + direction * 375.0;
            game.fighters[1].health = 15;
            let payload = ProjectilePayload {
                kind: super::super::ordnance::Kind::Rocket,
                damage: 85,
                blast: 144.0,
                gravity: 0.0,
                rebounds: 0,
                scale: 1.0,
            };
            for offset in [154.0, 268.0] {
                game.bolts.push(Bolt {
                    owner: 1,
                    position: eye + direction * offset,
                    velocity: Vec3::default(),
                    born: 0.0,
                    expires: 9.0,
                    payload,
                    pilot: None,
                });
            }
            if cover {
                let center = eye + direction * 205.0;
                let side = Vec3(-direction.1, direction.0, 0.0);
                let bounds = Vec3(direction.0.abs(), direction.1.abs(), 0.0) * 4.0
                    + Vec3(side.0.abs(), side.1.abs(), 0.0) * 65.0
                    + Vec3(0.0, 0.0, 72.0);
                map.collision.add_box(center - bounds, center + bounds);
            }
            let mut hits = Vec::new();
            let mut sounds = SoundEvents::default();
            game.fire(
                &map,
                0,
                Command {
                    primary: true,
                    ..Command::default()
                },
                &mut hits,
                &mut sounds,
            );
            assert_eq!(game.bolts.len(), usize::from(cover));
            assert_eq!(
                game.traces
                    .iter()
                    .filter(|trace| trace.explosion.is_some())
                    .count(),
                if cover { 1 } else { 2 }
            );
            for hit in hits {
                game.apply_hit(&map, hit, &mut sounds);
            }
            assert_eq!(game.fighters[0].score, i16::from(!cover));
            assert_eq!(game.fighters[1].score, 0);
        }
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn match_powerups_are_personal_and_compose_with_armor_and_shots() {
        let mut map = arena();
        let start = (0..map.starts.len())
            .find(|&id| {
                let pawn = Fighter::spawn(&map, id, 0.0);
                pawn.walker.grounded && !map.collision.regions.at(pawn.walker.position).water
            })
            .unwrap();
        let mut game = Match::new(
            &map,
            start,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        game.time = 2.0;
        let pawn = &mut game.fighters[0];
        for item in [
            Item::Armor(super::super::ArmorKind::Belt),
            Item::Power(Powerup::Boots, 3),
            Item::Power(Powerup::Damage, 300),
            Item::Power(Powerup::Invisibility, 59),
            Item::Gun(Weapon::Rocket),
        ] {
            assert!(take_item(&map.arsenal, pawn, item, game.time));
        }
        assert_eq!(game.fighters[1].boot_charges(), 0);
        assert_eq!(game.fighters[1].amp_left(game.time), 0.0);
        assert_eq!(game.fighters[0].invis_left(game.time), 30.0);
        let mut sounds = SoundEvents::default();
        game.apply_hit(
            &map,
            Hit {
                bypass_armor: false,
                victim: 0,
                owner: Some(1),
                damage: 45,
                impulse: Vec3::default(),
            },
            &mut sounds,
        );
        assert_eq!(game.fighters[0].health, 100);
        assert_eq!(game.fighters[0].armor(), 105);
        game.fighters[0].weapon = Weapon::Rocket;
        game.fire(
            &map,
            0,
            Command {
                primary: true,
                ..Command::default()
            },
            &mut Vec::new(),
            &mut sounds,
        );
        game.fire(&map, 0, Command::default(), &mut Vec::new(), &mut sounds);
        assert_eq!(
            game.bolts[0].payload.damage,
            arsenal::shot(Weapon::Rocket, false).damage * 3
        );
        game.step(
            &mut map,
            Command {
                jump: true,
                ..Command::default()
            },
            1.0 / 60.0,
        );
        assert_eq!(game.fighters[0].boot_charges(), 2);
        assert!(game.fighters[0].walker.velocity().2 > map.collision.profile.jump_speed);
        game.step(
            &mut map,
            Command {
                jump: true,
                ..Command::default()
            },
            1.0 / 60.0,
        );
        assert_eq!(
            game.fighters[0].boot_charges(),
            2,
            "holding jump in the air must not spend boots"
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn recent_knockback_earns_credit_for_a_fall() {
        let map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 2,
                ..Rules::default()
            },
        );
        game.time = 2.0;
        let mut sounds = SoundEvents::default();
        for victim in [1, 2] {
            game.apply_hit(
                &map,
                Hit {
                    bypass_armor: false,
                    victim,
                    owner: Some(0),
                    damage: 20,
                    impulse: Vec3(200.0, 0.0, 0.0),
                },
                &mut sounds,
            );
        }
        for (victim, elapsed) in [(1, 0.5), (2, 5.0)] {
            game.time = 2.0 + elapsed;
            game.apply_hit(
                &map,
                Hit {
                    bypass_armor: false,
                    victim,
                    owner: None,
                    damage: u16::MAX,
                    impulse: Vec3::default(),
                },
                &mut sounds,
            );
        }
        assert_eq!(game.fighters[0].score, 1);
        assert_eq!(game.fighters[1].score, 0);
        assert_eq!(game.fighters[2].score, -1);
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn a_weapon_pickup_goes_to_one_fighter_and_controls_their_next_shot() {
        let mut map = arena();
        let mut match_ = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let pickup = map
            .props
            .iter()
            .position(|prop| prop.visible && matches!(prop.item, Some(Item::Gun(Weapon::Rocket))))
            .expect("Deck16 rocket launcher pickup");
        let point = map.props[pickup].position;
        for fighter in &mut match_.fighters {
            fighter.walker = Walker::spawn(&map.collision, point);
        }
        let mut sounds = SoundEvents::default();
        let mut hits = Vec::new();
        match_.collect(&mut map, &mut sounds);
        assert!(!map.props[pickup].visible);
        assert_eq!(match_.fighters[0].weapon, Weapon::Rocket);
        assert!(!match_.fighters[1].inventory.owns(Weapon::Rocket));
        let ammo = match_.fighters[0].ammo().unwrap();
        match_.fire(
            &map,
            0,
            Command {
                primary: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert!(match_.bolts.is_empty(), "equipping delays firing");
        match_.time = 1.0;
        match_.fire(
            &map,
            0,
            Command {
                primary: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        match_.fire(&map, 0, Command::default(), &mut hits, &mut sounds);
        assert_eq!(match_.fighters[0].ammo(), Some(ammo - 1));
        assert!(match_.bolts[0].payload.kind == super::super::ordnance::Kind::Rocket);
        assert_eq!(match_.bolts[0].owner, 0);
        match_.fire(
            &map,
            0,
            Command {
                slot: Some(2),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert_eq!(match_.fighters[0].weapon, Weapon::Enforcer);
        match_.fire(
            &map,
            0,
            Command {
                slot: Some(9),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert_eq!(match_.fighters[0].ammo(), Some(ammo - 1));
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn every_weapon_feeds_the_shared_match_simulation() {
        let map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let mut sounds = SoundEvents::default();
        let mut hits = Vec::new();
        for weapon in Weapon::ALL {
            for alternate in [false, true] {
                game.time += 10.0;
                let fighter = &mut game.fighters[1];
                fighter.inventory.pickup(&map.arsenal, weapon);
                fighter.inventory.grant_ammo(&map.arsenal, weapon, u16::MAX);
                fighter.weapon = weapon;
                fighter.alt_held = false;
                let ammo_before = fighter.inventory.ammo(weapon);
                let bolts_before = game.bolts.len();
                let shot = arsenal::shot(weapon, alternate);
                game.fire(
                    &map,
                    1,
                    Command {
                        primary: !alternate,
                        alternate,
                        ..Command::default()
                    },
                    &mut hits,
                    &mut sounds,
                );
                if arsenal::charge_profile(weapon, alternate).is_some() {
                    assert_eq!(game.bolts.len(), bolts_before);
                    game.fire(
                        &map,
                        1,
                        Command {
                            release_charge: true,
                            ..Command::default()
                        },
                        &mut hits,
                        &mut sounds,
                    );
                }
                assert_eq!(
                    game.fighters[1].inventory.ammo(weapon),
                    ammo_before - shot.ammo,
                    "{weapon:?}"
                );
                match shot.path {
                    Path::Missile { kind, .. } => {
                        assert_eq!(game.bolts.len(), bolts_before + 1, "{weapon:?}");
                        let bolt = game.bolts.last().unwrap();
                        assert_eq!(bolt.owner, 1);
                        assert!(bolt.payload.kind == kind);
                    }
                    Path::Trace { .. } => assert_eq!(game.fighters[1].fired_at, game.time),
                    Path::Zoom => assert!(game.fighters[1].zoomed),
                }
            }
        }
        assert!(game.fighters[1].inventory.ammo(Weapon::Enforcer) > 0);
        assert_eq!(
            game.fighters[1].inventory.ammo(Weapon::Minigun),
            game.fighters[1].inventory.ammo(Weapon::Enforcer)
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn hard_bot_returns_fire_before_easy_bot() {
        let map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let bot = &game.fighters[1];
        let eye = bot.camera(&map).position;
        let (delta, opponent) = [
            Vec3(135.0, 0.0, 0.0),
            Vec3(0.0, 135.0, 0.0),
            Vec3(-135.0, 0.0, 0.0),
            Vec3(0.0, -135.0, 0.0),
        ]
        .into_iter()
        .find_map(|offset| {
            let walker = Walker::spawn(&map.collision, bot.walker.position + offset);
            let delta = walker.position + Vec3(0.0, 0.0, 12.0) - eye;
            map.collision
                .ray(eye, delta)
                .is_none()
                .then_some((delta, walker))
        })
        .expect("a clear firing lane");
        game.fighters[0].walker = opponent;
        game.fighters[0].protected_until = 0.0;
        let (yaw, pitch) = aim_error(delta, 0.0, 0.0);
        game.fighters[1].yaw = yaw;
        game.fighters[1].pitch = pitch;
        for (difficulty, fires) in [(Difficulty::Easy, false), (Difficulty::Hard, true)] {
            game.rules.difficulty = difficulty;
            game.fighters[1].bot = Bot::default();
            game.time = 0.0;
            let opening = game.bot_command(&map, 1, 1.0 / 60.0);
            assert!(!opening.primary && !opening.alternate);
            game.time = 0.32;
            assert_eq!(game.bot_command(&map, 1, 1.0 / 60.0).primary, fires);
        }
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn standard_bot_has_an_opening_and_misses_in_a_close_duel() {
        let (mut map, _) = flight_fixture();
        map.starts[1].position = Vec3(260.0, 0.0, 39.2);
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                difficulty: Difficulty::Standard,
                ..Rules::default()
            },
        );
        let mut shots = Vec::new();
        let mut hits = 0;
        for _ in 0..(60 * 8) {
            let before = game.fighters[0].health;
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            let bot = &game.fighters[1];
            if bot.fired_count as usize > shots.len() {
                shots.push(bot.fired_at);
                hits += usize::from(game.fighters[0].health < before);
                game.fighters[0].health = 100;
            }
        }
        assert!(shots.len() >= 6, "shots: {shots:?}");
        assert!(shots[0] >= 0.4, "first shot: {}", shots[0]);
        assert!(
            shots.windows(2).all(|s| s[1] - s[0] >= 0.38),
            "shots: {shots:?}"
        );
        assert!(
            hits > 0 && hits < shots.len(),
            "{hits}/{} hits",
            shots.len()
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn bot_keeps_its_target_through_a_brief_occlusion() {
        let (mut map, _) = flight_fixture();
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 2,
                ..Rules::default()
            },
        );
        for (id, x, y) in [(0, 400.0, 0.0), (1, 0.0, 0.0), (2, 365.0, 220.0)] {
            let fighter = &mut game.fighters[id];
            fighter.walker = Walker::spawn(&map.collision, Vec3(x, y, 39.2));
            fighter.yaw = 0.0;
            fighter.protected_until = 0.0;
        }
        game.time = 2.0;
        game.bot_command(&map, 1, 1.0 / 120.0);
        let reaction = game.fighters[1].bot.react_at;
        assert_eq!(game.fighters[1].bot.target, Some(0));
        game.fighters[2].walker.position = Vec3(320.0, 120.0, 39.2);
        game.time = 3.0;
        game.bot_command(&map, 1, 1.0 / 120.0);
        assert_eq!(game.fighters[1].bot.target, Some(0));
        map.collision
            .add_box(Vec3(180.0, -42.0, 0.0), Vec3(220.0, 42.0, 100.0));
        game.time = 3.1;
        let behind_cover = game.bot_command(&map, 1, 1.0 / 120.0);
        assert_eq!(game.fighters[1].bot.target, Some(0));
        assert!(!behind_cover.primary && !behind_cover.alternate);
        game.fighters[0].walker.position = Vec3(400.0, -120.0, 39.2);
        game.time = 3.2;
        game.bot_command(&map, 1, 1.0 / 120.0);
        assert_eq!(game.fighters[1].bot.target, Some(0));
        assert_eq!(game.fighters[1].bot.react_at, reaction);
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn bot_respawns_on_a_fresh_safe_start_and_keeps_its_score() {
        let (mut map, _) = flight_fixture();
        map.collision
            .add_box(Vec3(-120.0, 200.0, 0.0), Vec3(120.0, 215.0, 160.0));
        map.starts = [
            Vec3(0.0, 0.0, 39.2),
            Vec3(520.0, 0.0, 39.2),
            Vec3(0.0, 535.0, 39.2),
            Vec3(830.0, 0.0, 39.2),
            Vec3(0.0, -650.0, 500.0),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, position)| crate::map::Start {
            position,
            yaw: 0.0,
            enabled: index != 3,
        })
        .collect();
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        assert_eq!(game.fighters[1].spawn, 2);
        let bot = &mut game.fighters[1];
        bot.health = 0;
        bot.score = 3;
        bot.deaths = 1;
        bot.fell_at = Some(0.0);
        game.time = 1.0;
        game.step(&mut map, Command::default(), 1.0 / 120.0);
        assert_eq!(game.fighters[1].health, 0);
        game.time = 2.0;
        game.step(&mut map, Command::default(), 1.0 / 120.0);
        let bot = &game.fighters[1];
        assert_eq!(bot.spawn, 1);
        assert_eq!((bot.score, bot.deaths, bot.health), (3, 1, 100));
        assert!(bot.walker.grounded && bot.protected_until > game.time);
        let (before, yaw_before) = bot.body_transform(0.0);
        let (after, yaw_after) = bot.body_transform(1.0);
        assert!(
            distance_squared(before, after) < 0.001
                && angle_delta(yaw_after, yaw_before).abs() < 0.001
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn match_renders_shared_hud_and_reuploads_deck16_after_logo() {
        use crate::render::{Frame, commands::LIGHTMAPS};

        let logo_path = std::path::Path::new("local/goty/Maps/UT-Logo-Map.unr");
        let logo_bytes = std::fs::read(logo_path).unwrap();
        let logo = Map::load(&Package::read(&logo_bytes).unwrap(), Some(logo_path)).unwrap();
        let map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 0,
                ..Rules::default()
            },
        );
        game.time = 1.0;
        let mut sounds = SoundEvents::default();
        game.fire(
            &map,
            0,
            Command {
                primary: true,
                ..Command::default()
            },
            &mut Vec::new(),
            &mut sounds,
        );
        assert_eq!(game.fighters[0].ammo(), Some(29));
        assert_eq!(game.fighters[0].ready_at, game.time + 0.32);
        assert_eq!(game.ballistics.cases.len(), 1);
        assert!(game.traces.is_empty());
        let camera = game.fighters[0].camera(&map);
        let mut frame = Frame::hardware(&map);
        frame.draw_deathmatch(&map, &camera, &game, 90.0, 1.0);
        let first = frame.commands.take().unwrap();
        let menu = Frame::hardware(&logo).commands.unwrap();
        let mut frame = Frame::hardware(&map);
        frame.draw_deathmatch(&map, &camera, &game, 90.0, 1.0);
        let resumed = frame.commands.unwrap();
        assert_ne!(first.scene_id, menu.scene_id);
        assert_ne!(menu.scene_id, resumed.scene_id);
        assert_ne!(first.scene_id, resumed.scene_id);
        let atlas = |commands: &crate::render::commands::DrawList| {
            let (_, image, _) = commands
                .uploads
                .iter()
                .find(|(key, _, _)| *key == LIGHTMAPS)
                .unwrap();
            (image.width, image.height, image.pixels.len())
        };
        assert_eq!(atlas(&first), atlas(&resumed));
        assert!(resumed.passes[1].batches().next().is_some());
        assert!(resumed.passes[3].batches().next().is_some());
        for texture in [&map.hud.elements, &map.hud.weapons] {
            let key = std::ptr::from_ref(texture.as_ref().unwrap().image(0.0)) as usize;
            assert!(
                resumed.passes[4]
                    .batches()
                    .any(|(material, _)| material.texture == key)
            );
        }
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn wounded_bot_leaves_combat_to_collect_health() {
        let mut map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        game.fighters[0].protected_until = f32::INFINITY;
        let (pickup, walker) = map
            .props
            .iter()
            .enumerate()
            .filter(|(_, prop)| prop.visible && matches!(prop.item, Some(Item::Health(..))))
            .find_map(|(index, prop)| {
                (0..8).find_map(|step| {
                    let yaw = step as f32 * std::f32::consts::TAU / 8.0;
                    let offset = Vec3(yaw.cos() * 140.0, yaw.sin() * 140.0, 8.0);
                    let walker = Walker::spawn(&map.collision, prop.position + offset);
                    (walker.grounded && matches!(
                        game.navigation.next_step(&map.collision, walker.position, prop.position),
                        Some(Step::Walk(point)) if near_waypoint(point, prop.position)
                    )).then_some((index, walker))
                })
            })
            .expect("a reachable health pickup on Deck16");
        for (id, prop) in map.props.iter_mut().enumerate() {
            if matches!(prop.item, Some(Item::Health(..))) && id != pickup {
                prop.visible = false;
            }
        }
        game.fighters[1].walker = walker;
        game.fighters[1].health = 20;
        let retreat = try_retreat(
            &game.navigation,
            &map,
            &mut game.fighters[1],
            &Difficulty::Standard.combat(),
            true,
            game.time,
            1.0 / 60.0,
        )
        .unwrap();
        assert!(!retreat.primary && !retreat.alternate);
        assert_eq!(
            game.fighters[1].bot.retreat.as_ref().unwrap().pickup,
            pickup
        );
        for _ in 0..120 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            if !map.props[pickup].visible {
                break;
            }
        }
        assert!(!map.props[pickup].visible);
        assert!(game.fighters[1].health > 20);
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn bot_pose_uses_locomotion_fire_and_damage_clips() {
        let map = arena();
        let start = (0..map.starts.len())
            .find(|&id| {
                let pawn = Fighter::spawn(&map, id, 0.0);
                pawn.walker.grounded && !map.collision.regions.at(pawn.walker.position).water
            })
            .unwrap();
        let mut pawn = Fighter::spawn(&map, start, 0.0);
        pawn.yaw = 0.0;
        let pose = |pawn: &Fighter, clip: &super::super::Motion, time| {
            assert!(std::ptr::eq(
                pawn.pose(&map, 1, time).unwrap().current,
                clip.frames[0].as_slice()
            ));
        };
        pose(&pawn, &map.player_idle, 0.0);
        pawn.fired_at = 0.0;
        pose(&pawn, &map.player_clips.fire, 0.0);
        pawn.walker.add_velocity(Vec3(250.0, 0.0, 0.0));
        pose(&pawn, &map.player_clips.air, 0.0);
        pawn.walker.grounded = true;
        pawn.update_gait(0.0);
        pose(&pawn, &map.player_clips.fire_run, 0.0);
        pose(&pawn, &map.player_clips.run, 1.0);
        pawn.hurt_at = 1.0;
        pose(&pawn, &map.player_clips.hurt, 1.0);
        pawn.fell_at = Some(1.0);
        pose(&pawn, &map.player_deaths[1], 1.0);
        pawn.yaw = std::f32::consts::TAU - 0.04;
        pawn.capture_body();
        let origin = pawn.walker.position;
        pawn.yaw = 0.04;
        pawn.walker.position = origin + Vec3(12.0, 0.0, 0.0);
        let (midpoint, angle) = pawn.body_transform(0.5);
        assert_eq!(midpoint.0, origin.0 + 6.0);
        assert!(angle_delta(angle, 0.0).abs() < 0.001);
    }

    #[test]
    #[ignore = "requires the local GOTY map"]
    fn bot_navigates_both_deck16_lifts_in_both_directions() {
        let mut map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        game.fighters[0].protected_until = f32::INFINITY;
        assert_eq!(map.movers.len(), 2);
        for index in 0..map.movers.len() {
            let lift = &map.movers[index];
            let exits: Vec<_> = map
                .lift_exits
                .iter()
                .filter(|exit| Some(exit.tag) == lift.tag)
                .map(|exit| exit.position)
                .collect();
            assert_eq!(exits.len(), 2);
            let home = exits
                .iter()
                .copied()
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .unwrap();
            let away = exits
                .iter()
                .copied()
                .max_by(|a, b| a.2.total_cmp(&b.2))
                .unwrap();
            let bot = &mut game.fighters[1];
            bot.walker = Walker::spawn(&map.collision, home);
            for (from, to, stop) in [(home, away, LiftStop::Away), (away, home, LiftStop::Home)] {
                let position = game.fighters[1].walker.position;
                assert!(distance_squared(position, from) < 50.0_f32.powi(2));
                // Start with the platform at the far end so the bot must call it first.
                for _ in 0..300 {
                    map.movers[index].tick_many(
                        &mut map.collision,
                        &mut [],
                        1.0 / 60.0,
                        Some(stop),
                    );
                }
                assert!(map.movers[index].lift_at(stop));
                let route = game.navigation.next_step(&map.collision, position, to);
                assert!(
                    matches!(route, Some(Step::Lift(ride)) if ride.mover == index),
                    "{route:?}"
                );
                game.fighters[1].bot.goal = route;
                for _ in 0..720 {
                    game.step(&mut map, Command::default(), 1.0 / 60.0);
                    if game.fighters[1].bot.lift.is_none() && game.fighters[1].bot.goal.is_none() {
                        break;
                    }
                }
                let bot = &game.fighters[1];
                assert!(
                    bot.walker.grounded,
                    "{stop:?} {:?} vs {to:?}",
                    bot.walker.position
                );
                assert!(
                    distance_squared(bot.walker.position, to) < 50.0_f32.powi(2),
                    "bot failed to ride to {stop:?}: {:?}, destination {to:?}, lift at stop: {}",
                    bot.walker.position,
                    map.movers[index].lift_at(stop)
                );
            }
        }
    }

    #[test]
    #[ignore = "requires the local GOTY map"]
    fn route_through_a_portal_is_followed_by_the_actual_bot() {
        let data = std::fs::read("local/goty/Maps/CTF-Face.unr").unwrap();
        let package = Package::read(&data).unwrap();
        let mut map = Map::load(&package, None).unwrap();
        let mut game = Match::new(
            &map,
            0,
            Rules {
                bots: 1,
                ..Rules::default()
            },
        );
        let (entry, exit) = crate::teleport::links(&map.teleporters)
            .find(|(from, to)| {
                matches!(
                    game.navigation.next_step(&map.collision, *from, *to),
                    Some(Step::Teleport(_))
                )
            })
            .expect("a portal route through the tower");
        game.fighters[0].protected_until = f32::INFINITY;
        let bot = &mut game.fighters[1];
        bot.walker = Walker::spawn(&map.collision, entry);
        bot.bot.goal = game.navigation.next_step(&map.collision, entry, exit);
        bot.bot.rethink_at = 2.0;
        game.step(&mut map, Command::default(), 1.0 / 60.0);
        let bot = &game.fighters[1];
        assert!(distance_squared(bot.walker.position, exit) < 35.0_f32.powi(2));
        assert!(bot.portal.is_some());
        let (before, yaw_before) = bot.body_transform(0.0);
        let (after, yaw_after) = bot.body_transform(1.0);
        assert!(
            distance_squared(before, after) < 0.001
                && angle_delta(yaw_after, yaw_before).abs() < 0.001
        );
        assert!(bot.bot.goal.is_none());
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn offline_bots_complete_a_match_and_can_play_again() {
        let mut map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 4,
                frag_limit: 1,
                seconds: 120,
                ..Rules::default()
            },
        );
        game.fighters[0].protected_until = f32::INFINITY;
        let starts: Vec<_> = game.fighters.iter().map(|fighter| fighter.spawn).collect();
        let mut distinct = starts.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), starts.len());
        for _ in 0..(120 * 60 + 1) {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            if game.finished {
                break;
            }
        }
        assert!(game.finished);
        assert!(
            game.remaining() > 0,
            "the frag limit should end the match first"
        );
        assert!(
            game.fighters.iter().skip(1).any(|bot| bot.score > 0),
            "bots must fight and score without player input"
        );
        assert!(
            game.fighters.iter().skip(1).any(|bot| bot.deaths > 0),
            "bots must be able to kill one another"
        );
        #[cfg(windows)]
        {
            let camera = game.fighters[0].camera(&map);
            let mut frame = crate::render::Frame::new();
            frame.draw_deathmatch(&map, &camera, &game, 90.0, 1.0);
            let scene = frame.pixels.clone();
            crate::hud::Hud::new().draw_deathmatch(
                &mut frame,
                &game,
                false,
                false,
                map.hud.ready(),
            );
            let center = frame.width * (frame.height / 2) + frame.width / 2;
            assert_ne!(
                frame.pixels[center], scene[center],
                "the results panel covers the scene"
            );
        }
        let old_time = game.time;
        game.step(&mut map, Command::default(), 1.0);
        assert_eq!(game.time, old_time);
        game.step(
            &mut map,
            Command {
                restart: true,
                ..Command::default()
            },
            1.0 / 60.0,
        );
        assert!(!game.finished);
        assert_eq!(game.remaining(), 120);
        assert!(
            game.fighters
                .iter()
                .all(|fighter| fighter.score == 0 && fighter.deaths == 0)
        );
        assert!(game.bolts.is_empty() && game.traces.is_empty());
        assert!(
            map.props
                .iter()
                .zip(&game.initial_props)
                .all(|(prop, &visible)| prop.visible == visible)
        );
    }

    #[test]
    #[ignore = "requires local GOTY data"]
    fn match_credits_a_dead_shooter_and_handles_self_damage_and_time_limit() {
        let mut map = arena();
        let mut game = Match::new(
            &map,
            6,
            Rules {
                bots: 2,
                frag_limit: 10,
                seconds: 5,
                ..Rules::default()
            },
        );
        game.time = 2.0;
        let mut sounds = SoundEvents::default();
        let hit = |victim, owner| Hit {
            bypass_armor: false,
            victim,
            owner: Some(owner),
            damage: 1000,
            impulse: Vec3::default(),
        };
        game.apply_hit(&map, hit(0, 0), &mut sounds);
        assert_eq!(game.fighters[0].score, -1);
        game.fighters[1].health = 30;
        game.bolts.push(Bolt {
            owner: 0,
            position: game.fighters[1].walker.position,
            velocity: Vec3(1250.0, 0.0, 0.0),
            born: game.time,
            expires: game.time + 2.0,
            pilot: None,
            payload: ProjectilePayload {
                kind: super::super::ordnance::Kind::Rocket,
                damage: 85,
                blast: 190.0,
                gravity: 0.0,
                rebounds: 0,
                scale: 1.0,
            },
        });
        game.step(&mut map, Command::default(), 1.0 / 60.0);
        assert!(game.bolts.is_empty());
        assert_eq!(game.fighters[0].score, 0);
        assert_eq!(game.fighters[1].deaths, 1);
        for _ in 0..85 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert!(game.fighters[0].health > 0 && game.fighters[1].health > 0);
        assert_eq!(game.fighters[0].weapon, Weapon::Enforcer);
        assert_eq!(game.fighters[1].deaths, 1);
        for _ in 0..130 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert!(game.finished);
        assert_eq!(game.remaining(), 0);
    }
}
