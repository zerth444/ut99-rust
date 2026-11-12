use std::sync::Arc;

use super::{Motion, Pose, shock::Blast};
use crate::{
    map::{Map, Vec3},
    mover::unit,
    render::Camera,
    sound::Wave,
    texture::{Texture, TextureImage},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pulse,
    Rocket,
    Grenade,
    Gel,
    Glob,
    Splash,
    Chunk,
    FlakShell,
    Razor,
    RazorAlt,
}

impl Kind {
    pub const ALL: [Self; 10] = [
        Self::Pulse,
        Self::Rocket,
        Self::Grenade,
        Self::Gel,
        Self::Glob,
        Self::Splash,
        Self::Chunk,
        Self::FlakShell,
        Self::Razor,
        Self::RazorAlt,
    ];

    pub fn class(self) -> &'static str {
        match self {
            Self::Pulse => "PlasmaSphere",
            Self::Rocket => "RocketMk2",
            Self::Grenade => "UT_Grenade",
            Self::Gel => "UT_BioGel",
            Self::Glob => "BioGlob",
            Self::Splash => "BioSplash",
            Self::Chunk => "UTChunk1",
            Self::FlakShell => "flakslug",
            Self::Razor => "Razor2",
            Self::RazorAlt => "Razor2Alt",
        }
    }

    fn bio(self) -> bool {
        matches!(self, Self::Gel | Self::Glob | Self::Splash)
    }
}

#[derive(Default)]
pub struct Art {
    pub flight: Motion,
    pub stuck: Motion,
    pub sprite: Option<Arc<Texture>>,
    pub explosion: Option<Arc<Texture>>,
    pub trail: Option<Arc<Texture>>,
    pub sound: Option<Arc<Wave>>,
    pub contact: Option<Arc<Wave>>,
    pub speed: Option<f32>,
    pub damage: Option<f32>,
    pub momentum: Option<f32>,
    pub scale: Option<f32>,
}

pub type Arsenal = [Art; Kind::ALL.len()];

