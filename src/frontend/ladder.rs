use std::{fs, io, path::Path};

use crate::{
    game::deathmatch::{Difficulty, Mode, Rules},
    package::{Package, invalid},
    props::class_properties,
    settings::Settings,
};

pub(super) struct Stage {
    pub map: String,
    pub installed: Option<usize>,
    pub bots: usize,
    pub allies: usize,
    difficulty: f32,
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

pub(super) fn load(system: &Path, maps: &[String]) -> io::Result<Vec<Stage>> {
    let bytes = fs::read(system.join("Botpack.u"))?;
    let package = Package::read(&bytes)?;
    let defaults = |name: &str| {
        let class = package
            .exports
            .iter()
            .find(|export| {
                export.class == 0 && package.names[export.name].eq_ignore_ascii_case(name)
            })
            .ok_or_else(|| invalid(format!("Missing tournament class {name}")))?;
        class_properties(&package, class)
    };
    let mut ladder = defaults("LadderAS")?;
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
    map_names
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
            let mut config = defaults(class.rsplit('.').next().unwrap_or(&class))?;
            let (mut bots, mut allies, mut difficulty) = (0, 0, 0.0);
            while let Some((name, mut value)) = config.next()? {
                match name {
                    "NumBots" => bots = (value.u32()? as i32).clamp(0, 8) as usize,
                    "NumAllies" => allies = (value.u32()? as i32).clamp(0, 8) as usize,
                    "ModifiedDifficulty" => difficulty = value.f32()?,
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
            })
        })
        .collect()
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
        let stages = load(&root.join("System"), &maps).unwrap();
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
    }
}
