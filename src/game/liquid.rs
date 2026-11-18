use crate::{
    collision::PLAYER_HEIGHT,
    map::{Map, Vec3},
    movement::Zone,
    sound::{CONTACT_SOUND_RADIUS, SoundEvents},
};

pub(super) struct LiquidState {
    pain_at: Option<f32>,
    drown_at: Option<f32>,
    last_feet: Option<Zone>,
    splash_at: f32,
}

impl Default for LiquidState {
    fn default() -> Self {
        Self {
            pain_at: None,
            drown_at: None,
            last_feet: None,
            splash_at: f32::NEG_INFINITY,
        }
    }
}

pub(super) enum Impact {
    Splash(u16),
    Drown(u16),
    Heal(u16),
}

impl LiquidState {
    pub fn step(
        &mut self,
        map: &Map,
        center: Vec3,
        eye: Vec3,
        now: f32,
        local: bool,
        sounds: &mut SoundEvents,
    ) -> Option<Impact> {
        let regions = &map.collision.regions;
        let foot = center - Vec3(0.0, 0.0, PLAYER_HEIGHT);
        let feet = regions.at(foot);
        let head = regions.at(eye);
        if let Some(previous) = self.last_feet.replace(feet)
            && previous.water != feet.water
            && now - self.splash_at >= 0.25
        {
            self.splash_at = now;
            let zone = if feet.water { feet } else { previous };
            if let Some([entry, exit]) = zone.liquid_audio.and_then(|id| map.liquid_audio.get(id)) {
                let sound = if !feet.water {
                    exit.as_ref()
                } else if entry.is_some()
                    && map
                        .collision
                        .ray(foot + Vec3(0.0, 0.0, 8.0), Vec3(0.0, 0.0, -32.0))
                        .is_some()
                {
                    map.player_audio.water_step.as_ref().or(entry.as_ref())
                } else {
                    entry.as_ref()
                };
                sounds.extend_at(sound.cloned(), foot, CONTACT_SOUND_RADIUS, 1.0);
            }
        }
        if !head.water
            && let Some(deadline) = self.drown_at.take()
        {
            let remaining = deadline - now;
            if remaining < 8.0 {
                let sound = map.player_audio.gasp[usize::from(remaining >= 2.0)]
                    .iter()
                    .cloned();
                if local {
                    sounds.extend(sound);
                } else {
                    sounds.extend_at(sound, eye, CONTACT_SOUND_RADIUS, 0.8);
                }
            }
        }
        if feet.pain {
            self.drown_at = None;
            let next = self.pain_at.get_or_insert(now + 0.01);
            if now < *next {
                return None;
            }
            *next += 1.0;
            let depth = 0.4 + 0.4 * f32::from(regions.at(center).pain) + 0.2 * f32::from(head.pain);
            let damage = feet.damage_per_second as f32 * depth;
            return if damage > 0.0 {
                Some(Impact::Splash(damage as u16))
            } else if damage < 0.0 {
                Some(Impact::Heal((-damage) as u16))
            } else {
                None
            };
        }
        self.pain_at = None;
        if head.water {
            let next = self
                .drown_at
                .get_or_insert(now + map.collision.profile.underwater_time);
            if now >= *next {
                *next += 2.0;
                return Some(Impact::Drown(5));
            }
        }
        None
    }
}
