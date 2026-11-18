use std::sync::Arc;

use crate::{
    map::{Map, Vec3},
    mover::unit,
    render::Camera,
    sound::{CONTACT_SOUND_RADIUS, SoundEvents, Wave},
    texture::Texture,
};

use super::{Motion, TrainingTarget, Weapon, decals::Marks, ordnance::Visual};

pub(super) fn jitter(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*seed >> 16) as f32 / 32768.0) - 1.0
}

#[derive(Default)]
pub struct Art {
    pub tracer: Motion,
    pub cases: [Motion; 2],
    pub smoke: [Option<Arc<Texture>>; 4],
    pub spark: Option<Arc<Texture>>,
    pub wall_sounds: [Option<Arc<Wave>>; 3],
    pub flesh: Option<Arc<Wave>>,
    pub shell: Option<Arc<Wave>>,
    pub pock: [Option<Arc<Texture>>; 3],
    pub pock_scale: f32,
}

#[derive(Default)]
pub(super) struct Cadence {
    rounds: u8,
    last_case: f32,
}

#[derive(Default)]
pub struct Effects {
    pub(super) tracer: Vec<Moving>,
    pub(super) cases: Vec<Moving>,
    pub(super) sparks: Vec<Moving>,
    pub(super) smoke: Vec<(Vec3, f32, usize)>,
}

pub(super) struct Moving {
    pub position: Vec3,
    pub velocity: Vec3,
    pub born: f32,
    pub style: usize,
    pub scale: f32,
    pub bounces: u8,
}

pub(super) struct Bullet {
    pub start: Vec3,
    pub delta: Vec3,
    pub visible: Vec3,
    pub wall: Option<(Vec3, Vec3)>,
    pub case_origin: Option<Vec3>,
}

pub(super) struct BulletProfile {
    pub interval: f32,
    pub accuracy: f32,
    pub damage: u16,
    offset: [f32; 3],
}

impl BulletProfile {
    pub const RANGE: f32 = 10_000.0;

    pub fn for_weapon(
        map: &Map,
        weapon: Weapon,
        alternate: bool,
        enforcer_accuracy: &mut f32,
        mut jitter: impl FnMut() -> f32,
    ) -> Option<Self> {
        let (interval, accuracy, damage, offset) = match (weapon, alternate) {
            (Weapon::Enforcer, false) => (0.32, 0.2, 17, [3.3, 5.5, -4.4]),
            (Weapon::Enforcer, true) => {
                let accuracy = *enforcer_accuracy;
                if accuracy < 3.0 {
                    *enforcer_accuracy += 0.5;
                }
                let interval = map
                    .arsenal
                    .get(&weapon)
                    .map_or(0.0, |art| art.alternate_motion.duration());
                (
                    if interval > 0.0 { interval } else { 0.19 },
                    accuracy,
                    17,
                    [3.3, 5.5, -4.4],
                )
            }
            (Weapon::Minigun, alternate) => (
                if alternate { 0.08 } else { 0.13 },
                if alternate { 0.95 } else { 0.2 },
                9 + (((jitter() + 1.0) * 3.0) as u16).min(5),
                [0.0, 5.0, -4.0],
            ),
            (Weapon::Sniper, false) => (0.67, 0.0, 45, [0.0; 3]),
            _ => return None,
        };
        Some(Self {
            interval,
            accuracy,
            damage,
            offset,
        })
    }

    pub fn ray(&self, camera: &Camera, mut jitter: impl FnMut() -> f32) -> (Vec3, Vec3) {
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        let [ahead, side, height] = self.offset;
        let start = camera.position + right * side + up * height + forward * ahead;
        let delta =
            forward * Self::RANGE + (right * jitter() + up * jitter()) * (500.0 * self.accuracy);
        (start, delta)
    }
}

