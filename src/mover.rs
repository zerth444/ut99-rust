use std::{io, path::Path};

use crate::collision::{Collision, Hull, Plane, Walker};
use crate::map::{Face, Vec3, brush_polys, material_color};
use crate::package::{Export, Package, Reader, invalid};
use crate::texture::load_materials;

#[derive(Clone, Copy)]
enum Activation {
    Stand,
    Bump,
    Trigger,
    Control,
    Toggle,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
struct Trigger {
    position: Vec3,
    radius: f32,
    height: f32,
    event: usize,
}

impl Trigger {
    fn touches(&self, walker: &Walker) -> bool {
        let offset = walker.position - self.position;
        let radius = self.radius + 17.0;
        offset.0 * offset.0 + offset.1 * offset.1 <= radius * radius
            && offset.2.abs() <= self.height + 39.0
    }
}

pub struct Mover {
    pub faces: Vec<Face>,
    pub offset: Vec3,
    scene_rotation: Vec3,
    pub pivot: Vec3,
    home_rotation: Vec3,
    hull_vertices: Vec<Vec3>,
    hull_planes: Vec<Plane>,
    hull_parts: Vec<Hull>,
    min: Vec3,
    max: Vec3,
    travel: Vec3,
    time: f32,
    pause: f32,
    glide: bool,
    fraction: f32,
    countdown: f32,
    opening: bool,
    trigger_touching: bool,
    ready: bool,
    hull: Option<usize>,
    triggers: Vec<Trigger>,
    activation: Activation,
    pub(crate) tag: Option<usize>,
    event: Option<usize>,
    trigger_once: bool,
    signaled: Option<bool>,
    control_sources: usize,
    damage_threshold: Option<f32>,
    pub attach_tag: Option<usize>,
    #[cfg(windows)]
    scene_keys: Vec<(Vec3, Vec3)>,
    #[cfg(windows)]
    scene_progress: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiftStop {
    Home,
    Away,
}

pub(crate) struct Cargo<'a> {
    pub position: &'a mut Vec3,
    pub radius: f32,
    pub height: f32,
    pub resting: bool,
}

impl Cargo<'_> {
    fn supported_by(&self, world: &Collision, hull: usize) -> bool {
        self.resting
            && world
                .projectile_contact(
                    *self.position + Vec3(0.0, 0.0, 0.2),
                    Vec3(0.0, 0.0, -1.2),
                    self.radius,
                    self.height,
                )
                .is_some_and(|(_, normal, hit)| hit == hull && normal.2 > 0.7)
    }

    fn overlaps(&self, min: Vec3, max: Vec3) -> bool {
        let Vec3(x, y, z) = *self.position;
        let radius = (self.radius - 0.5).max(0.0);
        let dx = x - x.clamp(min.0, max.0);
        let dy = y - y.clamp(min.1, max.1);
        z + self.height > min.2 + 0.5
            && z - self.height < max.2 - 0.5
            && dx * dx + dy * dy < radius * radius
    }

    fn rests_on(&self, base: &Self) -> bool {
        let offset = *self.position - *base.position;
        self.resting
            && (offset.2 - self.height - base.height).abs() <= 1.0
            && offset.0.hypot(offset.1) < self.radius + base.radius
    }

    fn overlaps_cargo(&self, other: &Self, delta: Vec3) -> bool {
        let offset = *self.position + delta - *other.position;
        offset.2.abs() < self.height + other.height - 0.5
            && offset.0.hypot(offset.1) < self.radius + other.radius - 0.5
    }
}

impl LiftStop {
    pub fn opposite(self) -> Self {
        match self {
            Self::Home => Self::Away,
            Self::Away => Self::Home,
        }
    }
}

impl Mover {
    pub(crate) fn is_rotated(&self) -> bool {
        self.scene_rotation.dot(self.scene_rotation) > f32::EPSILON
    }

    pub(crate) fn transform_point(&self, point: Vec3) -> Vec3 {
        self.pivot + self.offset + self.transform_direction(point - self.pivot)
    }

    pub(crate) fn transform_direction(&self, direction: Vec3) -> Vec3 {
        self.rotate_from_home(direction, self.scene_rotation)
    }

    fn rotate_from_home(&self, direction: Vec3, key_rotation: Vec3) -> Vec3 {
        rotate_from_home(direction, self.home_rotation, key_rotation)
    }

