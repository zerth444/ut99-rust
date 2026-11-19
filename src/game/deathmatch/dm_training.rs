use std::sync::Arc;

use crate::{
    assault::Volume,
    game::{Motion, Weapon},
    map::{Map, Vec3},
    sound::{SoundEvents, Wave},
};

use super::{Command, Fighter};

pub(crate) struct Lesson {
    pub text: String,
    pub audio: Arc<Wave>,
}

pub(crate) struct Barrel {
    pub prop: usize,
    pub volume: Volume,
    pub health: i32,
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
    pub barrels: Vec<i32>,
    pub barrels_revealed: bool,
    pub nali: Option<Vec3>,
    pub nali_died: Option<f32>,
    falling: f32,
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
            barrels: data.barrels.iter().map(|barrel| barrel.health).collect(),
            barrels_revealed: false,
            nali: None,
            nali_died: None,
            falling: 0.0,
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
            22 => self.barrels.iter().all(|&health| health < 0),
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
                data.barrels
                    .iter()
                    .enumerate()
                    .filter_map(|(index, barrel)| {
                        (self.barrels_revealed && self.barrels[index] >= 0)
                            .then_some((index + 1, barrel.volume))
                    }),
            )
    }

    pub fn trace(&self, data: &Data, start: Vec3, delta: Vec3) -> Option<(f32, usize)> {
        self.targets(data)
            .filter_map(|(index, volume)| Some((volume.trace(start, delta)?, index)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    pub fn damage(&mut self, data: &Data, index: usize, damage: u16, time: f32, momentum: Vec3) {
        if damage == 0 {
            return;
        }
        if index == 0 && self.nali.is_some() {
            self.nali_died.get_or_insert(time);
        } else if self.barrels_revealed
            && let Some(barrel) = index.checked_sub(1)
            && let Some(health) = self.barrels.get_mut(barrel).filter(|health| **health >= 0)
        {
            *health = health.saturating_sub(i32::from(damage));
            if *health < 0 {
                self.debris
                    .break_decoration(&data.debris, barrel, time, momentum, 1.75);
            }
        }
    }
}
