//! Offline arena simulation. The window supplies input and displays snapshots.

mod ai;
mod arsenal;
mod navigation;

use super::{Item, Powerup, Weapon, WeaponAssets, shock::cylinder_hit};
use crate::{
    collision::{PLAYER_HEIGHT, PLAYER_RADIUS, Walker},
    map::{Map, Vec3},
    mover::{LiftStop, unit},
    render::Camera,
    sound::{BLAST_SOUND_RADIUS, CONTACT_SOUND_RADIUS, SoundEvents},
};
pub use ai::Difficulty;
use arsenal::{Inventory, Path};
use navigation::{LiftRide, Navigation, Step, WAYPOINT_RADIUS};

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
    pub jump: bool,
    pub primary: bool,
    pub alternate: bool,
    pub restart: bool,
    pub look: [f32; 2],
    pub slot: Option<u8>,
    pub cycle: i8,
    pub preferred: Option<Weapon>,
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
    pub last_alt: bool,
    pub zoomed: bool,
    alt_held: bool,
    pub score: i16,
    pub deaths: u16,
    pub fired_at: f32,
    pub fell_at: Option<f32>,
    hurt_at: f32,
    landed_at: f32,
    stride_time: f32,
    protected_until: f32,
    ready_at: f32,
    hazard_at: f32,
    last_hostile_impulse: Option<(usize, f32)>,
    portal: Option<usize>,
    spawn: usize,
    bot: Bot,
}

impl Fighter {
    fn spawn(map: &Map, spawn: usize, time: f32) -> Self {
        let start = map.starts[spawn];
        Self {
            walker: Walker::spawn(&map.collision, start.position),
            yaw: start.yaw,
            pitch: 0.0,
            health: 100,
            armor: [0; 3],
            power: ActivePowerups::default(),
            inventory: Inventory::new(map),
            weapon: Weapon::Enforcer,
            selected_at: f32::NEG_INFINITY,
            last_alt: false,
            zoomed: false,
            alt_held: false,
            score: 0,
            deaths: 0,
            fired_at: f32::NEG_INFINITY,
            fell_at: None,
            hurt_at: f32::NEG_INFINITY,
            landed_at: f32::NEG_INFINITY,
            stride_time: 0.0,
            protected_until: time + SPAWN_PROTECTION,
            ready_at: time,
            hazard_at: time,
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

    pub(crate) fn pose<'a>(&self, map: &'a Map, id: usize, time: f32) -> Option<super::Pose<'a>> {
        if let Some(death) = self.fell_at {
            let clip = &map.player_deaths[id % map.player_deaths.len()];
            return clip.sample((time - death).min((clip.duration() - 0.001).max(0.0)));
        }
        let clips = &map.player_clips;
        let velocity = self.walker.velocity();
        let speed = velocity.0.hypot(velocity.1);
        let firing = time - self.fired_at;
        let (clip, age) = if map.collision.regions.at(self.walker.position).water {
            (&clips.swim, time)
        } else if time - self.hurt_at < 0.12 {
            (&clips.hurt, 0.0)
        } else if !self.walker.grounded {
            (&clips.air, 0.0)
        } else if time - self.landed_at < 0.13 {
            (&clips.land, 0.0)
        } else if speed < 35.0 {
            if firing < clips.fire.duration() {
                (&clips.fire, firing)
            } else {
                (&map.player_idle, time)
            }
        } else {
            let (sin, cos) = self.yaw.sin_cos();
            let forward = velocity.dot(Vec3(cos, sin, 0.0));
            let side = velocity.dot(Vec3(-sin, cos, 0.0));
            let gait = if side.abs() > forward.abs() * 1.15 {
                &clips.strafe[usize::from(side > 0.0)]
            } else if forward < -30.0 {
                &clips.backpedal
            } else if firing < 0.27 {
                &clips.fire_run
            } else if speed < 165.0 {
                &clips.walk
            } else {
                &clips.run
            };
            (gait, self.stride_time)
        };
        if clip.duration() > 0.0 {
            clip.sample(age.rem_euclid(clip.duration()))
        } else {
            map.player_idle.sample(0.0)
        }
    }
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
    goal: Option<Step>,
    rethink_at: f32,
    last_seen: Option<(Vec3, f32)>,
    lift: Option<ActiveLift>,
    plan_origin: Option<Vec3>,
    retreat: Option<Retreat>,
    retreat_retry_at: f32,
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
}

#[derive(Clone, Copy)]
struct ProjectilePayload {
    kind: super::ordnance::Kind,
    damage: u16,
    blast: f32,
    gravity: f32,
    rebounds: u8,
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
    expires: f32,
}