    pub(crate) fn collision_hull(&self) -> Option<usize> {
        self.hull
    }

    fn set_collision(&self, world: &mut Collision, id: usize, offset: Vec3, rotation: Vec3) {
        if !self.hull_parts.is_empty() {
            let [x, y, z] = AXES.map(|axis| self.rotate_from_home(axis, rotation));
            let direction = |p: Vec3| x * p.0 + y * p.1 + z * p.2;
            let point = |p| self.pivot + offset + direction(p - self.pivot);
            world.set_compound_hull(
                id,
                self.hull_parts
                    .iter()
                    .map(|part| part.transformed(point, direction))
                    .collect(),
            );
            return;
        }
        let (min, max) = self.bounds_at(offset, rotation);
        if self.hull_planes.is_empty() {
            world.set_box(id, min, max);
            return;
        }
        let [x, y, z] = AXES.map(|axis| self.rotate_from_home(axis, rotation));
        world.set_hull(
            id,
            min,
            max,
            self.hull_planes.iter().map(|plane| {
                let normal = x * plane.normal.0 + y * plane.normal.1 + z * plane.normal.2;
                let distance =
                    plane.distance - plane.normal.dot(self.pivot) + normal.dot(self.pivot + offset);
                Plane { normal, distance }
            }),
        );
    }

    #[cfg(windows)]
    pub fn scene_event(&mut self, event: usize) {
        if self.tag == Some(event) {
            self.scene_progress = Some(0.0);
        }
    }

    #[cfg(windows)]
    pub fn tick_scene(&mut self, dt: f32) {
        let Some(progress) = self.scene_progress else {
            return;
        };
        let last = self.scene_keys.len().saturating_sub(1);
        let progress = (progress + dt / self.time).min(last as f32);
        let index = progress.floor() as usize;
        let fraction = progress.fract();
        let fraction = if self.glide {
            fraction * fraction * (3.0 - 2.0 * fraction)
        } else {
            fraction
        };
        let (a, ar) = self.scene_keys[index];
        let (b, br) = self.scene_keys[(index + 1).min(last)];
        self.offset = a + (b - a) * fraction;
        self.scene_rotation = ar + (br - ar) * fraction;
        self.scene_progress = Some(progress);
    }

    pub fn load(
        package: &Package<'_>,
        actors: &[&Export],
        path: Option<&Path>,
        world: &mut Collision,
    ) -> io::Result<Vec<Self>> {
        let triggers = read_triggers(package, actors)?;
        let mut brushes = Vec::new();
        for &actor in actors {
            if !matches!(
                package.object_name(actor.class),
                Some("Mover" | "AttachMover" | "GradualMover")
            ) {
                continue;
            }
            if let Some(brush) = Brush::read(package, actor, &triggers)? {
                brushes.push(brush);
            }
        }
        let textures = path
            .map(|path| {
                load_materials(
                    package,
                    path,
                    brushes
                        .iter()
                        .flat_map(|brush| brush.polys.iter().map(|(material, _)| *material)),
                    true,
                )
            })
            .unwrap_or_default();
        Ok(brushes
            .into_iter()
            .map(|brush| {
                let faces = brush
                    .polys
                    .into_iter()
                    .map(|(material, mut face)| {
                        face.texture = textures.get(&material).cloned();
                        (material, face)
                    })
                    .filter_map(|(material, face)| {
                        // Untextured polygons of an attachment brush are its
                        // carrier hull. Its visible geometry is the attached actor.
                        (face.flags & 1 == 0 && (material != 0 || brush.mover.attach_tag.is_none()))
                            .then_some(face)
                    })
                    .collect();
                let mut mover = brush.mover;
                if brush.solid {
                    let hull = world.add_box(mover.min, mover.max);
                    mover.hull = Some(hull);
                    if !mover.hull_planes.is_empty() || !mover.hull_parts.is_empty() {
                        mover.set_collision(world, hull, mover.offset, mover.scene_rotation);
                    }
                }
                mover.faces = faces;
                mover
            })
            .collect())
    }

    pub fn tick(&mut self, world: &mut Collision, walker: &mut Walker, dt: f32) {
        self.tick_many(world, &mut [walker], dt, None);
    }

    pub(crate) fn lift_ride_seconds(&self) -> Option<f32> {
        (self.hull.is_some()
            && self.travel.2.abs() > 45.0
            && matches!(self.activation, Activation::Stand))
        .then_some(self.time)
    }

