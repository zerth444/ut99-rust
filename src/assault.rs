use std::{io, sync::Arc};

use crate::{
    map::Vec3,
    package::{Export, Package},
};

pub(crate) struct AssaultMap {
    pub forts: Vec<Fort>,
    pub triggers: Vec<Trigger>,
    pub relays: Vec<Relay>,
    pub seconds: u16,
    pub turrets: Vec<Turret>,
}

pub(crate) struct Turret {
    pub volume: Volume,
    pub yaw: f32,
    pub tag: Option<usize>,
    pub health: i32,
    pub sight: f32,
    pub fov: f32,
    pub sample: f32,
    pub tracking: f32,
    pub lead: bool,
    pub minigun: bool,
    pub projectile: Option<CannonProjectile>,
    pub activate: crate::game::Motion,
    pub fire: [crate::game::Motion; 6],
    pub base: Vec<crate::map::Face>,
    pub muzzle: Vec<crate::map::Face>,
    pub audio: [Option<Arc<crate::sound::Wave>>; 3],
}

pub(crate) struct CannonProjectile {
    pub speed: f32,
    pub damage: u16,
    pub radius: f32,
    pub height: f32,
    pub lifetime: f32,
    pub scale: f32,
    pub sprites: Vec<Arc<crate::texture::Texture>>,
    pub shock: bool,
    pub impact_audio: Option<Arc<crate::sound::Wave>>,
}

#[derive(Default)]
pub(crate) struct Volume {
    pub position: Vec3,
    pub radius: f32,
    pub height: f32,
}

impl Volume {
    pub fn touches(&self, point: Vec3, radius: f32, height: f32) -> bool {
        let delta = point - self.position;
        delta.0.hypot(delta.1) <= self.radius + radius && delta.2.abs() <= self.height + height
    }

    pub fn trace(&self, from: Vec3, delta: Vec3) -> Option<f32> {
        let offset = from - self.position;
        let a = delta.0 * delta.0 + delta.1 * delta.1;
        let b = offset.0 * delta.0 + offset.1 * delta.1;
        let c = offset.0 * offset.0 + offset.1 * offset.1 - self.radius * self.radius;
        let (mut near, mut far): (f32, f32) = (0.0, 1.0);
        if a > 0.0001 {
            let root = (b * b - a * c).sqrt();
            if !root.is_finite() {
                return None;
            }
            near = near.max((-b - root) / a);
            far = far.min((-b + root) / a);
        } else if c > 0.0 {
            return None;
        }
        if delta.2.abs() > 0.0001 {
            let bottom = (-self.height - offset.2) / delta.2;
            let top = (self.height - offset.2) / delta.2;
            near = near.max(bottom.min(top));
            far = far.min(bottom.max(top));
        } else if offset.2.abs() > self.height {
            return None;
        }
        (near <= far).then_some(near)
    }
}

pub(crate) struct Fort {
    pub volume: Volume,
    pub name: String,
    pub message: String,
    pub health: i32,
    pub touch: bool,
    pub final_fort: bool,
    pub priority: u8,
    pub fallback: Option<usize>,
    pub tag: Option<usize>,
    pub event: Option<usize>,
    pub damage_events: Vec<(f32, usize)>,
    pub destination: Vec3,
}

