use std::collections::VecDeque;

use crate::{hud::Hud, render::Frame};

#[derive(Default)]
pub(super) struct Console {
    pub open: bool,
    input: String,
    previous: String,
    lines: VecDeque<String>,
}

impl Console {
    pub fn character(&mut self, key: usize) {
        if key == 8 {
            self.input.pop();
        } else if (32..=126).contains(&key) && self.input.len() < 96 {
            self.input.push(key as u8 as char);
        }
    }

    pub fn recall(&mut self) {
        self.input.clone_from(&self.previous);
    }

    pub fn submit(&mut self, run: impl FnOnce(&str) -> &'static str) {
        if self.input.trim().is_empty() {
            return;
        }
        self.previous = std::mem::take(&mut self.input);
        let answer = run(&self.previous);
        self.lines.push_back(format!("> {}", self.previous));
        self.lines.push_back(answer.to_owned());
        if self.lines.len() > 8 {
            self.lines.drain(..2);
        }
    }

    pub fn draw(&self, frame: &mut Frame, hud: &Hud) {
        if !self.open {
            return;
        }
        const LINE: i32 = 19;
        let width = frame.width as i32;
        let height = (frame.height as i32 / 2).clamp(1, 205);
        frame.shade([0, 0, width, height], [0.025, 0.042, 0.055], 0.95);
        let prompt = format!("> {}_", self.input);
        hud.text(
            frame,
            12,
            height - LINE - 5,
            0.92,
            &prompt,
            [1.0, 0.90, 0.65],
        );
        for (i, line) in self.lines.iter().rev().enumerate() {
            let y = height - LINE * (i as i32 + 2) - 5;
            if y < 4 {
                break;
            }
            hud.text(frame, 12, y, 0.85, line, [0.84, 0.90, 0.95]);
        }
    }
}