    pub(crate) fn lift_board_position(&self, stop: LiftStop) -> Vec3 {
        let center = (self.min + self.max) * 0.5;
        let home = Vec3(
            center.0,
            center.1,
            self.max.2 + crate::collision::PLAYER_HEIGHT,
        );
        if stop == LiftStop::Home {
            home
        } else {
            home + self.travel
        }
    }

    pub(crate) fn lift_at(&self, stop: LiftStop) -> bool {
        self.fraction == if stop == LiftStop::Home { 0.0 } else { 1.0 }
    }

    pub(crate) fn has_rider(&self, walker: &Walker) -> bool {
        self.hull.is_some() && walker.on_box(self.min + self.offset, self.max + self.offset)
    }

    pub(crate) fn reset_motion(&mut self, world: &mut Collision) {
        self.offset = Vec3::default();
        self.scene_rotation = Vec3::default();
        self.fraction = 0.0;
        self.countdown = 0.0;
        self.opening = false;
        self.trigger_touching = false;
        self.ready = true;
        self.signaled = None;
        self.control_sources = 0;
        if let Some(hull) = self.hull {
            self.set_collision(world, hull, self.offset, self.scene_rotation);
        }
    }

    pub(crate) fn tick_many(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
        dt: f32,
        service: Option<LiftStop>,
    ) {
        self.tick_motion(world, walkers, &mut [], dt, service, false);
    }

