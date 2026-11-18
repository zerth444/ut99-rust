//! Offline arena simulation. The window supplies input and displays snapshots.

mod navigation;

use super::{Item, Weapon, shock::cylinder_hit};
use crate::{
    collision::{PLAYER_HEIGHT, PLAYER_RADIUS, Walker},
    map::{Map, Vec3},
    mover::{LiftStop, unit},
    render::Camera,
    sound::{BLAST_SOUND_RADIUS, CONTACT_SOUND_RADIUS, SoundEvents},
};
use navigation::{LiftRide, Navigation, Step, WAYPOINT_RADIUS};

const RESPAWN_DELAY: f32 = 1.25;
const SPAWN_PROTECTION: f32 = 0.75;
const BOLT_SPEED: f32 = 1250.0;
const MAGAZINE_LIMIT: u16 = 100;
const BLAST_RANGE: f32 = 190.0;

#[derive(Clone, Copy)]
pub struct Rules {
    pub bots: usize,
    pub frag_limit: i16,
    pub seconds: u16,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            bots: 3,
            frag_limit: 10,
            seconds: 300,
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
}

pub struct Fighter {
    pub walker: Walker,
    pub yaw: f32,
    pitch: f32,
    pub health: u16,
    pub armor: u16,
    pub ammo: u16,
    pub score: i16,
    pub deaths: u16,
    pub fired_at: f32,
    pub fell_at: Option<f32>,
    protected_until: f32,
    ready_at: f32,
    hazard_at: f32,
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
            armor: 0,
            ammo: 34,
            score: 0,
            deaths: 0,
            fired_at: f32::NEG_INFINITY,
            fell_at: None,
            protected_until: time + SPAWN_PROTECTION,
            ready_at: time,
            hazard_at: time,
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
    direction: Vec3,
    expires: f32,
}

pub struct Trace {
    pub start: Vec3,
    pub end: Vec3,
    pub blast: bool,
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

