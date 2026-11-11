use std::{io, sync::Arc};

use crate::{collision::Plane, map::Vec3, package::Properties, sound::Wave};

#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub ground_speed: f32,
    pub acceleration: f32,
    pub air_control: f32,
    pub jump_speed: f32,
    pub eye_height: f32,
    pub step_height: f32,
    pub bob: f32,
}

impl Default for Profile {
    fn default() -> Self {
        // TournamentPlayer, PlayerPawn and Pawn defaults for the stock player.
        Self {
            ground_speed: 400.0,
            acceleration: 2048.0,
            air_control: 0.35,
            jump_speed: 325.0,
            eye_height: 27.0,
            step_height: 25.0,
            bob: 0.016,
        }
    }
}

#[derive(Default)]
pub struct PlayerAudio {
    pub steps: [Option<Arc<Wave>>; 3],
    pub jump: Option<Arc<Wave>>,
    pub land: Option<Arc<Wave>>,
    pub water_step: Option<Arc<Wave>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub gravity: Vec3,
    pub flow: Vec3,
    pub friction: f32,
    pub fluid_friction: f32,
    pub terminal: f32,
    pub water: bool,
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
        }
    }
}

impl Zone {
    pub fn apply(&mut self, mut props: Properties<'_, '_>) -> io::Result<()> {
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "ZoneGravity" => self.gravity = Vec3::read(&mut value)?,
                "ZoneVelocity" => self.flow = Vec3::read(&mut value)?,
                "ZoneGroundFriction" => self.friction = value.f32()?.max(0.0),
                "ZoneFluidFriction" => self.fluid_friction = value.f32()?.max(0.0),
                "ZoneTerminalVelocity" => self.terminal = value.f32()?.max(1.0),
                "bWaterZone" => self.water = prop.boolean.unwrap_or(self.water),
                _ => {}
            }
        }
        Ok(())
    }
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
