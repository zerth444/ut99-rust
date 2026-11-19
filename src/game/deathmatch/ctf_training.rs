use std::sync::Arc;

use crate::sound::{SoundEvents, Wave};

pub(crate) const LESSON_COUNT: usize = 27;

pub(crate) struct Data {
    pub lines: [(String, Arc<Wave>); LESSON_COUNT],
    pub bots: usize,
    pub allies: usize,
    pub teleport: Option<Arc<Wave>>,
}

pub(super) enum Event {
    ShowBases,
    StartMatch,
}

pub(super) struct Session {
    pub current: Option<usize>,
    pub next_at: f32,
    pub practice: bool,
    pub allies: usize,
    next: usize,
}

impl Session {
    pub fn new(data: &Data) -> Self {
        Self {
            current: None,
            next_at: 3.0,
            practice: false,
            allies: data.allies,
            next: 0,
        }
    }

    pub fn step(
        &mut self,
        data: &Data,
        time: f32,
        carrying: bool,
        captured: bool,
        sounds: &mut SoundEvents,
    ) -> Option<Event> {
        let advance = match self.next {
            18 => carrying,
            21 => captured,
            _ => time >= self.next_at,
        };
        if self.practice || !advance {
            return None;
        }
        let index = self.next;
        if index == 26 {
            self.current = None;
            self.practice = true;
            return Some(Event::StartMatch);
        }
        let (_, voice) = &data.lines[index];
        sounds.tutorial(voice.clone());
        self.current = Some(index);
        self.next = index + 1;
        self.next_at = time + voice.duration().floor() + 2.0;
        (index == 15).then_some(Event::ShowBases)
    }
}
