use std::sync::Arc;

use crate::{
    game::{Motion, Pose, ballistics::jitter},
    map::{Face, Map, Vec3},
    sound::{CONTACT_SOUND_RADIUS, SoundEvents, Wave},
};

pub(crate) struct Part {
    pub faces: Vec<Face>,
    pub height: f32,
    pub trail: bool,
}

pub(crate) struct Art {
    pub parts: Vec<Part>,
    pub spurt: Motion,
    pub trail: Motion,
    pub impacts: [Option<Arc<Wave>>; 5],
    pub collision: [f32; 2],
    pub mass: f32,
    pub health: f32,
    pub lifetime: Option<f32>,
}

struct Chunk {
    part: usize,
    position: Vec3,
    velocity: Vec3,
    rotation: Vec3,
    spin: Vec3,
    resting: bool,
    trailing: bool,
}

struct Spurt {
    position: Vec3,
    direction: Vec3,
    born: f32,
}

pub(crate) struct Gibs {
    chunks: Vec<Chunk>,
    spurts: Vec<Spurt>,
    expires: f32,
}

impl Gibs {
    pub fn spawn(
        art: &Art,
        origin: Vec3,
        momentum: Vec3,
        height: f32,
        time: f32,
        seed: &mut u32,
    ) -> Self {
        let base_velocity = momentum * art.mass.max(1.0).recip();
        let chunks = art
            .parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let yaw = jitter(seed) * 100_000.0;
                let pitch = jitter(seed) * 0.5 * (200_000.0 - yaw.abs());
                let velocity = if index == 0 {
                    base_velocity
                } else {
                    let z = jitter(seed);
                    let angle = roll(seed) * std::f32::consts::TAU;
                    let horizontal = 700.0 * roll(seed) * (1.0 - z * z).max(0.0).sqrt();
                    let variation = Vec3(
                        horizontal * angle.cos(),
                        horizontal * angle.sin(),
                        200.0 * roll(seed) - 50.0,
                    );
                    (base_velocity + variation) * (0.2 + roll(seed))
                };
                Chunk {
                    part: index,
                    position: origin + Vec3(0.0, 0.0, part.height * height),
                    velocity,
                    rotation: Vec3::default(),
                    spin: Vec3(pitch, yaw, 0.0) * (std::f32::consts::TAU / 65_536.0),
                    resting: false,
                    trailing: part.trail,
                }
            })
            .collect();
        Self {
            chunks,
            spurts: vec![Spurt {
                position: origin,
                direction: Vec3(0.0, 0.0, 1.0),
                born: time,
            }],
            expires: art.lifetime.map_or(f32::INFINITY, |age| time + age),
        }
    }

    pub fn only_part(&mut self, index: usize) {
        self.chunks.retain(|chunk| chunk.part == index);
    }

    pub fn expired(&self) -> bool {
        self.chunks.is_empty() && self.spurts.is_empty()
    }

    pub fn tick(
        &mut self,
        art: &Art,
        map: &Map,
        time: f32,
        dt: f32,
        seed: &mut u32,
        sounds: &mut SoundEvents,
    ) {
        self.spurts
            .retain(|spurt| time - spurt.born < art.spurt.duration());
        if time >= self.expires {
            self.chunks.clear();
            return;
        }
        for chunk in self.chunks.iter_mut().filter(|chunk| !chunk.resting) {
            let zone = map.collision.regions.at(chunk.position);
            if zone.water {
                chunk.trailing = false;
            }
            chunk.velocity = chunk.velocity + zone.gravity * dt;
            chunk.rotation = chunk.rotation + chunk.spin * dt;
            let travel = chunk.velocity * dt;
            if let Some((point, normal)) = map.collision.projectile_surface(
                chunk.position,
                travel,
                art.collision[0],
                art.collision[1],
            ) {
                chunk.position = point + normal * 0.2;
                chunk.velocity =
                    (chunk.velocity - normal * (2.0 * chunk.velocity.dot(normal))) * 0.8;
                chunk.velocity.2 = (chunk.velocity.2 * 0.8).min(700.0);
                let speed = chunk.velocity.dot(chunk.velocity).sqrt();
                if speed < 250.0 {
                    chunk.trailing = false;
                }
                if speed < 120.0 && normal.2 > 0.7 {
                    chunk.resting = true;
                    chunk.rotation = Vec3(0.0, chunk.rotation.1, 0.0);
                } else if speed > 350.0 {
                    if speed > 700.0 {
                        chunk.velocity = chunk.velocity * 0.8;
                    }
                    let index = ((roll(seed) * 5.0) as usize).min(4);
                    sounds.extend_at(
                        art.impacts[index].iter().cloned(),
                        point,
                        CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                if !chunk.trailing && self.spurts.len() < 24 {
                    self.spurts.push(Spurt {
                        position: point,
                        direction: normal,
                        born: time,
                    });
                }
            } else {
                chunk.position = chunk.position + travel;
            }
        }
    }

    pub fn models<'a>(
        &'a self,
        art: &'a Art,
    ) -> impl Iterator<Item = (Vec3, Vec3, &'a [Face])> + 'a {
        self.chunks.iter().map(|chunk| {
            (
                chunk.position,
                chunk.rotation,
                art.parts[chunk.part].faces.as_slice(),
            )
        })
    }

    pub fn spurts<'a>(
        &'a self,
        art: &'a Art,
        time: f32,
    ) -> impl Iterator<Item = (Vec3, Vec3, Pose<'a>)> + 'a {
        let spurts = self.spurts.iter().filter_map(move |spurt| {
            Some((
                spurt.position,
                spurt.direction,
                art.spurt.sample(time - spurt.born)?,
            ))
        });
        let loop_time = art.trail.duration();
        spurts.chain(self.chunks.iter().filter_map(move |chunk| {
            (chunk.trailing && loop_time > 0.0).then_some(())?;
            Some((
                chunk.position,
                Vec3(1.0, 0.0, 0.0),
                art.trail.sample(time % loop_time)?,
            ))
        }))
    }
}

pub(in crate::game::deathmatch) fn roll(seed: &mut u32) -> f32 {
    (jitter(seed) + 1.0) * 0.5
}
