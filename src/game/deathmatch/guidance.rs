use crate::map::Vec3;
use crate::render::Camera;

const STEER_RATE: f32 = 2.6;
const PITCH_LIMIT: f32 = 1.35;

pub(super) const FLIGHT_TIME: f32 = 9.0;

pub(super) struct Pilot {
    yaw: f32,
    pitch: f32,
    held: bool,
    roll: f32,
}

impl Pilot {
    pub fn new(aim: &Camera) -> Self {
        Self {
            yaw: aim.yaw,
            pitch: aim.pitch,
            held: true,
            roll: 0.0,
        }
    }

    pub fn update(&mut self, look: [f32; 2], fire: bool, dt: f32, flight_age: f32) -> bool {
        let turn = dt * STEER_RATE;
        self.yaw = (self.yaw + look[0].clamp(-turn, turn)).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + look[1].clamp(-turn, turn)).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        let pressed = fire && !self.held;
        self.held = fire;
        pressed && flight_age >= super::super::ordnance::WARHEAD_ARM_TIME
    }

    pub fn steer(&mut self, velocity: Vec3, dt: f32) -> Vec3 {
        let desired = Camera {
            position: Vec3::default(),
            yaw: self.yaw,
            pitch: self.pitch,
        };
        super::super::ordnance::guided_velocity(
            velocity,
            desired.look_direction(),
            dt,
            &mut self.roll,
        )
    }

    pub fn roll(&self) -> f32 {
        self.roll
    }

    pub fn view(&self, position: Vec3, velocity: Vec3) -> Camera {
        Camera {
            position,
            yaw: velocity
                .1
                .atan2(velocity.0)
                .rem_euclid(std::f32::consts::TAU),
            pitch: velocity.2.atan2(velocity.0.hypot(velocity.1)),
        }
    }
}