struct Hit {
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
    pub traces: Vec<Trace>,
    bolts: Vec<Bolt>,
    initial_props: Vec<bool>,
    pickups: Vec<Option<f32>>,
    kill_z: f32,
    navigation: Navigation,
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
            message: String::new(),
            message_until: 0.0,
        };
        game.spawn_all(map);
        game
    }

    fn spawn_all(&mut self, map: &Map) {
        self.fighters.clear();
        self.fighters
            .push(Fighter::spawn(map, self.preferred_start, 0.0));
        for id in 1..=self.rules.bots {
            let spawn = self.choose_spawn(map, id, None);
            self.fighters.push(Fighter::spawn(map, spawn, 0.0));
        }
    }

    fn restart(&mut self, map: &mut Map) {
        self.time = 0.0;
        self.finished = false;
        self.traces.clear();
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
            .max_by(|(a, start_a), (b, start_b)| {
                let value = |index, start: &crate::map::Start| {
                    let distance = self
                        .fighters
                        .iter()
                        .enumerate()
                        .filter(|(id, other)| *id != fighter && other.health > 0)
                        .map(|(_, other)| distance_squared(other.walker.position, start.position))
                        .fold(f32::INFINITY, f32::min);
                    (distance.min(700.0 * 700.0), previous != Some(index))
                };
                let (da, fresh_a) = value(*a, start_a);
                let (db, fresh_b) = value(*b, start_b);
                da.total_cmp(&db).then_with(|| fresh_a.cmp(&fresh_b))
            })
            .map_or(self.preferred_start, |(spawn, _)| spawn)
    }

    pub fn remaining(&self) -> u16 {
        (f32::from(self.rules.seconds) - self.time).ceil().max(0.0) as u16
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

    pub(crate) fn bolts(
        &self,
    ) -> impl Iterator<Item = (super::ordnance::Kind, Vec3, Vec3, f32)> + '_ {
        self.bolts.iter().map(|bolt| {
            (
                bolt.payload.kind,
                bolt.position,
                unit(bolt.velocity),
                self.time - bolt.born,
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
        self.traces.retain(|trace| trace.expires > self.time);
        let mut inputs = Vec::with_capacity(self.fighters.len());
        inputs.push(command);
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
                if self.time - fell >= RESPAWN_DELAY {
                    let old = self.fighters[id].spawn;
                    let spawn = self.choose_spawn(map, id, Some(old));
                    let mut respawned = Fighter::spawn(map, spawn, self.time);
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
            let speed = map.collision.profile.ground_speed / walk.dot(walk).sqrt().max(1.0);
            let swim = aim.look_direction() * forward
                + aim.right() * strafe
                + Vec3(0.0, 0.0, f32::from(input.jump));
            let base_jump = map.collision.profile.jump_speed;
            let has_boots = fighter.power.boot_charges > 0;
            let jump_speed = if has_boots {
                base_jump * 3.0
            } else {
                base_jump
            };
            fighter.walker.air_control_override =
                (has_boots || fighter.power.boosted_airborne).then_some(1.0);
            let motion = fighter.walker.advance_with_swim(
                &map.collision,
                walk * speed,
                swim,
                input.jump.then_some(jump_speed),
                dt,
            );
            let velocity = fighter.walker.velocity();
            fighter.stride_time += dt * (velocity.0.hypot(velocity.1) / 285.0).clamp(0.0, 2.0);
            let swimming = map.collision.regions.at(fighter.walker.position).water;
            if motion.jump {
                if has_boots {
                    fighter.power.boot_charges -= 1;
                    fighter.power.boosted_airborne = true;
                }
                let sound = if has_boots {
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
                fighter.power.boosted_airborne = false;
                fighter.bot.goal = None;
                fighter.bot.lift = None;
                fighter.bot.rethink_at = self.time;
            }
            let zone = map.collision.regions.at(fighter.walker.position);
            if fighter.walker.position.2 < self.kill_z {
                hits.push(Hit {
                    victim: id,
                    owner: None,
                    damage: u16::MAX,
                    impulse: Vec3::default(),
                });
            } else if zone.pain && zone.damage_per_second > 0 && self.time >= fighter.hazard_at {
                fighter.hazard_at = self.time + 0.5;
                hits.push(Hit {
                    victim: id,
                    owner: None,
                    damage: (zone.damage_per_second / 2).clamp(1, u16::MAX as i32) as u16,
                    impulse: Vec3::default(),
                });
            } else if !zone.pain {
                fighter.hazard_at = self.time;
            }
        }
        self.separate_fighters(map);
        self.collect(map, &mut sounds);
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
        self.finished = self.remaining() == 0
            || self
                .fighters
                .iter()
                .any(|fighter| fighter.score >= self.rules.frag_limit);
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
        for (index, prop) in map.props.iter_mut().enumerate() {
            if self.pickups[index].is_some_and(|due| self.time >= due) {
                self.pickups[index] = None;
                prop.visible = true;
            }
            let Some(item) = prop.item.filter(|_| prop.visible) else {
                continue;
            };
            for (id, fighter) in self.fighters.iter_mut().enumerate() {
                let delta = fighter.walker.position - prop.position;
                if fighter.health == 0
                    || delta.2.abs() > prop.pickup_height + PLAYER_HEIGHT
                    || delta.0.hypot(delta.1) > prop.pickup_radius + PLAYER_RADIUS
                    || map.collision.ray(prop.position, delta).is_some()
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
        let requested = input.preferred.or_else(|| {
            fighter
                .inventory
                .select(fighter.weapon, input.slot, input.cycle)
        });
        if let Some(weapon) = requested.filter(|&weapon| weapon != fighter.weapon) {
            fighter.equip(map.arsenal.get(&weapon), weapon, self.time, sounds, id == 0);
            return;
        }
        if self.time < fighter.ready_at || !(input.primary || input.alternate) {
            return;
        }

        let weapon = fighter.weapon;
        let mut alt = input.alternate && !input.primary;
        let mut shot = arsenal::shot(weapon, alt);
        if matches!(shot.path, Path::Zoom) {
            if pressed_alt {
                fighter.zoomed = !fighter.zoomed;
            }
            return;
        }
        if fighter.inventory.ammo(weapon) < shot.ammo {
            alt = false;
            shot = arsenal::shot(weapon, false);
        }
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
        fighter.last_alt = alt;
        fighter.ready_at = self.time + shot.interval;
        let aim = fighter.camera(map);
        let start = aim.position;
        let direction = aim.look_direction();
        let boosted = fighter.amp_left(self.time) > 0.0;
        let damage = shot.damage.saturating_mul(if boosted { 3 } else { 1 });
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
                self.bolts.push(Bolt {
                    owner: id,
                    position: start,
                    velocity: direction * speed + Vec3(0.0, 0.0, gravity * 115.0),
                    born: self.time,
                    expires: self.time
                        + if rebounds > 0 && gravity > 0.0 {
                            2.35
                        } else {
                            4.0
                        },
                    payload: ProjectilePayload {
                        kind,
                        damage,
                        blast,
                        gravity,
                        rebounds,
                    },
                });
            }
            Path::Trace {
                range,
                rays,
                spread,
            } => {
                let right = aim.right();
                let up = direction.cross(right);
                let mut seed =
                    self.time.to_bits().rotate_left(5) ^ (id as u32).wrapping_mul(0x9e37_79b9);
                let mut jitter = || {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    ((seed >> 8) as f32 / 8_388_608.0) - 1.0
                };
                for ray in 0..rays {
                    let spread = if ray == 0 && rays > 1 { 0.0 } else { spread };
                    let delta =
                        unit(direction + (right * jitter() + up * jitter()) * spread) * range;
                    let end = map.collision.ray(start, delta).unwrap_or(start + delta);
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
                        hits.push(Hit {
                            victim,
                            owner: Some(id),
                            damage,
                            impulse: unit(delta) * (f32::from(shot.damage) * 3.4),
                        });
                        at
                    } else {
                        1.0
                    };
                    if ray == 0 && weapon != Weapon::Hammer {
                        self.traces.push(Trace {
                            start,
                            end: start + clipped * fraction,
                            explosion: None,
                            born: self.time,
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
                    art.collision_radius.max(4.0),
                    art.collision_height.max(4.0),
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
            let radius = art.collision_radius.max(0.0);
            let height = art.collision_height.max(0.0);
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
        let (damage, shield) = super::absorb_damage(&mut victim.armor, hit.damage);
        if shield {
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
        victim.fell_at = Some(self.time);
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
        let fighter = &self.fighters[id];
        let eye = fighter.camera(map);
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
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let skill = self.rules.difficulty.combat();
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
            }
            let range = distance.sqrt();
            let weapon = fighter.inventory.preferred(range);
            let (alt, shot) = arsenal::bot_shot(weapon, fighter.inventory.ammo(weapon), range);
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
            let strafe = (self.time * 2.1 + id as f32).sin().signum();
            let error = range * skill.error;
            delta = delta
                + eye.right() * ((self.time * 4.2 + id as f32).sin() * error)
                + Vec3(0.0, 0.0, (self.time * 3.4 + id as f32).cos() * error * 0.43);
            let (turn, pitch) = aim_error(delta, fighter.yaw, fighter.pitch);
            let yaw = turn.clamp(-skill.turn * dt, skill.turn * dt);
            let tilt = pitch.clamp(-skill.turn * dt * 0.75, skill.turn * dt * 0.75);
            let firing = self.time >= bot.react_at
                && (turn - yaw).abs() < 0.045
                && (pitch - tilt).abs() < 0.045
                && range < attack_range
                && clear;
            return Command {
                movement: [
                    if weapon == Weapon::Hammer || range > (attack_range * 0.75).min(480.0) {
                        0.8
                    } else {
                        -0.22
                    },
                    strafe * skill.strafe,
                ],
                look: [yaw, tilt],
                primary: firing && !alt,
                alternate: firing && alt,
                preferred: (weapon != fighter.weapon).then_some(weapon),
                ..Command::default()
            };
        }
        bot.target = None;
        let position = fighter.walker.position;
        if self.time >= bot.rethink_at
            || bot.goal.is_some_and(|goal| {
                matches!(goal, Step::Walk(point)
                    if near_waypoint(point, position))
            })
        {
            let stalled = bot.plan_origin.is_some_and(|origin| {
                matches!(bot.goal, Some(Step::Walk(point)) if !near_waypoint(point, position))
                    && distance_squared(origin, position) < 24.0_f32.powi(2)
            });
            bot.plan_origin = Some(position);
            let visible = |point: Vec3| {
                let delta = point - position;
                delta.2.abs() < 100.0
                    && delta.dot(delta) > 70.0_f32.powi(2)
                    && map.collision.ray(position, delta).is_none()
            };
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
            bot.goal = supply
                .or_else(|| {
                    if stalled {
                        return None;
                    }
                    chase.and_then(|point| {
                        self.navigation.next_step(&map.collision, position, point)
                    })
                })
                .or_else(|| {
                    (0..12)
                        .filter_map(|offset| {
                            let yaw = fighter.yaw
                                + (offset as f32 + id as f32 + self.time.floor())
                                    * std::f32::consts::TAU
                                    / 12.0;
                            let direction = Vec3(yaw.cos(), yaw.sin(), 0.0);
                            let point = position + direction * 325.0;
                            (visible(point)
                                && map.collision.ray(point, Vec3(0.0, 0.0, -95.0)).is_some())
                            .then_some(Step::Walk(point))
                        })
                        .next()
                });
            bot.rethink_at = self.time + 1.35;
        }
        if let Some(command) = follow_transfer(map, fighter, self.time, dt) {
            return command;
        }
        if let Some(Step::Walk(goal)) = fighter.bot.goal {
            return steer(fighter, goal, dt);
        }
        Command {
            look: [(4.6 * dt).min(1.5), -fighter.pitch * (dt * 4.0).min(1.0)],
            ..Command::default()
        }
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

fn steer(fighter: &Fighter, point: Vec3, dt: f32) -> Command {
    let delta = point - fighter.walker.position;
    let distance = delta.0.hypot(delta.1);
    if distance < 10.0 {
        return Command::default();
    }
    let (turn, _) = aim_error(delta, fighter.yaw, fighter.pitch);
    let yaw_change = turn.clamp(-4.6 * dt, 4.6 * dt);
    let (side, forward) = (turn - yaw_change).sin_cos();
    let speed = (distance / 145.0).clamp(0.19, 0.8);
    Command {
        movement: [forward * speed, side * speed],
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

fn aim_error(delta: Vec3, yaw: f32, pitch: f32) -> (f32, f32) {
    let turn = (delta.1.atan2(delta.0) - yaw + std::f32::consts::PI)
        .rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    (turn, delta.2.atan2(delta.0.hypot(delta.1)) - pitch)
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
            };
            for offset in [154.0, 268.0] {
                game.bolts.push(Bolt {
                    owner: 1,
                    position: eye + direction * offset,
                    velocity: Vec3::default(),
                    born: 0.0,
                    expires: 9.0,
                    payload,
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
        pose(&pawn, &map.player_clips.fire_run, 0.0);
        pose(&pawn, &map.player_clips.run, 1.0);
        pawn.hurt_at = 1.0;
        pose(&pawn, &map.player_clips.hurt, 1.0);
        pawn.fell_at = Some(1.0);
        pose(&pawn, &map.player_deaths[1], 1.0);
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
            frame.draw_deathmatch(&map, &camera, &game, 90.0);
            let scene = frame.pixels.clone();
            crate::hud::Hud::new().draw_deathmatch(&mut frame, &game, false, false);
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
            payload: ProjectilePayload {
                kind: super::super::ordnance::Kind::Rocket,
                damage: 85,
                blast: 190.0,
                gravity: 0.0,
                rebounds: 0,
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
