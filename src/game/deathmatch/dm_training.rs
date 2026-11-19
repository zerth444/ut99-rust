use std::sync::Arc;

use crate::{
    assault::Volume,
    game::{Motion, Weapon},
    map::{Map, Vec3},
    sound::{SoundEvents, Wave},
};

use super::{Command, Fighter};
pub(crate) mod gibs;
use crate::{
    collision::{PLAYER_HEIGHT, PLAYER_RADIUS},
    sound::CONTACT_SOUND_RADIUS,
};

pub(crate) struct Lesson {
    pub text: String,
    pub audio: Arc<Wave>,
}

pub(crate) struct Barrel {
    pub prop: usize,
    pub volume: Volume,
    pub health: i32,
    pub pushable: bool,
    pub fixed: bool,
}

pub(super) struct BarrelState {
    pub volume: Volume,
    pub health: i32,
    velocity: Vec3,
    falling: bool,
    push_until: Option<f32>,
}

pub(crate) struct Data {
    pub lessons: Vec<Lesson>,
    pub barrels: Vec<Barrel>,
    pub nali_idle: Motion,
    pub nali_death: Motion,
    pub nali_radius: f32,
    pub nali_height: f32,
    pub debris: crate::assault::Effects,
    pub reveal_sound: Option<Arc<Wave>>,
    pub push_sounds: [Option<Arc<Wave>>; 2],
    pub nali_death_sound: Option<Arc<Wave>>,
    pub gibs: Option<gibs::Art>,
}

pub(super) enum Event {
    OpenLocker,
    Give(Weapon),
    ResetScore,
    StartDuel,
}

pub(super) struct Session {
    pub current: Option<usize>,
    pub duel: bool,
    next: usize,
    pub(super) at: f32,
    jump_origin: Option<f32>,
    aim_origin: Option<[f32; 2]>,
    pub barrels: Vec<BarrelState>,
    pub barrels_revealed: bool,
    pub nali: Option<Vec3>,
    pub nali_died: Option<f32>,
    pub gibs: Option<gibs::Gibs>,
    death_sound_pending: bool,
    falling: f32,
    rng: u32,
    pub debris: super::breakables::Debris,
}

impl Session {
    pub fn new(data: &Data) -> Self {
        Self {
            current: None,
            duel: false,
            next: 0,
            at: 3.0,
            jump_origin: None,
            aim_origin: None,
            barrels: data
                .barrels
                .iter()
                .map(|barrel| BarrelState {
                    volume: barrel.volume,
                    health: barrel.health,
                    velocity: Vec3::default(),
                    falling: false,
                    push_until: None,
                })
                .collect(),
            barrels_revealed: false,
            nali: None,
            nali_died: None,
            gibs: None,
            death_sound_pending: false,
            falling: 0.0,
            rng: 0x5c6b_b7e9,
            debris: super::breakables::Debris::new(&data.debris, 0),
        }
    }

