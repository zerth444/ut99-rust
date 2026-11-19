use std::{io, sync::Arc};

use crate::{
    map::Vec3,
    package::{Export, Package, Reader},
    sound::Wave,
    texture::Texture,
};

use super::Volume;

#[derive(Default)]
pub(crate) struct Effects {
    pub mortars: Vec<Mortar>,
    pub charges: Vec<Charge>,
    pub timers: Vec<Timer>,
    pub quakes: Vec<Quake>,
    pub charge_image: Option<Arc<Texture>>,
    pub charge_sound: Option<Arc<Wave>>,
    pub mortar_mark: Option<(Arc<Texture>, f32, f32)>,
    pub walls: Vec<super::breakables::Breakable>,
}

pub(crate) struct Quake {
    pub position: Vec3,
    pub tag: usize,
    pub magnitude: Option<f32>,
    pub duration: Option<f32>,
    pub radius: Option<f32>,
    pub toss: Option<bool>,
}

pub(crate) struct Mortar {
    pub location: Vec3,
    pub rotation: Vec3,
    pub interval: f32,
    pub deviation: f32,
    pub speed: f32,
    pub damage: f32,
    pub momentum: f32,
    pub radius: f32,
    pub sound_ref: Option<i32>,
    pub sound: Option<Arc<Wave>>,
}

pub(crate) struct Charge {
    pub volume: Volume,
    pub tag: Option<usize>,
    pub damage: f32,
    pub momentum: f32,
    pub scale: f32,
    pub delay: f32,
    pub trigger_only: bool,
}

pub(crate) enum Timer {
    Fixed {
        event: usize,
        interval: f32,
        repeat: bool,
    },
    Random {
        tag: Option<usize>,
        events: Vec<usize>,
        min: f32,
        max: f32,
        probability: f32,
        always: bool,
    },
}

