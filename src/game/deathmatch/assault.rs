use crate::{
    assault::{AssaultMap, RelayAction, Target, TriggerState},
    map::{Map, Vec3},
    sound::{BLAST_SOUND_RADIUS, SoundEvents},
};

use super::{Fighter, distance_squared};

struct FortState {
    health: i32,
    complete: bool,
    priority: u8,
    damage_pointer: usize,
}

struct TriggerStatus {
    enabled: bool,
    spent: bool,
    touching: u16,
}

pub(super) struct Round {
    pub defenses: super::turrets::Defenses,
    pub environment: super::assault_effects::Environment,
    pub debris: super::breakables::Debris,
    pub attack_team: usize,
    pub limit: f32,
    pub first_attack: Option<Option<f32>>,
    pub completed_at: Option<f32>,
    pub tutorial: Option<Briefing>,
    pub notice: Option<(usize, f32)>,
    forts: Vec<FortState>,
    triggers: Vec<TriggerStatus>,
    counts: Vec<usize>,
    dispatch_until: Vec<f32>,
    pending: Vec<(f32, usize, bool)>,
}

pub(super) struct Briefing {
    pub allies: usize,
    pub subtitle: Option<usize>,
    pub ready: bool,
    next_at: f32,
    next: usize,
    protected_fort: Option<usize>,
}

impl Round {
    pub fn new(map: &AssaultMap, attack_team: usize, first_attack: Option<Option<f32>>) -> Self {
        Self {
            defenses: super::turrets::Defenses::new(map),
            environment: super::assault_effects::Environment::new(&map.effects, attack_team),
            debris: super::breakables::Debris::new(&map.effects, attack_team),
            attack_team,
            limit: first_attack.flatten().unwrap_or(f32::from(map.seconds)),
            first_attack,
            completed_at: None,
            tutorial: map.tutorial.as_ref().map(|data| Briefing {
                allies: data.allies,
                subtitle: None,
                ready: false,
                next_at: 3.0,
                next: 0,
                protected_fort: data.protected_fort,
            }),
            notice: None,
            forts: map
                .forts
                .iter()
                .map(|fort| FortState {
                    health: fort.health,
                    complete: false,
                    priority: fort.priority,
                    damage_pointer: 0,
                })
                .collect(),
            triggers: map
                .triggers
                .iter()
                .map(|trigger| TriggerStatus {
                    enabled: trigger.active,
                    spent: false,
                    touching: 0,
                })
                .collect(),
            counts: map
                .relays
                .iter()
                .map(|relay| match relay.action {
                    RelayAction::Count { count, .. } => count.max(1),
                    _ => 0,
                })
                .collect(),
            dispatch_until: vec![f32::NEG_INFINITY; map.relays.len()],
            pending: Vec::new(),
        }
    }

    pub fn emit(&mut self, time: f32, event: usize, attacker: bool) {
        self.pending.push((time, event, attacker));
    }

    pub fn conquered_at(&self) -> Option<f32> {
        self.completed_at.filter(|&time| time <= self.limit)
    }

    pub fn briefing_step(
        &mut self,
        data: &crate::assault::Tutorial,
        time: f32,
        sounds: &mut SoundEvents,
    ) -> bool {
        let Some(state) = &mut self.tutorial else {
            return false;
        };
        if state.ready || time < state.next_at {
            return false;
        }
        if let Some(lesson) = data.lessons.get(state.next) {
            sounds.push(lesson.audio.clone());
            state.subtitle = Some(state.next);
            state.next += 1;
            state.next_at += lesson.audio.duration().floor() + 2.0;
            return false;
        }
        state.ready = true;
        state.subtitle = None;
        true
    }

    fn protected(&self, tag: Option<usize>) -> bool {
        self.tutorial
            .as_ref()
            .is_some_and(|state| !state.ready && tag.is_some() && tag == state.protected_fort)
    }

    fn finish_fort(&mut self, map: &AssaultMap, index: usize, time: f32) {
        let fort = &map.forts[index];
        if self.protected(fort.tag) {
            return;
        }
        let state = &mut self.forts[index];
        if state.complete {
            return;
        }
        state.complete = true;
        let priority = state.priority;
        if let Some(fallback) = fort.fallback
            && let Some(target) = map
                .forts
                .iter()
                .position(|other| other.tag == Some(fallback))
        {
            self.forts[target].priority = priority;
        }
        if let Some(event) = fort.event {
            self.emit(time, event, true);
        }
        self.notice = Some((index, time + 3.0));
        if (fort.final_fort || self.forts.iter().all(|state| state.complete))
            && self.completed_at.is_none()
        {
            self.completed_at = Some(time);
            if time <= self.limit
                && let Some(event) = map.end_camera.as_ref().and_then(|camera| camera.event)
            {
                self.emit(time, event, false);
            }
        }
    }

