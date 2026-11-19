use std::{io, sync::Arc};

use crate::{
    map::{Face, Vec3},
    package::{Export, Package},
    sound::Wave,
};

use super::Volume;

pub(crate) struct Breakable {
    pub volume: Volume,
    pub rotation: Vec3,
    pub tag: Option<usize>,
    pub event: Option<usize>,
    pub health: i32,
    pub damage_types: [Option<String>; 5],
    pub trigger_only: bool,
    pub projectile_target: bool,
    pub force: f32,
    pub spread: f32,
    pub materials: [FragmentMaterial; 3],
    pub sound_ref: i32,
    pub sound: Option<Arc<Wave>>,
}

pub(crate) struct FragmentMaterial {
    pub count: usize,
    pub scale: f32,
    pub texture_ref: i32,
    pub translucent: bool,
    pub unlit: bool,
    pub shapes: Vec<Vec<Face>>,
    pub impacts: [Option<Arc<Wave>>; 2],
    pub collision: [f32; 2],
}

impl FragmentMaterial {
    fn new(count: usize) -> Self {
        Self {
            count,
            scale: 1.0,
            texture_ref: 0,
            translucent: false,
            unlit: false,
            shapes: Vec::new(),
            impacts: [None, None],
            collision: [0.0, 0.0],
        }
    }
}

impl Breakable {
    pub fn read(package: &Package<'_>, actor: &Export) -> io::Result<Self> {
        let mut data = Self {
            volume: Volume {
                radius: 32.0,
                height: 32.0,
                ..Volume::default()
            },
            rotation: Vec3::default(),
            tag: None,
            event: None,
            health: 0,
            damage_types: [Some("exploded".into()), None, None, None, None],
            trigger_only: false,
            projectile_target: true,
            force: 200.0,
            spread: 120.0,
            materials: [
                FragmentMaterial::new(10),
                FragmentMaterial::new(3),
                FragmentMaterial::new(0),
            ],
            sound_ref: 0,
            sound: None,
        };
        let mut props = package.properties(actor)?;
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "Location" => data.volume.position = Vec3::read(&mut value)?,
                "Rotation" => data.rotation = crate::mover::read_rotation(&mut value)?,
                "Tag" | "Event" | "ActivatedBy" => {
                    let index = value.index()?;
                    let name = package.name(index)?;
                    let present = !name.eq_ignore_ascii_case("None");
                    match prop.name {
                        "Tag" => data.tag = present.then_some(index),
                        "Event" => data.event = present.then_some(index),
                        _ if prop.array_index < data.damage_types.len() => {
                            data.damage_types[prop.array_index] =
                                present.then(|| name.to_ascii_lowercase());
                        }
                        _ => {}
                    }
                }
                "Health" => data.health = value.u32()? as i32,
                "bOnlyTriggerable" => data.trigger_only = prop.boolean.unwrap_or(false),
                "bProjTarget" | "bCollideActors" if prop.boolean == Some(false) => {
                    data.projectile_target = false
                }
                "CollisionRadius" => data.volume.radius = value.f32()?.max(0.0),
                "CollisionHeight" => data.volume.height = value.f32()?.max(0.0),
                "ExplosionSize" => data.force = value.f32()?.max(0.0),
                "ExplosionDimensions" => data.spread = value.f32()?.max(0.0),
                "NumWallChunks" => {
                    data.materials[0].count = (value.u32()? as i32).clamp(0, 128) as usize
                }
                "NumWoodChunks" => {
                    data.materials[1].count = (value.u32()? as i32).clamp(0, 128) as usize
                }
                "NumGlassChunks" => {
                    data.materials[2].count = (value.u32()? as i32).clamp(0, 128) as usize
                }
                "WallParticleSize" => data.materials[0].scale = value.f32()?.max(0.0),
                "WoodParticleSize" => data.materials[1].scale = value.f32()?.max(0.0),
                "GlassParticleSize" => data.materials[2].scale = value.f32()?.max(0.0),
                "WallTexture" => data.materials[0].texture_ref = value.compact()?,
                "WoodTexture" => data.materials[1].texture_ref = value.compact()?,
                "GlassTexture" => data.materials[2].texture_ref = value.compact()?,
                "bTranslucentGlass" => {
                    data.materials[2].translucent = prop.boolean.unwrap_or(false)
                }
                "bUnlitGlass" => data.materials[2].unlit = prop.boolean.unwrap_or(false),
                "BreakingSound" => data.sound_ref = value.compact()?,
                _ => {}
            }
        }
        Ok(data)
    }

    pub fn accepts(&self, damage_type: &str) -> bool {
        !self.trigger_only
            && (damage_type.eq_ignore_ascii_case("all")
                || self
                    .damage_types
                    .iter()
                    .flatten()
                    .any(|allowed| allowed.eq_ignore_ascii_case(damage_type)))
    }
}
