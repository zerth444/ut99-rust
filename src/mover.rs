use std::{io, path::Path};

use crate::collision::{Collision, Walker};
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
    pub scene_rotation: Vec3,
    pub pivot: Vec3,
    min: Vec3,
    max: Vec3,
    travel: Vec3,
    time: f32,
    pause: f32,
    glide: bool,
    fraction: f32,
    countdown: f32,
    opening: bool,
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

impl LiftStop {
    pub fn opposite(self) -> Self {
        match self {
            Self::Home => Self::Away,
            Self::Away => Self::Home,
        }
    }
}

impl Mover {
    pub(crate) fn collision_hull(&self) -> Option<usize> {
        self.hull
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
                    mover.hull = Some(world.add_box(mover.min, mover.max));
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
        self.ready = true;
        self.signaled = None;
        self.control_sources = 0;
        if let Some(hull) = self.hull {
            world.set_box(hull, self.min, self.max);
        }
    }

    pub(crate) fn tick_many(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
        dt: f32,
        service: Option<LiftStop>,
    ) {
        self.tick_motion(world, walkers, dt, service, false);
    }

    pub(crate) fn signal(&mut self, tag: usize, attacker: bool) {
        if self.tag != Some(tag)
            || !matches!(
                self.activation,
                Activation::Trigger | Activation::Control | Activation::Toggle
            )
            || (self.trigger_once && self.signaled.is_some())
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
        self.tick_motion(world, walkers, dt, service, true);
        self.event
            .filter(|_| before != 1.0 && self.fraction == 1.0)
            .map(|event| (event, self.signaled.unwrap_or(false)))
    }

    fn tick_motion(
        &mut self,
        world: &mut Collision,
        walkers: &mut [&mut Walker],
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
            Activation::Trigger if !scripted => walkers
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
        let rotation = if scripted {
            self.scene_keys
                .get(1)
                .map_or(Vec3::default(), |(_, angle)| *angle * fraction)
        } else {
            Vec3::default()
        };
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
            let riders: Vec<_> = walkers
                .iter()
                .enumerate()
                .filter(|(_, walker)| on_board(walker))
                .map(|(index, walker)| (index, walker.position))
                .collect();
            world.set_box(id, new_min, new_max);
            for &(index, position) in &riders {
                let walker = &mut walkers[index];
                walker.carry(world, delta);
                let displacement = walker.position - position;
                if (displacement - delta).dot(displacement - delta) > 1.0 {
                    world.set_box(id, min, max);
                    for (index, position) in riders {
                        walkers[index].position = position;
                    }
                    return;
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
        for x in [self.min.0, self.max.0] {
            for y in [self.min.1, self.max.1] {
                for z in [self.min.2, self.max.2] {
                    let point = self.pivot + offset + rotate(Vec3(x, y, z) - self.pivot, rotation);
                    min = Vec3(min.0.min(point.0), min.1.min(point.1), min.2.min(point.2));
                    max = Vec3(max.0.max(point.0), max.1.max(point.1), max.2.max(point.2));
                }
            }
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
        Ok(Some(Self {
            solid,
            polys,
            mover: Mover {
                faces: Vec::new(),
                offset: Vec3::default(),
                scene_rotation: Vec3::default(),
                pivot: location,
                min,
                max,
                travel,
                time: time.max(0.05),
                pause: pause.max(0.0),
                glide,
                fraction: 0.0,
                countdown: 0.0,
                opening: false,
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
}
