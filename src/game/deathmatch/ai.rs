use crate::map::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Easy = 1,
    #[default]
    Standard = 2,
    Hard = 3,
}

impl Difficulty {
    pub fn from_level(level: u32) -> Self {
        match level {
            1 => Self::Easy,
            3 => Self::Hard,
            _ => Self::Standard,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Standard => "Standard",
            Self::Hard => "Hard",
        }
    }

    pub(super) fn combat(self) -> Combat {
        match self {
            Self::Easy => Combat {
                reaction: 0.55,
                turn: 2.65,
                error: 0.075,
                lead: 0.25,
                strafe: 0.3,
                rush_health: 23,
                evade_window: 0.22,
                dodge: false,
            },
            Self::Standard => Combat {
                reaction: 0.25,
                turn: 3.8,
                error: 0.028,
                lead: 0.72,
                strafe: 0.53,
                rush_health: 36,
                evade_window: 0.36,
                dodge: true,
            },
            Self::Hard => Combat {
                reaction: 0.13,
                turn: 5.4,
                error: 0.011,
                lead: 1.0,
                strafe: 0.72,
                rush_health: 48,
                evade_window: 0.52,
                dodge: true,
            },
        }
    }
}

pub(super) struct Combat {
    pub reaction: f32,
    pub turn: f32,
    pub error: f32,
    pub lead: f32,
    pub strafe: f32,
    pub rush_health: u16,
    pub evade_window: f32,
    pub dodge: bool,
}

pub(super) fn intercept(delta: Vec3, velocity: Vec3, speed: f32) -> Vec3 {
    let a = velocity.dot(velocity) - speed * speed;
    let b = delta.dot(velocity);
    let c = delta.dot(delta);
    let discriminant = b * b - a * c;
    if discriminant < 0.0 {
        return delta;
    }
    let divisor = discriminant.sqrt() - b;
    if divisor <= 0.0 {
        return delta;
    }
    let time = c / divisor;
    if time > 1.35 {
        delta
    } else {
        delta + velocity * time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projectile_lead_intersects_a_strafing_target() {
        let delta = Vec3(650.0, 0.0, 0.0);
        let velocity = Vec3(180.0, 375.0, 0.0);
        let speed = 1250.0;
        let impact = intercept(delta, velocity, speed);
        let flight = impact.dot(impact).sqrt() / speed;
        let miss = impact - (delta + velocity * flight);
        assert!(miss.dot(miss) < 0.001 && impact.1 > 175.0);
        let unreachable = intercept(delta, Vec3(1900.0, 0.0, 0.0), speed);
        assert_eq!(unreachable.0, delta.0);
    }
}
