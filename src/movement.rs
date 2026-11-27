use std::{io, sync::Arc};

use crate::{
    collision::Plane,
    map::Vec3,
    package::{Export, Package, Properties},
    sound::Wave,
};

#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub ground_speed: f32,
    pub water_speed: f32,
    pub cheat_flying_speed: f32,
    pub acceleration: f32,
    pub air_control: f32,
    pub jump_speed: f32,
    pub eye_height: f32,
    pub step_height: f32,
    pub bob: f32,
    pub mass: f32,
    pub buoyancy: f32,
    pub underwater_time: f32,
}

impl Default for Profile {
    fn default() -> Self {
        // TournamentPlayer, PlayerPawn and Pawn defaults for the stock player.
        Self {
            ground_speed: 400.0,
            water_speed: 200.0,
            cheat_flying_speed: 400.0,
            acceleration: 2048.0,
            air_control: 0.35,
            jump_speed: 325.0,
            eye_height: 27.0,
            step_height: 25.0,
            bob: 0.016,
            mass: 100.0,
            buoyancy: 99.0,
            underwater_time: 20.0,
        }
    }
}

#[derive(Default)]
pub struct PlayerAudio {
    pub steps: [Option<Arc<Wave>>; 3],
    pub jump: Option<Arc<Wave>>,
    pub land: Option<Arc<Wave>>,
    pub land_grunt: Option<Arc<Wave>>,
    pub water_step: Option<Arc<Wave>>,
    pub gasp: [Option<Arc<Wave>>; 2],
    pub pain: [Option<Arc<Wave>>; 4],
    pub death: [Option<Arc<Wave>>; 3],
}

impl PlayerAudio {
    pub fn landing(
        &self,
        defaults: &Self,
        speed: f32,
        jump_speed: f32,
        wet: bool,
    ) -> impl Iterator<Item = (Arc<Wave>, f32)> {
        let impact = 0.1 * (speed / jump_speed.max(1.0)).powi(2);
        let grunt = self
            .land_grunt
            .as_ref()
            .filter(|_| impact > 0.17)
            .map(|wave| (wave.clone(), (5.0 * impact).min(5.0)));
        let step = self
            .land
            .as_ref()
            .or(defaults.land.as_ref())
            .filter(|_| !wet && impact > 0.01)
            .map(|wave| (wave.clone(), (4.0 * impact).clamp(0.5, 5.0)));
        grunt.into_iter().chain(step)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub gravity: Vec3,
    pub flow: Vec3,
    pub friction: f32,
    pub fluid_friction: f32,
    pub terminal: f32,
    pub water: bool,
    pub pain: bool,
    pub damage_per_second: i32,
    pub liquid_audio: Option<usize>,
}

impl Default for Zone {
    fn default() -> Self {
        Self {
            gravity: Vec3(0.0, 0.0, -950.0),
            flow: Vec3::default(),
            friction: 8.0,
            fluid_friction: 1.2,
            terminal: 2500.0,
            water: false,
            pain: false,
            damage_per_second: 0,
            liquid_audio: None,
        }
    }
}

impl Zone {
    pub fn apply(&mut self, mut props: Properties<'_, '_>) -> io::Result<[Option<i32>; 2]> {
        let mut sounds = [None; 2];
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "ZoneGravity" => self.gravity = Vec3::read(&mut value)?,
                "ZoneVelocity" => self.flow = Vec3::read(&mut value)?,
                "ZoneGroundFriction" => self.friction = value.f32()?.max(0.0),
                "ZoneFluidFriction" => self.fluid_friction = value.f32()?.max(0.0),
                "ZoneTerminalVelocity" => self.terminal = value.f32()?.max(1.0),
                "bWaterZone" => self.water = prop.boolean.unwrap_or(self.water),
                "bPainZone" => self.pain = prop.boolean.unwrap_or(self.pain),
                "DamagePerSec" => self.damage_per_second = value.u32()? as i32,
                "EntrySound" => sounds[0] = Some(value.compact()?),
                "ExitSound" => sounds[1] = Some(value.compact()?),
                _ => {}
            }
        }
        Ok(sounds)
    }
}

pub fn load_zones<'p, 'a>(
    package: &'p Package<'a>,
    actors: &[i32],
    level: Zone,
    mut load: impl FnMut(&Export, Zone, Properties<'p, 'a>) -> io::Result<Zone>,
) -> io::Result<Vec<Zone>> {
    actors
        .iter()
        .map(|&reference| {
            let Some(actor) = usize::try_from(reference)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| package.exports.get(index))
            else {
                return Ok(level);
            };
            load(actor, level, package.properties(actor)?)
        })
        .collect()
}

