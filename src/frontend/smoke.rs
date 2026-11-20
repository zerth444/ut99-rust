use std::{io, path::Path, sync::Arc};

use crate::{
    map::Vec3,
    mover::Mover,
    package::Package,
    texture::{Texture, TextureImage, load_materials},
};

const PUFF_LIFETIME: f32 = 1.5;
// SmokeGenerator only overrides RisingRate on SpriteSmokePuff. The map
// spawns UT_SpriteSmokePuff, whose BeginPlay gives it its own velocity.
const PUFF_VELOCITY: Vec3 = Vec3(0.0, 0.0, 50.0);

struct Emitter {
    tag: Option<usize>,
    mover: Option<usize>,
    position: Vec3,
    flare: Option<Arc<Texture>>,
    flare_scale: f32,
    delay: f32,
    scale: f32,
    variance: f32,
    limit: u32,
    repeating: bool,
    destroyed: bool,
    puffs: u32,
    timer: Option<(f32, f32)>,
}

impl Emitter {
    fn position(&self, movers: &[Mover]) -> Vec3 {
        self.mover.map_or(self.position, |index| {
            let mover = &movers[index];
            mover.transform_point(self.position)
        })
    }
}

struct Puff {
    position: Vec3,
    born: f32,
    scale: f32,
    texture: usize,
}

pub(crate) struct Smoke {
    emitters: Vec<Emitter>,
    puffs: Vec<Puff>,
    random: u32,
}

impl Default for Smoke {
    fn default() -> Self {
        Self {
            emitters: Vec::new(),
            puffs: Vec::new(),
            random: 0x933a3e71,
        }
    }
}

impl Smoke {
    pub(crate) fn load(package: &Package<'_>, path: &Path, movers: &[Mover]) -> io::Result<Self> {
        let mut pending = Vec::new();
        for actor in &package.exports {
            if package.object_name(actor.class) != Some("SmokeGenerator") {
                continue;
            }
            let mut emitter = Emitter {
                tag: None,
                mover: None,
                position: Vec3::default(),
                flare: None,
                flare_scale: 1.0,
                delay: 0.15,
                scale: 1.75,
                variance: 1.0,
                limit: 200,
                repeating: false,
                destroyed: false,
                puffs: 0,
                timer: None,
            };
            let mut props = package.properties(actor)?;
            let (mut texture, mut hidden, mut ut_puff) = (0, true, false);
            while let Some(prop) = props.next_tag()? {
                let mut value = prop.value;
                match prop.name {
                    "Tag" => emitter.tag = Some(value.index()?),
                    "AttachTag" => {
                        let tag = value.index()?;
                        emitter.mover = movers.iter().position(|mover| mover.tag == Some(tag));
                    }
                    "Location" => emitter.position = Vec3::read(&mut value)?,
                    "bHidden" => hidden = prop.boolean.unwrap_or(true),
                    "Texture" => texture = value.compact()?,
                    "DrawScale" => emitter.flare_scale = value.f32()?,
                    "SmokeDelay" => emitter.delay = value.f32()?,
                    "BasePuffSize" => emitter.scale = value.f32()?,
                    "SizeVariance" => emitter.variance = value.f32()?,
                    "TotalNumPuffs" => emitter.limit = value.u32()?,
                    "bRepeating" => emitter.repeating = prop.boolean.unwrap_or(false),
                    "GenerationType" => {
                        ut_puff = package
                            .object_name(value.compact()?)
                            .is_some_and(|name| name.eq_ignore_ascii_case("UT_SpriteSmokePuff"));
                    }
                    _ => {}
                }
            }
            if ut_puff {
                pending.push((emitter, if hidden { 0 } else { texture }));
            }
        }
        let textures = load_materials(
            package,
            path,
            pending.iter().map(|(_, texture)| *texture),
            false,
        );
        Ok(Self {
            emitters: pending
                .into_iter()
                .map(|(mut emitter, texture)| {
                    emitter.flare = textures.get(&texture).cloned();
                    emitter
                })
                .collect(),
            ..Self::default()
        })
    }

    pub(crate) fn trigger(&mut self, event: usize, time: f32) {
        for emitter in self.emitters.iter_mut().filter(|emitter| {
            !emitter.destroyed && emitter.tag == Some(event) && emitter.delay > 0.0
        }) {
            let period = emitter.delay * (1.0 + random_fraction(&mut self.random));
            emitter.puffs = 0;
            emitter.timer = Some((time + period, period));
        }
    }

    pub(crate) fn tick(&mut self, time: f32, movers: &[Mover]) {
        self.puffs.retain(|puff| time - puff.born < PUFF_LIFETIME);
        for emitter in &mut self.emitters {
            while let Some((born, period)) = emitter.timer.filter(|&(born, _)| born <= time) {
                self.puffs.push(Puff {
                    position: emitter.position(movers),
                    born,
                    scale: emitter.scale + emitter.variance * random_fraction(&mut self.random),
                    texture: (4.0 * random_fraction(&mut self.random)) as usize,
                });
                emitter.puffs += 1;
                if emitter.limit != 0 && emitter.puffs > emitter.limit {
                    emitter.timer = None;
                    emitter.destroyed = !emitter.repeating;
                } else {
                    let next = born + period;
                    emitter.timer = (next > born).then_some((next, period));
                }
            }
        }
    }

    pub(crate) fn sprites<'a>(
        &'a self,
        time: f32,
        movers: &'a [Mover],
        textures: &'a [Option<Arc<Texture>>; 4],
    ) -> impl Iterator<Item = (Vec3, &'a TextureImage, f32, f32)> {
        let puffs = self.puffs.iter().filter_map(move |puff| {
            let age = time - puff.born;
            let texture = textures[puff.texture].as_ref()?;
            Some((
                puff.position + PUFF_VELOCITY * age,
                texture.image_once(age / PUFF_LIFETIME),
                puff.scale,
                0.4,
            ))
        });
        let flares = self.emitters.iter().filter_map(move |emitter| {
            let texture = emitter.flare.as_ref().filter(|_| !emitter.destroyed)?;
            Some((
                emitter.position(movers),
                texture.image(time),
                emitter.flare_scale,
                1.0,
            ))
        });
        puffs.chain(flares)
    }
}

fn random_fraction(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 8) as f32 / 16_777_216.0
}