    pub fn tick(
        &mut self,
        map: &mut Map,
        fighters: &[Fighter],
        time: f32,
        sounds: &mut SoundEvents,
    ) {
        let Some(data) = &map.assault else {
            return;
        };
        self.pending
            .extend(self.environment.timers(&data.effects, time));
        for (index, fort) in data.forts.iter().enumerate() {
            if fort.touch
                && !self.forts[index].complete
                && fighters.iter().any(|fighter| {
                    fighter.health > 0
                        && fighter.team == Some(self.attack_team)
                        && fort.volume.touches(fighter.walker.position, 17.0, 39.0)
                })
            {
                self.finish_fort(data, index, time);
            }
        }
        for (index, trigger) in data.triggers.iter().enumerate() {
            let status = &mut self.triggers[index];
            if trigger.damage.is_some() || !status.enabled || status.spent {
                if !status.enabled && status.touching != 0 && !status.spent {
                    self.environment.release(&data.effects, trigger.event);
                    for mover in &mut map.movers {
                        mover.release_signal(trigger.event);
                    }
                }
                status.touching = 0;
                continue;
            }
            let mut touching = 0;
            let mut attacker = false;
            for (id, fighter) in fighters.iter().enumerate() {
                if fighter.health > 0
                    && (!trigger.attack_only || fighter.team == Some(self.attack_team))
                    && trigger.team.is_none_or(|team| fighter.team == Some(team))
                    && trigger.volume.touches(fighter.walker.position, 17.0, 39.0)
                {
                    touching |= 1 << id;
                    attacker |= (status.touching & (1 << id) == 0)
                        && fighter.team == Some(self.attack_team);
                }
            }
            let entered = status.touching == 0 && touching != 0;
            let left = status.touching != 0 && touching == 0;
            status.touching = touching;
            if entered {
                self.activate_trigger(data, index, time, attacker);
            } else if left {
                self.environment.release(&data.effects, trigger.event);
                for mover in &mut map.movers {
                    mover.release_signal(trigger.event);
                }
            }
        }

        // A bounded queue lets event chains run in order even if malformed actors form a cycle.
        for _ in 0..128 {
            let Some(index) = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, event)| event.0 <= time)
                .min_by(|(_, a), (_, b)| a.0.total_cmp(&b.0))
                .map(|(index, _)| index)
            else {
                break;
            };
            let (due, event, attacker) = self.pending.remove(index);
            if attacker {
                for (i, _) in data
                    .forts
                    .iter()
                    .enumerate()
                    .filter(|(_, fort)| fort.tag == Some(event))
                {
                    self.finish_fort(data, i, due);
                }
            }
            for (trigger, status) in data.triggers.iter().zip(&mut self.triggers) {
                if trigger.tag != Some(event) || status.spent {
                    continue;
                }
                status.enabled = match trigger.state {
                    TriggerState::Unchanged => status.enabled,
                    TriggerState::Enable => true,
                    TriggerState::Disable => false,
                    TriggerState::Toggle => !status.enabled,
                };
            }
            for (i, relay) in data
                .relays
                .iter()
                .enumerate()
                .filter(|(_, relay)| relay.tag == event)
            {
                match &relay.action {
                    RelayAction::Dispatch(events) if due > self.dispatch_until[i] => {
                        self.dispatch_until[i] = due + events.last().map_or(0.0, |entry| entry.0);
                        for &(delay, out) in events {
                            self.emit(due + delay, out, attacker);
                        }
                    }
                    RelayAction::Dispatch(_) => {}
                    RelayAction::Count { event, .. } if self.counts[i] > 0 => {
                        self.counts[i] -= 1;
                        if self.counts[i] == 0 {
                            self.emit(due, *event, attacker);
                        }
                    }
                    RelayAction::Count { .. } => {}
                    RelayAction::Starts(starts) => {
                        for &start in starts {
                            if let Some(start) = map.starts.get_mut(start) {
                                start.enabled = !start.enabled;
                            }
                        }
                    }
                    RelayAction::Sound {
                        wave,
                        position,
                        local,
                        ..
                    } => {
                        if *local {
                            sounds.extend(wave.iter().cloned());
                        } else {
                            sounds.extend_at(
                                wave.iter().cloned(),
                                *position,
                                BLAST_SOUND_RADIUS,
                                1.0,
                            );
                        }
                    }
                }
            }
            for mover in &mut map.movers {
                mover.signal(event, attacker);
            }
            self.defenses.signal(data, event, due, sounds);
            self.environment.signal(&data.effects, event, due, attacker);
            for follow in self.debris.signal(&data.effects, event, due) {
                self.emit(due, follow, attacker);
            }
        }
    }

    fn activate_trigger(&mut self, data: &AssaultMap, index: usize, time: f32, attacker: bool) {
        let trigger = &data.triggers[index];
        self.triggers[index].spent |= trigger.once;
        self.emit(time, trigger.event, attacker);
    }

    pub fn trace(
        &self,
        data: &AssaultMap,
        start: Vec3,
        delta: Vec3,
        attacker: bool,
    ) -> Option<(f32, Target)> {
        let forts =
            data.forts
                .iter()
                .zip(&self.forts)
                .enumerate()
                .filter_map(|(index, (fort, state))| {
                    if !attacker || fort.touch || state.complete || self.protected(fort.tag) {
                        return None;
                    }
                    Some((fort.volume.trace(start, delta)?, Target::Fort(index)))
                });
        let triggers = data
            .triggers
            .iter()
            .zip(&self.triggers)
            .enumerate()
            .filter_map(|(index, (trigger, state))| {
                if trigger.damage.is_none()
                    || !state.enabled
                    || state.spent
                    || (trigger.attack_only && !attacker)
                {
                    return None;
                }
                Some((trigger.volume.trace(start, delta)?, Target::Trigger(index)))
            });
        let turrets = data
            .turrets
            .iter()
            .zip(&self.defenses.guns)
            .enumerate()
            .filter_map(|(index, (turret, gun))| {
                if !gun.targetable() {
                    return None;
                }
                Some((turret.volume.trace(start, delta)?, Target::Turret(index)))
            });
        let charges = data
            .effects
            .charges
            .iter()
            .enumerate()
            .filter_map(|(index, charge)| {
                if !self.environment.targetable(&data.effects, index) {
                    return None;
                }
                Some((charge.volume.trace(start, delta)?, Target::Charge(index)))
            });
        let walls = data
            .effects
            .walls
            .iter()
            .enumerate()
            .filter_map(|(index, wall)| {
                if !self.debris.targetable(&data.effects, index) {
                    return None;
                }
                Some((wall.volume.trace(start, delta)?, Target::Wall(index)))
            });
        forts
            .chain(triggers)
            .chain(turrets)
            .chain(charges)
            .chain(walls)
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    pub fn damage(
        &mut self,
        data: &AssaultMap,
        target: Target,
        damage: u16,
        time: f32,
        attacker: bool,
        damage_type: &str,
    ) {
        if damage == 0 {
            return;
        }
        match target {
            Target::Wall(index) => {
                if let Some(event) = self.debris.strike(
                    &data.effects,
                    index,
                    damage_type,
                    damage,
                    time,
                    Vec3::default(),
                ) {
                    self.emit(time, event, attacker);
                }
            }
            Target::Charge(index) => self.environment.arm(&data.effects, index, time),
            Target::Turret(index) => self.defenses.damage(index, damage, time),
            Target::Trigger(index) => {
                let state = &self.triggers[index];
                if state.enabled
                    && !state.spent
                    && (!data.triggers[index].attack_only || attacker)
                    && data.triggers[index]
                        .damage
                        .is_some_and(|threshold| f32::from(damage) >= threshold)
                {
                    self.activate_trigger(data, index, time, attacker);
                }
            }
            Target::Fort(index) => {
                let fort = &data.forts[index];
                if self.protected(fort.tag) {
                    return;
                }
                let state = &mut self.forts[index];
                if !attacker || state.complete || fort.touch {
                    return;
                }
                state.health = state.health.saturating_sub(i32::from(damage));
                if state.health < 0 {
                    self.finish_fort(data, index, time);
                } else if let Some(&(threshold, event)) =
                    fort.damage_events.get(state.damage_pointer)
                    && fort.health.saturating_sub(state.health) as f32 >= threshold
                {
                    state.damage_pointer += 1;
                    self.emit(time, event, true);
                }
            }
        }
    }

    pub fn destination(&self, map: &AssaultMap, position: Vec3) -> Option<usize> {
        self.forts
            .iter()
            .enumerate()
            .filter(|(_, state)| !state.complete)
            .max_by(|&(a, a_state), &(b, b_state)| {
                a_state.priority.cmp(&b_state.priority).then_with(|| {
                    distance_squared(map.forts[b].destination, position)
                        .total_cmp(&distance_squared(map.forts[a].destination, position))
                })
            })
            .map(|(index, _)| index)
    }

    pub fn active(&self, index: usize) -> bool {
        !self.forts[index].complete
    }

    pub fn completed(&self) -> usize {
        self.forts.iter().filter(|state| state.complete).count()
    }
}
