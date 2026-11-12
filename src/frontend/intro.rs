use std::{collections::HashMap, fs, io, path::Path, sync::Arc};

use crate::{
    collision::Walker,
    map::{Map, Vec3},
    package::{Package, invalid},
    render::Camera,
    sound::Wave,
};

struct Point {
    index: u32,
    position: Vec3,
    rotation: Vec3,
    rate: f32,
    speed: f32,
    fov: f32,
    flash: f32,
    skip_next: bool,
    end: bool,
}

pub struct Shot {
    pub camera: Camera,
    pub fov: f32,
    pub roll: f32,
    pub flash: f32,
}

struct Trigger {
    position: Vec3,
    radius: f32,
    height: f32,
    event: usize,
    fired: bool,
}

pub struct Intro {
    pub map: Map,
    points: Vec<Point>,
    triggers: Vec<Trigger>,
    voices: HashMap<usize, Arc<Wave>>,
    dispatchers: HashMap<usize, Vec<(f32, usize)>>,
    scheduled: Vec<(f32, usize)>,
    index: usize,
    alpha: f32,
    pub time: f32,
}

impl Intro {
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let package = Package::read(&bytes)?;
        let mut points = Vec::new();
        let mut triggers = Vec::new();
        let mut voices = HashMap::new();
        let mut dispatchers = HashMap::new();
        let path_tag = package.names.iter().position(|name| name == "Path");
        for actor in &package.exports {
            let Some(class @ ("InterpolationPoint" | "Trigger" | "SpecialEvent" | "Dispatcher")) =
                package.object_name(actor.class)
            else {
                continue;
            };
            let mut props = package.properties(actor)?;
            let mut point = Point {
                index: 0,
                position: Vec3::default(),
                rotation: Vec3::default(),
                rate: 1.0,
                speed: 1.0,
                fov: 1.0,
                flash: 1.0,
                skip_next: false,
                end: false,
            };
            let (mut tag, mut event, mut voice) = (None, None, 0);
            let (mut radius, mut height) = (40.0, 40.0);
            let mut out_events = [None; 8];
            let mut out_delays = [0.0; 8];
            while let Some(property) = props.next_tag()? {
                let mut value = property.value;
                match property.name {
                    "Position" => point.index = value.u32()?,
                    "Location" => point.position = Vec3::read(&mut value)?,
                    "Rotation" => point.rotation = crate::mover::read_rotation(&mut value)?,
                    "Tag" => tag = Some(value.index()?),
                    "Event" => event = Some(value.index()?),
                    "RateModifier" => point.rate = value.f32()?,
                    "GameSpeedModifier" => point.speed = value.f32()?,
                    "FovModifier" => point.fov = value.f32()?,
                    "ScreenFlashScale" => point.flash = value.f32()?,
                    "bSkipNextPath" => point.skip_next = property.boolean.unwrap_or(false),
                    "bEndOfPath" => point.end = property.boolean.unwrap_or(false),
                    "CollisionRadius" => radius = value.f32()?,
                    "CollisionHeight" => height = value.f32()?,
                    "Sound" => voice = value.compact()?,
                    "OutEvents" if property.array_index < out_events.len() => {
                        let index = value.index()?;
                        out_events[property.array_index] =
                            (package.name(index)? != "None").then_some(index);
                    }
                    "OutDelays" if property.array_index < out_delays.len() => {
                        out_delays[property.array_index] = value.f32()?.max(0.0);
                    }
                    _ => {}
                }
            }
            match class {
                "InterpolationPoint" if tag == path_tag => points.push(point),
                "Trigger" if let Some(event) = event => triggers.push(Trigger {
                    position: point.position,
                    radius,
                    height,
                    event,
                    fired: false,
                }),
                "SpecialEvent"
                    if let Some(tag) = tag
                        && let Some(voice) = load_voice(path, &package, voice) =>
                {
                    voices.insert(tag, voice);
                }
                "Dispatcher" if let Some(tag) = tag => {
                    let mut delay = 0.0;
                    let events = out_events
                        .into_iter()
                        .zip(out_delays)
                        .filter_map(|(event, next)| {
                            let event = event?;
                            delay += next;
                            Some((delay, event))
                        })
                        .collect();
                    dispatchers.insert(tag, events);
                }
                _ => {}
            }
        }
        points.sort_by_key(|point| point.index);
        points.dedup_by_key(|point| point.index);
        if points.len() < 2 || points[0].index != 0 {
            return Err(invalid("CityIntro camera path is missing"));
        }
        Ok(Self {
            map: Map::load(&package, Some(path))?,
            points,
            triggers,
            voices,
            dispatchers,
            scheduled: Vec::new(),
            index: 0,
            alpha: 0.0,
            time: 0.0,
        })
    }

    pub fn update(&mut self, dt: f32) -> Option<(Shot, Vec<Arc<Wave>>)> {
        self.time += dt;
        let mut left = dt;
        while left > 0.0 {
            let current = self.points.get(self.index)?;
            let next = self.points.get(self.index + 1)?;
            let rate = mix(current.rate, next.rate, self.alpha)
                * mix(current.speed, next.speed, self.alpha);
            let time = (1.0 - self.alpha) / rate.max(0.001);
            if left < time {
                self.alpha += left * rate;
                break;
            }
            if current.end {
                return None;
            }
            left -= time;
            self.index += 1;
            while self
                .points
                .get(self.index)
                .is_some_and(|point| point.skip_next)
            {
                self.index += 1;
            }
            self.alpha = 0.0;
        }
        let shot = self.shot()?;
        let position = shot.camera.position;
        let mut events = Vec::new();
        for trigger in self.triggers.iter_mut().filter(|trigger| !trigger.fired) {
            let delta = position - trigger.position;
            if delta.0.hypot(delta.1) <= trigger.radius + 17.0
                && delta.2.abs() <= trigger.height + 39.0
            {
                trigger.fired = true;
                events.push(trigger.event);
            }
        }
        self.scheduled.retain(|&(time, event)| {
            if time > self.time {
                true
            } else {
                events.push(event);
                false
            }
        });
        let mut voices = Vec::new();
        for event in events {
            voices.extend(self.voices.get(&event).cloned());
            if let Some(children) = self.dispatchers.get(&event) {
                self.scheduled.extend(
                    children
                        .iter()
                        .map(|&(delay, event)| (self.time + delay, event)),
                );
            }
            for prop in &mut self.map.props {
                if let Some(actor) = &mut prop.cinematic
                    && actor.event == event
                {
                    actor.started = if actor.started.is_some() {
                        None
                    } else {
                        Some(self.time)
                    };
                }
            }
            for mover in &mut self.map.movers {
                mover.scene_event(event);
            }
        }
        let mut viewer = Walker::spawn(&self.map.collision, position);
        viewer.position = position;
        for mover in &mut self.map.movers {
            mover.tick(&mut self.map.collision, &mut viewer, dt);
        }
        Some((shot, voices))
    }

    pub fn shot(&self) -> Option<Shot> {
        let a = self.points.get(self.index)?;
        let b = self.points.get(self.index + 1)?;
        let before = &self.points[self.index.saturating_sub(1)];
        let after = self.points.get(self.index + 2).unwrap_or(b);
        let t = self.alpha;
        let (r0, r1, r2, r3) = (before.rotation, a.rotation, b.rotation, after.rotation);
        let rotation = spline(
            r1 + wrapped(r0 - r1),
            r1,
            r1 + wrapped(r2 - r1),
            r1 + wrapped(r2 - r1) + wrapped(r3 - r2),
            t,
        );
        Some(Shot {
            camera: Camera {
                position: spline(before.position, a.position, b.position, after.position, t),
                yaw: rotation.1,
                pitch: rotation.0,
            },
            fov: 90.0 * mix(a.fov, b.fov, t),
            roll: rotation.2,
            flash: mix(a.flash, b.flash, t),
        })
    }
}

