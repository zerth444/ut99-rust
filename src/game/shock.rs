use super::{Motion, Weapon};
use crate::{
    map::{Map, Vec3},
    mover::unit,
    render::Camera,
    sound::{BLAST_SOUND_RADIUS, CONTACT_SOUND_RADIUS, SoundEvents, Wave},
    texture::{Texture, TextureImage},
};
use std::sync::Arc;

pub struct Assets {
    pub speed: f32,
    pub lifetime: f32,
    pub damage: f32,
    pub momentum: f32,
    pub radius: f32,
    pub half_height: f32,
    pub fire_offset: Vec3,
    pub bolt: Option<Arc<Texture>>,
    pub bolt_scale: f32,
    pub hit_sound: Option<Arc<Wave>>,
    pub combo_sound: Option<Arc<Wave>>,
    pub beam: Option<BeamArt>,
    pub rings: [Option<RingArt>; 3],
    pub blast: Option<BlastArt>,
    pub wavefront: Option<WavefrontArt>,
}

impl Default for Assets {
    fn default() -> Self {
        Self {
            speed: 1000.0,
            lifetime: 10.0,
            damage: 55.0,
            momentum: 70_000.0,
            radius: 12.0,
            half_height: 12.0,
            fire_offset: Vec3(10.0, -5.0, -8.0),
            bolt: None,
            bolt_scale: 0.4,
            hit_sound: None,
            combo_sound: None,
            beam: None,
            rings: [None, None, None],
            blast: None,
            wavefront: None,
        }
    }
}

impl Assets {
    pub(super) fn beam_origin(&self, camera: &Camera) -> Vec3 {
        let direction = camera.look_direction();
        let right = camera.right();
        let Vec3(_, y, z) = self.fire_offset;
        camera.position + right * y + direction.cross(right) * z
    }
}

pub struct BeamArt {
    pub texture: Arc<Texture>,
    pub points: Vec<Vec3>,
    pub scale: f32,
    pub lifetime: f32,
}

pub struct RingArt {
    pub motion: Motion,
    pub lifetime: f32,
}

pub struct BlastArt {
    pub texture: Arc<Texture>,
    pub scale: f32,
    pub lifetime: f32,
    pub sound: Option<Arc<Wave>>,
}

pub struct WavefrontArt {
    pub faces: Vec<crate::map::Face>,
    pub lifetime: f32,
}

#[derive(Default)]
pub struct Shots {
    bolts: Vec<Bolt>,
    beams: Vec<Beam>,
    rings: Vec<Ring>,
}

pub(super) struct Blast {
    position: Vec3,
    radius: f32,
    damage: f32,
    momentum: f32,
}

impl Blast {
    pub(super) fn radial(position: Vec3, radius: f32, damage: f32, momentum: f32) -> Self {
        Self {
            position,
            radius,
            damage,
            momentum,
        }
    }
    fn new(position: Vec3, assets: &Assets, combo: bool) -> Self {
        Self {
            position,
            radius: if combo { 250.0 } else { 70.0 },
            damage: assets.damage * if combo { 3.0 } else { 1.0 },
            momentum: assets.momentum * if combo { 2.0 } else { 1.0 },
        }
    }

    pub(super) fn hit_player(
        &self,
        collision: &crate::collision::Collision,
        center: Vec3,
    ) -> Option<(u16, Vec3)> {
        self.hit_actor(collision, center, crate::collision::PLAYER_RADIUS)
    }

    pub(super) fn hit_actor(
        &self,
        collision: &crate::collision::Collision,
        center: Vec3,
        radius: f32,
    ) -> Option<(u16, Vec3)> {
        let offset = center - self.position;
        let distance = offset.dot(offset).sqrt();
        let scale = 1.0 - (distance.max(1.0) - radius).max(0.0) / self.radius;
        // Stop just short of the origin so the surface hit by the projectile does
        // not occlude the explosion on its own side of the wall.
        if scale <= 0.0
            || (distance > 0.2
                && collision
                    .ray(center, offset * (0.2 / distance - 1.0))
                    .is_some())
        {
            return None;
        }
        let direction = offset * distance.max(1.0).recip();
        Some((
            (scale * self.damage) as u16,
            direction * (scale * self.momentum),
        ))
    }
}

