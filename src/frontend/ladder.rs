use std::{fs, io, path::Path};

use crate::{
    game::{
        Weapon,
        deathmatch::{Difficulty, Mode, RatedRoster, Rules},
    },
    package::{Package, invalid},
    props::class_properties,
    settings::Settings,
};

#[derive(Default)]
pub(super) struct Campaign {
    pub stages: Vec<Stage>,
    pub teams: Vec<Team>,
    alternate: Option<Team>,
}

pub(super) struct Team {
    class: i32,
    pub name: String,
    bots: [String; 8],
    weapons: [Option<Weapon>; 8],
}

pub(super) struct Stage {
    pub map: String,
    pub installed: Option<usize>,
    pub bots: usize,
    pub allies: usize,
    difficulty: f32,
    enemy: Option<Team>,
    names: [String; 8],
    weapons: [Option<Weapon>; 8],
    frag_limit: i16,
    seconds: u16,
}

impl Campaign {
    pub fn roster(&self, mode: Mode, stage: &Stage, team: usize) -> RatedRoster {
        if mode == Mode::Deathmatch {
            return RatedRoster {
                allies: 0,
                teams: [None, None],
                bots: stage
                    .names
                    .iter()
                    .take(stage.bots)
                    .map(|name| (!name.is_empty()).then(|| name.clone()))
                    .collect(),
                favorites: stage.weapons[..stage.bots].to_vec(),
            };
        }
        let allies = self.teams.get(team).or_else(|| self.teams.first());
        let enemy = stage.enemy.as_ref().and_then(|enemy| {
            if allies.is_some_and(|allies| allies.class == enemy.class) {
                self.alternate.as_ref()
            } else {
                Some(enemy)
            }
        });
        let team_name = |team: Option<&Team>| team.map(|team| team.name.clone());
        let members = (0..stage.bots).map(|index| {
            if index < stage.allies {
                (allies, index)
            } else {
                (enemy, index - stage.allies)
            }
        });
        RatedRoster {
            allies: stage.allies,
            teams: [team_name(allies), team_name(enemy)],
            bots: members
                .clone()
                .map(|(team, index)| {
                    team.and_then(|team| team.bots.get(index))
                        .filter(|name| !name.is_empty())
                        .cloned()
                })
                .collect(),
            favorites: members
                .map(|(team, index)| team.and_then(|team| team.weapons[index]))
                .collect(),
        }
    }
}

fn favorite_weapon(qualified: &str) -> Option<Weapon> {
    let class = qualified.rsplit('.').next()?;
    Weapon::ALL
        .into_iter()
        .find(|weapon| weapon.class().eq_ignore_ascii_case(class))
}

impl Stage {
    pub fn rules(&self, mode: Mode, settings: &Settings) -> Rules {
        let difficulty = (settings.bot_difficulty as f32 + self.difficulty).round();
        Rules {
            mode,
            bots: self.bots,
            difficulty: Difficulty::from_level(difficulty.clamp(1.0, 3.0) as u32),
            frag_limit: self.frag_limit,
            seconds: self.seconds,
        }
    }
}

