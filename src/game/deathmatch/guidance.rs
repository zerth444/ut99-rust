use crate::map::Vec3;
use crate::render::Camera;

const STEER_RATE: f32 = 2.6;
const PITCH_LIMIT: f32 = 1.35;

pub(super) const FLIGHT_TIME: f32 = 9.0;

pub(super) struct Pilot {
    yaw: f32,
    pitch: f32,
    armed: bool,
}

impl Pilot {
    pub fn new(aim: &Camera) -> Self {
        Self {
            yaw: aim.yaw,
            pitch: aim.pitch,
            armed: false,
        }
    }

    pub fn update(&mut self, look: [f32; 2], fire: bool, dt: f32) -> bool {
        let turn = dt * STEER_RATE;
        self.yaw = (self.yaw + look[0].clamp(-turn, turn)).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + look[1].clamp(-turn, turn)).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        // The launch press cannot also detonate the shot.
        self.armed |= !fire;
        self.armed && fire
    }

    pub fn view(&self, position: Vec3) -> Camera {
        Camera {
            position,
            yaw: self.yaw,
            pitch: self.pitch,
        }
    }
}