pub fn stock_liquid_zone(class: Option<&str>, mut zone: Zone) -> Zone {
    match class {
        Some("WaterZone") => zone.water = true,
        Some("SlimeZone" | "LavaZone") => {
            zone.water = true;
            zone.pain = true;
            zone.damage_per_second = 40;
        }
        _ => {}
    }
    zone
}

pub struct RegionNode {
    pub plane: Plane,
    pub children: [i32; 2], // Back, front.
    pub zones: [i32; 2],
}

#[derive(Default)]
pub struct Regions {
    pub nodes: Vec<RegionNode>,
    pub zones: Vec<Zone>,
    pub level: Zone,
}

impl Regions {
    pub fn water_crossings(&self, start: Vec3, delta: Vec3) -> Vec<(f32, bool)> {
        let mut crossings = Vec::new();
        let mut water = self.at(start).water;
        let mut stack = vec![(0_i32, -1_i32, 0.0_f32, 1.0_f32)];
        let mut visited = 0;
        while let Some((child, zone, from, to)) = stack.pop() {
            visited += 1;
            if visited > self.nodes.len().saturating_mul(2) + 1 {
                break;
            }
            let Some(node) = usize::try_from(child)
                .ok()
                .and_then(|index| self.nodes.get(index))
            else {
                let wet = usize::try_from(zone)
                    .ok()
                    .and_then(|index| self.zones.get(index))
                    .unwrap_or(&self.level)
                    .water;
                if wet != water {
                    crossings.push((from, wet));
                    water = wet;
                }
                continue;
            };
            let origin = start.dot(node.plane.normal) - node.plane.distance;
            let along = delta.dot(node.plane.normal);
            let split = -origin / along;
            if split > from && split < to {
                let near = usize::from(along < 0.0);
                let far = 1 - near;
                stack.push((node.children[far], node.zones[far], split, to));
                stack.push((node.children[near], node.zones[near], from, split));
            } else {
                let side = usize::from(origin + along * ((from + to) * 0.5) >= 0.0);
                stack.push((node.children[side], node.zones[side], from, to));
            }
        }
        let end_water = self.at(start + delta).water;
        if end_water != water {
            crossings.push((1.0, end_water));
        }
        crossings
    }

    pub fn at(&self, position: Vec3) -> Zone {
        let mut index = 0;
        for _ in 0..self.nodes.len() {
            let Some(node) = self.nodes.get(index) else {
                break;
            };
            let side = usize::from(position.dot(node.plane.normal) >= node.plane.distance);
            if let Ok(child) = usize::try_from(node.children[side]) {
                index = child;
            } else {
                return usize::try_from(node.zones[side])
                    .ok()
                    .and_then(|zone| self.zones.get(zone))
                    .copied()
                    .unwrap_or(self.level);
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_crossings_find_a_thin_zone_even_with_both_ends_in_air() {
        let regions = Regions {
            nodes: vec![
                RegionNode {
                    plane: Plane {
                        normal: Vec3(1.0, 0.0, 0.0),
                        distance: 5.0,
                    },
                    children: [-1, 1],
                    zones: [0, -1],
                },
                RegionNode {
                    plane: Plane {
                        normal: Vec3(1.0, 0.0, 0.0),
                        distance: 6.0,
                    },
                    children: [-1, -1],
                    zones: [1, 0],
                },
            ],
            zones: vec![
                Zone::default(),
                Zone {
                    water: true,
                    ..Zone::default()
                },
            ],
            level: Zone::default(),
        };
        let hits = regions.water_crossings(Vec3::default(), Vec3(20.0, 0.0, 0.0));
        assert_eq!(hits.len(), 2);
        assert!((hits[0].0 - 0.25).abs() < 0.00001 && hits[0].1);
        assert!((hits[1].0 - 0.30).abs() < 0.00001 && !hits[1].1);
        let hits = regions.water_crossings(Vec3(20.0, 0.0, 0.0), Vec3(-20.0, 0.0, 0.0));
        assert_eq!(hits.len(), 2);
        assert!((hits[0].0 - 0.70).abs() < 0.00001 && hits[0].1);
        assert!((hits[1].0 - 0.75).abs() < 0.00001 && !hits[1].1);
        assert!(
            regions
                .water_crossings(Vec3(5.5, 0.0, 0.0), Vec3::default())
                .is_empty()
        );
        let boundary = regions.water_crossings(Vec3(5.0, 0.0, 0.0), Vec3(-1.0, 0.0, 0.0));
        assert_eq!(boundary, vec![(0.0, false)]);
        assert_eq!(
            regions.water_crossings(Vec3::default(), Vec3(5.0, 0.0, 0.0)),
            vec![(1.0, true)]
        );
        assert_eq!(
            regions.water_crossings(Vec3(5.5, 0.0, 0.0), Vec3(0.5, 0.0, 0.0)),
            vec![(1.0, false)]
        );
    }
}