pub fn logo_camera(package: &Package<'_>, map: &Map) -> io::Result<Camera> {
    let Some(actor) = package
        .exports
        .iter()
        .find(|actor| package.object_name(actor.class) == Some("SpectatorCam"))
    else {
        return Ok(Camera::from_start(map.starts[0]));
    };
    let mut camera = Camera {
        position: Vec3::default(),
        yaw: 0.0,
        pitch: 0.0,
    };
    let mut props = package.properties(actor)?;
    while let Some((name, mut value)) = props.next()? {
        match name {
            "Location" => camera.position = Vec3::read(&mut value)?,
            "Rotation" => {
                let rotation = crate::mover::read_rotation(&mut value)?;
                camera.yaw = rotation.1;
                camera.pitch = rotation.0;
            }
            _ => {}
        }
    }
    Ok(camera)
}

fn load_voice(map_path: &Path, package: &Package<'_>, reference: i32) -> Option<Arc<Wave>> {
    let (name, path) = crate::texture::import_path(package, reference)?;
    let sounds = map_path.parent()?.join("../Sounds");
    let file = fs::read_dir(sounds)
        .ok()?
        .filter_map(Result::ok)
        .find(|file| {
            let file = file.path();
            file.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("uax"))
                && file
                    .file_stem()
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(&name))
        })?;
    let bytes = fs::read(file.path()).ok()?;
    let package = Package::read(&bytes).ok()?;
    let sound = package.exports.iter().find(|export| {
        package.object_name(export.class) == Some("Sound")
            && crate::texture::export_path(&package, export).as_ref() == Some(&path)
    })?;
    Wave::from_export(&package, sound).ok().map(Arc::new)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn wrapped(value: Vec3) -> Vec3 {
    let angle = |value: f32| {
        (value + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    };
    Vec3(angle(value.0), angle(value.1), angle(value.2))
}

fn spline(a: Vec3, b: Vec3, c: Vec3, d: Vec3, t: f32) -> Vec3 {
    b + (c - a) * (0.5 * t)
        + (a * 2.0 - b * 5.0 + c * 4.0 - d) * (0.5 * t * t)
        + (b * 3.0 - c * 3.0 + d - a) * (0.5 * t * t * t)
}
