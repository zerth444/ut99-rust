use crate::{
    assault::Volume,
    collision::{PLAYER_HEIGHT, PLAYER_RADIUS},
    map::{Map, Vec3},
    sound::{CONTACT_SOUND_RADIUS, SoundEvents},
};

use super::{Data, Fighter, Session};

struct Contact {
    fraction: f32,
    normal: Vec3,
    barrel: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collision::Collision, package::Package};

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn barrels_stop_on_contact_stack_and_damage_their_support_on_a_hard_landing() {
        let path = std::path::Path::new("local/reference-client/Maps/DM-Tutorial.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        map.collision = Collision::test_world();
        map.collision
            .add_box(Vec3(-400.0, -400.0, -64.0), Vec3(400.0, 400.0, 0.0));
        let data = map.dm_training.as_ref().unwrap();
        let mut session = Session::new(data);
        session.barrels_revealed = true;
        session.barrels[2].health = -1;
        let radius = data.barrels[0].volume.radius + data.barrels[1].volume.radius;
        let height = data.barrels[0].volume.height + data.barrels[1].volume.height;
        let base = Vec3(0.0, 0.0, data.barrels[1].volume.height + 0.2);
        let incoming = &mut session.barrels[0];
        incoming.volume.position = Vec3(-radius - 4.0, 0.0, data.barrels[0].volume.height + 0.2);
        incoming.velocity = Vec3(1000.0, 0.0, 10.0);
        incoming.falling = true;
        session.barrels[1].volume.position = base;
        let mut sounds = SoundEvents::default();
        let dt = 1.0 / 120.0;
        session.move_barrels(&map, &mut [], dt, dt, &mut sounds);
        assert!((session.barrels[0].volume.position.0 + radius + 0.2).abs() < 0.01);
        assert_eq!(
            session.barrels[0].velocity.dot(session.barrels[0].velocity),
            0.0
        );
        assert_eq!(
            (session.barrels[1].volume.position - base)
                .dot(session.barrels[1].volume.position - base),
            0.0
        );

        let top = base + Vec3(0.0, 0.0, height + 0.2);
        session.barrels[0].volume.position = top + Vec3(0.0, 0.0, 8.0);
        for step in 2..=60 {
            session.move_barrels(&map, &mut [], step as f32 * dt, dt, &mut sounds);
        }
        assert!((session.barrels[0].volume.position.2 - top.2).abs() < 0.01);
        assert!(!session.barrels[0].falling);

        session.barrels[0].health = 10;
        session.barrels[1].health = 10;
        session.barrels[0].volume.position = top + Vec3(0.0, 0.0, 1.0);
        session.barrels[0].velocity = Vec3(0.0, 0.0, -700.0);
        session.barrels[0].falling = true;
        session.move_barrels(&map, &mut [], 61.0 * dt, dt, &mut sounds);
        assert!(session.barrels[0].health < 0 && session.barrels[1].health < 0);
        assert_eq!(session.debris.visuals(&data.debris).count(), 24);
    }
}

