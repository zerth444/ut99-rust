use crate::map::{Map, Vec3};

use super::{Fighter, distance_squared};

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Deathmatch,
    TeamDeathmatch,
    CaptureTheFlag,
    Domination,
    LastManStanding,
}

impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Deathmatch,
        Self::TeamDeathmatch,
        Self::CaptureTheFlag,
        Self::Domination,
        Self::LastManStanding,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Deathmatch => "Deathmatch",
            Self::TeamDeathmatch => "Team Deathmatch",
            Self::CaptureTheFlag => "Capture the Flag",
            Self::Domination => "Domination",
            Self::LastManStanding => "Last Man Standing",
        }
    }

    pub fn team_based(self) -> bool {
        matches!(
            self,
            Self::TeamDeathmatch | Self::CaptureTheFlag | Self::Domination
        )
    }

    pub fn map_prefix(self) -> &'static str {
        match self {
            Self::CaptureTheFlag => "ctf-",
            Self::Domination => "dom-",
            _ => "dm-",
        }
    }

    pub fn goal_label(self) -> &'static str {
        match self {
            Self::CaptureTheFlag => "Capture limit",
            Self::Domination => "Score limit",
            Self::LastManStanding => "Lives",
            _ => "Frag limit",
        }
    }
}

#[derive(Clone, Copy)]
enum FlagState {
    Home,
    Held(usize),
    Dropped(Vec3, f32),
}

struct Flag {
    base: Vec3,
    state: FlagState,
}

#[derive(Clone, Copy)]
pub(crate) enum Marker {
    Fighter,
    Flag,
    ControlPoint,
}

pub(super) struct Objectives {
    pub scores: [i16; 2],
    flags: Option<[Flag; 2]>,
    zones: Vec<(Vec3, Option<usize>)>,
    next_score: f32,
}

fn touching(map: &Map, position: Vec3, fighter: &Fighter) -> bool {
    let offset = fighter.walker.position - position;
    fighter.health > 0
        && offset.0.hypot(offset.1) < 50.0
        && offset.2.abs() < 72.0
        && map.collision.ray(position, offset).is_none()
}

impl Objectives {
    pub(super) fn new(map: &Map, mode: Mode) -> Self {
        let flags = if mode == Mode::CaptureTheFlag {
            map.flag_bases[0].zip(map.flag_bases[1]).map(|(red, blue)| {
                [red, blue].map(|base| Flag {
                    base,
                    state: FlagState::Home,
                })
            })
        } else {
            None
        };
        let zones = if mode == Mode::Domination {
            map.control_points
                .iter()
                .map(|&point| (point, None))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            scores: [0; 2],
            flags,
            zones,
            next_score: 2.0,
        }
    }

