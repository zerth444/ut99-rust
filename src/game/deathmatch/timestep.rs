use super::{Command, Match};
use crate::{map::Map, sound::SoundEvents};

const STEP: f64 = 1.0 / 120.0;
const MAX_FRAME: f64 = 0.25;

/// Collects transient controls between render frames. Only held controls repeat
/// when a single frame must run multiple simulation steps.
#[derive(Default)]
pub struct Timestep {
    remainder: f64,
    look: [f32; 2],
    fire: [bool; 2],
    jump: bool,
    restart: bool,
    slot: Option<u8>,
    cycle: i8,
    dodge: Option<[f32; 2]>,
}

impl Timestep {
    pub fn body_blend(&self) -> f32 {
        (self.remainder / STEP).clamp(0.0, 1.0) as f32
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn advance(
        &mut self,
        game: &mut Match,
        map: &mut Map,
        command: Command,
        presses: [bool; 2],
        jump_pressed: bool,
        elapsed: f64,
    ) -> SoundEvents {
        let mut sounds = SoundEvents::default();
        if !elapsed.is_finite() || elapsed < 0.0 {
            return sounds;
        }
        let frame = elapsed.min(MAX_FRAME);
        let before = self.remainder;
        self.remainder += frame;
        self.fire[0] |= presses[0] || command.primary;
        self.fire[1] |= presses[1] || command.alternate;
        self.jump |= command.jump || jump_pressed;
        self.restart |= command.restart;
        if let Some(slot) = command.slot {
            self.slot = Some(slot);
        }
        self.cycle = self.cycle.saturating_add(command.cycle);
        self.dodge = command.dodge.or(self.dodge);
        let ticks = ((self.remainder + 1e-10) / STEP).floor() as u32;
        if ticks == 0 {
            self.look[0] += command.look[0];
            self.look[1] += command.look[1];
            return sounds;
        }
        self.remainder = (self.remainder - f64::from(ticks) * STEP).max(0.0);
        let first_look_fraction = ((STEP - before) / frame).clamp(0.0, 1.0) as f32;
        let tick_look_fraction = (STEP / frame).min(1.0) as f32;
        let remaining_look_fraction = (self.remainder / frame).min(1.0) as f32;
        let first_look = [
            self.look[0] + command.look[0] * first_look_fraction,
            self.look[1] + command.look[1] * first_look_fraction,
        ];
        self.look = command.look.map(|axis| axis * remaining_look_fraction);
        let fire = std::mem::take(&mut self.fire);
        let jump = std::mem::take(&mut self.jump);
        let first = Command {
            look: first_look,
            primary: command.primary || fire[0],
            alternate: command.alternate || fire[1],
            jump: command.jump || jump,
            restart: std::mem::take(&mut self.restart),
            slot: self.slot.take(),
            cycle: std::mem::take(&mut self.cycle),
            dodge: self.dodge.take(),
            ..command
        };
        let held = Command {
            look: command.look.map(|axis| axis * tick_look_fraction),
            restart: false,
            slot: None,
            cycle: 0,
            dodge: None,
            ..command
        };
        for index in 0..ticks {
            sounds.append(game.step(map, if index == 0 { first } else { held }, STEP as f32));
        }
        sounds
    }
}
