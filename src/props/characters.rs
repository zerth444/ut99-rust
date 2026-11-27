use super::*;

pub(crate) type Skin = [Option<Arc<Texture>>; 8];

#[derive(Default)]
pub struct Characters {
    pub models: Vec<CharacterModel>,
    pub roster: Vec<Character>,
}

pub struct Character {
    pub name: String,
    pub model: usize,
    pub skins: [Skin; 3],
}

pub struct CharacterModel {
    pub idle: Motion,
    pub deaths: [Motion; 3],
    pub decapitation: Motion,
    pub gib_family: Option<usize>,
    pub clips: ActorClips,
    pub slots: Vec<usize>,
    pub audio: crate::movement::PlayerAudio,
    #[cfg(test)]
    class: &'static str,
    pub taunts: Vec<Arc<Wave>>,
    pub team_calls: [Option<Arc<Wave>>; 17],
}

pub(super) fn supporting_package(name: &str) -> bool {
    matches!(
        name,
        "commandoskins"
            | "fcommandoskins"
            | "sgirlskins"
            | "bossskins"
            | "multimesh"
            | "epiccustommodels"
            | "tskmskins"
            | "tnalimeshskins"
            | "tcowmeshskins"
            | "male1voice"
            | "male2voice"
            | "female1voice"
            | "female2voice"
            | "bossvoice"
            | "malesounds"
            | "femalesounds"
    )
}

pub(super) fn load(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
) -> Characters {
    let mut characters = Characters::default();
    for (package, class, skins, default_skin, face, team, voice) in [
        (
            "botpack",
            "TMale2",
            "soldierskins",
            "blkt",
            3,
            [0, 1],
            "VoiceMaleTwo",
        ),
        (
            "botpack",
            "TFemale1",
            "fcommandoskins",
            "cmdo",
            3,
            [0, 1],
            "VoiceFemaleOne",
        ),
        (
            "botpack",
            "TMale1",
            "commandoskins",
            "cmdo",
            1,
            [2, 3],
            "VoiceMaleOne",
        ),
        (
            "botpack",
            "TFemale2",
            "sgirlskins",
            "army",
            3,
            [0, 1],
            "VoiceFemaleTwo",
        ),
        (
            "botpack",
            "TBoss",
            "bossskins",
            "boss",
            3,
            [0, 1],
            "VoiceBoss",
        ),
        (
            "multimesh",
            "TSkaarj",
            "tskmskins",
            "warr",
            1,
            [2, 3],
            "SkaarjVoice",
        ),
        (
            "multimesh",
            "TNali",
            "tnalimeshskins",
            "priest",
            0,
            [1, 1],
            "NaliVoice",
        ),
        (
            "multimesh",
            "TCow",
            "tcowmeshskins",
            "warcow",
            0,
            [1, 1],
            "CowVoice",
        ),
    ] {
        let Some(class_object) = find_export(assets, package, class, "Class") else {
            continue;
        };
        let Some(&skin_package) = assets.names.get(skins) else {
            continue;
        };
        let appearance = appearance(assets, classes, Some(class_object));
        let Some(object) = appearance.mesh else {
            continue;
        };
        let (data, export) = assets.object(object);
        let Ok(mesh) = Mesh::read(data, export) else {
            continue;
        };
        let direct = matches!(class, "TNali" | "TCow");
        let names = skin_names(&assets.packages[skin_package], class, face, default_skin);
        let mut roster = Vec::new();
        for (base, face_name) in names {
            let mut lookup = |name: &str| {
                cached_texture(
                    assets,
                    textures,
                    find_export(assets, skins, name, "Texture")?,
                )
            };
            let palettes = std::array::from_fn(|side| {
                std::array::from_fn(|slot| {
                    if direct {
                        return match (class, slot) {
                            ("TNali", 0) => {
                                if side < 2 {
                                    lookup(&format!("T_Nali_{side}")).or_else(|| lookup(&base))
                                } else {
                                    lookup(&base)
                                }
                            }
                            ("TCow", 1) => lookup(&base),
                            ("TCow", 2) if side < 2 => lookup(&format!("T_Cow_{side}")),
                            _ => None,
                        };
                    }
                    let part = format!("{base}{}", slot + 1);
                    if side < 2
                        && (class == "TBoss" || team.contains(&slot))
                        && let Some(texture) = lookup(&format!("{part}T_{side}")).or_else(|| {
                            (class == "TFemale1")
                                .then(|| lookup(&format!("{part}_{side}")))
                                .flatten()
                        })
                    {
                        return Some(texture);
                    }
                    if slot == face
                        && !face_name.is_empty()
                        && let Some(texture) = lookup(&format!("{part}{face_name}"))
                    {
                        return Some(texture);
                    }
                    lookup(&part)
                })
            });
            if palettes[2].iter().all(Option::is_none) {
                continue;
            }
            let name = if !face_name.is_empty() {
                face_name
            } else if class == "TBoss" && base.eq_ignore_ascii_case("boss") {
                String::from("Xan")
            } else {
                base
            };
            roster.push(Character {
                name,
                model: characters.models.len(),
                skins: palettes,
            });
        }
        if roster.is_empty() {
            continue;
        }
        let (deaths, idle, clips) = player_animations(assets, textures, object, appearance, &mesh);
        if idle.frames.is_empty() || clips.run.frames.is_empty() {
            continue;
        }
        let (_, audio) = read_player_class(assets, waves, Some(class_object));
        let (taunts, team_calls) = find_export(assets, package, voice, "Class")
            .map(|voice| voice_samples(assets, waves, voice))
            .unwrap_or_default();
        let decapitation = match class {
            "TMale1" | "TMale2" | "TBoss" | "TCow" => &["Dead4"][..],
            "TFemale1" | "TFemale2" => &["Dead6"][..],
            "TSkaarj" => &["Dead5"][..],
            "TNali" => &["Dead3"][..],
            _ => &[][..],
        };
        let decapitation = effect_motion(assets, textures, object, appearance, &mesh, decapitation);
        characters.models.push(CharacterModel {
            idle,
            deaths,
            decapitation,
            gib_family: match class {
                "TMale1" | "TMale2" => Some(0),
                "TFemale1" | "TFemale2" => Some(1),
                "TBoss" => Some(2),
                "TSkaarj" => Some(3),
                "TNali" => Some(4),
                "TCow" => Some(5),
                _ => None,
            },
            clips,
            audio,
            #[cfg(test)]
            class,
            taunts,
            team_calls,
            slots: mesh.triangles.iter().map(|tri| tri.texture).collect(),
        });
        characters.roster.extend(roster);
    }
    characters
}