pub(super) fn load(system: &Path, maps: &[String], mode: Mode) -> io::Result<Campaign> {
    let bytes = fs::read(system.join("Botpack.u"))?;
    let package = Package::read(&bytes)?;
    let find_class = |name: &str| {
        package
            .exports
            .iter()
            .position(|export| {
                export.class == 0 && package.names[export.name].eq_ignore_ascii_case(name)
            })
            .map(|index| index as i32 + 1)
            .ok_or_else(|| invalid(format!("Missing tournament class {name}")))
    };
    let defaults = |class: i32| {
        let index = usize::try_from(class).ok().and_then(|id| id.checked_sub(1));
        let export = index
            .and_then(|index| package.exports.get(index))
            .filter(|export| export.class == 0)
            .ok_or_else(|| invalid("Invalid tournament class reference"))?;
        class_properties(&package, export)
    };
    let read_team = |class| -> io::Result<Team> {
        let mut props = defaults(class)?;
        let mut team = Team {
            class,
            name: String::new(),
            bots: Default::default(),
            weapons: Default::default(),
        };
        while let Some(property) = props.next_tag()? {
            let mut value = property.value;
            match property.name {
                "TeamName" => team.name = value.string(package.version)?,
                "BotNames" if property.array_index < team.bots.len() => {
                    team.bots[property.array_index] = value.string(package.version)?;
                }
                "FavoriteWeapon" if property.array_index < team.weapons.len() => {
                    team.weapons[property.array_index] =
                        favorite_weapon(&value.string(package.version)?);
                }
                _ => {}
            }
        }
        if team.name.is_empty() {
            return Err(invalid("Tournament team has no name"));
        }
        Ok(team)
    };
    let mut base = defaults(find_class("Ladder")?)?;
    let mut team_count = 0;
    let mut team_classes = [0; 32];
    while let Some(property) = base.next_tag()? {
        let mut value = property.value;
        match property.name {
            "NumTeams" => team_count = (value.u32()? as usize).min(team_classes.len()),
            "LadderTeams" if property.array_index < team_classes.len() => {
                team_classes[property.array_index] = value.compact()?;
            }
            _ => {}
        }
    }
    let teams = team_classes
        .into_iter()
        .take(team_count)
        .filter_map(|class| read_team(class).ok())
        .collect();
    let alternate = find_class("RatedTeamInfoS").and_then(read_team).ok();
    let ladder_name = match mode {
        Mode::Assault => "LadderAS",
        Mode::Deathmatch => "LadderDM",
        _ => return Err(invalid("Unsupported tournament mode")),
    };
    let mut ladder = defaults(find_class(ladder_name)?)?;
    let mut count = 0;
    let mut prefix = String::new();
    let mut map_names: [String; 32] = Default::default();
    let mut configs: [String; 32] = Default::default();
    let mut frag_limits = [0; 32];
    let mut time_limits = [0; 32];
    let mut demo_only = [false; 32];
    while let Some(property) = ladder.next_tag()? {
        let mut value = property.value;
        let index = property.array_index;
        match property.name {
            "Matches" => count = value.u32()? as usize,
            "MapPrefix" => prefix = value.string(package.version)?,
            "Maps" if index < map_names.len() => {
                map_names[index] = value.string(package.version)?
            }
            "MatchInfo" if index < configs.len() => {
                configs[index] = value.string(package.version)?
            }
            "FragLimits" if index < frag_limits.len() => {
                frag_limits[index] = (value.u32()? as i32).clamp(0, i32::from(i16::MAX)) as i16;
            }
            "TimeLimits" if index < time_limits.len() => {
                time_limits[index] =
                    (value.u32()? as i32).clamp(0, i32::from(u16::MAX) / 60) as u16 * 60;
            }
            "DemoDisplay" if index < demo_only.len() => demo_only[index] = value.u32()? != 0,
            _ => {}
        }
    }
    if count == 0 || count > map_names.len() {
        return Err(invalid("Invalid tournament ladder length"));
    }
    let stages = map_names
        .into_iter()
        .zip(configs)
        .take(count)
        .enumerate()
        .filter(|(index, _)| !demo_only[*index])
        .map(|(index, (map, class))| {
            if map.is_empty() {
                return Err(invalid("Missing tournament map"));
            }
            let map = Path::new(&map)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| invalid("Invalid tournament map name"))?;
            let map = format!("{prefix}{map}");
            let mut config = defaults(find_class(class.rsplit('.').next().unwrap_or(&class))?)?;
            let (mut bots, mut allies, mut difficulty) = (0, 0, 0.0);
            let mut enemy = None;
            let mut names: [String; 8] = Default::default();
            let mut weapons = [None; 8];
            while let Some(property) = config.next_tag()? {
                let mut value = property.value;
                match property.name {
                    "NumBots" => bots = (value.u32()? as i32).clamp(0, 8) as usize,
                    "NumAllies" => allies = (value.u32()? as i32).clamp(0, 8) as usize,
                    "ModifiedDifficulty" => difficulty = value.f32()?,
                    "EnemyTeam" => enemy = read_team(value.compact()?).ok(),
                    "BotNames" if property.array_index < names.len() => {
                        names[property.array_index] = value.string(package.version)?;
                    }
                    "FavoriteWeapon" if property.array_index < weapons.len() => {
                        weapons[property.array_index] =
                            favorite_weapon(&value.string(package.version)?);
                    }
                    _ => {}
                }
            }
            Ok(Stage {
                installed: maps
                    .iter()
                    .position(|entry| entry.eq_ignore_ascii_case(&map)),
                map,
                bots,
                allies: allies.min(bots),
                difficulty,
                enemy,
                names,
                weapons,
                frag_limit: frag_limits[index],
                seconds: time_limits[index],
            })
        })
        .collect::<io::Result<_>>()?;
    Ok(Campaign {
        stages,
        teams,
        alternate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the local reference-client installation"]
    fn reads_ranked_deathmatch_stages_with_stock_limits_and_opponents() {
        let root = Path::new("local/reference-client");
        let maps = fs::read_dir(root.join("Maps"))
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let campaign = load(&root.join("System"), &maps, Mode::Deathmatch).unwrap();
        assert_eq!(campaign.stages[0].map, "DM-Tutorial");
        assert!(campaign.stages[0].installed.is_some());
        for stage in &campaign.stages[1..] {
            assert!(stage.bots > 0, "{}", stage.map);
            assert!(stage.frag_limit > 0 || stage.seconds > 0, "{}", stage.map);
            assert!(stage.installed.is_some(), "{}", stage.map);
            assert!(
                campaign
                    .roster(Mode::Deathmatch, stage, 0)
                    .bots
                    .iter()
                    .all(Option::is_some),
                "{}",
                stage.map
            );
        }
        assert_eq!(
            campaign
                .stages
                .iter()
                .skip(1)
                .map(|stage| (stage.map.as_str(), stage.bots, stage.frag_limit))
                .collect::<Vec<_>>(),
            [
                ("DM-Oblivion", 1, 10),
                ("DM-Stalwart", 2, 10),
                ("DM-Fractal", 1, 15),
                ("DM-Turbine", 4, 15),
                ("DM-Codex", 3, 20),
                ("DM-Pressure", 4, 20),
                ("DM-Grinder", 3, 20),
                ("DM-Kgalleon", 4, 25),
                ("DM-Tempest", 5, 25),
                ("DM-Barricade", 5, 25),
                ("DM-Liandri", 5, 30),
                ("DM-Conveyor", 6, 30),
                ("DM-Peak", 4, 30)
            ],
        );
        assert_eq!(campaign.stages[1].names[0], "Blake");
        assert!(
            campaign
                .stages
                .iter()
                .any(|stage| stage.weapons.iter().any(Option::is_some))
        );
    }

    #[test]
    #[ignore = "requires the local reference-client installation"]
    fn reads_assault_ladder_order_and_rated_rosters() {
        let root = Path::new("local/reference-client");
        let maps = fs::read_dir(root.join("Maps"))
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let campaign = load(&root.join("System"), &maps, Mode::Assault).unwrap();
        let stages = &campaign.stages;
        assert_eq!(
            stages
                .iter()
                .map(|s| (s.map.as_str(), s.bots, s.allies, s.difficulty))
                .collect::<Vec<_>>(),
            [
                ("AS-Tutorial", 3, 1, 0.0),
                ("AS-Frigate", 7, 3, 0.5),
                ("AS-HiSpeed", 7, 3, 1.0),
                ("AS-Rook", 7, 3, 1.5),
                ("AS-Mazon", 7, 3, 2.0),
                ("AS-OceanFloor", 7, 3, 2.5),
                ("AS-Overlord", 8, 4, 3.0),
            ],
        );
        assert!(stages.iter().all(|stage| stage.installed.is_some()));
        assert!(campaign.teams.len() >= 6);
        for stage in stages {
            let roster = campaign.roster(Mode::Assault, stage, 0);
            assert!(roster.teams.iter().all(Option::is_some), "{}", stage.map);
            assert_eq!(roster.bots.len(), stage.bots);
            assert!(roster.bots.iter().all(Option::is_some), "{}", stage.map);
        }
        let frigate = &stages[1];
        if let Some(enemy_team) = frigate.enemy.as_ref()
            && let Some(index) = campaign
                .teams
                .iter()
                .position(|team| team.class == enemy_team.class)
        {
            let roster = campaign.roster(Mode::Assault, frigate, index);
            assert_eq!(roster.teams[0].as_deref(), Some(enemy_team.name.as_str()));
            assert_eq!(
                roster.teams[1].as_deref(),
                campaign.alternate.as_ref().map(|team| team.name.as_str())
            );
        }
    }
}