impl Effects {
    pub fn load(package: &Package<'_>, actors: &[&Export]) -> io::Result<Self> {
        let mut effects = Self::default();
        let read_name = |value: &mut Reader<'_>| -> io::Result<Option<usize>> {
            let index = value.index()?;
            Ok((!package.name(index)?.eq_ignore_ascii_case("None")).then_some(index))
        };
        for &actor in actors {
            let kind = package.object_name(actor.class).unwrap_or_default();
            if !matches!(
                kind,
                "MortarSpawner"
                    | "ExplosionChain"
                    | "TimedTrigger"
                    | "StochasticTrigger"
                    | "Earthquake"
                    | "ExplodingWall"
            ) {
                continue;
            }
            let mut values = package.properties(actor)?;
            match kind {
                "ExplodingWall" => effects
                    .walls
                    .push(super::breakables::Breakable::read(package, actor)?),
                "MortarSpawner" => {
                    let mut mortar = Mortar {
                        location: Vec3::default(),
                        rotation: Vec3::default(),
                        interval: 5.0,
                        deviation: 4096.0,
                        speed: 1000.0,
                        damage: 70.0,
                        momentum: 150_000.0,
                        radius: 400.0,
                        sound_ref: None,
                        sound: None,
                    };
                    let mut deviate = true;
                    while let Some(property) = values.next_tag()? {
                        let mut value = property.value;
                        match property.name {
                            "Location" => mortar.location = Vec3::read(&mut value)?,
                            "Rotation" => {
                                mortar.rotation = crate::mover::read_rotation(&mut value)?
                            }
                            "ShellDamage" => mortar.damage = (value.u32()? as i32).max(0) as f32,
                            "ShellMomentumTransfer" => {
                                mortar.momentum = (value.u32()? as i32).max(0) as f32
                            }
                            "ShellBlastRadius" => {
                                mortar.radius = (value.u32()? as i32).max(1) as f32
                            }
                            "RateOfFire" => mortar.interval = (value.u32()? as i32).max(0) as f32,
                            "Deviation" => {
                                mortar.deviation = (value.u32()? as i32).abs_diff(0) as f32
                            }
                            "ShellSpeed" => mortar.speed = value.f32()?.max(0.0),
                            "bDeviate" => deviate = property.boolean.unwrap_or(true),
                            "FireSound" => mortar.sound_ref = Some(value.compact()?),
                            _ => {}
                        }
                    }
                    if deviate {
                        mortar.deviation *= std::f32::consts::TAU / 131_072.0;
                    } else {
                        mortar.deviation = 0.0;
                    }
                    if mortar.interval > 0.0 {
                        effects.mortars.push(mortar);
                    }
                }
                "ExplosionChain" => {
                    let mut charge = Charge {
                        volume: Volume {
                            radius: 22.0,
                            height: 22.0,
                            ..Volume::default()
                        },
                        tag: None,
                        damage: 100.0,
                        momentum: 100_000.0,
                        scale: 1.0,
                        delay: 0.3,
                        trigger_only: false,
                    };
                    while let Some(property) = values.next_tag()? {
                        let mut value = property.value;
                        match property.name {
                            "Location" => charge.volume.position = Vec3::read(&mut value)?,
                            "CollisionRadius" => charge.volume.radius = value.f32()?.max(0.0),
                            "CollisionHeight" => charge.volume.height = value.f32()?.max(0.0),
                            "Tag" => charge.tag = read_name(&mut value)?,
                            "Damage" => charge.damage = value.f32()?.max(0.0),
                            "MomentumTransfer" => charge.momentum = value.f32()?.max(0.0),
                            "Size" => charge.scale = value.f32()?.max(0.0),
                            "Delaytime" | "DelayTime" => charge.delay = value.f32()?.max(0.0),
                            "bOnlyTriggerable" => {
                                charge.trigger_only = property.boolean.unwrap_or(false)
                            }
                            _ => {}
                        }
                    }
                    effects.charges.push(charge);
                }
                "Earthquake" => {
                    let mut tag = None;
                    let mut quake = Quake {
                        position: Vec3::default(),
                        tag: 0,
                        magnitude: None,
                        duration: None,
                        radius: None,
                        toss: None,
                    };
                    while let Some(property) = values.next_tag()? {
                        let mut value = property.value;
                        match property.name {
                            "Tag" => tag = read_name(&mut value)?,
                            "Location" => quake.position = Vec3::read(&mut value)?,
                            name if name.eq_ignore_ascii_case("Magnitude") => {
                                quake.magnitude = Some(value.f32()?.max(0.0))
                            }
                            name if name.eq_ignore_ascii_case("Duration") => {
                                quake.duration = Some(value.f32()?.max(0.0))
                            }
                            name if name.eq_ignore_ascii_case("Radius") => {
                                quake.radius = Some(value.f32()?.max(0.0))
                            }
                            "bThrowPlayer" => quake.toss = property.boolean,
                            _ => {}
                        }
                    }
                    if let Some(tag) = tag {
                        quake.tag = tag;
                        effects.quakes.push(quake);
                    }
                }
                _ => {
                    let mut tag = None;
                    let mut event = None;
                    let mut events = [None; 6];
                    let (mut interval, mut min, mut max, mut probability) =
                        (1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
                    let (mut repeat, mut always) = (false, false);
                    while let Some(property) = values.next_tag()? {
                        let mut value = property.value;
                        match property.name {
                            "Tag" => tag = read_name(&mut value)?,
                            "Event" => event = read_name(&mut value)?,
                            "DelaySeconds" => interval = value.f32()?,
                            "bRepeating" => repeat = property.boolean.unwrap_or(false),
                            "Events" if property.array_index < events.len() => {
                                events[property.array_index] = read_name(&mut value)?
                            }
                            "minReCheckTime" => min = value.f32()?,
                            "maxReCheckTime" => max = value.f32()?,
                            "triggerProbability" => probability = value.f32()?,
                            "InitialState" => {
                                always = read_name(&mut value)?.is_some_and(|name| {
                                    package.names[name].eq_ignore_ascii_case("AlwaysActive")
                                })
                            }
                            _ => {}
                        }
                    }
                    if kind == "TimedTrigger"
                        && interval > 0.0
                        && let Some(event) = event
                    {
                        effects.timers.push(Timer::Fixed {
                            event,
                            interval,
                            repeat,
                        });
                    } else if kind == "StochasticTrigger" && max.max(min) > 0.0 {
                        let events: Vec<_> = events.into_iter().map_while(|event| event).collect();
                        if !events.is_empty() {
                            effects.timers.push(Timer::Random {
                                tag,
                                events,
                                min: min.min(max).max(0.01),
                                max: max.max(min).max(0.01),
                                probability: probability.clamp(0.0, 1.0),
                                always,
                            });
                        }
                    }
                }
            }
        }
        Ok(effects)
    }
}