fn skin_names(
    data: &Package<'_>,
    class: &str,
    face_slot: usize,
    default_skin: &str,
) -> Vec<(String, String)> {
    if matches!(class, "TNali" | "TCow") {
        let alternate = if class == "TNali" {
            "Ouboudah"
        } else {
            "AtomicCow"
        };
        return [default_skin, alternate]
            .map(|name| (name.to_owned(), String::new()))
            .into();
    }
    let textures: Vec<_> = data
        .exports
        .iter()
        .filter(|export| data.object_name(export.class) == Some("Texture"))
        .map(|export| data.names[export.name].as_str())
        .collect();
    let mut bases: Vec<_> = textures
        .iter()
        .filter_map(|name| name.strip_suffix('1'))
        .filter(|base| {
            !base.to_ascii_lowercase().ends_with("t_")
                && textures
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&format!("{base}2")))
        })
        .collect();
    bases.sort_unstable_by_key(|base| {
        (
            !base.eq_ignore_ascii_case(default_skin),
            base.to_ascii_lowercase(),
        )
    });
    let mut roster = Vec::new();
    for base in bases {
        let prefix = format!("{base}{}", face_slot + 1).to_ascii_lowercase();
        let mut faces: Vec<_> = textures
            .iter()
            .filter_map(|name| {
                name.to_ascii_lowercase()
                    .starts_with(&prefix)
                    .then(|| &name[prefix.len()..])
            })
            .filter(|face| !face.is_empty() && !face.to_ascii_lowercase().starts_with("t_"))
            .collect();
        faces.sort_unstable_by_key(|face| {
            (
                !face.eq_ignore_ascii_case("Malcom"),
                face.to_ascii_lowercase(),
            )
        });
        if faces.is_empty() {
            faces.push("");
        }
        roster.extend(
            faces
                .into_iter()
                .map(|face| (base.to_owned(), face.to_owned())),
        );
    }
    roster
}

fn voice_samples(
    assets: &mut Assets<'_>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
    voice: Object,
) -> (Vec<Arc<Wave>>, [Option<Arc<Wave>>; 17]) {
    let (data, export) = assets.object(voice);
    let mut entries = Vec::new();
    if let Ok(mut properties) = class_properties(data, export) {
        while let Ok(Some(mut property)) = properties.next_tag() {
            if matches!(property.name, "TauntSound" | "OtherSound")
                && let Ok(reference) = property.value.compact()
            {
                entries.push((
                    property.name == "TauntSound",
                    property.array_index,
                    reference,
                ));
            }
        }
    }
    entries.sort_unstable_by_key(|&(_, index, _)| index);
    let mut taunts = Vec::new();
    let mut calls = std::array::from_fn(|_| None);
    for (taunt, index, reference) in entries {
        let object = assets.resolve(voice.package, reference);
        if let Some(wave) = cached_wave(assets, waves, object) {
            if taunt {
                taunts.push(wave);
            } else if let Some(call) = calls.get_mut(index) {
                *call = Some(wave);
            }
        }
    }
    (taunts, calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the reference-client installation"]
    fn installed_characters_have_team_skins_and_voices() {
        let path = Path::new("local/reference-client/Maps/CTF-Face.unr");
        let data = fs::read(path).unwrap();
        let map = crate::map::Map::load(&Package::read(&data).unwrap(), Some(path)).unwrap();
        assert_eq!(map.characters.models.len(), 8);
        assert!(map.characters.roster.len() >= 70);
        for (index, model) in map.characters.models.iter().enumerate() {
            assert!(!model.taunts.is_empty(), "{}", model.class);
            assert!(
                model.deaths.iter().all(|clip| clip.frames.len() > 1),
                "{}",
                model.class
            );
            assert!(
                model.audio.pain.iter().any(Option::is_some),
                "{}",
                model.class
            );
            for entry in map
                .characters
                .roster
                .iter()
                .filter(|character| character.model == index)
            {
                assert!(
                    model.slots.iter().any(|&slot| {
                        entry.skins[0][slot]
                            .as_ref()
                            .zip(entry.skins[1][slot].as_ref())
                            .is_some_and(|(red, blue)| !Arc::ptr_eq(red, blue))
                    }),
                    "{} has no team texture on its mesh",
                    entry.name
                );
            }
        }
    }
}
