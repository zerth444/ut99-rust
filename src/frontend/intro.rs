use std::{collections::HashMap, fs, io, path::Path, sync::Arc};

use crate::{
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
    previous_shot: Option<Vec3>,
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
        // BeginPlay resolves a Position by the first matching actor in level order.
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
            previous_shot: None,
            time: 0.0,
        })
    }

    pub fn update(&mut self, dt: f32) -> Option<(Shot, Vec<Arc<Wave>>)> {
        self.time += dt;
        let mut left = dt;
        let mut cut = false;
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
                cut = true;
            }
            self.alpha = 0.0;
        }
        let shot = self.shot()?;
        let position = shot.camera.position;
        let previous = self
            .previous_shot
            .replace(position)
            .filter(|_| !cut)
            .unwrap_or(position);
        let mut events = Vec::new();
        for trigger in self.triggers.iter_mut().filter(|trigger| !trigger.fired) {
            if trigger.touches_path(previous, position) {
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
        for mover in &mut self.map.movers {
            mover.tick_scene(dt);
        }
        Some((shot, voices))
    }

    pub fn shot(&self) -> Option<Shot> {
        let a = self.points.get(self.index)?;
        let b = self.points.get(self.index + 1)?;
        // InterpolationPoint links the last key to Position=0, even when
        // the front end ends playback at the end of this pass.
        let before = &self.points[(self.index + self.points.len() - 1) % self.points.len()];
        let after = &self.points[(self.index + 2) % self.points.len()];
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

impl Trigger {
    fn touches_path(&self, start: Vec3, end: Vec3) -> bool {
        let delta = end - start;
        let offset = self.position - start;
        let height = self.height + 39.0;
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        if delta.2.abs() < 0.001 {
            if offset.2.abs() > height {
                return false;
            }
        } else {
            let a = (offset.2 - height) / delta.2;
            let b = (offset.2 + height) / delta.2;
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
            if lo > hi {
                return false;
            }
        }
        let horizontal = delta.0 * delta.0 + delta.1 * delta.1;
        let along = if horizontal > 0.001 {
            ((offset.0 * delta.0 + offset.1 * delta.1) / horizontal).clamp(lo, hi)
        } else {
            lo
        };
        (offset.0 - delta.0 * along).hypot(offset.1 - delta.1 * along) <= self.radius + 17.0
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
    // UE1's four-point interpolation weights. The curve is pulled toward
    // each point, so the camera need not pass through the actor itself.
    let [w0, w1, w2, w3] = [t + 1.0, t, t - 1.0, t - 2.0].map(|offset| {
        let squared = offset * offset;
        let weight = 1.0 - 0.25 * squared;
        weight * weight
    });
    (a * w0 + b * w1 + c * w2 + d * w3) * (w0 + w1 + w2 + w3).recip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ue1_camera_rounds_corners_and_stays_continuous() {
        let a = Vec3(-100.0, 0.0, 0.0);
        let b = Vec3(0.0, 0.0, 0.0);
        let c = Vec3(0.0, 100.0, 0.0);
        let d = Vec3(100.0, 100.0, 0.0);
        let start = spline(a, b, c, d, 0.0);
        assert!(start.0 < -25.0 && start.1 > 25.0);
        let end = spline(a, b, c, d, 1.0);
        let next = spline(b, c, d, d, 0.0);
        assert!((end - next).dot(end - next) < 0.001);
    }

    #[test]
    fn scene_triggers_detect_a_fast_crossing() {
        let trigger = Trigger {
            position: Vec3::default(),
            radius: 5.0,
            height: 5.0,
            event: 0,
            fired: false,
        };
        assert!(trigger.touches_path(Vec3(-80.0, 0.0, 0.0), Vec3(80.0, 0.0, 0.0)));
        assert!(!trigger.touches_path(Vec3(-80.0, 0.0, 50.0), Vec3(80.0, 0.0, 50.0)));
    }
}