struct Bolt {
    owner: usize,
    position: Vec3,
    direction: Vec3,
    born: f32,
}

struct Beam {
    owner: usize,
    start: Vec3,
    end: Vec3,
    born: f32,
}

#[derive(Clone, Copy)]
enum Impact {
    Wall,
    Beam,
    Combo,
}

struct Ring {
    impact: Impact,
    position: Vec3,
    direction: Vec3,
    born: f32,
}

pub enum Visual<'a> {
    Sprite {
        position: Vec3,
        image: &'a TextureImage,
        scale: f32,
        glow: f32,
    },
    Ring {
        position: Vec3,
        direction: Vec3,
        pose: super::Pose<'a>,
        scale: f32,
        glow: f32,
    },
}

pub fn assets(map: &Map) -> Option<&Assets> {
    map.arsenal.get(&Weapon::Shock)?.shock.as_ref()
}

pub(crate) fn firing_rate(alternate: bool) -> f32 {
    // ShockRifle.PlayFiring and PlayAltFiring with FireAdjust = 1 for the player.
    if alternate { 0.8 } else { 0.6 }
}

pub(super) fn fallback_cooldown(alternate: bool) -> f32 {
    let sequence_rate = if alternate { 24.0 } else { 21.0 };
    10.0 / (sequence_rate * firing_rate(alternate))
}

pub(crate) fn axes(forward: Vec3) -> (Vec3, Vec3) {
    let flat = Vec3(-forward.1, forward.0, 0.0);
    let right = if flat.dot(flat) > 0.000001 {
        unit(flat)
    } else {
        Vec3(0.0, 1.0, 0.0)
    };
    (right, forward.cross(right))
}

impl Impact {
    fn art(self, assets: &Assets) -> Option<&RingArt> {
        assets.rings[self as usize].as_ref()
    }

    fn lifetime(self, assets: &Assets) -> f32 {
        let ring = self.art(assets).map_or(0.0, |art| art.lifetime);
        let blast = assets.blast.as_ref().map_or(0.0, |art| art.lifetime);
        let wave = match self {
            Self::Combo => assets.wavefront.as_ref().map_or(0.0, |art| art.lifetime),
            _ => 0.0,
        };
        ring.max(blast).max(wave)
    }
}

fn blast_sound(assets: &Assets) -> Option<Arc<Wave>> {
    assets.blast.as_ref()?.sound.clone()
}

#[derive(Clone, Copy)]
pub(super) enum Target {
    Pawn(usize),
    Projectile(usize),
    Translocator(usize),
    Objective(crate::assault::Target),
}

pub(super) trait Targets {
    fn trace(&self, owner: usize, start: Vec3, delta: Vec3) -> Option<(Target, f32)>;
    fn hit(
        &mut self,
        target: Target,
        owner: usize,
        damage: u16,
        direction: Vec3,
        sounds: &mut SoundEvents,
    ) -> Option<Blast>;
    fn local(&self, owner: usize) -> bool {
        owner == 0
    }
}

pub(super) fn nearest(
    pawn: Option<(Target, f32)>,
    projectile: Option<(Target, f32)>,
) -> Option<(Target, f32)> {
    projectile
        .filter(|&(_, at)| pawn.is_none_or(|(_, other)| at < other))
        .or(pawn)
}

struct PracticeTargets<'a> {
    map: &'a Map,
    pawn: Option<&'a mut super::TrainingTarget>,
    ordnance: &'a mut super::ordnance::Shots,
    now: f32,
}

