use std::sync::Arc;

use crate::{
    map::{Map, Vec3},
    mover::unit,
    sound::Wave,
    texture::Texture,
};

use super::{Motion, TrainingTarget, ordnance::Visual};

#[derive(Default)]
pub struct Art {
    pub tracer: Motion,
    pub cases: [Motion; 2],
    pub smoke: [Option<Arc<Texture>>; 4],
    pub spark: Option<Arc<Texture>>,
    pub wall_sounds: [Option<Arc<Wave>>; 3],
    pub flesh: Option<Arc<Wave>>,
    pub shell: Option<Arc<Wave>>,
}

#[derive(Default)]
pub struct Effects {
    pub(super) rounds: u8,
    pub(super) last_case: f32,
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

impl Effects {
    pub(super) fn update(
        &mut self,
        map: &Map,
        target: Option<&TrainingTarget>,
        time: f32,
        dt: f32,
        sounds: &mut Vec<Arc<Wave>>,
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
                    sounds.extend(map.ballistics.shell.iter().cloned());
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