    pub(super) fn tick(
        &mut self,
        map: &Map,
        fighters: &[Fighter],
        time: f32,
    ) -> Option<&'static str> {
        let mut notice = None;
        if let Some(flags) = &mut self.flags {
            for (team, flag) in flags.iter_mut().enumerate() {
                let position = match flag.state {
                    FlagState::Held(owner) => {
                        let fighter = &fighters[owner];
                        if fighter.health == 0 {
                            let point = fighter.walker.position;
                            flag.state =
                                if map.collision.ray(point, Vec3(0.0, 0.0, -250.0)).is_some() {
                                    FlagState::Dropped(point, time + 20.0)
                                } else {
                                    FlagState::Home
                                };
                        }
                        continue;
                    }
                    FlagState::Dropped(_, until) if time >= until => {
                        flag.state = FlagState::Home;
                        continue;
                    }
                    FlagState::Dropped(position, _) => position,
                    FlagState::Home => flag.base,
                };
                if let Some((id, fighter)) = fighters
                    .iter()
                    .enumerate()
                    .filter(|(_, fighter)| touching(map, position, fighter))
                    .filter(|(_, fighter)| {
                        fighter.team != Some(team) || matches!(flag.state, FlagState::Dropped(..))
                    })
                    .min_by(|(_, a), (_, b)| {
                        distance_squared(a.walker.position, position)
                            .total_cmp(&distance_squared(b.walker.position, position))
                    })
                {
                    flag.state = if fighter.team == Some(team) {
                        FlagState::Home
                    } else {
                        FlagState::Held(id)
                    };
                    notice = Some(if fighter.team == Some(team) {
                        "Flag returned"
                    } else {
                        "Flag taken"
                    });
                }
            }
            for team in 0..2 {
                if let FlagState::Held(owner) = flags[1 - team].state
                    && matches!(flags[team].state, FlagState::Home)
                    && touching(map, flags[team].base, &fighters[owner])
                {
                    flags[1 - team].state = FlagState::Home;
                    self.scores[team] = self.scores[team].saturating_add(1);
                    notice = Some(if team == 0 {
                        "Red scored a capture"
                    } else {
                        "Blue scored a capture"
                    });
                }
            }
        }
        for (position, control) in &mut self.zones {
            if let Some(team) = fighters
                .iter()
                .filter(|fighter| touching(map, *position, fighter))
                .min_by(|a, b| {
                    distance_squared(a.walker.position, *position)
                        .total_cmp(&distance_squared(b.walker.position, *position))
                })
                .and_then(|fighter| fighter.team)
            {
                *control = Some(team);
            }
        }
        if time >= self.next_score && !self.zones.is_empty() {
            self.next_score += 2.0;
            for &(_, owner) in &self.zones {
                if let Some(team) = owner {
                    self.scores[team] = self.scores[team].saturating_add(1);
                }
            }
        }
        notice
    }

    pub(super) fn destination(&self, fighters: &[Fighter], id: usize) -> Option<Vec3> {
        let fighter = &fighters[id];
        let team = fighter.team?;
        if let Some(flags) = &self.flags {
            if matches!(flags[1 - team].state, FlagState::Held(owner) if owner == id) {
                return Some(flags[team].base);
            }
            match flags[team].state {
                FlagState::Dropped(point, _) => return Some(point),
                FlagState::Held(carrier) => return Some(fighters[carrier].walker.position),
                FlagState::Home => {}
            }
            return Some(match flags[1 - team].state {
                FlagState::Home => flags[1 - team].base,
                FlagState::Dropped(point, _) => point,
                FlagState::Held(owner) => fighters[owner].walker.position,
            });
        }
        self.zones
            .iter()
            .filter(|(_, owner)| *owner != Some(team))
            .min_by(|(a, _), (b, _)| {
                distance_squared(fighter.walker.position, *a)
                    .total_cmp(&distance_squared(fighter.walker.position, *b))
            })
            .map(|&(position, _)| position)
    }

    pub(super) fn flag_status(&self) -> Option<[&'static str; 2]> {
        let flags = self.flags.as_ref()?;
        Some(std::array::from_fn(|team| match flags[team].state {
            FlagState::Home => "At base",
            FlagState::Held(0) => "You have it",
            FlagState::Held(_) => "Taken",
            FlagState::Dropped(..) => "Dropped",
        }))
    }

    pub(super) fn markers<'a>(
        &'a self,
        fighters: &'a [Fighter],
    ) -> impl Iterator<Item = (Vec3, Marker, Option<usize>)> + 'a {
        let flags = self
            .flags
            .iter()
            .flatten()
            .enumerate()
            .map(move |(team, flag)| {
                let position = match flag.state {
                    FlagState::Home => flag.base,
                    FlagState::Dropped(position, _) => position,
                    FlagState::Held(owner) => fighters[owner].walker.position,
                };
                (position, Marker::Flag, Some(team))
            });
        flags.chain(
            self.zones
                .iter()
                .map(|&(position, owner)| (position, Marker::ControlPoint, owner)),
        )
    }
}
