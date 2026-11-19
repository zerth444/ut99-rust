use std::{fs, io, path::Path};

use crate::{
    game::deathmatch::{AssaultRoster, Difficulty, Mode, Rules},
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
}

pub(super) struct Stage {
    pub map: String,
    pub installed: Option<usize>,
    pub bots: usize,
    pub allies: usize,
    difficulty: f32,
    enemy: Option<Team>,
}

impl Campaign {
    pub fn roster(&self, stage: &Stage, team: usize) -> AssaultRoster {
        let allies = self.teams.get(team).or_else(|| self.teams.first());
        let enemy = stage.enemy.as_ref().and_then(|enemy| {
            if allies.is_some_and(|allies| allies.class == enemy.class) {
                self.alternate.as_ref()
            } else {
                Some(enemy)
            }
        });
        let team_name = |team: Option<&Team>| team.map(|team| team.name.clone());
        AssaultRoster {
            allies: stage.allies,
            teams: [team_name(allies), team_name(enemy)],
            bots: (0..stage.bots)
                .map(|index| {
                    let (team, index) = if index < stage.allies {
                        (allies, index)
                    } else {
                        (enemy, index - stage.allies)
                    };
                    team.and_then(|team| team.bots.get(index))
                        .filter(|name| !name.is_empty())
                        .cloned()
                })
                .collect(),
        }
    }
}

impl Stage {
    pub fn rules(&self, settings: &Settings) -> Rules {
        let difficulty = (settings.bot_difficulty as f32 + self.difficulty).round();
        Rules {
            mode: Mode::Assault,
            bots: self.bots,
            difficulty: Difficulty::from_level(difficulty.clamp(1.0, 3.0) as u32),
            ..settings.match_rules()
        }
    }
}

pub(super) fn load(system: &Path, maps: &[String]) -> io::Result<Campaign> {
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
        };
        while let Some(property) = props.next_tag()? {
            let mut value = property.value;
            match property.name {
                "TeamName" => team.name = value.string(package.version)?,
                "BotNames" if property.array_index < team.bots.len() => {
                    team.bots[property.array_index] = value.string(package.version)?;
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
    let mut ladder = defaults(find_class("LadderAS")?)?;
    let mut count = 0;
    let mut prefix = String::new();
    let mut map_names: [String; 32] = Default::default();
    let mut configs: [String; 32] = Default::default();
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
            _ => {}
        }
    }
    if count == 0 || count > map_names.len() {
        return Err(invalid("Invalid Assault ladder length"));
    }
    let stages = map_names
        .into_iter()
        .zip(configs)
        .take(count)
        .map(|(map, class)| {
            if map.is_empty() {
                return Err(invalid("Missing Assault ladder map"));
            }
            let map = Path::new(&map)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| invalid("Invalid Assault ladder map name"))?;
            let map = format!("{prefix}{map}");
            let mut config = defaults(find_class(class.rsplit('.').next().unwrap_or(&class))?)?;
            let (mut bots, mut allies, mut difficulty) = (0, 0, 0.0);
            let mut enemy = None;
            while let Some((name, mut value)) = config.next()? {
                match name {
                    "NumBots" => bots = (value.u32()? as i32).clamp(0, 8) as usize,
                    "NumAllies" => allies = (value.u32()? as i32).clamp(0, 8) as usize,
                    "ModifiedDifficulty" => difficulty = value.f32()?,
                    "EnemyTeam" => enemy = read_team(value.compact()?).ok(),
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
        let campaign = load(&root.join("System"), &maps).unwrap();
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
            let roster = campaign.roster(stage, 0);
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
            let roster = campaign.roster(frigate, index);
            assert_eq!(roster.teams[0].as_deref(), Some(enemy_team.name.as_str()));
            assert_eq!(
                roster.teams[1].as_deref(),
                campaign.alternate.as_ref().map(|team| team.name.as_str())
            );
        }
    }
}
