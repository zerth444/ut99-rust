use std::sync::Arc;

use super::{Motion, Pose, TrainingTarget, shock::Blast};
use crate::{
    map::{Face, Map, Vec3},
    mover::unit,
    render::Camera,
    sound::{BLAST_SOUND_RADIUS, CONTACT_SOUND_RADIUS, SoundEvents, Wave},
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
    Warhead,
    GuidedWarhead,
}

impl Kind {
    pub const ALL: [Self; 12] = [
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
        Self::Warhead,
        Self::GuidedWarhead,
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
            Self::Warhead => "WarShell",
            Self::GuidedWarhead => "GuidedWarShell",
        }
    }

    pub(super) fn launch_offset(self, camera: &Camera, side: f32, variation: f32) -> Vec3 {
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        match self {
            Kind::Pulse => {
                let (sin, cos) = variation.sin_cos();
                forward * 16.5 + right * (15.0 - 4.0 * sin) + up * (4.0 * cos - 10.78)
            }
            Kind::Rocket | Kind::Grenade => forward * 20.0 + right * (8.0 + side) - up * 9.0,
            Kind::FlakShell | Kind::Chunk => forward * 20.0 + right * (11.0 + side) - up * 15.0,
            Kind::Razor | Kind::RazorAlt => forward * 20.0 + right * 15.0 - up * 13.0,
            Kind::Warhead | Kind::GuidedWarhead => forward * 19.8 + right - up * 11.89,
            Kind::Gel | Kind::Glob | Kind::Splash => forward * 13.7 + right * 11.85 - up * 6.95,
        }
    }

    pub(super) fn launch_speed(self, art: &Art, variation: f32) -> f32 {
        art.speed
            .filter(|speed| speed.is_finite() && *speed > 0.0)
            .unwrap_or(match self {
                Kind::Pulse => 1450.0,
                Kind::Rocket => 900.0,
                Kind::Grenade => 600.0,
                Kind::Gel => 840.0,
                Kind::Glob => 700.0,
                Kind::Splash => 300.0,
                Kind::Chunk => 2500.0 + variation * 200.0 - 100.0,
                Kind::FlakShell => 1200.0,
                Kind::Razor | Kind::RazorAlt => 1300.0,
                Kind::Warhead | Kind::GuidedWarhead => 600.0,
            })
    }

    pub(crate) fn bio(self) -> bool {
        matches!(self, Self::Gel | Self::Glob | Self::Splash)
    }

    pub(crate) fn warhead(self) -> bool {
        matches!(self, Self::Warhead | Self::GuidedWarhead)
    }
}

pub(super) const FLAK_CHUNK_LIFETIME: f32 = 2.9;
pub(super) const FLAK_CHUNK_REBOUNDS: u8 = 6;
pub(super) const FLAK_SHELL_BLAST: (f32, f32, f32) = (150.0, 70.0, 75_000.0);

pub(super) fn flak_pattern(camera: &Camera, mut jitter: impl FnMut() -> f32) -> [(Vec3, f32); 8] {
    let forward = camera.look_direction();
    let right = camera.right();
    let up = forward.cross(right);
    std::array::from_fn(|_| {
        let direction = unit(forward + right * (jitter() * 0.11) + up * (jitter() * 0.11));
        (direction, (jitter() + 1.0) * 0.5)
    })
}

pub(super) fn flak_fragments(normal: Vec3) -> [Vec3; 5] {
    let (right, up) = super::shock::axes(normal);
    std::array::from_fn(|index| {
        let angle = index as f32 * std::f32::consts::TAU / 5.0;
        unit(normal + (right * angle.cos() + up * angle.sin()) * 0.8)
    })
}

pub(super) fn ricochet(velocity: Vec3, normal: Vec3, retain: f32) -> Vec3 {
    (velocity - normal * (2.0 * velocity.dot(normal))) * retain
}

pub(super) struct FlightSegment {
    pub kind: Kind,
    pub start: Vec3,
    pub velocity: Vec3,
}

pub(super) fn intercept_fraction(
    map: &Map,
    source: FlightSegment,
    target: FlightSegment,
    dt: f32,
) -> Option<f32> {
    let art = &map.ordnance[target.kind as usize];
    let source_art = &map.ordnance[source.kind as usize];
    let fraction = super::shock::cylinder_hit(
        source.start - target.start,
        (source.velocity - target.velocity) * dt,
        art.collision_radius + source_art.collision_radius,
        art.collision_height + source_art.collision_height,
    )?;
    if map
        .collision
        .ray(source.start, source.velocity * (dt * fraction))
        .is_some()
        || map
            .collision
            .projectile_surface(
                target.start,
                target.velocity * (dt * fraction),
                art.collision_radius,
                art.collision_height,
            )
            .is_some()
    {
        return None;
    }
    Some(fraction)
}

#[derive(Default)]
pub struct Art {
    pub flight: Motion,
    pub stuck: Motion,
    pub dripping: Motion,
    pub sliding: Motion,
    pub sprite: Option<Arc<Texture>>,
    pub explosion: Option<Arc<Texture>>,
    pub trail: Option<Arc<Texture>>,
    pub sound: Option<Arc<Wave>>,
    pub contact: Option<Arc<Wave>>,
    pub ambience: Option<(Arc<Wave>, f32, f32)>,
    pub lifetime: Option<f32>,
    pub speed: Option<f32>,
    pub damage: Option<f32>,
    pub damage_type: Option<String>,
    pub momentum: Option<f32>,
    pub scale: Option<f32>,
    pub wavefront: Vec<Face>,
    pub second_explosion: Option<Arc<Texture>>,
    pub smoke: [Option<Arc<Texture>>; 4],
    pub smoke_offsets: Vec<Vec3>,
    pub scorch: Option<(Arc<Texture>, f32, f32)>,
    pub collision_radius: f32,
    pub collision_height: f32,
    pub water: Motion,
    pub water_lifetime: f32,
}

pub type Arsenal = [Art; Kind::ALL.len()];