    pub(crate) fn tick_with_cargo(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
        cargo: &mut [Cargo<'_>],
        dt: f32,
        service: Option<LiftStop>,
    ) {
        self.tick_motion(world, walkers, cargo, dt, service, false);
    }

    pub(crate) fn signal(&mut self, tag: usize, attacker: bool) {
        if self.tag == Some(tag) {
            self.trigger(attacker);
        }
    }

    pub(crate) fn trigger(&mut self, attacker: bool) {
        if !matches!(
            self.activation,
            Activation::Trigger | Activation::Control | Activation::Toggle
        ) || (self.trigger_once && self.signaled.is_some())
        {
            return;
        }
        self.signaled = Some(attacker);
        if matches!(self.activation, Activation::Control) {
            self.control_sources = self.control_sources.saturating_add(1);
        }
        self.opening = if matches!(self.activation, Activation::Toggle) {
            !self.opening
        } else {
            true
        };
        self.countdown = self.pause;
    }

    pub(crate) fn release_signal(&mut self, tag: usize) {
        if self.tag == Some(tag)
            && matches!(self.activation, Activation::Control)
            && !self.trigger_once
        {
            self.control_sources = self.control_sources.saturating_sub(1);
            self.opening = self.control_sources > 0;
        }
    }

    pub(crate) fn damage_event(&self, hull: usize, damage: u16) -> Option<usize> {
        self.tag.filter(|_| {
            self.hull == Some(hull)
                && self
                    .damage_threshold
                    .is_some_and(|threshold| f32::from(damage) >= threshold)
        })
    }

    pub(crate) fn tick_scripted(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
        dt: f32,
        service: Option<LiftStop>,
    ) -> Option<(usize, bool)> {
        let before = self.fraction;
        self.tick_motion(world, walkers, &mut [], dt, service, true);
        self.event
            .filter(|_| before != 1.0 && self.fraction == 1.0)
            .map(|event| (event, self.signaled.unwrap_or(false)))
    }

    fn tick_motion(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
        cargo: &mut [Cargo<'_>],
        dt: f32,
        service: Option<LiftStop>,
        scripted: bool,
    ) {
        let (min, max) = self.bounds_at(self.offset, self.scene_rotation);
        let on_board = |walker: &Walker| self.hull.is_some() && walker.on_box(min, max);
        let requested = match self.activation {
            Activation::Stand => walkers.iter().any(|walker| on_board(walker)),
            Activation::Bump => {
                self.hull.is_some()
                    && walkers
                        .iter()
                        .any(|walker| walker.touches_box(min, max, 1.0))
            }
            Activation::Trigger | Activation::Control | Activation::Toggle if !scripted => walkers
                .iter()
                .any(|walker| self.triggers.iter().any(|trigger| trigger.touches(walker))),
            _ => false,
        };
        if let Some(stop) = service {
            self.opening = stop == LiftStop::Away;
            if self.fraction == 1.0 && self.opening {
                self.countdown = self.pause;
            }
            if self.fraction == 0.0 {
                self.ready = true;
            }
        } else if !scripted && matches!(self.activation, Activation::Control | Activation::Toggle) {
            let entered = requested && !self.trigger_touching;
            self.trigger_touching = requested;
            if matches!(self.activation, Activation::Control) {
                self.opening = requested || (self.trigger_once && self.signaled.is_some());
            } else if entered && (!self.trigger_once || self.signaled.is_none()) {
                self.opening = !self.opening;
            }
            if entered && self.trigger_once {
                self.signaled = Some(false);
            }
        } else if self.fraction == 0.0 {
            self.ready |= !requested;
            if requested && self.ready {
                self.opening = true;
                self.ready = false;
            }
        } else if self.fraction == 1.0
            && !(scripted
                && self.signaled.is_some()
                && (self.trigger_once
                    || matches!(self.activation, Activation::Toggle | Activation::Control)))
        {
            self.countdown -= dt;
            if self.countdown <= 0.0 {
                self.opening = false;
            }
        }

        let direction = if self.opening { 1.0 } else { -1.0 };
        let next = (self.fraction + direction * dt / self.time).clamp(0.0, 1.0);
        if next == self.fraction {
            return;
        }
        let fraction = if self.glide {
            next * next * (3.0 - 2.0 * next)
        } else {
            next
        };
        let offset = self.travel * fraction;
        #[cfg(windows)]
        let rotation = self
            .scene_keys
            .get(1)
            .map_or(Vec3::default(), |(_, angle)| *angle * fraction);
        #[cfg(not(windows))]
        let rotation = Vec3::default();
        let delta = offset - self.offset;
        if let Some(id) = self.hull {
            let (new_min, new_max) = self.bounds_at(offset, rotation);
            if walkers
                .iter()
                .any(|walker| !on_board(walker) && walker.touches_box(new_min, new_max, -0.5))
            {
                self.opening = !self.opening;
                return;
            }
            let mut carried: Vec<_> = cargo
                .iter()
                .map(|item| item.supported_by(world, id))
                .collect();
            for _ in 0..cargo.len() {
                let mut changed = false;
                for (index, item) in cargo.iter().enumerate() {
                    if !carried[index]
                        && cargo
                            .iter()
                            .zip(&carried)
                            .any(|(base, moved)| *moved && item.rests_on(base))
                    {
                        carried[index] = true;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            for (item, &rider) in cargo.iter().zip(&carried) {
                if rider {
                    if !world.has_clear_cargo_path(
                        *item.position,
                        delta,
                        item.radius,
                        item.height,
                        id,
                    ) || cargo.iter().zip(&carried).any(|(other, &other_rider)| {
                        !other_rider && item.overlaps_cargo(other, delta)
                    }) {
                        return;
                    }
                } else if item.overlaps(new_min, new_max) {
                    self.opening = !self.opening;
                    return;
                }
            }
            let riders: Vec<_> = walkers
                .iter()
                .enumerate()
                .filter(|(_, walker)| on_board(walker))
                .map(|(index, walker)| (index, walker.position))
                .collect();
            self.set_collision(world, id, offset, rotation);
            for &(index, position) in &riders {
                let walker = &mut walkers[index];
                walker.carry(world, delta);
                let displacement = walker.position - position;
                if (displacement - delta).dot(displacement - delta) > 1.0 {
                    self.set_collision(world, id, self.offset, self.scene_rotation);
                    for (index, position) in riders {
                        walkers[index].position = position;
                    }
                    return;
                }
            }
            for (item, rider) in cargo.iter_mut().zip(carried) {
                if rider {
                    *item.position = *item.position + delta;
                }
            }
        }
        self.offset = offset;
        self.scene_rotation = rotation;
        self.fraction = next;
        if next == 1.0 {
            self.countdown = self.pause;
        }
    }

    fn bounds_at(&self, offset: Vec3, rotation: Vec3) -> (Vec3, Vec3) {
        if rotation.dot(rotation) <= f32::EPSILON {
            return (self.min + offset, self.max + offset);
        }
        let mut min = Vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = min * -1.0;
        let [x, y, z] = AXES.map(|axis| self.rotate_from_home(axis, rotation));
        for &point in &self.hull_vertices {
            let relative = point - self.pivot;
            let point = self.pivot + offset + x * relative.0 + y * relative.1 + z * relative.2;
            min = Vec3(min.0.min(point.0), min.1.min(point.1), min.2.min(point.2));
            max = Vec3(max.0.max(point.0), max.1.max(point.1), max.2.max(point.2));
        }
        (min, max)
    }
}

struct Brush {
    mover: Mover,
    solid: bool,
    polys: Vec<(i32, Face)>,
}

impl Brush {
    fn read(
        package: &Package<'_>,
        actor: &Export,
        triggers: &[Trigger],
    ) -> io::Result<Option<Self>> {
        let mut props = package.properties(actor)?;
        let mut location = Vec3::default();
        let mut pivot = Vec3::default();
        let mut scale = Vec3(1.0, 1.0, 1.0);
        let mut rotation = Vec3::default();
        let mut travel = Vec3::default();
        let mut reference = 0;
        let mut time: f32 = 1.0;
        let mut pause: f32 = 4.0;
        let mut glide = true;
        let mut tag = None;
        let mut attach_tag = None;
        let mut hidden = false;
        let mut activation = Activation::Bump;
        #[cfg(windows)]
        let mut keys = [(Vec3::default(), Vec3::default()); 8];
        #[cfg(windows)]
        let mut key_count = 2;
        let mut event = None;
        let mut trigger_once = false;
        let mut damage_triggered = false;
        let mut damage_threshold = 0.0;
        while let Some(property) = props.next_tag()? {
            let mut value = property.value;
            match property.name {
                "Location" => location = Vec3::read(&mut value)?,
                "PrePivot" => pivot = Vec3::read(&mut value)?,
                "MainScale" => scale = Vec3::read(&mut value)?,
                "Rotation" => rotation = read_rotation(&mut value)?,
                "Brush" => reference = value.compact()?,
                "KeyPos" => {
                    let position = Vec3::read(&mut value)?;
                    if property.array_index == 1 {
                        travel = position;
                    }
                    #[cfg(windows)]
                    if let Some(key) = keys.get_mut(property.array_index) {
                        key.0 = position;
                        key_count = key_count.max(property.array_index + 1);
                    }
                }
                #[cfg(windows)]
                "KeyRot" if property.array_index < keys.len() => {
                    let turns = read_rotation(&mut value)?;
                    let wrap = |angle| {
                        if angle > std::f32::consts::PI {
                            angle - std::f32::consts::TAU
                        } else {
                            angle
                        }
                    };
                    keys[property.array_index].1 =
                        Vec3(wrap(turns.0), wrap(turns.1), wrap(turns.2));
                    key_count = key_count.max(property.array_index + 1);
                }
                #[cfg(windows)]
                "NumKeys" => key_count = key_count.max(usize::from(value.byte()?).min(keys.len())),
                "MoveTime" => time = value.f32()?,
                "StayOpenTime" => pause = value.f32()?,
                "MoverGlideType" => glide = value.byte()? != 0,
                "Tag" => tag = Some(value.index()?),
                "Event" => {
                    let index = value.index()?;
                    event = (package.name(index)? != "None").then_some(index);
                }
                "bTriggerOnceOnly" => trigger_once = property.boolean.unwrap_or(false),
                "bDamageTriggered" => damage_triggered = property.boolean.unwrap_or(false),
                "DamageThreshold" => damage_threshold = value.f32()?,
                "AttachTag" => {
                    let index = value.index()?;
                    attach_tag = (package.name(index)? != "None").then_some(index);
                }
                "bHidden" => hidden = property.boolean.unwrap_or(false),
                "InitialState" => {
                    activation = match package.name(value.index()?)? {
                        "StandOpenTimed" => Activation::Stand,
                        "BumpOpenTimed" => Activation::Bump,
                        "TriggerOpenTimed" => Activation::Trigger,
                        "TriggerControl" => Activation::Control,
                        "TriggerToggle" => Activation::Toggle,
                        _ => Activation::Unsupported,
                    };
                }
                _ => {}
            }
        }
        let Some(brush) = usize::try_from(reference)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| package.exports.get(index))
        else {
            return Ok(None);
        };
        let Some(mut reader) = brush_polys(package, brush)? else {
            return Ok(None);
        };
        let count = reader.u32()? as usize;
        reader.u32()?;
        if count > reader.remaining.len() / 60 {
            return Err(invalid("Invalid brush polygon count"));
        }
        let mut polys = Vec::new();
        let mut min = Vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = min * -1.0;
        let mut solid = false;
        let transform = |point: Vec3| rotate(product(point - pivot, scale), rotation) + location;
        let reciprocal = Vec3(inv(scale.0), inv(scale.1), inv(scale.2));
        for _ in 0..count {
            let vertex_count = reader.count(12)?;
            let origin = transform(Vec3::read(&mut reader)?);
            let normal = unit(rotate(
                product(Vec3::read(&mut reader)?, reciprocal),
                rotation,
            ));
            let u_axis = rotate(product(Vec3::read(&mut reader)?, reciprocal), rotation);
            let v_axis = rotate(product(Vec3::read(&mut reader)?, reciprocal), rotation);
            let mut vertices = Vec::new();
            for _ in 0..vertex_count {
                let point = transform(Vec3::read(&mut reader)?);
                min = Vec3(min.0.min(point.0), min.1.min(point.1), min.2.min(point.2));
                max = Vec3(max.0.max(point.0), max.1.max(point.1), max.2.max(point.2));
                vertices.push(point);
            }
            let flags = reader.u32()?;
            reader.compact()?; // Brush actor.
            let material = reader.compact()?;
            reader.compact()?; // Item name.
            reader.compact()?; // Link index.
            reader.compact()?; // Brush polygon index.
            let pan = [reader.u16()? as i16 as f32, reader.u16()? as i16 as f32];
            if vertices.len() >= 3 {
                solid |= flags & 8 == 0;
                polys.push((
                    material,
                    Face {
                        mesh_triangle: None,
                        vertices,
                        normal,
                        origin,
                        u_axis,
                        v_axis,
                        pan,
                        flags: flags | u32::from(hidden),
                        color: material_color(package.object_name(material)),
                        texture: None,
                        lightmap: None,
                        sky: false,
                    },
                ));
            }
        }
        if polys.is_empty() {
            return Ok(None);
        }
        let triggers = triggers
            .iter()
            .filter(|trigger| Some(trigger.event) == tag)
            .copied()
            .collect();
        #[cfg(windows)]
        let rotates = keys
            .get(1)
            .is_some_and(|(_, key)| key.dot(*key) > f32::EPSILON);
        #[cfg(not(windows))]
        let rotates = false;
        let hull_vertices: Vec<_> = if solid && rotates {
            polys
                .iter()
                .flat_map(|(_, face)| face.vertices.iter().copied())
                .collect()
        } else {
            Vec::new()
        };
        let hull_planes = if !hull_vertices.is_empty() {
            convex_planes(&polys, &hull_vertices)
        } else {
            Vec::new()
        };
        let uses_scripted_motion = matches!(
            activation,
            Activation::Trigger | Activation::Control | Activation::Toggle
        );
        let hull_parts =
            if solid && uses_scripted_motion && hull_planes.is_empty() && polys.len() > 6 {
                let parts = crate::map::brush_collision(package, brush)?;
                if parts.len() > 1 {
                    parts
                        .iter()
                        .map(|part| {
                            part.transformed(transform, |normal| {
                                unit(rotate(product(normal, reciprocal), rotation))
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
        Ok(Some(Self {
            solid,
            polys,
            mover: Mover {
                faces: Vec::new(),
                offset: Vec3::default(),
                scene_rotation: Vec3::default(),
                pivot: location,
                home_rotation: rotation,
                hull_vertices,
                hull_planes,
                hull_parts,
                min,
                max,
                travel,
                time: time.max(0.05),
                pause: pause.max(0.0),
                glide,
                fraction: 0.0,
                countdown: 0.0,
                opening: false,
                trigger_touching: false,
                ready: true,
                hull: None,
                triggers,
                activation,
                tag,
                event,
                trigger_once,
                signaled: None,
                control_sources: 0,
                damage_threshold: damage_triggered.then_some(damage_threshold),
                attach_tag,
                #[cfg(windows)]
                scene_keys: keys[..key_count].to_vec(),
                #[cfg(windows)]
                scene_progress: None,
            },
        }))
    }
}

fn read_triggers(package: &Package<'_>, actors: &[&Export]) -> io::Result<Vec<Trigger>> {
    let mut triggers = Vec::new();
    for &actor in actors {
        if package.object_name(actor.class) != Some("Trigger") {
            continue;
        }
        let mut location = Vec3::default();
        let mut event = None;
        let mut radius: f32 = 40.0;
        let mut height: f32 = 40.0;
        let mut props = package.properties(actor)?;
        while let Some((name, mut value)) = props.next()? {
            match name {
                "Location" => location = Vec3::read(&mut value)?,
                "Event" => event = Some(value.index()?),
                "CollisionRadius" => radius = value.f32()?,
                "CollisionHeight" => height = value.f32()?,
                _ => {}
            }
        }
        if let Some(event) = event {
            triggers.push(Trigger {
                event,
                position: location,
                radius: radius.max(0.0),
                height: height.max(0.0),
            });
        }
    }
    Ok(triggers)
}

pub(crate) fn read_rotation(reader: &mut Reader<'_>) -> io::Result<Vec3> {
    let mut read =
        || Ok::<_, io::Error>((reader.u32()? as u16) as f32 * std::f32::consts::TAU / 65536.0);
    Ok(Vec3(read()?, read()?, read()?))
}

pub(crate) fn rotate(point: Vec3, rotation: Vec3) -> Vec3 {
    let (sr, cr) = rotation.2.sin_cos();
    let (sp, cp) = rotation.0.sin_cos();
    let (sy, cy) = rotation.1.sin_cos();
    let Vec3(x, y, z) = Vec3(
        point.0,
        point.1 * cr + point.2 * sr,
        point.2 * cr - point.1 * sr,
    );
    let Vec3(x, y, z) = Vec3(x * cp - z * sp, y, x * sp + z * cp);
    Vec3(x * cy - y * sy, x * sy + y * cy, z)
}

const AXES: [Vec3; 3] = [
    Vec3(1.0, 0.0, 0.0),
    Vec3(0.0, 1.0, 0.0),
    Vec3(0.0, 0.0, 1.0),
];

fn convex_planes(polys: &[(i32, Face)], vertices: &[Vec3]) -> Vec<Plane> {
    let center = vertices.iter().copied().fold(Vec3::default(), |a, b| a + b)
        * (vertices.len() as f32).recip();
    let mut planes = Vec::with_capacity(polys.len());
    for (_, face) in polys {
        let on_plane = face.vertices[0];
        let mut normal = face.normal;
        if normal.dot(normal) < 0.5 {
            return Vec::new();
        }
        if normal.dot(center - on_plane) > 0.0 {
            normal = normal * -1.0;
        }
        if vertices.iter().any(|&p| normal.dot(p - on_plane) > 0.1) {
            return Vec::new();
        }
        planes.push(Plane {
            normal,
            distance: normal.dot(on_plane),
        });
    }
    planes
}

fn rotate_from_home(direction: Vec3, home: Vec3, key: Vec3) -> Vec3 {
    if key.dot(key) <= f32::EPSILON {
        return direction;
    }
    // KeyRot offsets the actor's home rotator. The mesh vertices are already
    // baked in that home orientation, so undo it before applying the new pose.
    let [x, y, z] = AXES.map(|axis| rotate(axis, home));
    let local = Vec3(direction.dot(x), direction.dot(y), direction.dot(z));
    rotate(local, home + key)
}

pub(crate) fn product(a: Vec3, b: Vec3) -> Vec3 {
    Vec3(a.0 * b.0, a.1 * b.1, a.2 * b.2)
}
pub(crate) fn unit(a: Vec3) -> Vec3 {
    a * inv(a.dot(a).sqrt())
}
fn inv(value: f32) -> f32 {
    if value.abs() > 0.00001 {
        value.recip()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Map;

    #[test]
    #[ignore = "requires local CTF map packages"]
    fn niven_and_coret_doors_stay_open_until_their_trigger_is_clear() {
        for name in ["CTF-Niven.unr", "CTF-Coret.unr"] {
            let path = Path::new("local/reference-client/Maps").join(name);
            let bytes = std::fs::read(path).unwrap();
            let mut map = Map::load(&Package::read(&bytes).unwrap(), None).unwrap();
            let mover = map
                .movers
                .iter_mut()
                .find(|mover| {
                    matches!(mover.activation, Activation::Control) && !mover.triggers.is_empty()
                })
                .expect("trigger-controlled door");
            let mut walker = mover
                .triggers
                .iter()
                .find_map(|trigger| {
                    [
                        Vec3(0.0, 0.0, 0.0),
                        Vec3(0.8, 0.0, 0.0),
                        Vec3(-0.8, 0.0, 0.0),
                        Vec3(0.0, 0.8, 0.0),
                        Vec3(0.0, -0.8, 0.0),
                    ]
                    .into_iter()
                    .find_map(|offset| {
                        let walker = Walker::spawn(
                            &map.collision,
                            trigger.position + offset * trigger.radius,
                        );
                        (trigger.touches(&walker)
                            && !walker.touches_box(mover.min, mover.max, -0.5))
                        .then_some(walker)
                    })
                })
                .expect("clear position inside the door trigger");
            let seconds = mover.time + mover.pause + 1.0;
            for _ in 0..(seconds * 120.0).ceil() as usize {
                mover.tick(&mut map.collision, &mut walker, 1.0 / 120.0);
            }
            assert!(mover.lift_at(LiftStop::Away), "{name}");
            for _ in 0..((mover.time + 0.1) * 120.0).ceil() as usize {
                mover.tick_many(&mut map.collision, &mut [], 1.0 / 120.0, None);
            }
            assert!(mover.lift_at(LiftStop::Home), "{name}");
        }
    }

    #[test]
    #[ignore = "requires the local GOTY Deck16 map"]
    fn deck16_lift_carries_player() {
        let bytes = std::fs::read("local/goty/Maps/DM-Deck16][.unr").unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, None).unwrap();
        let lift = map
            .movers
            .iter_mut()
            .find(|mover| mover.travel.2 > 64.0)
            .expect("Deck16 lift");
        let center = (lift.min + lift.max) * 0.5;
        let initial = Vec3(center.0, center.1, lift.max.2 + 39.2);
        let mut walker = Walker::spawn(&map.collision, initial);
        assert!(walker.grounded, "{:?}", walker.position);
        let initial = walker.position;
        let mut highest = initial.2;
        for _ in 0..120 {
            lift.tick(&mut map.collision, &mut walker, 1.0 / 60.0);
            walker.advance(&map.collision, Vec3::default(), None, 1.0 / 60.0);
            highest = highest.max(walker.position.2);
        }
        assert!(
            highest > initial.2 + 64.0,
            "initial={initial:?} highest={highest} travel={:?} offset={:?} triggers={:?}",
            lift.travel,
            lift.offset,
            lift.triggers
        );
    }

    #[test]
    #[ignore = "requires the local GOTY Deck16 map"]
    fn deck16_lift_carries_a_stack_and_stops_before_crushing_it() {
        let bytes = std::fs::read("local/goty/Maps/DM-Deck16][.unr").unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, None).unwrap();
        let lift = map
            .movers
            .iter_mut()
            .find(|mover| mover.travel.2 > 64.0)
            .unwrap();
        let center = (lift.min + lift.max) * 0.5;
        let mut lower = Vec3(center.0, center.1, lift.max.2 + 6.2);
        let mut upper = lower + Vec3(0.0, 0.0, 12.2);
        let origin = [lower, upper];
        // A short ceiling is placed partway through the lift's travel.
        let ceiling = upper.2 + 30.0;
        map.collision.add_box(
            Vec3(center.0 - 20.0, center.1 - 20.0, ceiling),
            Vec3(center.0 + 20.0, center.1 + 20.0, ceiling + 8.0),
        );
        let mut cargo = [
            Cargo {
                position: &mut lower,
                radius: 6.0,
                height: 6.0,
                resting: true,
            },
            Cargo {
                position: &mut upper,
                radius: 6.0,
                height: 6.0,
                resting: true,
            },
        ];
        for _ in 0..240 {
            lift.tick_with_cargo(
                &mut map.collision,
                &mut [],
                &mut cargo,
                1.0 / 120.0,
                Some(LiftStop::Away),
            );
        }
        assert!(
            lift.offset.2 > 18.0 && lift.offset.2 <= 24.0,
            "{:?}",
            lift.offset
        );
        for (item, start) in cargo.iter().zip(origin) {
            let error = *item.position - start - lift.offset;
            assert!(error.dot(error) < 0.01);
        }
        assert!(cargo[1].position.2 + cargo[1].height <= ceiling);
        for _ in 0..240 {
            lift.tick_with_cargo(
                &mut map.collision,
                &mut [],
                &mut cargo,
                1.0 / 120.0,
                Some(LiftStop::Home),
            );
        }
        assert!(lift.offset.dot(lift.offset) < 0.01);
        for (item, start) in cargo.iter().zip(origin) {
            let error = *item.position - start;
            assert!(error.dot(error) < 0.01);
        }
    }
}