    pub fn bolts(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.bolts.iter().map(|bolt| bolt.position)
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
            let motion = fighter.walker.advance_with_swim(
                &map.collision,
                walk * speed,
                swim,
                input.jump.then_some(map.collision.profile.jump_speed),
                dt,
            );
            if let Some(step) = motion.footstep {
                sounds.extend_at(
                    map.player_audio.steps[step].iter().cloned(),
                    fighter.walker.position,
                    500.0,
                    0.7,
                );
            }
            if let Some((position, yaw)) = crate::teleport::touched(
                &map.teleporters,
                fighter.walker.position,
                &mut fighter.portal,
            ) {
                fighter.walker = Walker::spawn(&map.collision, position);
                fighter.yaw = yaw.unwrap_or(fighter.yaw);
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
            for fighter in &mut self.fighters {
                let delta = fighter.walker.position - prop.position;
                if fighter.health == 0
                    || delta.2.abs() > prop.pickup_height + PLAYER_HEIGHT
                    || delta.0.hypot(delta.1) > prop.pickup_radius + PLAYER_RADIUS
                    || map.collision.ray(prop.position, delta).is_some()
                {
                    continue;
                }
                if !take_item(fighter, item) {
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
        if fighter.health == 0
            || self.time < fighter.ready_at
            || !(input.primary || input.alternate)
        {
            return;
        }
        let bolt = input.alternate && fighter.ammo >= 3;
        let ammo = if bolt { 3 } else { u16::from(fighter.ammo > 0) };
        if ammo == 0 && !input.primary {
            return;
        }
        fighter.ammo -= ammo;
        fighter.fired_at = self.time;
        fighter.ready_at = self.time
            + if bolt {
                0.85
            } else if ammo > 0 {
                0.32
            } else {
                0.52
            };
        let aim = fighter.camera(map);
        let direction = aim.look_direction();
        let start = aim.position;
        let distance = if ammo > 0 { 3500.0 } else { 96.0 };
        if let Some(art) = map.arsenal.get(&Weapon::Enforcer) {
            if id == 0 {
                sounds.extend(art.fire.iter().cloned());
            } else {
                sounds.extend_at(art.fire.iter().cloned(), start, CONTACT_SOUND_RADIUS, 1.0);
            }
        }
        if bolt {
            self.bolts.push(Bolt {
                owner: id,
                position: start,
                direction,
                expires: self.time + 3.0,
            });
            return;
        }
        let delta = direction * distance;
        let end = map.collision.ray(start, delta).unwrap_or(start + delta);
        let clipped = end - start;
        let target = nearest_hit(&self.fighters, id, start, clipped);
        if let Some((victim, _)) = target {
            hits.push(Hit {
                victim,
                owner: Some(id),
                damage: if ammo > 0 { 26 } else { 34 },
                impulse: direction * 110.0,
            });
        }
        let end = target.map_or(end, |(_, fraction)| start + clipped * fraction);
        self.traces.push(Trace {
            start,
            end,
            blast: false,
            expires: self.time + 0.065,
        });
    }

    fn move_bolts(&mut self, map: &Map, dt: f32, hits: &mut Vec<Hit>, sounds: &mut SoundEvents) {
        self.bolts.retain_mut(|bolt| {
            let delta = bolt.direction * (BOLT_SPEED * dt);
            let wall = map.collision.ray(bolt.position, delta);
            let end = wall.unwrap_or(bolt.position + delta);
            let target = nearest_hit(
                &self.fighters,
                bolt.owner,
                bolt.position,
                end - bolt.position,
            );
            let contact =
                target.map(|(_, fraction)| bolt.position + (end - bolt.position) * fraction);
            if contact.is_none() && wall.is_none() && self.time < bolt.expires {
                bolt.position = end;
                return true;
            }
            let position = contact.unwrap_or(end) - bolt.direction * 0.5;
            for (victim, fighter) in self
                .fighters
                .iter()
                .enumerate()
                .filter(|(_, f)| f.health > 0)
            {
                let delta = fighter.walker.position - position;
                let distance = (delta.dot(delta).sqrt() - PLAYER_RADIUS).max(0.0);
                if distance >= BLAST_RANGE || map.collision.ray(position, delta).is_some() {
                    continue;
                }
                let strength = 1.0 - distance / BLAST_RANGE;
                hits.push(Hit {
                    victim,
                    owner: Some(bolt.owner),
                    damage: (85.0 * strength) as u16,
                    impulse: unit(delta) * (370.0 * strength),
                });
            }
            self.traces.push(Trace {
                start: position,
                end: position,
                blast: true,
                expires: self.time + 0.21,
            });
            let art = &map.ordnance[super::ordnance::Kind::Rocket as usize];
            sounds.extend_at(art.sound.iter().cloned(), position, BLAST_SOUND_RADIUS, 1.0);
            false
        });
    }

    fn apply_hit(&mut self, map: &Map, hit: Hit, sounds: &mut SoundEvents) {
        let victim = &mut self.fighters[hit.victim];
        if victim.health == 0
            || hit.damage == 0
            || (hit.owner.is_some() && self.time < victim.protected_until)
        {
            return;
        }
        let absorbed = victim.armor.min(hit.damage / 2);
        victim.armor -= absorbed;
        victim.health = victim.health.saturating_sub(hit.damage - absorbed);
        victim.walker.add_velocity(hit.impulse);
        if victim.health > 0 {
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
        if let Some(owner) = hit.owner.filter(|&owner| owner != hit.victim) {
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
                (distance < 1450.0_f32.powi(2) && map.collision.ray(eye.position, delta).is_none())
                    .then_some((other, fighter.walker.position, delta, distance))
            })
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let fighter = &mut self.fighters[id];
        let bot = &mut fighter.bot;
        if let Some((other, target_position, mut delta, distance)) = target {
            bot.last_seen = Some((target_position, self.time));
            bot.plan_origin = None;
            if bot.target != Some(other) {
                bot.target = Some(other);
                bot.react_at = self.time + 0.23 + (id % 3) as f32 * 0.09;
            }
            let strafe = (self.time * 2.1 + id as f32).sin().signum();
            delta.1 += (self.time * 4.2 + id as f32).sin() * (distance.sqrt() * 0.027);
            let (turn, pitch) = aim_error(delta, fighter.yaw, fighter.pitch);
            let firing = self.time >= bot.react_at && turn.abs() < 0.13 && pitch.abs() < 0.11;
            let bolt = firing
                && distance > 190.0_f32.powi(2)
                && fighter.ammo >= 5
                && (self.time + id as f32).rem_euclid(4.7) < 0.6;
            return Command {
                movement: [
                    if fighter.ammo == 0 || distance > 360.0_f32.powi(2) {
                        0.8
                    } else {
                        -0.22
                    },
                    strafe * 0.53,
                ],
                look: [
                    turn.clamp(-3.8 * dt, 3.8 * dt),
                    pitch.clamp(-2.8 * dt, 2.8 * dt),
                ],
                primary: firing && !bolt,
                alternate: bolt,
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
                            Some(Item::Armor(kind)) => fighter.armor < kind.charge(),
                            Some(Item::Ammo(..)) => fighter.ammo < 65,
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

fn take_item(fighter: &mut Fighter, item: Item) -> bool {
    match item {
        Item::Health(amount, limit) if fighter.health < limit => {
            fighter.health = fighter.health.saturating_add(amount).min(limit);
        }
        Item::Armor(kind) if fighter.armor < kind.charge() => fighter.armor = kind.charge(),
        Item::Ammo(_, amount) if fighter.ammo < MAGAZINE_LIMIT => {
            fighter.ammo = fighter.ammo.saturating_add(amount).min(MAGAZINE_LIMIT);
        }
        _ => return false,
    }
    true
}

fn nearest_hit(
    fighters: &[Fighter],
    owner: usize,
    start: Vec3,
    delta: Vec3,
) -> Option<(usize, f32)> {
    fighters
        .iter()
        .enumerate()
        .filter(|(id, fighter)| *id != owner && fighter.health > 0)
        .filter_map(|(id, fighter)| {
            let fraction = cylinder_hit(
                start - fighter.walker.position,
                delta,
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
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
            direction: Vec3(1.0, 0.0, 0.0),
            expires: game.time + 2.0,
        });
        game.step(&mut map, Command::default(), 1.0 / 60.0);
        assert!(game.bolts.is_empty());
        assert_eq!(game.fighters[0].score, 0);
        assert_eq!(game.fighters[1].deaths, 1);
        for _ in 0..85 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert!(game.fighters[0].health > 0 && game.fighters[1].health > 0);
        assert_eq!(game.fighters[1].deaths, 1);
        for _ in 0..130 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
        }
        assert!(game.finished);
        assert_eq!(game.remaining(), 0);
    }
}