pub enum Visual<'a> {
    Sprite(Vec3, &'a TextureImage, f32, f32),
    Mesh(Vec3, Vec3, Pose<'a>, f32),
}

#[derive(Default)]
pub struct Shots {
    projectiles: Vec<Projectile>,
    effects: Vec<Effect>,
}

struct Projectile {
    kind: Kind,
    position: Vec3,
    velocity: Vec3,
    direction: Vec3,
    born: f32,
    expires: f32,
    scale: f32,
    stuck: Option<f32>,
    next_trail: f32,
    wall_hits: u8,
}

struct Effect {
    kind: Kind,
    position: Vec3,
    born: f32,
    duration: f32,
    scale: f32,
    trail: bool,
}

impl Shots {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fire(
        &mut self,
        map: &Map,
        camera: &Camera,
        kind: Kind,
        direction: Vec3,
        side: f32,
        scale: f32,
        time: f32,
        variation: f32,
        sounds: &mut Vec<Arc<Wave>>,
    ) -> Option<Blast> {
        let art = &map.ordnance[kind as usize];
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        let offset = match kind {
            Kind::Pulse => forward * 24.0 + right * 14.0 - up * 8.0,
            Kind::Rocket | Kind::Grenade => forward * 20.0 + right * (8.0 + side) - up * 9.0,
            Kind::FlakShell | Kind::Chunk => forward * 20.0 + right * (11.0 + side) - up * 15.0,
            Kind::Razor | Kind::RazorAlt => forward * 20.0 + right * 15.0 - up * 13.0,
            _ => forward * 18.0 + right * 11.0 - up * 6.0,
        };
        let speed = art.speed.unwrap_or(match kind {
            Kind::Pulse => 1450.0,
            Kind::Rocket => 900.0,
            Kind::Grenade => 600.0,
            Kind::Gel => 840.0,
            Kind::Glob => 700.0,
            Kind::Splash => 300.0,
            Kind::Chunk => 2500.0 + variation * 200.0 - 100.0,
            Kind::FlakShell => 1200.0,
            Kind::Razor | Kind::RazorAlt => 1300.0,
        });
        let lift = match kind {
            Kind::Grenade => 210.0,
            Kind::FlakShell => 200.0,
            Kind::Gel | Kind::Glob => 120.0,
            _ => 0.0,
        };
        let mut shot = Projectile {
            kind,
            direction,
            position: camera.position + offset,
            velocity: direction * speed + Vec3(0.0, 0.0, lift),
            born: time,
            expires: time
                + match kind {
                    Kind::Pulse => 2.0,
                    Kind::Grenade => 2.5 + variation * 0.5,
                    Kind::Rocket | Kind::FlakShell | Kind::Razor | Kind::RazorAlt => 6.0,
                    Kind::Chunk => 2.9,
                    _ => 12.0,
                },
            scale,
            stuck: None,
            next_trail: time,
            wall_hits: 0,
        };
        if let Some((point, normal)) = map.collision.ray_surface(camera.position, offset) {
            shot.position = point + normal * 0.5;
            if !matches!(
                kind,
                Kind::Grenade | Kind::Chunk | Kind::Razor | Kind::FlakShell
            ) && !kind.bio()
            {
                return explode(&shot, art, &mut self.effects, sounds, time);
            }
        }
        self.projectiles.push(shot);
        None
    }

    pub(super) fn update(
        &mut self,
        map: &Map,
        player: Option<Vec3>,
        time: f32,
        dt: f32,
        sounds: &mut Vec<Arc<Wave>>,
        blasts: &mut Vec<Blast>,
    ) {
        self.effects
            .retain(|effect| time - effect.born < effect.duration);
        let mut fragments = Vec::new();
        self.projectiles.retain_mut(|shot| {
            let kind = shot.kind;
            let art = &map.ordnance[kind as usize];
            if time >= shot.expires {
                if kind == Kind::Grenade || shot.stuck.is_some() {
                    blasts.extend(explode(shot, art, &mut self.effects, sounds, time));
                }
                return false;
            }
            if shot.stuck.is_some() {
                if kind.bio()
                    && player.is_some_and(|center| {
                        let d = center - shot.position;
                        d.0.hypot(d.1) < crate::collision::PLAYER_RADIUS + 7.0 * shot.scale
                            && d.2.abs() < 43.0
                            && map.collision.ray(shot.position, d).is_none()
                    })
                {
                    blasts.extend(explode(shot, art, &mut self.effects, sounds, time));
                    return false;
                }
                return true;
            }
            if kind == Kind::Rocket {
                let speed = (shot.velocity.dot(shot.velocity).sqrt() + 50.0 * dt).min(1600.0);
                shot.velocity = shot.direction * speed;
                if time >= shot.next_trail {
                    shot.next_trail = time + 0.035;
                    self.effects.push(Effect {
                        kind,
                        position: shot.position - shot.direction * 10.0,
                        born: time,
                        duration: 0.22,
                        scale: 0.5,
                        trail: true,
                    });
                }
            } else if matches!(kind, Kind::Grenade | Kind::FlakShell)
                || (kind == Kind::Chunk && shot.wall_hits > 0)
                || kind.bio()
            {
                shot.velocity =
                    shot.velocity + map.collision.regions.at(shot.position).gravity * dt;
            }
            let delta = shot.velocity * dt;
            let Some((point, normal)) = map.collision.ray_surface(shot.position, delta) else {
                shot.position = shot.position + delta;
                if shot.velocity.dot(shot.velocity) > 1.0 {
                    shot.direction = unit(shot.velocity);
                }
                return true;
            };
            shot.position = point + normal * 0.5;
            shot.wall_hits += 1;
            if kind == Kind::Grenade {
                let velocity = (shot.velocity - normal * (2.0 * shot.velocity.dot(normal))) * 0.75;
                if shot.velocity.dot(shot.velocity) > 10_000.0 {
                    sounds.extend(art.contact.iter().cloned());
                }
                shot.velocity = if velocity.2 > 400.0 {
                    Vec3(velocity.0, velocity.1, (400.0 + velocity.2) * 0.5)
                } else {
                    velocity
                };
                if normal.2 > 0.7 && shot.velocity.dot(shot.velocity) < 900.0 {
                    shot.velocity = Vec3::default();
                    shot.stuck = Some(time);
                }
                true
            } else if matches!(kind, Kind::Chunk | Kind::Razor) {
                if shot.wall_hits > 6 {
                    return false;
                }
                let retain = if kind == Kind::Chunk { 0.8 } else { 1.0 };
                shot.velocity =
                    (shot.velocity - normal * (2.0 * shot.velocity.dot(normal))) * retain;
                if shot.velocity.dot(shot.velocity) < 10_000.0 {
                    return false;
                }
                shot.direction = unit(shot.velocity);
                if shot.wall_hits == 1 {
                    sounds.extend(art.contact.iter().cloned());
                }
                true
            } else if kind.bio() {
                if kind == Kind::Glob {
                    let draw_scale = shot.scale * 2.0;
                    let (right, up) = super::shock::axes(normal);
                    let count = if draw_scale > 1.0 {
                        (draw_scale * 2.0) as usize - 1
                    } else {
                        0
                    };
                    for index in 0..count {
                        let angle = index as f32 * 2.399963;
                        let direction =
                            unit(normal + (right * angle.cos() + up * angle.sin()) * 0.75);
                        fragments.push(Projectile {
                            kind: Kind::Splash,
                            position: shot.position + direction * 5.0,
                            direction,
                            velocity: direction
                                * map.ordnance[Kind::Splash as usize].speed.unwrap_or(300.0)
                                + Vec3(0.0, 0.0, 120.0),
                            born: time,
                            expires: time + 12.0,
                            scale: 0.55,
                            stuck: None,
                            next_trail: time,
                            wall_hits: 0,
                        });
                    }
                    shot.scale = shot.scale.min(1.5);
                }
                shot.direction = normal;
                shot.stuck = Some(time);
                shot.expires = time + 3.0;
                sounds.extend(art.contact.iter().cloned());
                true
            } else {
                if kind == Kind::FlakShell {
                    let (right, up) = super::shock::axes(normal);
                    for index in 0..5 {
                        let angle = index as f32 * std::f32::consts::TAU / 5.0;
                        let direction =
                            unit(normal + (right * angle.cos() + up * angle.sin()) * 0.8);
                        fragments.push(Projectile {
                            kind: Kind::Chunk,
                            position: shot.position + direction * 2.0,
                            direction,
                            velocity: direction * 2500.0,
                            born: time,
                            expires: time + 2.9,
                            scale: 1.0,
                            stuck: None,
                            next_trail: time,
                            wall_hits: 0,
                        });
                    }
                }
                blasts.extend(explode(shot, art, &mut self.effects, sounds, time));
                false
            }
        });
        self.projectiles.extend(fragments);
    }

    pub fn visuals<'a>(&self, assets: &'a Arsenal, time: f32, mut draw: impl FnMut(Visual<'a>)) {
        for shot in &self.projectiles {
            let art = &assets[shot.kind as usize];
            if shot.kind == Kind::Pulse {
                if let Some(sprite) = &art.sprite {
                    draw(Visual::Sprite(
                        shot.position,
                        sprite.image(time - shot.born),
                        art.scale.unwrap_or(0.19),
                        1.0,
                    ));
                }
                continue;
            }
            let (motion, age) = if let Some(at) = shot.stuck.filter(|_| shot.kind.bio()) {
                (
                    &art.stuck,
                    (time - at).min((art.stuck.duration() - 0.001).max(0.0)),
                )
            } else {
                (&art.flight, time - shot.born)
            };
            let duration = motion.duration();
            if duration > 0.0
                && let Some(pose) = motion.sample(age.rem_euclid(duration))
            {
                draw(Visual::Mesh(
                    shot.position,
                    shot.direction,
                    pose,
                    shot.scale,
                ));
            }
        }
        for effect in &self.effects {
            let art = &assets[effect.kind as usize];
            let age = (time - effect.born) / effect.duration;
            let (texture, glow) = if effect.trail {
                (&art.trail, 1.0 - age)
            } else {
                (&art.explosion, 1.0)
            };
            if let Some(texture) = texture {
                draw(Visual::Sprite(
                    effect.position,
                    texture.image_once(age),
                    effect.scale,
                    glow,
                ));
            }
        }
    }

    #[cfg(test)]
    pub(super) fn active_count(&self) -> usize {
        self.projectiles.len()
    }
}

fn explode(
    shot: &Projectile,
    art: &Art,
    effects: &mut Vec<Effect>,
    sounds: &mut Vec<Arc<Wave>>,
    time: f32,
) -> Option<Blast> {
    sounds.extend(art.sound.iter().cloned());
    let kind = shot.kind;
    let (duration, scale) = match kind {
        Kind::Pulse => (0.5, 0.65),
        Kind::Rocket | Kind::Grenade => (0.7, 1.4),
        Kind::FlakShell => (0.5, 2.0),
        Kind::RazorAlt => (0.7, 1.0),
        _ => (1.0, shot.scale * 1.4),
    };
    effects.push(Effect {
        kind,
        position: shot.position,
        born: time,
        duration,
        scale,
        trail: false,
    });
    let (radius, damage, momentum) = match kind {
        Kind::Pulse | Kind::Chunk | Kind::Razor => return None,
        Kind::Rocket => (220.0, 75.0, 80_000.0),
        Kind::Grenade => (200.0, 80.0, 50_000.0),
        Kind::FlakShell => (150.0, 70.0, 75_000.0),
        Kind::RazorAlt => (180.0, 34.0, 87_000.0),
        Kind::Glob => ((150.0 * shot.scale).min(250.0), 75.0, 30_000.0),
        _ => ((150.0 * shot.scale).min(250.0), 20.0, 20_000.0),
    };
    let multiplier = if kind.bio() { shot.scale * 2.0 } else { 1.0 };
    Some(Blast::radial(
        shot.position,
        radius,
        art.damage.unwrap_or(damage) * multiplier,
        art.momentum.unwrap_or(momentum) * multiplier,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collision::Collision, package::Package};

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn stock_grenade_bounces_and_charged_bio_splits_on_impact() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        for kind in [
            Kind::Rocket,
            Kind::Grenade,
            Kind::Gel,
            Kind::Glob,
            Kind::Splash,
        ] {
            let art = &map.ordnance[kind as usize];
            assert!(
                art.flight
                    .frames
                    .iter()
                    .any(|faces| faces.iter().any(|face| face.texture.is_some())),
                "{} textured mesh",
                kind.class()
            );
            assert!(
                art.explosion.is_some() && art.sound.is_some(),
                "{} impact",
                kind.class()
            );
        }
        for kind in [Kind::Chunk, Kind::FlakShell, Kind::Razor, Kind::RazorAlt] {
            let art = &map.ordnance[kind as usize];
            assert!(
                art.flight
                    .frames
                    .iter()
                    .any(|faces| faces.iter().any(|face| face.texture.is_some())),
                "{} textured mesh",
                kind.class()
            );
        }
        let mut collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        collision.add_box(Vec3(-2000.0, -2000.0, -100.0), Vec3(2000.0, 2000.0, 0.0));
        collision.add_box(Vec3(190.0, -500.0, 0.0), Vec3(220.0, 500.0, 500.0));
        map.collision = collision;
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        let forward = camera.forward();
        let mut sounds = Vec::new();
        let mut blasts = Vec::new();
        let mut shots = Shots::default();
        shots.fire(
            &map,
            &camera,
            Kind::Grenade,
            forward,
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        for frame in 1..=42 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert!(shots.projectiles[0].velocity.0 < 0.0);
        assert!(!sounds.is_empty() && blasts.is_empty());
        for frame in 43..=255 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert_eq!(blasts.len(), 1);
        assert!(shots.projectiles.is_empty());

        shots.fire(
            &map,
            &camera,
            Kind::Glob,
            forward,
            0.0,
            1.6,
            3.0,
            0.0,
            &mut sounds,
        );
        for frame in 301..=335 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert!(
            shots
                .projectiles
                .iter()
                .any(|shot| shot.kind == Kind::Glob && shot.stuck.is_some())
        );
        assert!(
            shots
                .projectiles
                .iter()
                .any(|shot| shot.kind == Kind::Splash)
        );
        let mut shots = Shots::default();
        shots.fire(
            &map,
            &camera,
            Kind::Razor,
            forward,
            0.0,
            1.0,
            4.0,
            0.0,
            &mut sounds,
        );
        for frame in 401..=420 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert!(shots.projectiles[0].velocity.0 < 0.0);
        let mut shots = Shots::default();
        shots.fire(
            &map,
            &camera,
            Kind::FlakShell,
            forward,
            0.0,
            1.0,
            5.0,
            0.0,
            &mut sounds,
        );
        for frame in 501..=517 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert_eq!(
            shots
                .projectiles
                .iter()
                .filter(|shot| shot.kind == Kind::Chunk)
                .count(),
            5
        );
    }
}