impl Targets for PracticeTargets<'_> {
    fn trace(&self, _: usize, start: Vec3, delta: Vec3) -> Option<(Target, f32)> {
        nearest(
            self.pawn
                .as_ref()
                .and_then(|pawn| pawn.ray(start, delta))
                .map(|at| (Target::Pawn(0), at)),
            self.ordnance
                .trace_target(&self.map.ordnance, start, delta)
                .map(|(index, at)| (Target::Projectile(index), at)),
        )
    }

    fn hit(
        &mut self,
        target: Target,
        _: usize,
        damage: u16,
        _: Vec3,
        sounds: &mut SoundEvents,
    ) -> Option<Blast> {
        match target {
            Target::Pawn(_) => {
                if let Some(pawn) = &mut self.pawn {
                    pawn.damage(damage);
                }
                None
            }
            Target::Projectile(index) => {
                self.ordnance
                    .damage_target(&self.map.ordnance, index, damage, self.now, sounds)
            }
            Target::Objective(_) | Target::Translocator(_) => None,
        }
    }
}
impl Shots {
    pub(super) fn incoming(
        &self,
        speed: f32,
    ) -> impl Iterator<Item = (usize, Vec3, Vec3, f32, f32)> + '_ {
        self.bolts.iter().map(move |bolt| {
            (
                bolt.owner,
                bolt.position,
                bolt.direction * speed,
                bolt.born,
                70.0,
            )
        })
    }

    pub(super) fn deflect(&mut self, map: &Map, camera: &Camera) {
        for bolt in &mut self.bolts {
            let speed = assets(map).map_or(1000.0, |art| art.speed);
            if let Some(velocity) =
                super::hammer_deflection(map, camera, bolt.position, bolt.direction * speed)
            {
                bolt.direction = unit(velocity);
            }
        }
    }

    pub(super) fn fire(
        &mut self,
        map: &Map,
        camera: &Camera,
        alternate: bool,
        now: f32,
        ammo: &mut u16,
        sounds: &mut SoundEvents,
        pawn: Option<&mut super::TrainingTarget>,
        ordnance: &mut super::ordnance::Shots,
    ) -> Option<Blast> {
        let mut targets = PracticeTargets {
            map,
            pawn,
            ordnance,
            now,
        };
        let (blast, combo) = self.fire_at(map, camera, 0, alternate, now, &mut targets, sounds);
        if combo && *ammo >= 2 {
            *ammo -= 2;
        }
        blast
    }

    pub(super) fn fire_at(
        &mut self,
        map: &Map,
        camera: &Camera,
        owner: usize,
        alternate: bool,
        now: f32,
        targets: &mut impl Targets,
        sounds: &mut SoundEvents,
    ) -> (Option<Blast>, bool) {
        let fallback = Assets::default();
        let assets = assets(map).unwrap_or(&fallback);
        let direction = camera.look_direction();
        let x = assets.fire_offset.0;
        let start = assets.beam_origin(camera);
        if alternate {
            let muzzle = start + direction * x;
            if let Some((point, normal)) = map
                .collision
                .ray_surface(camera.position, muzzle - camera.position)
            {
                let blast = self.bolt_impact(assets, point, normal * 8.0, direction, now, sounds);
                return (Some(blast), false);
            }
            self.bolts.push(Bolt {
                owner,
                position: muzzle,
                direction,
                born: now,
            });
            return (None, false);
        }

        let ray = direction * 10_000.0;
        let (end, normal) = map
            .collision
            .ray_surface(start, ray)
            .unwrap_or((start + ray, direction * -1.0));
        let delta = end - start;
        let bolt_target = self
            .bolts
            .iter()
            .enumerate()
            .filter_map(|(index, bolt)| {
                let at = cylinder_hit(
                    start - bolt.position,
                    delta,
                    assets.radius,
                    assets.half_height,
                )?;
                (at < 1.0).then_some((index, at))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let target = targets.trace(owner, start, delta);
        let combo = bolt_target.filter(|&(_, at)| target.is_none_or(|(_, other)| at < other));
        let (beam_end, blast) = if let Some((index, at)) = combo {
            let bolt = self.bolts.swap_remove(index);
            self.rings.push(Ring {
                impact: Impact::Combo,
                position: bolt.position,
                direction,
                born: now,
            });
            sounds.extend_at(
                assets.combo_sound.iter().cloned(),
                bolt.position,
                BLAST_SOUND_RADIUS,
                1.0,
            );
            (
                start + delta * at,
                Some(Blast::new(bolt.position, assets, true)),
            )
        } else {
            let (end, normal, blob, ring) = match target {
                Some((target, at)) => {
                    let point = start + delta * at;
                    let blast = targets.hit(target, owner, 40, direction, sounds);
                    (
                        point,
                        direction * -1.0,
                        blast,
                        matches!(target, Target::Pawn(_)),
                    )
                }
                None => (end, normal, None, true),
            };
            if ring {
                self.rings.push(Ring {
                    impact: Impact::Beam,
                    position: end + normal * 8.0,
                    direction: normal,
                    born: now,
                });
            }
            (end, blob)
        };
        self.beams.push(Beam {
            owner,
            start: start + direction * (x + 20.0),
            end: beam_end,
            born: now,
        });
        if targets.local(owner) {
            sounds.extend(blast_sound(assets));
        } else {
            sounds.extend_at(blast_sound(assets), start, CONTACT_SOUND_RADIUS, 1.0);
        }
        (blast, combo.is_some())
    }

    pub(super) fn bolt_impact(
        &mut self,
        assets: &Assets,
        position: Vec3,
        offset: Vec3,
        direction: Vec3,
        now: f32,
        sounds: &mut SoundEvents,
    ) -> Blast {
        self.rings.push(Ring {
            impact: Impact::Wall,
            position: position + offset,
            direction,
            born: now,
        });
        sounds.extend_at(
            assets.hit_sound.iter().cloned(),
            position,
            CONTACT_SOUND_RADIUS,
            1.0,
        );
        sounds.extend_at(blast_sound(assets), position, BLAST_SOUND_RADIUS, 1.0);
        Blast::new(position, assets, false)
    }

    pub(super) fn update(
        &mut self,
        map: &Map,
        dt: f32,
        now: f32,
        sounds: &mut SoundEvents,
        pawn: Option<&mut super::TrainingTarget>,
        ordnance: &mut super::ordnance::Shots,
    ) -> Vec<Blast> {
        let mut targets = PracticeTargets {
            map,
            pawn,
            ordnance,
            now,
        };
        self.advance(map, dt, now, &mut targets, sounds)
            .into_iter()
            .map(|(_, blast)| blast)
            .collect()
    }

    pub(super) fn advance(
        &mut self,
        map: &Map,
        dt: f32,
        now: f32,
        targets: &mut impl Targets,
        sounds: &mut SoundEvents,
    ) -> Vec<(usize, Blast)> {
        let fallback = Assets::default();
        let assets = assets(map).unwrap_or(&fallback);
        self.rings
            .retain(|ring| now - ring.born < ring.impact.lifetime(assets));
        self.beams.retain(|beam| {
            assets.beam.as_ref().is_some_and(|art| {
                now - beam.born < art.lifetime + (beam.count().saturating_sub(1)) as f32 * 0.05
            })
        });
        let mut blasts = Vec::new();
        let mut index = 0;
        while index < self.bolts.len() {
            let bolt = &mut self.bolts[index];
            if now - bolt.born >= assets.lifetime {
                self.bolts.swap_remove(index);
                continue;
            }
            let delta = bolt.direction * (assets.speed * dt);
            let wall = map.collision.ray_surface(bolt.position, delta);
            let visible = wall.map_or(delta, |(point, _)| point - bolt.position);
            let impact = match targets.trace(bolt.owner, bolt.position, visible) {
                Some((target, at)) => {
                    let offset = if matches!(target, Target::Projectile(_)) {
                        blasts.extend(
                            targets
                                .hit(
                                    target,
                                    bolt.owner,
                                    assets.damage as u16,
                                    bolt.direction,
                                    sounds,
                                )
                                .map(|blast| (bolt.owner, blast)),
                        );
                        Vec3::default()
                    } else {
                        bolt.direction * -8.0
                    };
                    Some((bolt.position + visible * at, offset))
                }
                None => wall.map(|(point, normal)| (point, normal * 8.0)),
            };
            if let Some((position, offset)) = impact {
                let bolt = self.bolts.swap_remove(index);
                let blast = self.bolt_impact(assets, position, offset, bolt.direction, now, sounds);
                blasts.push((bolt.owner, blast));
            } else {
                bolt.position = bolt.position + delta;
                index += 1;
            }
        }
        blasts
    }
    pub fn visuals<'a>(
        &self,
        assets: &'a Assets,
        now: f32,
        local_muzzle: impl Fn(Vec3, Option<f32>) -> Vec3,
        mut draw: impl FnMut(Visual<'a>),
    ) {
        if let Some(texture) = &assets.bolt {
            for bolt in &self.bolts {
                draw(Visual::Sprite {
                    position: if bolt.owner == 0 {
                        local_muzzle(bolt.position, Some(now - bolt.born))
                    } else {
                        bolt.position
                    },
                    image: texture.image(now - bolt.born),
                    scale: assets.bolt_scale,
                    glow: 1.0,
                });
            }
        }
        if let Some(art) = &assets.beam {
            for beam in &self.beams {
                let start = if beam.owner == 0 {
                    local_muzzle(beam.start, None)
                } else {
                    beam.start
                };
                let delta = beam.end - start;
                let forward = unit(delta);
                let (right, up) = axes(forward);
                let count = beam.count();
                for index in 0..count {
                    let age = now - beam.born - index as f32 * 0.05;
                    if !(0.0..art.lifetime).contains(&age) {
                        continue;
                    }
                    let center = start + delta * (index as f32 / count as f32);
                    let (sin, cos) =
                        (age * (1_000_000.0 / 65_536.0) * std::f32::consts::TAU).sin_cos();
                    for &Vec3(x, y, z) in &art.points {
                        draw(Visual::Sprite {
                            position: center
                                + forward * x
                                + right * (y * cos - z * sin)
                                + up * (y * sin + z * cos),
                            image: art.texture.image(age),
                            scale: art.scale,
                            glow: 1.0 - age / art.lifetime,
                        });
                    }
                }
            }
        }
        for ring in &self.rings {
            let age = now - ring.born;
            if let Some(art) = ring.impact.art(assets)
                && age < art.lifetime
            {
                let last_frame = (art.motion.frames.len() - 1) as f32 / art.motion.rate;
                if let Some(pose) = art.motion.sample(age.min(last_frame)) {
                    draw(Visual::Ring {
                        position: ring.position,
                        direction: ring.direction,
                        pose,
                        scale: 1.0,
                        glow: (1.0 - age / art.lifetime) * 0.7,
                    });
                }
            }
            if let Some(art) = &assets.blast
                && age < art.lifetime
            {
                draw(Visual::Sprite {
                    position: ring.position,
                    image: art.texture.image_once(age / art.lifetime),
                    scale: art.scale,
                    glow: 1.0,
                });
            }
            if matches!(ring.impact, Impact::Combo)
                && let Some(art) = &assets.wavefront
                && age < art.lifetime
            {
                let glow = 1.0 - age / art.lifetime;
                draw(Visual::Ring {
                    position: ring.position,
                    direction: Vec3(1.0, 0.0, 0.0),
                    pose: super::Pose {
                        current: &art.faces,
                        next: &art.faces,
                        fraction: 0.0,
                    },
                    scale: 0.7 / (glow + 0.05),
                    glow,
                });
            }
        }
    }
}

impl Beam {
    fn count(&self) -> usize {
        let delta = self.end - self.start;
        (delta.dot(delta).sqrt() / 135.0) as usize
    }
}

pub(super) fn cylinder_hit(start: Vec3, delta: Vec3, radius: f32, half_height: f32) -> Option<f32> {
    let a = delta.0 * delta.0 + delta.1 * delta.1;
    let b = start.0 * delta.0 + start.1 * delta.1;
    let c = start.0 * start.0 + start.1 * start.1 - radius * radius;
    let (mut enter, mut exit) = (0.0_f32, 1.0_f32);
    if a > 0.0 {
        let discriminant = b * b - a * c;
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        enter = enter.max((-b - root) / a);
        exit = exit.min((-b + root) / a);
    } else if c > 0.0 {
        return None;
    }
    if delta.2 != 0.0 {
        let bottom = (-half_height - start.2) / delta.2;
        let top = (half_height - start.2) / delta.2;
        enter = enter.max(bottom.min(top));
        exit = exit.min(bottom.max(top));
    } else if start.2.abs() > half_height {
        return None;
    }
    (enter <= exit).then_some(enter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collision::Walker,
        game::{Input, Practice, WeaponState},
        package::Package,
        render::Frame,
    };

    #[test]
    #[ignore = "requires the local GOTY Deck16 map, models, textures and sounds"]
    fn deck16_shock_combo_uses_original_assets_and_respects_walls() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let art = assets(&map).unwrap();
        assert_eq!((art.speed, art.damage, art.lifetime), (1000.0, 55.0, 10.0));
        assert_eq!((art.radius, art.half_height), (12.0, 12.0));
        assert_eq!(art.momentum, 70_000.0);
        assert_eq!(map.collision.profile.mass, 100.0);
        assert!(art.bolt.is_some());
        let beam = art.beam.as_ref().expect("ShockBeam particle art");
        assert!(!beam.points.is_empty());
        assert!(art.hit_sound.is_some() && art.combo_sound.is_some());
        let blast = art.blast.as_ref().expect("ShockExplo sprite");
        assert!(blast.sound.is_some());
        assert!(!std::ptr::eq(
            blast.texture.image_once(0.0),
            blast.texture.image_once(0.5)
        ));
        let wavefront = art.wavefront.as_ref().expect("ShockrifleWave mesh");
        assert!(wavefront.faces.iter().any(|face| face.texture.is_some()));
        assert!(art.rings.iter().flatten().all(|ring| {
            ring.motion.frames.len() == 9
                && ring.motion.frames.iter().all(|frame| {
                    !frame.is_empty() && frame.iter().all(|face| face.texture.is_some())
                })
        }));
        assert!(art.rings.iter().all(Option::is_some));
        let combo_sound = Arc::clone(art.combo_sound.as_ref().unwrap());

        // Find a clear firing lane, using the stock BSP and the normal player eye height.
        let camera = map
            .starts
            .iter()
            .flat_map(|start| {
                let walker = Walker::spawn(&map.collision, start.position);
                let profile = map.collision.profile;
                (0..16).map(move |direction| {
                    let yaw = direction as f32 * std::f32::consts::TAU / 16.0;
                    let mut camera = Camera {
                        position: start.position,
                        yaw,
                        pitch: 0.0,
                    };
                    camera.position = walker.eye(profile, camera.right());
                    camera
                })
            })
            .find(|camera| {
                let origin = camera.position + camera.right() * -5.0 - Vec3(0.0, 0.0, 8.0);
                map.collision
                    .ray(origin, camera.forward() * 700.0)
                    .is_none()
            })
            .expect("long firing lane on Deck16");
        let input = |primary, alternate| Input {
            primary,
            alternate,
            slot: None,
            cycle: 0,
        };
        let mut game = Practice::new(&map);
        game.weapon = Weapon::Shock;
        game.weapon_state = WeaponState::Ready;
        game.ammo[Weapon::Shock.ammo_pool()] = 20;
        game.update(&mut map, &camera, 0.01, input(false, true), false, None);
        assert_eq!(game.ammo(), Some(19));
        for _ in 0..52 {
            game.update(&mut map, &camera, 0.01, input(false, false), false, None);
        }
        assert_eq!(game.shock.bolts.len(), 1);
        let mut bolt_frame = Frame::new();
        bolt_frame.draw_practice(&map, &camera, game.time, &game);
        let effects = std::mem::take(&mut game.shock);
        let mut without_bolt = Frame::new();
        without_bolt.draw_practice(&map, &camera, game.time, &game);
        assert_ne!(bolt_frame.pixels, without_bolt.pixels);
        game.shock = effects;
        let sounds = game.update(&mut map, &camera, 0.01, input(true, false), false, None);
        assert!(game.shock.bolts.is_empty());
        assert_eq!(game.ammo(), Some(16));
        assert!(matches!(game.shock.rings[0].impact, Impact::Combo));
        let sound_offset = sounds.position_of(&combo_sound).unwrap() - game.shock.rings[0].position;
        assert!(sound_offset.dot(sound_offset) < 0.001);
        game.update(&mut map, &camera, 0.12, input(false, false), false, None);
        let mut combo_frame = Frame::new();
        combo_frame.draw_practice(&map, &camera, game.time, &game);
        let effects = std::mem::take(&mut game.shock);
        let mut without_combo = Frame::new();
        without_combo.draw_practice(&map, &camera, game.time, &game);
        assert_ne!(combo_frame.pixels, without_combo.pixels);
        game.shock = effects;

        let direction = camera.look_direction();
        let start = camera.position + camera.right() * -5.0 - Vec3(0.0, 0.0, 8.0);
        let wall = map.collision.ray(start, direction * 10_000.0).unwrap();
        game.shock.bolts.push(Bolt {
            owner: 0,
            position: wall + direction * 32.0,
            direction,
            born: game.time,
        });
        let mut ammo = 2;
        let mut sounds = SoundEvents::default();
        assert!(
            game.shock
                .fire(
                    &map,
                    &camera,
                    false,
                    game.time,
                    &mut ammo,
                    &mut sounds,
                    None,
                    &mut game.ordnance
                )
                .is_none()
        );
        assert_eq!(game.shock.bolts.len(), 1);
        assert_eq!(ammo, 2);
        assert!(!sounds.waves().any(|sound| Arc::ptr_eq(sound, &combo_sound)));
        assert!(matches!(
            game.shock.rings.last().unwrap().impact,
            Impact::Beam
        ));

        // UseAmmo(2) fails without blocking the original SuperExplosion call.
        game.shock.bolts[0].position = start + direction * 180.0;
        ammo = 1;
        let blast = game
            .shock
            .fire(
                &map,
                &camera,
                false,
                game.time,
                &mut ammo,
                &mut sounds,
                None,
                &mut game.ordnance,
            )
            .unwrap();
        assert!(game.shock.bolts.is_empty());
        assert_eq!(ammo, 1);
        let center = camera.position - Vec3(0.0, 0.0, map.collision.profile.eye_height);
        let (damage, momentum) = blast.hit_player(&map.collision, center).unwrap();
        assert!(damage > 0 && momentum.dot(direction) < 0.0);
        let assets = assets(&map).unwrap();
        let close = center + direction * 48.0;
        let (plasma_damage, plasma_momentum) = Blast::new(close, assets, false)
            .hit_player(&map.collision, center)
            .unwrap();
        let (combo_damage, combo_momentum) = Blast::new(close, assets, true)
            .hit_player(&map.collision, center)
            .unwrap();
        assert!(combo_damage > plasma_damage);
        assert!(combo_momentum.dot(direction) < 2.0 * plasma_momentum.dot(direction));

        let mut walker = Walker::spawn(&map.collision, map.starts[6].position);
        assert!(walker.grounded);
        let origin = walker.position;
        let mut aim = Camera {
            position: origin,
            yaw: map.starts[6].yaw,
            pitch: -1.4,
        };
        aim.position = walker.eye(map.collision.profile, aim.right());
        game.next_shot = game.time;
        game.update(
            &mut map,
            &aim,
            0.01,
            input(false, true),
            false,
            Some(&mut walker),
        );
        for _ in 0..15 {
            game.update(
                &mut map,
                &aim,
                0.01,
                input(false, false),
                false,
                Some(&mut walker),
            );
        }
        assert!(
            game.health < 100,
            "a nearby plasma impact should deal splash damage"
        );
        assert!(
            !walker.grounded,
            "grounded player should enter falling physics"
        );
        walker.advance(&map.collision, Vec3::default(), None, 0.05);
        assert!(
            walker.position.2 > origin.2 + 3.0,
            "plasma impact should launch the player"
        );

        let near_wall = blast.position - direction * 90.0;
        let half = Vec3(10.0, 10.0, 32.0);
        map.collision.add_box(near_wall - half, near_wall + half);
        assert!(
            blast.hit_player(&map.collision, center).is_none(),
            "a solid wall inside the combo radius should shield the player"
        );
    }
}