    pub fn step(
        &mut self,
        map: &Map,
        player: &Fighter,
        command: Command,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
    ) -> Option<Event> {
        let data = map.dm_training.as_ref()?;
        if std::mem::take(&mut self.death_sound_pending)
            && let Some(position) = self.nali
        {
            sounds.extend_at(
                data.nali_death_sound.iter().cloned(),
                position,
                CONTACT_SOUND_RADIUS,
                1.0,
            );
        }
        if let Some((gibs, art)) = self.gibs.as_mut().zip(data.gibs.as_ref()) {
            gibs.tick(art, map, time, dt, &mut self.rng, sounds);
        }
        if let Some(mut position) = self.nali.filter(|_| self.nali_died.is_none()) {
            self.falling += map.collision.regions.at(position).gravity.2 * dt;
            let delta = Vec3(0.0, 0.0, self.falling * dt);
            if let Some((point, _)) = map.collision.projectile_surface(
                position,
                delta,
                data.nali_radius,
                data.nali_height,
            ) {
                position = point + Vec3(0.0, 0.0, 0.2);
                self.falling = 0.0;
            } else {
                position = position + delta;
            }
            self.nali = Some(position);
        }
        if self.duel || time < self.at {
            return None;
        }
        let ready = match self.next {
            6 => command.movement[1] != 0.0,
            7 => {
                player.walker.position.2
                    != *self.jump_origin.get_or_insert(player.walker.position.2)
            }
            10 => {
                [player.yaw, player.pitch]
                    != *self.aim_origin.get_or_insert([player.yaw, player.pitch])
            }
            14 => player.has_weapon(),
            15 | 19 => command.primary,
            16 => command.alternate,
            17 => player.weapon == Weapon::Enforcer,
            18 => player.weapon == Weapon::Hammer,
            20 => {
                if self.nali.is_none() {
                    self.nali = Some(data.barrels[0].volume.position + Vec3(0.0, 0.0, 100.0));
                }
                self.nali_died.is_some()
            }
            22 => self.barrels.iter().all(|barrel| barrel.health < 0),
            _ => true,
        };
        if !ready {
            return None;
        }
        let index = self.next;
        self.next += 1;
        if index == 23 {
            self.current = None;
            self.duel = true;
            return Some(Event::StartDuel);
        }
        let lesson = &data.lessons[index];
        sounds.push(lesson.audio.clone());
        self.current = Some(index);
        self.at = time + lesson.audio.duration().floor() + 2.0;
        match index {
            13 => Some(Event::OpenLocker),
            16 => Some(Event::Give(Weapon::Enforcer)),
            17 => Some(Event::Give(Weapon::Hammer)),
            20 => Some(Event::ResetScore),
            21 => {
                self.barrels_revealed = true;
                for barrel in &data.barrels {
                    sounds.extend_at(
                        data.reveal_sound.iter().cloned(),
                        barrel.volume.position,
                        crate::sound::CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                None
            }
            _ => None,
        }
    }

    pub fn targets<'a>(&'a self, data: &'a Data) -> impl Iterator<Item = (usize, Volume)> + 'a {
        self.nali
            .filter(|_| self.nali_died.is_none())
            .map(|position| {
                (
                    0,
                    Volume {
                        position,
                        radius: data.nali_radius,
                        height: data.nali_height,
                    },
                )
            })
            .into_iter()
            .chain(
                self.barrels
                    .iter()
                    .enumerate()
                    .filter_map(|(index, barrel)| {
                        (self.barrels_revealed && barrel.health >= 0)
                            .then_some((index + 1, barrel.volume))
                    }),
            )
    }

    pub fn move_barrels(
        &mut self,
        map: &Map,
        fighters: &mut [Fighter],
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
    ) {
        let Some(data) = map.dm_training.as_ref().filter(|_| self.barrels_revealed) else {
            return;
        };
        for (index, (state, barrel)) in self.barrels.iter_mut().zip(&data.barrels).enumerate() {
            if state.health < 0 {
                continue;
            }
            if state.push_until.is_some_and(|until| time >= until) {
                state.push_until = None;
                sounds.extend_at(
                    data.push_sounds[1].iter().cloned(),
                    state.volume.position,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            }
            if state.falling {
                let start = state.volume.position;
                state.velocity = state.velocity + map.collision.regions.at(start).gravity * dt;
                let travel = state.velocity * dt;
                if let Some((point, normal)) = map.collision.projectile_surface(
                    start,
                    travel,
                    state.volume.radius,
                    state.volume.height,
                ) {
                    state.volume.position = point + normal * 0.2;
                    if state.velocity.2 < -200.0 {
                        state.health = state.health.saturating_sub(100);
                        if state.health < 0 {
                            self.debris.break_decoration(
                                &data.debris,
                                index,
                                time,
                                state.volume.position,
                                normal * 10_000.0,
                                1.75,
                            );
                            continue;
                        }
                    }
                    state.velocity = Vec3::default();
                    state.falling = false;
                } else {
                    state.volume.position = start + travel;
                }
            }
            for fighter in fighters.iter_mut().filter(|fighter| fighter.health > 0) {
                let delta = fighter.walker.position - state.volume.position;
                let distance = delta.0.hypot(delta.1);
                let radius = PLAYER_RADIUS + state.volume.radius;
                if distance >= radius || delta.2.abs() >= PLAYER_HEIGHT + state.volume.height {
                    continue;
                }
                let velocity = fighter.walker.velocity();
                let outward = if distance > 0.001 {
                    Vec3(delta.0 / distance, delta.1 / distance, 0.0)
                } else {
                    Vec3(-fighter.yaw.cos(), -fighter.yaw.sin(), 0.0)
                };
                fighter
                    .walker
                    .carry(&map.collision, outward * (radius - distance));
                if barrel.fixed
                    || !barrel.pushable
                    || map.collision.profile.mass <= 40.0
                    || velocity.dot(outward) >= 0.0
                {
                    continue;
                }
                let speed = velocity.dot(velocity).sqrt();
                let push = velocity * ((20.0 + speed).min(120.0) / speed);
                state.velocity = Vec3(
                    push.0,
                    push.1,
                    if state.falling {
                        state.velocity.2
                    } else {
                        25.0
                    },
                );
                state.falling = true;
                if state.push_until.is_none() {
                    sounds.extend_at(
                        data.push_sounds[0].iter().cloned(),
                        state.volume.position,
                        CONTACT_SOUND_RADIUS,
                        0.25,
                    );
                }
                state.push_until = Some(time + 0.3);
            }
        }
    }

    pub fn trace(&self, data: &Data, start: Vec3, delta: Vec3) -> Option<(f32, usize)> {
        self.targets(data)
            .filter_map(|(index, volume)| Some((volume.trace(start, delta)?, index)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    pub fn damage(
        &mut self,
        data: &Data,
        index: usize,
        damage: u16,
        time: f32,
        momentum: Vec3,
        damage_type: &str,
    ) {
        if damage == 0 {
            return;
        }
        if index == 0 {
            let Some(origin) = self.nali.filter(|_| self.nali_died.is_none()) else {
                return;
            };
            self.nali_died = Some(time);
            if let Some(art) = &data.gibs
                && !damage_type.eq_ignore_ascii_case("shot")
                && !damage_type.eq_ignore_ascii_case("decapitated")
                && art.mass < 500.0
            {
                let health = 1.0 - f32::from(damage);
                let size = art.mass + art.health;
                if health < -0.5 * size
                    || (health < -25.0 - 0.1 * size * (0.2 + gibs::roll(&mut self.rng))
                        && gibs::roll(&mut self.rng) < 0.8)
                {
                    let zone = if self.falling < 0.0 {
                        self.falling
                    } else {
                        0.0
                    };
                    let force = momentum + Vec3(0.0, 0.0, zone * art.mass);
                    self.gibs = Some(gibs::Gibs::spawn(
                        art,
                        origin,
                        force,
                        data.nali_height,
                        time,
                        &mut self.rng,
                    ));
                }
            }
            self.death_sound_pending = self.gibs.is_none();
        } else if self.barrels_revealed
            && let Some(barrel) = index.checked_sub(1)
            && let Some(state) = self
                .barrels
                .get_mut(barrel)
                .filter(|state| state.health >= 0)
            && !data.barrels[barrel].fixed
        {
            state.health = state.health.saturating_sub(i32::from(damage));
            if state.health < 0 {
                self.debris.break_decoration(
                    &data.debris,
                    barrel,
                    time,
                    state.volume.position,
                    momentum,
                    1.75,
                );
            } else {
                state.velocity = Vec3(momentum.0 * 0.01, momentum.1 * 0.01, 10.0);
                state.falling = true;
            }
        }
    }
}