impl Session {
    pub fn move_barrels(
        &mut self,
        map: &Map,
        fighters: &mut [Fighter],
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
    ) {
        let Some(data) = map.dm_training.as_ref().filter(|_| self.barrels_revealed) else {
            return;
        };
        for (index, barrel) in data.barrels.iter().enumerate() {
            self.advance_barrel(map, data, index, time, dt);
            let state = &mut self.barrels[index];
            if state.health < 0 {
                continue;
            }
            if state.push_until.is_some_and(|until| time >= until) {
                state.push_until = None;
                sounds.extend_at(
                    data.push_sounds[1].iter().cloned(),
                    state.volume.position,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            }
            for fighter in fighters.iter_mut().filter(|fighter| fighter.health > 0) {
                let delta = fighter.walker.position - state.volume.position;
                let distance = delta.0.hypot(delta.1);
                let radius = PLAYER_RADIUS + state.volume.radius;
                if distance >= radius || delta.2.abs() >= PLAYER_HEIGHT + state.volume.height {
                    continue;
                }
                let velocity = fighter.walker.velocity();
                let outward = if distance > 0.001 {
                    Vec3(delta.0 / distance, delta.1 / distance, 0.0)
                } else {
                    Vec3(-fighter.yaw.cos(), -fighter.yaw.sin(), 0.0)
                };
                fighter
                    .walker
                    .carry(&map.collision, outward * (radius - distance));
                if barrel.fixed
                    || !barrel.pushable
                    || map.collision.profile.mass <= 40.0
                    || velocity.dot(outward) >= 0.0
                {
                    continue;
                }
                let speed = velocity.dot(velocity).sqrt();
                let push = velocity * ((20.0 + speed).min(120.0) / speed);
                state.velocity = Vec3(
                    push.0,
                    push.1,
                    if state.falling {
                        state.velocity.2
                    } else {
                        25.0
                    },
                );
                state.falling = true;
                if state.push_until.is_none() {
                    sounds.extend_at(
                        data.push_sounds[0].iter().cloned(),
                        state.volume.position,
                        CONTACT_SOUND_RADIUS,
                        0.25,
                    );
                }
                state.push_until = Some(time + 0.3);
            }
        }
    }

    fn advance_barrel(&mut self, map: &Map, data: &Data, index: usize, time: f32, dt: f32) {
        let state = &self.barrels[index];
        let barrel = &data.barrels[index];
        if state.health < 0 || barrel.fixed {
            return;
        }
        let volume = state.volume;
        if !state.falling {
            let down = Vec3(0.0, 0.0, -1.0);
            if !barrel.pushable
                || self
                    .barrel_contact(map, index, volume, down)
                    .is_some_and(|hit| hit.normal.2 > 0.7)
            {
                return;
            }
        }
        let velocity = state.velocity + map.collision.regions.at(volume.position).gravity * dt;
        let travel = velocity * dt;
        let hit = self.barrel_contact(map, index, volume, travel);
        let state = &mut self.barrels[index];
        let Some(hit) = hit else {
            state.volume.position = volume.position + travel;
            state.velocity = velocity;
            state.falling = true;
            return;
        };
        state.volume.position = volume.position + travel * hit.fraction + hit.normal * 0.2;
        state.velocity = Vec3::default();
        state.falling = false;
        if let Some(base) = hit.barrel {
            if hit.normal.2 > 0.7 && velocity.2 < -500.0 {
                let damage =
                    (1.0 - barrel.mass / data.barrels[base].mass * velocity.2 / 30.0) as u16;
                let impact = (1.0 - velocity.2 / 30.0) as u16;
                self.damage(data, base + 1, damage, time, velocity * 0.2, "stomped");
                self.damage(data, index + 1, impact, time, Vec3::default(), "crushed");
                let state = &mut self.barrels[index];
                if state.health >= 0 {
                    let side = if crate::game::ballistics::jitter(&mut self.rng) < 0.0 {
                        -70.0
                    } else {
                        70.0
                    };
                    state.velocity = Vec3(side, 0.0, 100.0);
                }
            }
        } else if velocity.2 < -200.0 {
            state.health = state.health.saturating_sub(100);
            if state.health < 0 {
                self.debris.break_decoration(
                    &data.debris,
                    index,
                    time,
                    state.volume.position,
                    hit.normal * 10_000.0,
                    1.75,
                );
            }
        }
    }

    fn barrel_contact(
        &self,
        map: &Map,
        index: usize,
        from: Volume,
        delta: Vec3,
    ) -> Option<Contact> {
        let travel = delta.dot(delta);
        if travel <= f32::EPSILON {
            return None;
        }
        let world = map
            .collision
            .projectile_surface(from.position, delta, from.radius, from.height)
            .map(|(point, normal)| Contact {
                fraction: (point - from.position).dot(delta) / travel,
                normal,
                barrel: None,
            });
        self.barrels
            .iter()
            .enumerate()
            .filter_map(|(other, state)| {
                if other == index || state.health < 0 {
                    return None;
                }
                let expanded = Volume {
                    position: state.volume.position,
                    radius: state.volume.radius + from.radius,
                    height: state.volume.height + from.height,
                };
                let fraction = expanded.trace(from.position, delta)?;
                let offset = from.position + delta * fraction - expanded.position;
                let radial_gap = expanded.radius - offset.0.hypot(offset.1);
                let height_gap = expanded.height - offset.2.abs();
                let normal = if height_gap <= radial_gap + 0.001 {
                    Vec3(0.0, 0.0, offset.2.signum())
                } else {
                    crate::mover::unit(Vec3(offset.0, offset.1, 0.0))
                };
                (normal.dot(delta) < 0.0).then_some(Contact {
                    fraction,
                    normal,
                    barrel: Some(other),
                })
            })
            .chain(world)
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
    }
}
