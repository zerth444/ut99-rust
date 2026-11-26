use super::Weapon;
use crate::{
    collision::{Collision, Walker},
    map::Vec3,
    render::Camera,
};

pub(super) const LOADED: [Weapon; 8] = [
    Weapon::Pulse,
    Weapon::Shock,
    Weapon::Flak,
    Weapon::Bio,
    Weapon::Minigun,
    Weapon::Sniper,
    Weapon::Ripper,
    Weapon::Rocket,
];
pub(super) const AMMO: u16 = 999;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Flight {
    #[default]
    Walk,
    Fly,
    Ghost,
}

impl Flight {
    pub fn advance(
        self,
        walker: &mut Walker,
        world: &Collision,
        view: &Camera,
        movement: [f32; 3],
        dt: f32,
    ) {
        let [forward, right, up] = movement;
        let direction = view.look_direction() * forward + view.right() * right + Vec3(0.0, 0.0, up);
        let delta = direction
            * (world.profile.cheat_flying_speed * dt / direction.dot(direction).sqrt().max(1.0));
        if self == Self::Ghost {
            walker.position = walker.position + delta;
        } else {
            walker.carry(world, delta);
        }
        walker.grounded = false;
        walker.set_teleport_velocity(Vec3::default());
    }
}

pub(super) enum Action {
    AllAmmo,
    Loaded,
    Suicide,
    Message(&'static str),
}

#[derive(Default)]
pub(super) struct Cheats {
    enabled: bool,
    pub god: bool,
    pub flight: Flight,
}

impl Cheats {
    pub fn command(&mut self, command: &str, fits: bool) -> Action {
        let command = command.trim().to_ascii_lowercase();
        match (self.enabled, command.as_str()) {
            (_, "iamtheone") => {
                self.enabled = true;
                Action::Message("Cheats enabled")
            }
            (_, "suicide") => Action::Suicide,
            (true, "god") => {
                self.god = !self.god;
                Action::Message(if self.god {
                    "God mode on"
                } else {
                    "God mode off"
                })
            }
            (true, "allammo") => Action::AllAmmo,
            (true, "loaded") => Action::Loaded,
            (true, "fly") if fits => {
                self.flight = Flight::Fly;
                Action::Message("You feel much lighter")
            }
            (true, "ghost") => {
                self.flight = Flight::Ghost;
                Action::Message("You feel ethereal")
            }
            (_, "walk") if self.flight == Flight::Walk || fits => {
                self.flight = Flight::Walk;
                Action::Message("Walking")
            }
            (true, "walk" | "fly") => Action::Message("Move out of solid geometry first"),
            (false, "god" | "allammo" | "loaded" | "fly" | "ghost") => {
                Action::Message("Enter iamtheone first")
            }
            _ => Action::Message("Unknown command"),
        }
    }

    pub fn protects(&self) -> bool {
        self.god || self.flight != Flight::Walk
    }
}