impl Effects {
    pub(super) fn bullet(
        &mut self,
        map: &Map,
        decals: &mut Marks,
        camera: &Camera,
        weapon: Weapon,
        time: f32,
        shot: Bullet,
        cadence: &mut Cadence,
        sounds: &mut SoundEvents,
        mut jitter: impl FnMut() -> f32,
    ) {
        let Bullet {
            start,
            delta,
            visible,
            wall,
            case_origin,
        } = shot;
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        if weapon == Weapon::Minigun {
            cadence.rounds = (cadence.rounds + 1) % 4;
            if cadence.rounds == 0 && visible.dot(visible) > 250.0_f32.powi(2) {
                self.tracer.push(Moving {
                    position: start + forward * 96.0,
                    velocity: unit(delta) * 4000.0,
                    born: time,
                    style: 0,
                    scale: 1.0,
                    bounces: 0,
                });
            }
        }
        if weapon != Weapon::Minigun || time - cadence.last_case > 0.125 {
            let (ahead, side, height) = match weapon {
                Weapon::Enforcer => (20.0, 3.5, 1.0),
                Weapon::Minigun => (30.0, 5.0, -5.0),
                _ => (30.0, 5.0, -1.0),
            };
            let offset =
                case_origin.map_or(forward * ahead + right * side + up * height, |point| {
                    // Keep the ejection distance while matching the projected viewmodel.
                    (forward * point.0 + right * point.1 + up * point.2) * (ahead / point.0)
                });
            if map.collision.ray(camera.position, offset).is_none() {
                let velocity = (forward * (0.55 + 0.15 * jitter())
                    + right * (0.3 + 0.1 * jitter())
                    + up * (1.15 + 0.15 * jitter()))
                    * 160.0;
                self.cases.push(Moving {
                    position: camera.position + offset,
                    velocity,
                    born: time,
                    style: usize::from(weapon == Weapon::Minigun),
                    scale: if weapon == Weapon::Sniper { 2.0 } else { 1.0 },
                    bounces: 0,
                });
            }
            if weapon == Weapon::Minigun {
                cadence.last_case = time;
            }
        }
        if let Some((point, normal)) = wall {
            let pock = (((jitter() + 1.0) * 1.5) as usize).min(2);
            let expires = time + 19.0 + (jitter() + 1.0) * 2.5;
            if let Some(texture) = &map.ballistics.pock[pock] {
                decals.project(
                    &map.faces,
                    point,
                    normal,
                    texture,
                    map.ballistics.pock_scale,
                    expires,
                );
            }
            let sound = (((jitter() + 1.0) * 2.0) as usize).min(3);
            if let Some(sound) = map.ballistics.wall_sounds.get(sound) {
                sounds.extend_at(sound.iter().cloned(), point, CONTACT_SOUND_RADIUS, 1.0);
            }
            let origin = point + normal * if weapon == Weapon::Enforcer { 1.0 } else { 9.0 };
            let style = (((jitter() + 1.0) * 2.0) as usize).min(3);
            self.smoke.push((origin, time, style));
            let count = if weapon == Weapon::Minigun { 2 } else { 3 };
            let sparks = ((jitter() + 1.0) * 0.5 * count as f32) as u8;
            for _ in 0..sparks {
                let direction = unit(Vec3(jitter(), jitter(), jitter()));
                let velocity = (normal + direction) * (100.0 * (jitter() + 1.0));
                self.sparks.push(Moving {
                    position: point + normal * 9.0,
                    velocity,
                    born: time,
                    style: 0,
                    scale: 1.0,
                    bounces: 0,
                });
            }
        }
    }

    pub(super) fn update(
        &mut self,
        map: &Map,
        target: Option<&TrainingTarget>,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
    ) {
        self.tracer.retain_mut(|shot| {
            let delta = shot.velocity * dt;
            if time - shot.born >= 2.0
                || map.collision.ray(shot.position, delta).is_some()
                || target.is_some_and(|target| target.ray(shot.position, delta).is_some())
            {
                return false;
            }
            shot.position = shot.position + delta;
            true
        });
        self.smoke.retain(|&(_, born, _)| time - born < 0.4);
        self.sparks.retain_mut(|spark| {
            spark.velocity = spark.velocity + map.collision.regions.at(spark.position).gravity * dt;
            let delta = spark.velocity * dt;
            if time - spark.born >= 1.0 || map.collision.ray(spark.position, delta).is_some() {
                return false;
            }
            spark.position = spark.position + delta;
            true
        });
        self.cases.retain_mut(|case| {
            if time - case.born >= 3.0 {
                return false;
            }
            if case.bounces >= 3 {
                return true;
            }
            case.velocity = case.velocity + map.collision.regions.at(case.position).gravity * dt;
            let delta = case.velocity * dt;
            if let Some((point, normal)) = map.collision.ray_surface(case.position, delta) {
                case.position = point + normal * 0.5;
                if case.bounces == 0 {
                    sounds.extend_at(map.ballistics.shell.iter().cloned(), point, 480.0, 1.0);
                }
                if case.bounces >= 2 && normal.2 <= 0.7 {
                    return false;
                }
                case.bounces += 1;
                case.velocity = (case.velocity - normal * (2.0 * case.velocity.dot(normal))) * 0.5;
                if normal.2 > 0.7 && case.velocity.2 < 40.0 {
                    case.bounces = 3;
                }
            } else {
                case.position = case.position + delta;
            }
            true
        });
    }

    pub fn visuals<'a>(&self, art: &'a Art, time: f32, mut draw: impl FnMut(Visual<'a>)) {
        if let Some(pose) = art.tracer.sample(0.0) {
            for shot in &self.tracer {
                draw(Visual::Mesh(
                    shot.position,
                    unit(shot.velocity),
                    pose,
                    1.0,
                    None,
                ));
            }
        }
        for case in &self.cases {
            if let Some(pose) = art.cases[case.style].sample(0.0) {
                let phase = (time - case.born) * 17.0;
                let direction = if case.bounces >= 3 {
                    Vec3(1.0, 0.0, 0.0)
                } else {
                    unit(Vec3(phase.cos(), phase.sin(), (phase * 1.3).sin()))
                };
                draw(Visual::Mesh(
                    case.position,
                    direction,
                    pose,
                    case.scale,
                    None,
                ));
            }
        }
        if let Some(texture) = &art.spark {
            for spark in &self.sparks {
                draw(Visual::Sprite(spark.position, texture.image(0.0), 0.1, 1.0));
            }
        }
        for &(position, born, style) in &self.smoke {
            if let Some(texture) = &art.smoke[style] {
                let age = time - born;
                draw(Visual::Sprite(
                    position + Vec3(0.0, 0.0, 50.0 * age),
                    texture.image_once(age / 0.4),
                    2.0,
                    0.4,
                ));
            }
        }
    }
}