pub enum Visual<'a> {
    Sprite(Vec3, &'a TextureImage, f32, f32),
    Mesh(Vec3, Vec3, Pose<'a>, f32, Option<f32>),
}

#[derive(Default)]
pub struct RocketTrails {
    flares: Vec<(Vec3, f32)>,
    smoke: Vec<(Vec3, f32)>,
}

impl RocketTrails {
    const LIFETIME: f32 = 0.22;
    const FLARE_INTERVAL: f32 = 0.035;
    const SMOKE_INTERVAL: f32 = 0.16;
    const SMOKE_LIFETIME: f32 = 1.5;

    pub(super) fn emit(
        &mut self,
        map: &Map,
        position: Vec3,
        direction: Vec3,
        time: f32,
        born: f32,
        next: &mut f32,
    ) {
        if time >= *next && !map.collision.regions.at(position).water {
            let position = position - direction * 10.0;
            let smoke_step = |at: f32| ((at - born).max(0.0) / Self::SMOKE_INTERVAL) as u32;
            if smoke_step(time) > smoke_step(*next - Self::FLARE_INTERVAL) {
                self.smoke.push((position, time));
            }
            self.flares.push((position, time));
            *next = time + Self::FLARE_INTERVAL;
        }
    }

    pub(super) fn expire(&mut self, time: f32) {
        self.flares
            .retain(|&(_, born)| time - born < Self::LIFETIME);
        self.smoke
            .retain(|&(_, born)| time - born < Self::SMOKE_LIFETIME);
    }

    pub fn visuals<'a>(&self, assets: &'a Arsenal, time: f32, mut draw: impl FnMut(Visual<'a>)) {
        let art = &assets[Kind::Rocket as usize];
        for &(position, born) in &self.smoke {
            let age = time - born;
            let index = ((position.0.to_bits() ^ position.1.to_bits()).rotate_left(9)
                ^ born.to_bits()) as usize
                % art.smoke.len();
            if let Some(texture) = &art.smoke[index] {
                draw(Visual::Sprite(
                    position + Vec3(0.0, 0.0, 50.0 * age),
                    texture.image_once((age / 0.4).min(1.0)),
                    2.0,
                    0.4 * (1.0 - age / Self::SMOKE_LIFETIME),
                ));
            }
        }
        if let Some(texture) = &art.trail {
            for &(position, born) in &self.flares {
                let age = ((time - born) / Self::LIFETIME).clamp(0.0, 1.0);
                draw(Visual::Sprite(
                    position,
                    texture.image_once(age),
                    0.5,
                    1.0 - age,
                ));
            }
        }
    }
}

#[derive(Default)]
pub struct WarheadSmoke(Vec<WarheadPuff>);

struct WarheadPuff {
    kind: Kind,
    position: Vec3,
    born: f32,
    duration: f32,
}

impl WarheadSmoke {
    pub(super) fn emit(
        &mut self,
        map: &Map,
        kind: Kind,
        position: Vec3,
        direction: Vec3,
        time: f32,
        next: &mut f32,
    ) {
        if time < *next {
            return;
        }
        let art = &map.ordnance[kind as usize];
        *next = time
            + if kind == Kind::Warhead {
                152.0 / art.speed.unwrap_or(600.0).max(1.0)
            } else {
                0.02
            };
        if !map.collision.regions.at(position).water {
            let count = if kind == Kind::Warhead {
                art.smoke_offsets.len()
            } else {
                1
            };
            let (right, up) = super::shock::axes(direction);
            for i in 0..count {
                let offset = art.smoke_offsets.get(i).copied().unwrap_or_default();
                let particle = if kind == Kind::Warhead {
                    position + direction * offset.0 + right * offset.1 + up * offset.2
                } else {
                    position
                };
                if map.collision.ray(position, particle - position).is_some() {
                    break;
                }
                self.0.push(WarheadPuff {
                    kind,
                    position: particle,
                    born: time,
                    duration: if kind == Kind::Warhead {
                        1.4 + i as f32 * 0.025
                    } else {
                        1.5
                    },
                });
            }
        }
    }

    pub(super) fn expire(&mut self, time: f32) {
        self.0.retain(|effect| time - effect.born < effect.duration);
    }

    pub fn visuals<'a>(&self, assets: &'a Arsenal, time: f32, mut draw: impl FnMut(Visual<'a>)) {
        for effect in &self.0 {
            let art = &assets[effect.kind as usize];
            let age = (time - effect.born) / effect.duration;
            let seed = (effect.position.0.to_bits() ^ effect.position.1.to_bits()).rotate_left(9);
            let index = ((effect.born * 1000.0) as usize ^ seed as usize) % art.smoke.len();
            if let Some(texture) = &art.smoke[index] {
                draw(Visual::Sprite(
                    effect.position + Vec3(0.0, 0.0, 50.0 * (time - effect.born)),
                    texture.image_once(if effect.kind == Kind::Warhead {
                        (seed % 1000) as f32 / 1000.0
                    } else {
                        age
                    }),
                    2.0,
                    if effect.kind == Kind::Warhead {
                        0.8
                    } else {
                        0.4
                    },
                ));
            }
        }
    }
}

impl Art {
    pub fn warhead_wave_visuals<'a>(
        &'a self,
        position: Vec3,
        elapsed: f32,
        mut draw: impl FnMut(Visual<'a>),
    ) {
        if !self.wavefront.is_empty() {
            let pose = Pose {
                current: &self.wavefront,
                next: &self.wavefront,
                fraction: 0.0,
            };
            draw(Visual::Mesh(
                position,
                Vec3(1.0, 0.0, 0.0),
                pose,
                shock_size(elapsed),
                Some((WARHEAD_WAVE_LIFETIME - elapsed).clamp(0.0, 1.0)),
            ));
        }
        if elapsed < 1.0
            && let Some(texture) = &self.explosion
        {
            draw(Visual::Sprite(
                position,
                texture.image_once(elapsed),
                2.8,
                1.0,
            ));
        }
        if (0.5..WARHEAD_WAVE_LIFETIME).contains(&elapsed)
            && let Some(texture) = &self.second_explosion
        {
            draw(Visual::Sprite(
                position,
                texture.image_once(elapsed - 0.5),
                3.2,
                1.0,
            ));
        }
    }

    pub fn impact_sprite(&self, position: Vec3, progress: f32, scale: f32) -> Option<Visual<'_>> {
        Some(Visual::Sprite(
            position,
            self.explosion.as_ref()?.image_once(progress),
            scale,
            1.0,
        ))
    }

    pub fn flight_visual(
        &self,
        kind: Kind,
        position: Vec3,
        direction: Vec3,
        age: f32,
        scale: f32,
    ) -> Option<Visual<'_>> {
        if kind == Kind::Pulse {
            return Some(Visual::Sprite(
                position,
                self.sprite.as_ref()?.image(age),
                self.scale.unwrap_or(0.19),
                1.0,
            ));
        }
        Some(Visual::Mesh(
            position,
            direction,
            self.flight.sample_loop(age)?,
            scale,
            None,
        ))
    }

    pub fn bio_surface_visual(
        &self,
        position: Vec3,
        normal: Vec3,
        elapsed: f32,
        scale: f32,
    ) -> Option<Visual<'_>> {
        let surface = if normal.2 < -0.7 {
            &self.dripping
        } else if normal.2 < 0.5 {
            &self.sliding
        } else {
            &self.stuck
        };
        let after_hit = elapsed - self.stuck.duration();
        let pose = if after_hit >= 0.0 && surface.duration() > 0.0 {
            surface.hold_pose(after_hit)
        } else {
            self.stuck.hold_pose(elapsed)
        }?;
        Some(Visual::Mesh(position, normal, pose, scale, None))
    }
}

pub(super) const BIO_SURFACE_LIFETIME: f32 = 3.8;

pub(super) fn bio_splash_count(scale: f32) -> u8 {
    if scale > 0.5 {
        ((scale * 4.0) as u8).saturating_sub(1)
    } else {
        0
    }
}

pub(super) fn bio_trigger_delay(born: f32) -> f32 {
    0.1 + 0.2 * ((born * 997.0).sin() * 0.5 + 0.5)
}

#[derive(Default)]
pub struct Shots {
    projectiles: Vec<Projectile>,
    effects: Vec<Effect>,
    ripples: Vec<Ripple>,
    self_hits: Vec<(u16, Vec3)>,
    marks: Vec<(Kind, Vec3, Vec3, Option<usize>)>,
    rocket_trails: RocketTrails,
    warhead_smoke: WarheadSmoke,
    pub guided_roll: f32,
}

#[derive(Clone, Copy)]
struct Projectile {
    kind: Kind,
    position: Vec3,
    velocity: Vec3,
    direction: Vec3,
    born: f32,
    expires: f32,
    scale: f32,
    stuck: Option<f32>,
    base: Option<(usize, Vec3)>,
    next_trail: f32,
    wall_hits: u8,
    seeking: Option<(Vec3, f32)>,
    hit_water: bool,
    spray: Option<(u8, Vec3)>,
}

struct Ripple {
    kind: Kind,
    position: Vec3,
    born: f32,
    scale: f32,
}

struct Effect {
    kind: Kind,
    position: Vec3,
    born: f32,
    duration: f32,
    scale: f32,
    trail: bool,
    nuclear: bool,
    struck: [bool; 2],
}

impl Shots {
    pub(super) fn take_self_hits(&mut self) -> Vec<(u16, Vec3)> {
        std::mem::take(&mut self.self_hits)
    }

    pub(super) fn follow_movers(&mut self, movers: &[crate::mover::Mover]) {
        for shot in &mut self.projectiles {
            if let Some((index, local)) = shot.base
                && let Some(mover) = movers.get(index)
            {
                shot.position = local + mover.offset;
            }
        }
    }

    pub(super) fn damage_blasts(
        &mut self,
        map: &Map,
        blasts: &mut Vec<Blast>,
        time: f32,
        sounds: &mut SoundEvents,
    ) {
        let fronts: Vec<_> = self
            .effects
            .iter()
            .filter(|effect| effect.nuclear)
            .map(|effect| (effect.position, ((time - effect.born) * 10.0).floor() * 0.1))
            .collect();
        for (origin, age) in fronts {
            for index in (0..self.projectiles.len()).rev() {
                let shot = &self.projectiles[index];
                if (shot.kind.bio() || shot.kind.warhead())
                    && let Some((damage, _)) = wave_hit(map, origin, shot.position, age)
                {
                    blasts.extend(self.damage_target(&map.ordnance, index, damage, time, sounds));
                }
            }
        }
        let mut cursor = 0;
        while cursor < blasts.len() {
            for index in (0..self.projectiles.len()).rev() {
                let shot = &self.projectiles[index];
                if !shot.kind.bio() && !shot.kind.warhead() {
                    continue;
                }
                let art = &map.ordnance[shot.kind as usize];
                let Some((damage, _)) =
                    blasts[cursor].hit_actor(&map.collision, shot.position, art.collision_radius)
                else {
                    continue;
                };
                blasts.extend(self.damage_target(&map.ordnance, index, damage, time, sounds));
            }
            cursor += 1;
        }
    }