pub(crate) struct Trigger {
    pub volume: Volume,
    pub tag: Option<usize>,
    pub event: usize,
    pub once: bool,
    pub active: bool,
    pub state: TriggerState,
    pub damage: Option<f32>,
    pub team: Option<usize>,
    pub attack_only: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum TriggerState {
    Unchanged,
    Enable,
    Disable,
    Toggle,
}

pub(crate) struct Relay {
    pub tag: usize,
    pub action: RelayAction,
}

pub(crate) enum RelayAction {
    Dispatch(Vec<(f32, usize)>),
    Count {
        count: usize,
        event: usize,
    },
    Starts(Vec<usize>),
    Sound {
        reference: i32,
        wave: Option<Arc<crate::sound::Wave>>,
        position: Vec3,
        local: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Fort(usize),
    Trigger(usize),
    Turret(usize),
}

impl AssaultMap {
    pub fn load(package: &Package<'_>, actors: &[&Export]) -> io::Result<Option<Self>> {
        if !actors
            .iter()
            .any(|actor| package.object_name(actor.class) == Some("FortStandard"))
        {
            return Ok(None);
        }

        let mut map = Self {
            forts: Vec::new(),
            triggers: Vec::new(),
            relays: Vec::new(),
            seconds: 60,
            turrets: Vec::new(),
        };
        let mut paths = std::collections::HashMap::new();
        let mut destinations = Vec::new();
        let read_name = |value: &mut crate::package::Reader<'_>| -> io::Result<Option<usize>> {
            let index = value.index()?;
            Ok((!package.name(index)?.eq_ignore_ascii_case("None")).then_some(index))
        };
        for &actor in actors {
            let class = package.object_name(actor.class).unwrap_or("");
            if !matches!(
                class,
                "FortStandard"
                    | "Trigger"
                    | "TeamTrigger"
                    | "Dispatcher"
                    | "Counter"
                    | "SpecialEvent"
                    | "PathNode"
            ) {
                continue;
            }
            let mut properties = package.properties(actor)?;
            let is_fort = class == "FortStandard";
            let mut volume = Volume {
                radius: if is_fort { 17.0 } else { 40.0 },
                height: if is_fort { 39.0 } else { 40.0 },
                ..Volume::default()
            };
            let mut fort_name = String::from("Objective");
            let mut message = String::new();
            let (mut tag, mut event, mut path, mut fallback) = (None, None, None, None);
            let (mut touch, mut final_fort, mut once, mut active) = (false, false, false, true);
            let (mut health, mut priority, mut minutes) = (100, 0, 10);
            let (mut damage, mut kind, mut team, mut count) = (0.0, 0, None, 2);
            let mut state = TriggerState::Unchanged;
            let mut events = [None; 8];
            let mut delays = [0.0; 8];
            let mut damage_events = [None; 8];
            let mut thresholds = [0.0; 8];
            let mut sound = 0;
            let mut sound_local = None;
            while let Some(property) = properties.next_tag()? {
                let mut value = property.value;
                let slot = property.array_index;
                match property.name {
                    "Tag" => tag = read_name(&mut value)?,
                    "Event" => event = read_name(&mut value)?,
                    "Location" => volume.position = Vec3::read(&mut value)?,
                    "CollisionRadius" => volume.radius = value.f32()?.max(0.0),
                    "CollisionHeight" => volume.height = value.f32()?.max(0.0),
                    "FortName" => fort_name = value.string(package.version)?,
                    "DestroyedMessage" => message = value.string(package.version)?,
                    "Health" => health = value.u32()? as i32,
                    "DefenseTime" => minutes = (value.u32()? as i32).clamp(1, 60) as u16,
                    "DefensePriority" => priority = value.byte()?,
                    "NearestPathNodeTag" => path = read_name(&mut value)?,
                    "FallBackFort" => fallback = read_name(&mut value)?,
                    "bTriggerOnly" => touch = property.boolean.unwrap_or(false),
                    "bFinalFort" => final_fort = property.boolean.unwrap_or(false),
                    "bTriggerOnceOnly" => once = property.boolean.unwrap_or(false),
                    "bInitiallyActive" => active = property.boolean.unwrap_or(true),
                    "TriggerType" => kind = value.byte()?,
                    "DamageThreshold" => damage = value.f32()?,
                    "Team" | "TeamNumber" if class == "TeamTrigger" => {
                        team = Some(value.byte()? as usize)
                    }
                    "NumToCount" => count = value.u32()? as usize,
                    "Sound" if class == "SpecialEvent" => sound = value.compact()?,
                    "OutEvents" if slot < events.len() => events[slot] = read_name(&mut value)?,
                    "OutDelays" if slot < delays.len() => delays[slot] = value.f32()?.max(0.0),
                    "DamageEvent" if slot < damage_events.len() => {
                        damage_events[slot] = read_name(&mut value)?
                    }
                    "DamageEventThreshold" if slot < thresholds.len() => {
                        thresholds[slot] = value.f32()?
                    }
                    "InitialState" if matches!(class, "Trigger" | "TeamTrigger") => {
                        state = match package.name(value.index()?)? {
                            "OtherTriggerTurnsOn" => TriggerState::Enable,
                            "OtherTriggerTurnsOff" => TriggerState::Disable,
                            "OtherTriggerToggles" => TriggerState::Toggle,
                            _ => TriggerState::Unchanged,
                        };
                    }
                    "InitialState" if class == "SpecialEvent" => {
                        sound_local = match package.name(value.index()?)? {
                            "PlaySoundEffect" => Some(false),
                            "PlayersPlaySoundEffect" => Some(true),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
            match class {
                "FortStandard" => {
                    map.seconds = map.seconds.max(minutes * 60);
                    destinations.push(path);
                    map.forts.push(Fort {
                        destination: volume.position,
                        volume,
                        name: fort_name,
                        message,
                        health,
                        touch,
                        final_fort,
                        priority,
                        fallback,
                        tag,
                        event,
                        damage_events: damage_events
                            .into_iter()
                            .zip(thresholds)
                            .filter_map(|(event, threshold)| event.map(|event| (threshold, event)))
                            .collect(),
                    });
                }
                "Trigger" | "TeamTrigger" if matches!(kind, 0 | 1 | 3 | 4) => {
                    if let Some(event) = event {
                        map.triggers.push(Trigger {
                            volume,
                            tag,
                            event,
                            once,
                            active,
                            state,
                            team,
                            attack_only: class == "TeamTrigger" && team.is_none(),
                            damage: (kind == 4).then_some(damage),
                        });
                    }
                }
                "Dispatcher" => {
                    if let Some(tag) = tag {
                        let mut due = 0.0;
                        let events = events
                            .into_iter()
                            .zip(delays)
                            .filter_map(|(event, delay)| {
                                let event = event?;
                                due += delay;
                                Some((due, event))
                            })
                            .collect();
                        map.relays.push(Relay {
                            tag,
                            action: RelayAction::Dispatch(events),
                        });
                    }
                }
                "Counter" => {
                    if let (Some(tag), Some(event)) = (tag, event) {
                        map.relays.push(Relay {
                            tag,
                            action: RelayAction::Count { count, event },
                        });
                    }
                }
                "SpecialEvent" if sound != 0 => {
                    if let (Some(tag), Some(local)) = (tag, sound_local) {
                        map.relays.push(Relay {
                            tag,
                            action: RelayAction::Sound {
                                reference: sound,
                                wave: None,
                                position: volume.position,
                                local,
                            },
                        });
                    }
                }
                "PathNode" => {
                    if let Some(tag) = tag {
                        paths.insert(tag, volume.position);
                    }
                }
                _ => {}
            }
        }
        let mut switches = std::collections::BTreeMap::<usize, Vec<usize>>::new();
        for (index, actor) in actors
            .iter()
            .filter(|actor| package.object_name(actor.class) == Some("PlayerStart"))
            .enumerate()
        {
            let mut properties = package.properties(actor)?;
            while let Some((name, mut value)) = properties.next()? {
                if name == "Tag"
                    && let Some(tag) = read_name(&mut value)?
                {
                    switches.entry(tag).or_default().push(index);
                }
            }
        }
        map.relays
            .extend(switches.into_iter().map(|(tag, starts)| Relay {
                tag,
                action: RelayAction::Starts(starts),
            }));
        for (fort, path) in map.forts.iter_mut().zip(destinations) {
            if let Some(point) = path.and_then(|tag| paths.get(&tag)) {
                fort.destination = *point;
            }
        }
        Ok(Some(map))
    }
}