    pub(super) fn trace_target(
        &self,
        assets: &Arsenal,
        start: Vec3,
        delta: Vec3,
    ) -> Option<(usize, f32)> {
        self.projectiles
            .iter()
            .enumerate()
            .filter(|(_, shot)| shot.kind.warhead() || shot.kind.bio())
            .filter_map(|(index, shot)| {
                let art = &assets[shot.kind as usize];
                super::shock::cylinder_hit(
                    start - shot.position,
                    delta,
                    art.collision_radius,
                    art.collision_height,
                )
                .map(|fraction| (index, fraction))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    pub(super) fn damage_target(
        &mut self,
        assets: &Arsenal,
        index: usize,
        damage: u16,
        time: f32,
        sounds: &mut SoundEvents,
    ) -> Option<Blast> {
        let shot = &mut self.projectiles[index];
        if shot.kind.bio() {
            let delay = bio_trigger_delay(shot.born);
            shot.expires = shot.expires.min(time + delay);
            return None;
        }
        if damage <= 5 {
            return None;
        }
        let shot = self.projectiles.remove(index);
        Some(shoot_down(
            &shot,
            &assets[shot.kind as usize],
            &mut self.effects,
            time,
            sounds,
        ))
    }

    fn intercept_projectiles(
        &mut self,
        map: &Map,
        pawn: Option<&TrainingTarget>,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
        blasts: &mut Vec<Blast>,
    ) {
        let mut impacts = Vec::new();
        for (source_index, source) in
            self.projectiles.iter().enumerate().filter(|(_, shot)| {
                !shot.kind.warhead() && shot.stuck.is_none() && shot.expires > time
            })
        {
            for (target_index, target) in self
                .projectiles
                .iter()
                .enumerate()
                .filter(|(_, shot)| shot.kind.warhead() && shot.expires > time)
            {
                let Some(fraction) = intercept_fraction(
                    map,
                    FlightSegment {
                        kind: source.kind,
                        start: source.position,
                        velocity: source.velocity,
                    },
                    FlightSegment {
                        kind: target.kind,
                        start: target.position,
                        velocity: target.velocity,
                    },
                    dt,
                ) else {
                    continue;
                };
                let travel = source.velocity * (dt * fraction);
                if pawn.is_some_and(|pawn| pawn.ray(source.position, travel).is_some()) {
                    continue;
                }
                impacts.push((source_index, target_index, fraction));
            }
        }
        if impacts.is_empty() {
            return;
        }
        impacts.sort_unstable_by(|a, b| a.2.total_cmp(&b.2));
        let mut removed = vec![false; self.projectiles.len()];
        for (source_index, target_index, fraction) in impacts {
            if removed[source_index] || removed[target_index] {
                continue;
            }
            let mut source = self.projectiles[source_index];
            let mut target = self.projectiles[target_index];
            source.position = source.position + source.velocity * (dt * fraction);
            target.position = target.position + target.velocity * (dt * fraction);
            blasts.extend(explode(
                &source,
                &map.ordnance[source.kind as usize],
                &mut self.effects,
                sounds,
                time,
            ));
            blasts.push(shoot_down(
                &target,
                &map.ordnance[target.kind as usize],
                &mut self.effects,
                time,
                sounds,
            ));
            removed[source_index] = true;
            removed[target_index] = true;
        }
        let mut index = 0;
        self.projectiles.retain(|_| {
            let keep = !removed[index];
            index += 1;
            keep
        });
    }

    pub(super) fn project_marks(&mut self, map: &Map, marks: &mut super::decals::Marks, time: f32) {
        for (kind, point, normal, mover) in self.marks.drain(..) {
            if let Some((texture, scale, duration)) = &map.ordnance[kind as usize].scorch {
                if let Some(index) = mover {
                    marks.project_mover(
                        &map.movers,
                        index,
                        point,
                        normal,
                        texture,
                        *scale,
                        time + duration,
                    );
                } else {
                    marks.project(&map.faces, point, normal, texture, *scale, time + duration);
                }
            }
        }
    }

    pub fn nuclear_shake(&self, eye: Vec3, time: f32) -> (f32, f32) {
        nuclear_shake(
            eye,
            time,
            self.effects
                .iter()
                .filter(|effect| effect.nuclear)
                .map(|effect| (effect.position, effect.born)),
        )
    }

    pub(super) fn nuclear_wave(
        &mut self,
        map: &Map,
        time: f32,
        center: Vec3,
        victim: usize,
    ) -> Option<(u16, Vec3)> {
        let mut hit: Option<(u16, Vec3)> = None;
        for effect in &mut self.effects {
            if !effect.nuclear || effect.struck[victim] {
                continue;
            }
            let age = ((time - effect.born) * 10.0).floor() * 0.1;
            let Some((force, impulse)) = wave_hit(map, effect.position, center, age) else {
                continue;
            };
            effect.struck[victim] = true;
            let (damage, velocity) = hit.get_or_insert((0u16, Vec3::default()));
            *damage = damage.saturating_add(force);
            *velocity = *velocity + impulse;
        }
        hit
    }
    pub(super) fn clear_seeking(&mut self) {
        for shot in &mut self.projectiles {
            shot.seeking = None;
        }
    }

    pub(super) fn seek_latest(&mut self, time: f32) {
        if let Some(shot) = self
            .projectiles
            .last_mut()
            .filter(|shot| shot.kind == Kind::Rocket && shot.born == time)
        {
            shot.seeking = Some((shot.direction, time + 0.1));
            shot.expires = time + 10.0;
        }
    }

    pub fn guided_view(&self) -> Option<Camera> {
        let missile = self
            .projectiles
            .iter()
            .find(|shot| shot.kind == Kind::GuidedWarhead)?;
        let direction = missile.direction;
        Some(Camera {
            position: missile.position,
            yaw: direction.1.atan2(direction.0),
            pitch: direction.2.atan2(direction.0.hypot(direction.1)),
        })
    }

    pub(super) fn guide(
        &mut self,
        map: &Map,
        aim: Vec3,
        dt: f32,
        time: f32,
        detonate: bool,
        owner_alive: bool,
        sounds: &mut SoundEvents,
    ) -> Option<Blast> {
        let index = self
            .projectiles
            .iter()
            .position(|shot| shot.kind == Kind::GuidedWarhead)?;
        let shot = &mut self.projectiles[index];
        if !owner_alive || (detonate && time - shot.born >= WARHEAD_ARM_TIME) {
            let shot = self.projectiles.remove(index);
            return explode(
                &shot,
                &map.ordnance[shot.kind as usize],
                &mut self.effects,
                sounds,
                time,
            );
        }
        let velocity = guided_velocity(shot.velocity, aim, dt, &mut self.guided_roll);
        shot.direction = unit(velocity);
        shot.velocity = velocity;
        None
    }

    pub(super) fn deflect(&mut self, map: &Map, camera: &Camera) {
        for shot in self
            .projectiles
            .iter_mut()
            .filter(|shot| shot.stuck.is_none())
        {
            if let Some(velocity) =
                super::hammer_deflection(map, camera, shot.position, shot.velocity)
            {
                shot.direction = unit(velocity);
                shot.velocity = velocity;
            }
        }
    }

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
        sounds: &mut SoundEvents,
    ) -> Option<Blast> {
        let art = &map.ordnance[kind as usize];
        if kind == Kind::GuidedWarhead {
            self.guided_roll = 0.0;
        }
        let offset = kind.launch_offset(camera, side, variation);
        let speed = kind.launch_speed(art, variation);
        let lift = match kind {
            Kind::Grenade => 210.0,
            Kind::FlakShell => 200.0,
            Kind::Gel | Kind::Glob | Kind::Splash => 120.0,
            _ => 0.0,
        };
        let mut shot = Projectile {
            kind,
            direction,
            position: camera.position + offset,
            velocity: direction
                * (speed
                    + if kind == Kind::Grenade {
                        variation * 100.0
                    } else {
                        0.0
                    })
                + Vec3(0.0, 0.0, lift),
            born: time,
            expires: time
                + match kind {
                    Kind::Pulse => 2.0,
                    Kind::Grenade => 2.5 + variation * 0.5,
                    Kind::Rocket | Kind::FlakShell | Kind::Razor | Kind::RazorAlt => 6.0,
                    Kind::Chunk => FLAK_CHUNK_LIFETIME,
                    Kind::Warhead | Kind::GuidedWarhead => 30.0,
                    _ => 3.1 + 0.2 * variation,
                },
            scale,
            stuck: None,
            base: None,
            next_trail: time + if kind.warhead() { 0.3 } else { 0.0 },
            wall_hits: 0,
            seeking: None,
            hit_water: false,
            spray: None,
        };
        if map.collision.regions.at(shot.position).water {
            let factor = match kind {
                Kind::Rocket | Kind::Grenade => 0.6,
                Kind::Chunk => 0.65,
                Kind::Gel | Kind::Glob | Kind::Splash => 0.7,
                _ => 1.0,
            };
            shot.velocity = shot.velocity * factor;
            shot.hit_water = matches!(
                kind,
                Kind::Rocket | Kind::Grenade | Kind::Razor | Kind::RazorAlt
            );
        }
        if let Some((point, normal)) = map.collision.ray_surface(camera.position, offset) {
            shot.position = point + normal * 0.5;
            if art.scorch.is_some() {
                self.marks.push((kind, point, normal, None));
            }
            if !matches!(
                kind,
                Kind::Grenade | Kind::Chunk | Kind::Razor | Kind::FlakShell
            ) && !kind.bio()
            {
                return explode_at(&shot, art, &mut self.effects, sounds, time, normal * 15.5);
            }
        }
        self.projectiles.push(shot);
        None
    }

    #[cfg(test)]
    pub(super) fn update(
        &mut self,
        map: &Map,
        player: Option<Vec3>,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
        blasts: &mut Vec<Blast>,
    ) {
        self.update_with_target(map, player, None, time, dt, sounds, blasts);
    }

    pub(super) fn update_with_target(
        &mut self,
        map: &Map,
        player: Option<Vec3>,
        mut target: Option<&mut TrainingTarget>,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
        blasts: &mut Vec<Blast>,
    ) {
        self.rocket_trails.expire(time);
        self.warhead_smoke.expire(time);
        self.effects
            .retain(|effect| time - effect.born < effect.duration);
        self.ripples.retain(|ripple| {
            time - ripple.born < map.ordnance[ripple.kind as usize].water_lifetime
        });
        self.follow_movers(&map.movers);
        for effect in self.effects.iter().filter(|effect| {
            effect.nuclear && time >= effect.born + 0.5 && time - dt < effect.born + 0.5
        }) {
            sounds.extend_at(
                map.ordnance[effect.kind as usize].sound.iter().cloned(),
                effect.position,
                BLAST_SOUND_RADIUS,
                1.0,
            );
        }
        let mut fragments = Vec::new();
        self.intercept_projectiles(map, target.as_deref(), time, dt, sounds, blasts);
        self.projectiles.retain_mut(|shot| {
            let kind = shot.kind;
            let art = &map.ordnance[kind as usize];
            if time >= shot.expires {
                if matches!(kind, Kind::Grenade | Kind::GuidedWarhead | Kind::Warhead)
                    || shot.stuck.is_some()
                    || kind.bio()
                {
                    let mut source = *shot;
                    if kind == Kind::Grenade {
                        source.position = source.position + Vec3(0.0, 0.0, 16.0);
                    }
                    blasts.extend(explode(&source, art, &mut self.effects, sounds, time));
                }
                return false;
            }
            if shot.stuck.is_some() {
                spawn_splashes(shot, map, time, &mut fragments);
                let touched = player.is_some_and(|center| {
                    super::shock::cylinder_hit(
                        shot.position - center,
                        Vec3::default(),
                        crate::collision::PLAYER_RADIUS + art.collision_radius,
                        crate::collision::PLAYER_HEIGHT + art.collision_height,
                    )
                    .is_some()
                        && map
                            .collision
                            .ray(shot.position, center - shot.position)
                            .is_none()
                });
                if touched && kind == Kind::Grenade {
                    blasts.extend(explode(shot, art, &mut self.effects, sounds, time));
                    return false;
                }
                if kind.bio() && touched {
                    let delay = bio_trigger_delay(shot.born);
                    shot.expires = shot.expires.min(time + delay);
                    return true;
                }
                return true;
            }
            if matches!(kind, Kind::Rocket | Kind::Warhead | Kind::GuidedWarhead) {
                if let Some((initial, next)) = &mut shot.seeking
                    && time >= *next
                {
                    *next = time + 0.1;
                    if let Some(target) = target.as_deref().filter(|target| target.health > 0) {
                        let aim = unit(target.position - shot.position);
                        if aim.dot(*initial) > 0.0 {
                            shot.direction = unit(shot.direction + aim * 0.5);
                        }
                    }
                }
                let acceleration = if shot.seeking.is_some() { 25.0 } else { 50.0 };
                let speed =
                    (shot.velocity.dot(shot.velocity).sqrt() + acceleration * dt).min(1600.0);
                if kind != Kind::GuidedWarhead {
                    shot.velocity = shot.direction * speed;
                }
                if time >= shot.next_trail {
                    if kind.warhead() {
                        self.warhead_smoke.emit(
                            map,
                            kind,
                            shot.position,
                            shot.direction,
                            time,
                            &mut shot.next_trail,
                        );
                    } else {
                        self.rocket_trails.emit(
                            map,
                            shot.position,
                            shot.direction,
                            time,
                            shot.born,
                            &mut shot.next_trail,
                        );
                    }
                }
            } else if matches!(kind, Kind::Grenade | Kind::FlakShell)
                || (kind == Kind::Chunk && shot.wall_hits > 0)
                || kind.bio()
            {
                shot.velocity =
                    shot.velocity + map.collision.regions.at(shot.position).gravity * dt;
            }
            let delta = shot.velocity * dt;
            let wall = map.collision.projectile_contact(
                shot.position,
                delta,
                art.collision_radius,
                art.collision_height,
            );
            let visible_delta = wall.map_or(delta, |(point, _, _)| point - shot.position);
            let owner_armed = match kind {
                Kind::Grenade | Kind::Chunk => shot.wall_hits > 0,
                Kind::Razor => shot.wall_hits > 0 || time - shot.born >= 0.2,
                Kind::Gel | Kind::Glob | Kind::Splash => shot.hit_water,
                _ => false,
            };
            let owner = player.filter(|_| owner_armed).and_then(|center| {
                super::shock::cylinder_hit(
                    shot.position - center,
                    visible_delta,
                    crate::collision::PLAYER_RADIUS + art.collision_radius,
                    crate::collision::PLAYER_HEIGHT + art.collision_height,
                )
                .map(|fraction| (center, fraction))
            });
            if let Some((center, fraction)) = owner
                && target
                    .as_deref()
                    .and_then(|target| target.ray(shot.position, visible_delta))
                    .is_none_or(|target| fraction < target)
            {
                water_crossing(shot, map, visible_delta * fraction, time, &mut self.ripples);
                shot.position = shot.position + visible_delta * fraction;
                let blast = explode(shot, art, &mut self.effects, sounds, time);
                if blast.is_none() {
                    self.self_hits.push(projectile_damage(shot, art, center));
                }
                blasts.extend(blast);
                return false;
            }
            if let Some(target) = target.as_deref_mut()
                && let Some(fraction) = target.ray(shot.position, visible_delta)
            {
                water_crossing(shot, map, visible_delta * fraction, time, &mut self.ripples);
                shot.position = shot.position + visible_delta * fraction;
                let offset = unit(shot.position - target.position) * 16.0;
                let blast = explode_at(shot, art, &mut self.effects, sounds, time, offset);
                if blast.is_none() {
                    target.damage(projectile_damage(shot, art, target.position).0);
                }
                blasts.extend(blast);
                return false;
            }
            let Some((point, normal, hull)) = wall else {
                water_crossing(shot, map, delta, time, &mut self.ripples);
                shot.position = shot.position + delta;
                if shot.velocity.dot(shot.velocity) > 1.0 {
                    shot.direction = unit(shot.velocity);
                }
                return true;
            };
            water_crossing(shot, map, point - shot.position, time, &mut self.ripples);
            shot.position = point + normal * 0.5;
            shot.wall_hits += 1;
            if kind == Kind::Grenade {
                let velocity = (shot.velocity - normal * (2.0 * shot.velocity.dot(normal))) * 0.75;
                if shot.velocity.dot(shot.velocity) > 10_000.0 {
                    sounds.extend_at(
                        art.contact.iter().cloned(),
                        point,
                        CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                shot.velocity = if velocity.2 > 400.0 {
                    Vec3(velocity.0, velocity.1, (400.0 + velocity.2) * 0.5)
                } else {
                    velocity
                };
                if shot.velocity.dot(shot.velocity) < 400.0 {
                    shot.velocity = Vec3::default();
                    shot.stuck = Some(time);
                }
                true
            } else if matches!(kind, Kind::Chunk | Kind::Razor) {
                if shot.wall_hits > FLAK_CHUNK_REBOUNDS {
                    return false;
                }
                let retain = if kind == Kind::Chunk { 0.8 } else { 1.0 };
                shot.velocity = ricochet(shot.velocity, normal, retain);
                if shot.velocity.dot(shot.velocity) < 10_000.0 {
                    return false;
                }
                shot.direction = unit(shot.velocity);
                if shot.wall_hits == 1 {
                    sounds.extend_at(
                        art.contact.iter().cloned(),
                        point,
                        CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                true
            } else if kind.bio() {
                if kind == Kind::Glob {
                    let count = bio_splash_count(shot.scale);
                    shot.spray = Some((count, shot.position + normal * 5.0));
                    shot.scale = shot.scale.min(1.5);
                }
                shot.direction = normal;
                shot.stuck = Some(time);
                shot.base = map
                    .movers
                    .iter()
                    .enumerate()
                    .find(|(_, mover)| mover.collision_hull() == Some(hull))
                    .map(|(index, mover)| (index, shot.position - mover.offset));
                shot.expires = time + BIO_SURFACE_LIFETIME;
                spawn_splashes(shot, map, time, &mut fragments);
                if art.scorch.is_some() {
                    let contact = map.collision.ray(point, normal * -8.0).unwrap_or(point);
                    self.marks
                        .push((kind, contact, normal, shot.base.map(|(index, _)| index)));
                }
                sounds.extend_at(
                    art.contact.iter().cloned(),
                    point,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
                true
            } else {
                if kind == Kind::FlakShell {
                    for direction in flak_fragments(normal) {
                        fragments.push(Projectile {
                            kind: Kind::Chunk,
                            position: shot.position + direction * 2.0,
                            direction,
                            velocity: direction * 2500.0,
                            born: time,
                            expires: time + FLAK_CHUNK_LIFETIME,
                            scale: 1.0,
                            stuck: None,
                            base: None,
                            next_trail: time,
                            wall_hits: 0,
                            seeking: None,
                            hit_water: false,
                            spray: None,
                        });
                    }
                }
                if art.scorch.is_some() {
                    let contact = map.collision.ray(point, normal * -32.0).unwrap_or(point);
                    let mover = map
                        .movers
                        .iter()
                        .position(|mover| mover.collision_hull() == Some(hull));
                    self.marks.push((kind, contact, normal, mover));
                }
                blasts.extend(explode_at(
                    shot,
                    art,
                    &mut self.effects,
                    sounds,
                    time,
                    normal * 15.5,
                ));
                false
            }
        });
        self.projectiles.extend(fragments);
    }

    pub fn visuals<'a>(&self, assets: &'a Arsenal, time: f32, mut draw: impl FnMut(Visual<'a>)) {
        self.rocket_trails.visuals(assets, time, &mut draw);
        self.warhead_smoke.visuals(assets, time, &mut draw);
        for ripple in &self.ripples {
            let art = &assets[ripple.kind as usize];
            let age = time - ripple.born;
            let elapsed = age.min((art.water.duration() - 0.001).max(0.0));
            if let Some(pose) = art.water.sample(elapsed) {
                let direction = if matches!(ripple.kind, Kind::Razor | Kind::RazorAlt) {
                    Vec3(1.0, 0.0, 0.0)
                } else {
                    Vec3(0.0, 0.0, 1.0)
                };
                draw(Visual::Mesh(
                    ripple.position,
                    direction,
                    pose,
                    ripple.scale,
                    Some((1.0 - age / art.water_lifetime).clamp(0.0, 1.0)),
                ));
            }
        }
        for shot in &self.projectiles {
            let art = &assets[shot.kind as usize];
            let visual = if let Some(at) = shot.stuck.filter(|_| shot.kind.bio()) {
                art.bio_surface_visual(shot.position, shot.direction, time - at, shot.scale)
            } else {
                art.flight_visual(
                    shot.kind,
                    shot.position,
                    shot.direction,
                    time - shot.born,
                    shot.scale,
                )
            };
            if let Some(visual) = visual {
                draw(visual);
            }
        }
        for effect in &self.effects {
            let art = &assets[effect.kind as usize];
            let age = (time - effect.born) / effect.duration;
            if effect.nuclear {
                art.warhead_wave_visuals(effect.position, time - effect.born, &mut draw);
                continue;
            }
            if !effect.trail {
                if let Some(visual) = art.impact_sprite(effect.position, age, effect.scale) {
                    draw(visual);
                }
            } else if let Some(texture) = &art.trail {
                draw(Visual::Sprite(
                    effect.position,
                    texture.image_once(age),
                    effect.scale,
                    1.0 - age,
                ));
            }
        }
    }

    #[cfg(test)]
    pub(super) fn active_count(&self) -> usize {
        self.projectiles.len()
    }
}

pub(super) fn guided_velocity(previous: Vec3, aim: Vec3, dt: f32, roll: &mut f32) -> Vec3 {
    let velocity = unit(previous + aim * (1500.0 * dt)) * 550.0;
    if dt > 0.0 {
        let (right, _) = super::shock::axes(unit(velocity));
        let bank = (10.0 * right.dot(velocity - previous) / dt).clamp(-12000.0, 12000.0)
            * std::f32::consts::TAU
            / 65_536.0;
        *roll += (bank - *roll) * (5.0 * dt).min(1.0);
    }
    velocity
}

pub(super) const FLAK_SHELL_IMPACT: (f32, f32) = (0.5, 2.0);
pub(super) const ROCKET_IMPACT: (f32, f32) = (0.7, 1.4);

pub(super) fn direct_damage(
    kind: Kind,
    velocity: Vec3,
    hit: Vec3,
    center: Vec3,
    damage: u16,
    momentum: f32,
) -> (u16, Vec3) {
    if kind == Kind::Chunk && velocity.dot(velocity) <= 200.0 * 200.0 {
        return (0, Vec3::default());
    }
    let damage = if kind == Kind::Razor && hit.2 - center.2 > 0.62 * crate::collision::PLAYER_HEIGHT
    {
        (3.5 * f32::from(damage)) as u16
    } else {
        damage
    };
    (damage, unit(velocity) * momentum)
}

fn projectile_damage(shot: &Projectile, art: &Art, center: Vec3) -> (u16, Vec3) {
    let damage = art.damage.unwrap_or(match shot.kind {
        Kind::Pulse => 20.0,
        Kind::Razor => 30.0,
        Kind::Chunk => 16.0,
        _ => 0.0,
    });
    direct_damage(
        shot.kind,
        shot.velocity,
        shot.position,
        center,
        damage as u16,
        art.momentum.unwrap_or(10_000.0),
    )
}

fn water_crossing(
    shot: &mut Projectile,
    map: &Map,
    delta: Vec3,
    time: f32,
    ripples: &mut Vec<Ripple>,
) {
    for (fraction, wet) in map.collision.regions.water_crossings(shot.position, delta) {
        let mut scale = None;
        if shot.kind.warhead() {
            scale = Some(0.2);
        } else if wet {
            let factor = match shot.kind {
                Kind::Rocket | Kind::Grenade | Kind::Razor | Kind::RazorAlt if !shot.hit_water => {
                    scale = Some(if matches!(shot.kind, Kind::Razor | Kind::RazorAlt) {
                        0.5
                    } else {
                        0.2
                    });
                    0.6
                }
                Kind::Chunk => 0.65,
                Kind::Gel | Kind::Glob | Kind::Splash => {
                    if !shot.hit_water {
                        scale = Some(0.1);
                    }
                    0.1
                }
                _ => 1.0,
            };
            shot.velocity = shot.velocity * factor;
        }
        if wet {
            shot.hit_water = true;
        }
        if let Some(scale) = scale {
            ripples.push(Ripple {
                kind: shot.kind,
                position: shot.position + delta * fraction,
                born: time,
                scale,
            });
        }
    }
}

pub(super) fn bio_splash(
    map: &Map,
    parent: Vec3,
    origin: Vec3,
    born: f32,
    left: u8,
) -> (Vec3, Vec3, f32) {
    let seed = born
        .to_bits()
        .wrapping_add(u32::from(left))
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let z = 2.0 * (seed >> 8) as f32 / 16_777_216.0 - 1.0;
    let angle = seed.wrapping_mul(22_695_477) as f32 / u32::MAX as f32 * std::f32::consts::TAU;
    let radius = (1.0 - z * z).max(0.0).sqrt();
    let position = origin + Vec3(radius * angle.cos(), radius * angle.sin(), z) * 4.0;
    let direction = unit(position - parent);
    let mut velocity = direction * map.ordnance[Kind::Splash as usize].speed.unwrap_or(300.0)
        + Vec3(0.0, 0.0, 120.0);
    if map.collision.regions.at(position).water {
        velocity = velocity * 0.7;
    }
    (
        position,
        velocity,
        3.1 + 0.2 * (seed >> 8) as f32 / 16_777_216.0,
    )
}

fn spawn_splashes(shot: &mut Projectile, map: &Map, time: f32, fragments: &mut Vec<Projectile>) {
    let Some((left, origin)) = &mut shot.spray else {
        return;
    };
    for _ in 0..(*left).min(2) {
        *left -= 1;
        let (position, velocity, lifetime) =
            bio_splash(map, shot.position, *origin, shot.born, *left);
        let direction = unit(position - shot.position);
        fragments.push(Projectile {
            kind: Kind::Splash,
            position,
            direction,
            velocity,
            born: time,
            expires: time + lifetime,
            scale: 1.0,
            stuck: None,
            base: None,
            next_trail: time,
            wall_hits: 0,
            seeking: None,
            hit_water: false,
            spray: None,
        });
    }
    if *left == 0 {
        shot.spray = None;
    }
}

fn shoot_down(
    shot: &Projectile,
    art: &Art,
    effects: &mut Vec<Effect>,
    time: f32,
    sounds: &mut SoundEvents,
) -> Blast {
    effects.push(Effect {
        kind: shot.kind,
        position: shot.position,
        born: time,
        duration: 1.0,
        scale: 2.8,
        trail: false,
        nuclear: false,
        struck: [false; 2],
    });
    sounds.extend_at(
        art.contact.iter().chain(&art.sound).cloned(),
        shot.position,
        BLAST_SOUND_RADIUS,
        1.0,
    );
    warhead_blast(art, shot.position, WARHEAD_INTERCEPT_RADIUS)
}

fn explode(
    shot: &Projectile,
    art: &Art,
    effects: &mut Vec<Effect>,
    sounds: &mut SoundEvents,
    time: f32,
) -> Option<Blast> {
    explode_at(shot, art, effects, sounds, time, Vec3(0.0, 0.0, 16.0))
}

fn explode_at(
    shot: &Projectile,
    art: &Art,
    effects: &mut Vec<Effect>,
    sounds: &mut SoundEvents,
    time: f32,
    wave_offset: Vec3,
) -> Option<Blast> {
    sounds.extend_at(
        art.sound.iter().cloned(),
        shot.position,
        BLAST_SOUND_RADIUS,
        1.0,
    );
    let kind = shot.kind;
    let position = if kind.warhead() {
        sounds.extend_at(
            art.contact.iter().cloned(),
            shot.position,
            BLAST_SOUND_RADIUS,
            1.0,
        );
        shot.position + wave_offset
    } else {
        shot.position
    };
    let (duration, scale) = match kind {
        Kind::Pulse => (0.5, 0.65),
        Kind::Rocket | Kind::Grenade => ROCKET_IMPACT,
        Kind::FlakShell => FLAK_SHELL_IMPACT,
        Kind::RazorAlt => (0.7, 1.0),
        Kind::Warhead | Kind::GuidedWarhead => (1.5, 2.8),
        _ => (1.0, shot.scale * 1.4),
    };
    effects.push(Effect {
        kind,
        position,
        born: time,
        duration,
        scale,
        trail: false,
        nuclear: kind.warhead(),
        struck: [false; 2],
    });
    let (radius, damage, momentum) = match kind {
        Kind::Pulse | Kind::Chunk | Kind::Razor => return None,
        Kind::Rocket => (220.0, 75.0, 80_000.0),
        Kind::Grenade => (200.0, 80.0, 50_000.0),
        Kind::FlakShell => FLAK_SHELL_BLAST,
        Kind::RazorAlt => (180.0, 34.0, 87_000.0),
        Kind::Warhead | Kind::GuidedWarhead => {
            return Some(warhead_blast(art, shot.position, WARHEAD_INITIAL_RADIUS));
        }
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

pub(super) const WARHEAD_WAVE_LIFETIME: f32 = 1.5;
pub(super) const WARHEAD_PHASE_TWO: f32 = 0.5;
pub(super) const WARHEAD_ARM_TIME: f32 = 1.0;
pub(super) const WARHEAD_INITIAL_RADIUS: f32 = 300.0;
pub(super) const WARHEAD_INTERCEPT_RADIUS: f32 = 350.0;

pub(super) fn warhead_blast(art: &Art, position: Vec3, radius: f32) -> Blast {
    Blast::radial(
        position,
        radius,
        art.damage.unwrap_or(1000.0),
        art.momentum.unwrap_or(100_000.0),
    )
}

pub(super) fn nuclear_shake(
    eye: Vec3,
    time: f32,
    waves: impl Iterator<Item = (Vec3, f32)>,
) -> (f32, f32) {
    waves
        .filter_map(|(origin, born)| {
            let age = time - born;
            let offset = eye - origin;
            let distance = offset.dot(offset).sqrt();
            (age < 0.5 && distance < 3000.0).then_some((distance, age))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or((0.0, 0.0), |(distance, age)| {
            let fade = 1.0 - 2.0 * age;
            let roll =
                (600_000.0 / distance.max(1.0)).min(4096.0) * std::f32::consts::TAU / 65_536.0;
            (
                roll * fade * (46.0 * age).sin(),
                10.0 * fade * (32.0 * age).sin(),
            )
        })
}

fn shock_size(age: f32) -> f32 {
    let age = age.clamp(0.0, 1.5);
    13.0 * age + 3.5 / ((1.5 - age) / 1.5 + 0.05)
}

pub(super) fn wave_hit(map: &Map, origin: Vec3, center: Vec3, age: f32) -> Option<(u16, Vec3)> {
    if !(0.1..=1.5).contains(&age) {
        return None;
    }
    let offset = center - origin;
    let distance = offset.dot(offset).sqrt().max(1.0);
    if distance > shock_size(age) * 29.0 || map.collision.ray(origin, offset).is_some() {
        return None;
    }
    let force = (1100.0 - 1.1 * distance).max(0.0);
    let impulse = (offset * distance.recip() + Vec3(0.0, 0.0, 0.3)) * (force + 20.0);
    Some((force as u16, impulse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collision::Collision, package::Package};

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn stock_projectiles_slow_in_water_and_use_original_ripples() {
        use crate::{
            collision::Plane,
            movement::{RegionNode, Zone},
        };
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        map.collision.regions.nodes = vec![RegionNode {
            plane: Plane {
                normal: Vec3(1.0, 0.0, 0.0),
                distance: 50.0,
            },
            children: [-1, -1],
            zones: [0, 1],
        }];
        map.collision.regions.zones = vec![
            Zone::default(),
            Zone {
                water: true,
                ..Zone::default()
            },
        ];
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        for (kind, factor) in [
            (Kind::Rocket, 0.6),
            (Kind::Grenade, 0.6),
            (Kind::Razor, 0.6),
            (Kind::RazorAlt, 0.6),
            (Kind::Chunk, 0.65),
            (Kind::Gel, 0.1),
            (Kind::Glob, 0.1),
            (Kind::Splash, 0.1),
            (Kind::Warhead, 1.0),
        ] {
            let mut shots = Shots::default();
            let mut sounds = SoundEvents::default();
            let mut blasts = Vec::new();
            shots.fire(
                &map,
                &camera,
                kind,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            let before = shots.projectiles[0].velocity;
            shots.update(&map, None, 0.2, 0.2, &mut sounds, &mut blasts);
            let shot = &shots.projectiles[0];
            assert!(shot.position.0 > 50.0 && shot.hit_water, "{}", kind.class());
            let expected = if matches!(kind, Kind::Rocket | Kind::Warhead) {
                (before.dot(before).sqrt() + 10.0) * factor
            } else if matches!(kind, Kind::Grenade) || kind.bio() {
                let velocity = before + Vec3(0.0, 0.0, -190.0);
                velocity.dot(velocity).sqrt() * factor
            } else {
                before.dot(before).sqrt() * factor
            };
            assert!(
                (shot.velocity.dot(shot.velocity).sqrt() - expected).abs() < 0.01,
                "{}",
                kind.class()
            );
            if kind != Kind::Chunk {
                let art = &map.ordnance[kind as usize];
                assert!(!art.water.frames.is_empty(), "{}", kind.class());
                assert!(
                    art.water
                        .frames
                        .iter()
                        .all(|frame| frame.iter().all(|face| face.texture.is_some()))
                );
                assert!(art.water_lifetime > 0.0);
                assert_eq!(shots.ripples.len(), 1);
                assert!((shots.ripples[0].position.0 - 50.0).abs() < 0.001);
            }
            let underwater = Camera {
                position: Vec3(150.0, 0.0, 65.0),
                ..camera
            };
            let mut wet = Shots::default();
            wet.fire(
                &map,
                &underwater,
                kind,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            let initial_factor = match kind {
                Kind::Rocket | Kind::Grenade => 0.6,
                Kind::Chunk => 0.65,
                Kind::Gel | Kind::Glob | Kind::Splash => 0.7,
                _ => 1.0,
            };
            let error = wet.projectiles[0].velocity - before * initial_factor;
            assert!(error.dot(error) < 0.001);
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn charged_bio_emits_two_full_size_splashes_per_tick() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        map.collision
            .add_box(Vec3(190.0, -500.0, -500.0), Vec3(220.0, 500.0, 500.0));
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        let mut shots = Shots::default();
        let mut sounds = SoundEvents::default();
        let mut blasts = Vec::new();
        shots.fire(
            &map,
            &camera,
            Kind::Glob,
            camera.forward(),
            0.0,
            2.14,
            0.0,
            0.0,
            &mut sounds,
        );
        let mut tick = 0;
        while shots.projectiles[0].stuck.is_none() {
            tick += 1;
            shots.update(
                &map,
                None,
                tick as f32 / 240.0,
                1.0 / 240.0,
                &mut sounds,
                &mut blasts,
            );
            assert!(tick < 240);
        }
        assert_eq!(shots.projectiles[0].scale, 1.5);
        for expected in [2, 4, 6, 7] {
            assert_eq!(
                shots
                    .projectiles
                    .iter()
                    .filter(|shot| shot.kind == Kind::Splash)
                    .count(),
                expected
            );
            for splash in shots
                .projectiles
                .iter()
                .filter(|shot| shot.kind == Kind::Splash)
            {
                assert_eq!(splash.scale, 1.0);
                assert!((3.0999..=3.3001).contains(&(splash.expires - splash.born)));
            }
            tick += 1;
            shots.update(
                &map,
                None,
                tick as f32 / 240.0,
                1.0 / 240.0,
                &mut sounds,
                &mut blasts,
            );
        }
        assert!(shots.projectiles[0].spray.is_none());
        assert!(blasts.is_empty());
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn ricochets_and_armed_grenades_can_hit_the_player() {
        use super::super::{Input, Practice};
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        let camera = Camera {
            position: Vec3(0.0, 0.0, 27.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        let mut sounds = SoundEvents::default();
        for armed in [false, true] {
            let mut shots = Shots::default();
            let mut blasts = Vec::new();
            shots.fire(
                &map,
                &camera,
                Kind::Grenade,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            let shot = &mut shots.projectiles[0];
            shot.position = Vec3::default();
            shot.velocity = Vec3::default();
            shot.wall_hits = u8::from(armed);
            shots.update(
                &map,
                Some(Vec3::default()),
                0.1,
                0.01,
                &mut sounds,
                &mut blasts,
            );
            assert_eq!(shots.projectiles.is_empty(), armed);
            assert_eq!(blasts.len(), usize::from(armed));
        }
        let mut practice = Practice::new(&map);
        practice.time = 0.2;
        practice.ordnance.fire(
            &map,
            &camera,
            Kind::Razor,
            camera.forward(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        let shot = &mut practice.ordnance.projectiles[0];
        shot.position = Vec3::default();
        shot.velocity = Vec3(500.0, 0.0, 0.0);
        let mut player = crate::collision::Walker::spawn(&map.collision, Vec3::default());
        practice.update(
            &mut map,
            &camera,
            0.01,
            Input {
                primary: false,
                alternate: false,
                slot: None,
                cycle: 0,
            },
            false,
            Some(&mut player),
        );
        assert_eq!(practice.health, 70);
        assert!(practice.ordnance.projectiles.is_empty());
        assert!(practice.ordnance.take_self_hits().is_empty());
        for (height, expected) in [(0.0, 170), (30.0, 95)] {
            let mut shots = Shots::default();
            let mut blasts = Vec::new();
            let mut target = TrainingTarget {
                position: Vec3(100.0, 0.0, 0.0),
                yaw: 0.0,
                health: 200,
                outgoing_scale: 1,
            };
            shots.fire(
                &map,
                &camera,
                Kind::Razor,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            shots.projectiles[0].position = Vec3(90.0, 0.0, height);
            shots.projectiles[0].velocity = Vec3(500.0, 0.0, 0.0);
            shots.update_with_target(
                &map,
                None,
                Some(&mut target),
                0.1,
                0.01,
                &mut sounds,
                &mut blasts,
            );
            assert_eq!(target.health, expected);
            assert!(shots.projectiles.is_empty());
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn nuclear_wave_reaches_other_projectiles_and_guidance_ends_on_blast_damage() {
        use super::super::{Input, Practice};
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        for covered in [false, true] {
            map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
            if covered {
                map.collision
                    .add_box(Vec3(250.0, -200.0, -200.0), Vec3(270.0, 200.0, 300.0));
            }
            let mut shots = Shots::default();
            let mut sounds = SoundEvents::default();
            shots.fire(
                &map,
                &camera,
                Kind::Warhead,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            let mut source = shots.projectiles.remove(0);
            source.position = camera.position;
            let _ = explode(
                &source,
                &map.ordnance[Kind::Warhead as usize],
                &mut shots.effects,
                &mut sounds,
                0.0,
            );
            shots.fire(
                &map,
                &camera,
                Kind::Warhead,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            shots.projectiles[0].position = Vec3(500.0, 0.0, 81.0);
            shots.fire(
                &map,
                &camera,
                Kind::Gel,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            shots.projectiles[1].position = Vec3(700.0, 0.0, 81.0);
            let mut blasts = Vec::new();
            shots.damage_blasts(&map, &mut blasts, 0.1, &mut sounds);
            assert_eq!(shots.projectiles.len(), 2);
            shots.damage_blasts(&map, &mut blasts, 1.0, &mut sounds);
            assert_eq!(
                shots
                    .projectiles
                    .iter()
                    .filter(|shot| shot.kind.warhead())
                    .count(),
                usize::from(covered)
            );
            assert_eq!(
                shots.effects.iter().filter(|effect| effect.nuclear).count(),
                1
            );
            let gel = shots
                .projectiles
                .iter()
                .find(|shot| shot.kind.bio())
                .unwrap();
            assert_eq!(gel.expires <= 1.3, !covered);
        }
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        let mut game = Practice::new(&map);
        let mut sounds = SoundEvents::default();
        game.ordnance.fire(
            &map,
            &camera,
            Kind::GuidedWarhead,
            camera.forward(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        game.ordnance.projectiles[0].position = Vec3(100.0, 0.0, 65.0);
        game.was_guiding = true;
        game.ordnance.fire(
            &map,
            &camera,
            Kind::Grenade,
            camera.forward(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        let grenade = game.ordnance.projectiles.last_mut().unwrap();
        grenade.position = Vec3(100.0, 20.0, 65.0);
        grenade.expires = 0.005;
        game.update(
            &mut map,
            &camera,
            0.01,
            Input {
                primary: false,
                alternate: false,
                slot: None,
                cycle: 0,
            },
            false,
            None,
        );
        assert!(game.ordnance.guided_view().is_none());
        assert!(game.guidance_lost_until > game.time);
        assert!(game.next_shot >= game.time + 1.3 - 0.001);
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn explosions_propagate_through_projectiles_and_stop_at_cover() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        for covered in [false, true] {
            map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
            if covered {
                map.collision
                    .add_box(Vec3(155.0, -200.0, -200.0), Vec3(165.0, 200.0, 300.0));
            }
            let mut shots = Shots::default();
            let mut sounds = SoundEvents::default();
            for x in [100.0, 210.0, 320.0] {
                shots.fire(
                    &map,
                    &camera,
                    Kind::Gel,
                    camera.forward(),
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    &mut sounds,
                );
                let shot = shots.projectiles.last_mut().unwrap();
                shot.position = Vec3(x, 0.0, 65.0);
                shot.velocity = Vec3::default();
                shot.stuck = Some(0.0);
            }
            let mut seed = vec![Blast::radial(camera.position, 120.0, 75.0, 50_000.0)];
            shots.damage_blasts(&map, &mut seed, 0.0, &mut sounds);
            assert!(shots.projectiles[0].expires <= 0.3);
            assert!(shots.projectiles[1].expires > 3.0);
            let mut count = 0;
            for tick in 1..=100 {
                let time = tick as f32 * 0.01;
                let mut blasts = Vec::new();
                shots.update(&map, None, time, 0.01, &mut sounds, &mut blasts);
                shots.damage_blasts(&map, &mut blasts, time, &mut sounds);
                count += blasts.len();
            }
            assert_eq!(count, if covered { 1 } else { 3 });
            assert_eq!(shots.projectiles.len(), if covered { 2 } else { 0 });
        }
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        let mut shots = Shots::default();
        let mut sounds = SoundEvents::default();
        for x in [100.0, 400.0, 700.0, 1000.0] {
            shots.fire(
                &map,
                &camera,
                Kind::Warhead,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            shots.projectiles.last_mut().unwrap().position = Vec3(x, 0.0, 65.0);
        }
        let mut blasts = vec![Blast::radial(camera.position, 150.0, 75.0, 50_000.0)];
        shots.damage_blasts(&map, &mut blasts, 0.0, &mut sounds);
        assert!(shots.projectiles.is_empty());
        assert_eq!(blasts.len(), 5);
        assert_eq!(shots.effects.len(), 4);
        assert!(shots.effects.iter().all(|effect| !effect.nuclear));
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn deck16_gel_and_its_mark_follow_the_lift() {
        use crate::collision::Walker;
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let index = 0;
        let face = map.movers[index]
            .faces
            .iter()
            .filter(|face| face.normal.2 > 0.95)
            .max_by(|a, b| a.vertices[0].2.total_cmp(&b.vertices[0].2))
            .unwrap();
        let center = face
            .vertices
            .iter()
            .copied()
            .fold(Vec3::default(), |sum, point| sum + point)
            * (1.0 / face.vertices.len() as f32);
        let up = Vec3(0.0, 0.0, 1.0);
        let mut walker = Walker::spawn(&map.collision, center + up * 39.2);
        let camera = Camera {
            position: center + up * 80.0,
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_2,
        };
        let mut shots = Shots::default();
        let mut sounds = SoundEvents::default();
        let mut blasts = Vec::new();
        shots.fire(
            &map,
            &camera,
            Kind::Gel,
            camera.look_direction(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        for tick in 1..=80 {
            shots.update(
                &map,
                None,
                tick as f32 / 240.0,
                1.0 / 240.0,
                &mut sounds,
                &mut blasts,
            );
        }
        assert_eq!(
            shots.projectiles[0].base.map(|(index, _)| index),
            Some(index)
        );
        let initial = shots.projectiles[0].position;
        let mut marks = super::super::decals::Marks::default();
        shots.project_marks(&map, &mut marks, 1.0 / 3.0);
        let mark_origin = marks.visible().next().expect("BioMark on the lift")[0].origin;
        for tick in 81..=440 {
            let dt = 1.0 / 240.0;
            map.movers[index].tick(&mut map.collision, &mut walker, dt);
            walker.advance(&map.collision, Vec3::default(), None, dt);
            shots.update(&map, None, tick as f32 * dt, dt, &mut sounds, &mut blasts);
            marks.follow_movers(&map.movers);
        }
        let delta = map.movers[index].offset;
        assert!(delta.2 > 64.0, "lift displacement {delta:?}");
        let error = shots.projectiles[0].position - initial - delta;
        assert!(error.dot(error) < 0.001);
        let error = marks.visible().next().unwrap()[0].origin - mark_origin - delta;
        assert!(error.dot(error) < 0.001);
        map.movers[index].tick(&mut map.collision, &mut walker, 0.05);
        let (_, local) = shots.projectiles[0].base.unwrap();
        let target = local + map.movers[index].offset;
        let anchor = target + up * 20.0;
        let offset = Vec3(-150.0, -70.0, 110.0);
        let position = map
            .collision
            .ray(anchor, offset)
            .map_or(anchor + offset, |point| point - unit(offset) * 8.0);
        let look = target - position;
        let camera = Camera {
            position,
            yaw: look.1.atan2(look.0),
            pitch: look.2.atan2(look.0.hypot(look.1)),
        };
        let mut practice = super::super::Practice::new(&map);
        practice.time = 440.0 / 240.0;
        practice.weapon = super::super::Weapon::Sniper;
        practice.ammo[super::super::Weapon::Sniper.ammo_pool()] = 1;
        practice.ordnance = shots;
        practice.update(
            &mut map,
            &camera,
            1.0 / 240.0,
            super::super::Input {
                primary: true,
                alternate: false,
                slot: None,
                cycle: 0,
            },
            false,
            None,
        );
        assert!(
            practice.ordnance.projectiles[0].expires <= practice.time + 0.3,
            "the sniper trace must use the current lift position"
        );
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn bio_gel_keeps_stock_surface_animations_and_detonation_timers() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let art = &map.ordnance[Kind::Gel as usize];
        assert!(!art.dripping.frames.is_empty() && !art.sliding.frames.is_empty());
        assert!(
            art.flight.frames[0]
                .iter()
                .all(|face| face.flags & 0x10 != 0)
        );
        assert!(art.scorch.is_some());
        assert_eq!(art.collision_radius, 2.0);
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        for (min, max, direction) in [
            (
                Vec3(190.0, -500.0, -500.0),
                Vec3(220.0, 500.0, 500.0),
                Vec3(1.0, 0.0, 0.0),
            ),
            (
                Vec3(-500.0, -500.0, -100.0),
                Vec3(500.0, 500.0, 0.0),
                Vec3(0.0, 0.0, -1.0),
            ),
            (
                Vec3(-500.0, -500.0, 150.0),
                Vec3(500.0, 500.0, 200.0),
                Vec3(0.0, 0.0, 1.0),
            ),
        ] {
            map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
            map.collision.add_box(min, max);
            let mut shots = Shots::default();
            let mut sounds = SoundEvents::default();
            let mut blasts = Vec::new();
            shots.fire(
                &map,
                &camera,
                Kind::Gel,
                direction,
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            for tick in 1..=180 {
                shots.update(
                    &map,
                    None,
                    tick as f32 / 240.0,
                    1.0 / 240.0,
                    &mut sounds,
                    &mut blasts,
                );
            }
            let shot = &shots.projectiles[0];
            let stuck = shot.stuck.expect("gel adhered to the surface");
            assert!((shot.expires - stuck - 3.8).abs() < 0.001);
            assert_eq!(shots.marks.len(), 1);
            assert!(blasts.is_empty());
            let mut rendered = false;
            shots.visuals(&map.ordnance, 0.75, |visual| {
                if let Visual::Mesh(_, _, pose, _, _) = visual {
                    rendered = !pose.current.is_empty();
                }
            });
            assert!(rendered);
            shots.damage_target(&map.ordnance, 0, 17, 0.75, &mut sounds);
            assert!((0.85..=1.05).contains(&shots.projectiles[0].expires));
            shots.update(&map, None, 1.1, 0.01, &mut sounds, &mut blasts);
            assert!(shots.projectiles.is_empty() && blasts.len() == 1);
        }
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        let mut shots = Shots::default();
        let mut sounds = SoundEvents::default();
        let mut blasts = Vec::new();
        shots.fire(
            &map,
            &camera,
            Kind::Gel,
            camera.forward(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        shots.update(&map, None, 3.4, 0.01, &mut sounds, &mut blasts);
        assert!(shots.projectiles.is_empty() && blasts.len() == 1);
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn weapons_intercept_redeemer_and_respect_cover() {
        use super::super::{Input, Practice, Weapon, WeaponState};
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        for weapon in [
            Weapon::Enforcer,
            Weapon::Minigun,
            Weapon::Sniper,
            Weapon::Shock,
            Weapon::Pulse,
        ] {
            for covered in [false, true] {
                map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
                if covered {
                    map.collision
                        .add_box(Vec3(150.0, -100.0, 0.0), Vec3(170.0, 100.0, 200.0));
                }
                let mut game = Practice::new(&map);
                game.weapon = weapon;
                game.weapon_state = WeaponState::Ready;
                game.ammo[weapon.ammo_pool()] = 20;
                let mut sounds = SoundEvents::default();
                game.ordnance.fire(
                    &map,
                    &camera,
                    Kind::Warhead,
                    camera.forward(),
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    &mut sounds,
                );
                let missile = &mut game.ordnance.projectiles[0];
                missile.position = Vec3(350.0, 5.0, 60.0);
                missile.velocity = Vec3::default();
                let alternate = weapon == Weapon::Pulse;
                game.update(
                    &mut map,
                    &camera,
                    1.0 / 240.0,
                    Input {
                        primary: !alternate,
                        alternate,
                        slot: None,
                        cycle: 0,
                    },
                    false,
                    None,
                );
                let alive = game
                    .ordnance
                    .projectiles
                    .iter()
                    .any(|shot| shot.kind.warhead());
                assert_eq!(alive, covered, "{weapon:?} cover={covered}");
                assert!(!game.ordnance.effects.iter().any(|effect| effect.nuclear));
                if !covered {
                    assert!(
                        game.ordnance
                            .effects
                            .iter()
                            .any(|effect| effect.kind.warhead() && !effect.trail)
                    );
                    assert!(
                        game.ordnance
                            .nuclear_wave(&map, game.time + 0.5, camera.position, 0)
                            .is_none()
                    );
                }
            }
        }
        map.collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        for kind in [Kind::Rocket, Kind::Pulse, Kind::Chunk, Kind::Razor] {
            let mut shots = Shots::default();
            let mut sounds = SoundEvents::default();
            let mut blasts = Vec::new();
            shots.fire(
                &map,
                &camera,
                Kind::Warhead,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            shots.projectiles[0].position = Vec3(350.0, 10.0, 55.0);
            shots.projectiles[0].velocity = Vec3::default();
            shots.fire(
                &map,
                &camera,
                kind,
                camera.forward(),
                0.0,
                1.0,
                0.0,
                0.0,
                &mut sounds,
            );
            for tick in 1..=120 {
                shots.update(
                    &map,
                    None,
                    tick as f32 / 240.0,
                    1.0 / 240.0,
                    &mut sounds,
                    &mut blasts,
                );
                if shots.projectiles.is_empty() {
                    break;
                }
            }
            assert!(shots.projectiles.is_empty(), "{}", kind.class());
            assert!(!shots.effects.iter().any(|effect| effect.nuclear));
            assert!(blasts.iter().any(|blast| {
                blast
                    .hit_player(&map.collision, Vec3(350.0, 10.0, 55.0))
                    .is_some()
            }));
        }
    }

    #[test]
    #[ignore = "requires the local GOTY game packages"]
    fn redeemer_uses_stock_missile_smoke_and_full_nuclear_wave() {
        let path = std::path::Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let launcher = &map.arsenal[&super::super::Weapon::Redeemer];
        assert!(launcher.guided_hud.iter().all(Option::is_some));
        assert!(launcher.primary_motion.duration() > 0.7);
        for kind in [Kind::Warhead, Kind::GuidedWarhead] {
            let art = &map.ordnance[kind as usize];
            assert_eq!(art.flight.frames.len(), 100);
            assert!(art.flight.frames.iter().all(|frame| {
                !frame.is_empty() && frame.iter().all(|face| face.texture.is_some())
            }));
            assert!(art.smoke.iter().all(Option::is_some));
            assert!(art.scorch.is_some());
            assert!(art.contact.is_some() && art.sound.is_some());
            assert!(art.explosion.is_some() && art.second_explosion.is_some());
            let mut min = Vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
            let mut max = min * -1.0;
            for face in &art.wavefront {
                assert!(face.texture.is_some());
                for &p in &face.vertices {
                    min = Vec3(min.0.min(p.0), min.1.min(p.1), min.2.min(p.2));
                    max = Vec3(max.0.max(p.0), max.1.max(p.1), max.2.max(p.2));
                }
            }
            let size = max - min;
            assert!(size.0.max(size.1).max(size.2) * shock_size(1.4) > 2000.0);
        }
        let mut collision = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        collision.add_box(Vec3(800.0, -1000.0, -1000.0), Vec3(850.0, 1000.0, 1000.0));
        map.collision = collision;
        let camera = Camera {
            position: Vec3(0.0, 0.0, 65.0),
            yaw: 0.0,
            pitch: 0.0,
        };
        let mut shots = Shots::default();
        let mut sounds = SoundEvents::default();
        let mut blasts = Vec::new();
        shots.fire(
            &map,
            &camera,
            Kind::Warhead,
            camera.forward(),
            0.0,
            1.0,
            0.0,
            0.0,
            &mut sounds,
        );
        for frame in 1..=160 {
            shots.update(
                &map,
                None,
                frame as f32 * 0.01,
                0.01,
                &mut sounds,
                &mut blasts,
            );
        }
        assert!(shots.projectiles.is_empty());
        let effect = shots.effects.iter().find(|effect| !effect.trail).unwrap();
        assert!((effect.position.0 - 769.0).abs() < 0.01);
        assert!(!shots.warhead_smoke.0.is_empty());
        assert_eq!(shots.marks.len(), 1);
        assert!((shots.marks[0].1.0 - 800.0).abs() < 0.01);
        assert!(
            map.ordnance[Kind::Warhead as usize]
                .scorch
                .as_ref()
                .unwrap()
                .2
                > 0.0
        );
        assert_eq!(blasts.len(), 1);
        let origin = effect.position;
        let born = effect.born;
        let exposed = origin - Vec3(500.0, 0.0, 0.0);
        assert!(shots.nuclear_wave(&map, born + 1.0, exposed, 0).is_some());
        assert!(shots.nuclear_wave(&map, born + 1.1, exposed, 0).is_none());
        assert!(
            shots
                .nuclear_wave(&map, born + 0.8, origin + Vec3(200.0, 0.0, 0.0), 1)
                .is_none()
        );
        let mut sprites = Vec::new();
        shots.visuals(&map.ordnance, born + 0.75, |visual| {
            if let Visual::Sprite(_, _, scale, _) = visual {
                sprites.push(scale);
            }
        });
        assert!(sprites.contains(&2.8) && sprites.contains(&3.2));
    }

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
        let mut sounds = SoundEvents::default();
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
