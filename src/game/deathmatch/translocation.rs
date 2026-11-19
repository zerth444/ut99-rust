use std::sync::Arc;

use super::{Command, Hit, Match, PLAYER_HEIGHT, PLAYER_RADIUS, allied, cylinder_hit};
use crate::{
    collision::Walker,
    game::{Motion, Pose, Weapon},
    map::{Map, Vec3},
    sound::{CONTACT_SOUND_RADIUS, SoundEvents, Wave},
};

pub struct Art {
    pub module: Motion,
    pub launch_speed: f32,
    pub radius: f32,
    pub height: f32,
    pub threshold: f32,
    pub mass: f32,
    pub impact: Option<Arc<Wave>>,
    pub pickup: Option<Arc<Wave>>,
    pub disrupted: Option<Arc<Wave>>,
    pub teleport: Option<Arc<Wave>>,
    pub offset: Vec3,
}

pub(super) struct Module {
    pub point: Vec3,
    pub velocity: Vec3,
    pub born: f32,
    pub landed: Option<f32>,
    pub damage: u16,
    pub disruptor: Option<usize>,
    bounced_on_floor: bool,
    support: Option<(usize, Vec3)>,
}

impl Module {
    pub fn new(point: Vec3, velocity: Vec3, born: f32) -> Self {
        Self {
            point,
            velocity,
            born,
            landed: None,
            damage: 0,
            disruptor: None,
            bounced_on_floor: false,
            support: None,
        }
    }

    pub fn advance(&mut self, map: &Map, art: &Art, time: f32, dt: f32, sounds: &mut SoundEvents) {
        if self.landed.is_some() {
            if let Some((index, relative)) = self.support {
                self.point = map.movers[index].offset + relative;
            }
            return;
        }
        self.velocity = self.velocity + map.collision.regions.at(self.point).gravity * dt;
        let delta = self.velocity * dt;
        if let Some((point, normal, hull)) = map
            .collision
            .projectile_contact(self.point, delta, art.radius, art.height)
        {
            self.point = point + normal * 0.25;
            if normal.2 > 0.7 && self.bounced_on_floor {
                self.landed = Some(time);
                self.velocity = Vec3::default();
                self.support = map
                    .movers
                    .iter()
                    .enumerate()
                    .find(|(_, mover)| mover.collision_hull() == Some(hull))
                    .map(|(index, mover)| (index, self.point - mover.offset));
                return;
            }
            self.bounced_on_floor |= normal.2 > 0.7;
            self.velocity = (self.velocity - normal * (2.0 * self.velocity.dot(normal))) * 0.3;
            sounds.extend_at(
                art.impact.iter().cloned(),
                self.point,
                CONTACT_SOUND_RADIUS,
                0.75,
            );
        } else {
            self.point = self.point + delta;
        }
    }

    pub fn hit(&mut self, art: &Art, damage: u16, attacker: usize, impulse: Vec3, hostile: bool) {
        self.landed = None;
        self.support = None;
        self.bounced_on_floor = false;
        self.velocity = impulse * art.mass.recip();
        self.velocity.2 = self
            .velocity
            .2
            .max(0.7 * self.velocity.dot(self.velocity).sqrt());
        if hostile {
            self.damage = self.damage.saturating_add(damage);
            self.disruptor = Some(attacker);
        }
    }
}

impl Match {
    pub(super) fn move_translocators(&mut self, map: &Map, dt: f32, sounds: &mut SoundEvents) {
        let Some(art) = equipment(map) else { return };
        for fighter in &mut self.fighters {
            let Some(module) = &mut fighter.translocator else {
                continue;
            };
            if fighter.health == 0 || module.point.2 < self.kill_z {
                fighter.translocator = None;
                continue;
            }
            module.advance(map, art, self.time, dt, sounds);
            let distance = fighter.walker.position - module.point;
            if module.landed.is_some()
                && distance.0.hypot(distance.1) < PLAYER_RADIUS + art.radius
                && distance.2.abs() < PLAYER_HEIGHT + art.height
                && map.collision.ray(module.point, distance).is_none()
            {
                sounds.extend_at(
                    art.pickup.iter().cloned(),
                    module.point,
                    CONTACT_SOUND_RADIUS,
                    0.8,
                );
                fighter.translocator = None;
            }
        }
    }

    pub(super) fn use_translocator(
        &mut self,
        map: &Map,
        id: usize,
        input: Command,
        hits: &mut Vec<Hit>,
        sounds: &mut SoundEvents,
    ) {
        let Some(art) = equipment(map) else { return };
        let fighter = &mut self.fighters[id];
        if (!input.primary && !input.alternate) || self.time < fighter.ready_at {
            return;
        }
        let start = fighter.walker.position;
        let alt = input.alternate && !input.primary;
        if alt {
            if let Some(module) = &fighter.translocator {
                self.objectives.drop_for_teleport(id, start, self.time, map);
                let dest =
                    module.point + Vec3(0.0, 0.0, if module.landed.is_some() { 40.0 } else { 0.0 });
                let clear = map.collision.player_fits(dest)
                    && map
                        .collision
                        .ray(module.point, dest - module.point)
                        .is_none();
                let disruptor =
                    (f32::from(module.damage) > art.threshold).then_some(module.disruptor);
                let module = fighter.translocator.take().unwrap();
                fighter.fired_at = self.time;
                fighter.last_alt = true;
                fighter.ready_at = self.time + 0.1;
                if !clear {
                    super::super::emit_shot_sound(
                        map,
                        Weapon::Translocator,
                        true,
                        (id != 0).then_some(start),
                        sounds,
                    );
                } else {
                    fighter.walker = Walker::spawn(&map.collision, dest);
                    let arrival = fighter.walker.position;
                    fighter.capture_body();
                    fighter.airborne_since = None;
                    fighter.power.boosted_airborne = false;
                    fighter.portal = None;
                    let team = fighter.team;
                    if let Some(disruptor) = disruptor {
                        hits.push(telefrag(id, disruptor));
                        sounds.extend_at(
                            art.disrupted.iter().cloned(),
                            module.point,
                            CONTACT_SOUND_RADIUS,
                            1.0,
                        );
                    } else {
                        for (victim, other) in
                            self.fighters.iter().enumerate().filter(|(victim, other)| {
                                *victim != id && other.health > 0 && !allied(team, other.team)
                            })
                        {
                            let delta = other.walker.position - arrival;
                            if delta.0.hypot(delta.1) < 2.0 * PLAYER_RADIUS
                                && delta.2.abs() < 2.0 * PLAYER_HEIGHT
                                && map.collision.ray(arrival, delta).is_none()
                            {
                                hits.push(telefrag(victim, Some(id)));
                            }
                        }
                        sounds.extend_at(
                            art.teleport.iter().cloned(),
                            arrival,
                            CONTACT_SOUND_RADIUS,
                            1.0,
                        );
                    }
                }
            }
        } else if let Some(module) = &fighter.translocator {
            if self.time - module.born <= 0.8 {
                return;
            }
            if f32::from(module.damage) > art.threshold {
                hits.push(telefrag(id, module.disruptor));
                sounds.extend_at(
                    art.disrupted.iter().cloned(),
                    start,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            } else {
                super::super::emit_shot_sound(
                    map,
                    Weapon::Translocator,
                    true,
                    (id != 0).then_some(start),
                    sounds,
                );
            }
            fighter.translocator = None;
            fighter.translocator_ready = self.time + 0.5;
        } else if self.time >= fighter.translocator_ready {
            let aim = fighter.camera(map);
            let direction = aim.look_direction();
            let offset = direction * art.offset.0
                + aim.right() * art.offset.1
                + Vec3(0.0, 0.0, art.offset.2);
            let point = map
                .collision
                .projectile_surface(aim.position, offset, art.radius, art.height)
                .map_or(aim.position + offset, |(point, normal)| {
                    point + normal * 0.25
                });
            let lob = Vec3(0.0, 0.0, 0.35 * (1.0 - direction.2.abs()));
            let velocity = super::unit(direction + lob) * art.launch_speed;
            fighter.translocator = Some(Module::new(point, velocity, self.time));
            fighter.fired_at = self.time;
            fighter.last_alt = false;
            fighter.ready_at = self.time + 0.1;
            fighter.translocator_ready = self.time + 0.6;
            super::super::emit_shot_sound(
                map,
                Weapon::Translocator,
                false,
                (id != 0).then_some(start),
                sounds,
            );
        }
    }

    pub(super) fn translocator_hit(
        &self,
        map: &Map,
        attacker: usize,
        start: Vec3,
        delta: Vec3,
    ) -> Option<(usize, f32)> {
        let art = equipment(map)?;
        self.fighters
            .iter()
            .enumerate()
            .filter(|(id, _)| *id != attacker)
            .filter_map(|(id, fighter)| {
                Some((
                    id,
                    cylinder_hit(
                        start - fighter.translocator.as_ref()?.point,
                        delta,
                        art.radius,
                        art.height,
                    )?,
                ))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    pub(super) fn damage_translocator(
        &mut self,
        map: &Map,
        owner: usize,
        attacker: usize,
        damage: u16,
        impulse: Vec3,
    ) {
        let Some(art) = equipment(map) else { return };
        let hostile = !allied(self.fighters[owner].team, self.fighters[attacker].team);
        if let Some(module) = &mut self.fighters[owner].translocator {
            module.hit(art, damage, attacker, impulse, hostile);
        }
    }

    pub(super) fn blast_translocators(
        &mut self,
        map: &Map,
        attacker: usize,
        blast: &super::shock::Blast,
        scale: u16,
    ) {
        let Some(art) = equipment(map) else { return };
        let team = self.fighters[attacker].team;
        for (id, fighter) in self.fighters.iter_mut().enumerate() {
            if let Some(module) = &mut fighter.translocator
                && let Some((damage, momentum)) =
                    blast.hit_actor(&map.collision, module.point, art.radius)
                && damage > 0
            {
                module.hit(
                    art,
                    damage.saturating_mul(scale),
                    attacker,
                    momentum,
                    id != attacker && !allied(team, fighter.team),
                );
            }
        }
    }

    pub(crate) fn translocator_modules<'a>(
        &'a self,
        map: &'a Map,
    ) -> impl Iterator<Item = (Vec3, Vec3, Pose<'a>)> + 'a {
        let art = equipment(map);
        self.fighters.iter().filter_map(move |fighter| {
            let art = art?;
            let module = fighter.translocator.as_ref()?;
            let pose = if let Some(landed) = module.landed {
                art.module.hold_pose(self.time - landed)?
            } else {
                art.module.sample(0.0)?
            };
            let orientation = Vec3(module.velocity.0, module.velocity.1, 0.0);
            let direction = if orientation.dot(orientation) > 0.1 {
                super::unit(orientation)
            } else {
                Vec3(1.0, 0.0, 0.0)
            };
            Some((module.point, direction, pose))
        })
    }
}

fn equipment(map: &Map) -> Option<&Art> {
    map.arsenal
        .get(&Weapon::Translocator)?
        .translocator
        .as_ref()
}

fn telefrag(victim: usize, owner: Option<usize>) -> Hit {
    Hit {
        victim,
        owner,
        damage: u16::MAX,
        bypass_armor: true,
        impulse: Vec3::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collision::Collision,
        game::deathmatch::{Mode, Rules},
        package::Package,
    };

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn installed_translocator_launches_lands_and_drops_the_carried_flag() {
        let path = std::path::Path::new("local/reference-client/Maps/CTF-Tutorial.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let art = &map.arsenal[&Weapon::Translocator];
        assert!(!art.view.is_empty());
        assert!(!art.primary_motion.frames.is_empty() && !art.alternate_motion.frames.is_empty());
        assert!(art.fire.is_some() && art.alternate.is_some());
        let module_art = art.translocator.as_ref().unwrap();
        assert_eq!(module_art.module.frames.len(), 3);
        assert!(module_art.teleport.is_some() && module_art.disrupted.is_some());

        map.collision = Collision::test_world();
        map.collision
            .add_box(Vec3(-2200.0, -800.0, -128.0), Vec3(2200.0, 800.0, 0.0));
        map.flag_bases = [Some(Vec3(-300.0, 0.0, 39.2)), Some(Vec3(0.0, 0.0, 39.2))];
        let mut game = Match::new(
            &map,
            0,
            Rules {
                mode: Mode::CaptureTheFlag,
                bots: 0,
                ..Rules::default()
            },
        );
        let player = &mut game.fighters[0];
        player.walker = Walker::spawn(&map.collision, Vec3(0.0, 0.0, 39.2));
        player.yaw = 0.0;
        let start = player.walker.position;
        let mut hits = Vec::new();
        let mut sounds = SoundEvents::default();
        game.objectives.tick(&map, &mut game.fighters, 1.0);
        assert!(game.objectives.carries_flag(0));
        game.time = 1.0;
        game.fire(
            &map,
            0,
            Command {
                slot: Some(1),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert_eq!(game.fighters[0].weapon, Weapon::Translocator);
        assert_eq!(game.fighters[0].ammo(), None);
        game.time = 2.0;
        game.fire(
            &map,
            0,
            Command {
                primary: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert!(game.fighters[0].translocator.is_some());
        game.fire(
            &map,
            0,
            Command {
                slot: Some(2),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert_eq!(game.fighters[0].weapon, Weapon::Enforcer);
        for _ in 0..240 {
            game.time += 1.0 / 120.0;
            game.move_translocators(&map, 1.0 / 120.0, &mut sounds);
        }
        let module = game.fighters[0].translocator.as_ref().unwrap();
        assert!(module.landed.is_some());
        let destination = module.point;
        assert!(destination.0 > start.0 + 100.0);
        assert!(game.translocator_modules(&map).next().is_some());
        game.fire(
            &map,
            0,
            Command {
                slot: Some(1),
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        game.time += 1.0;
        game.fire(
            &map,
            0,
            Command {
                alternate: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert_eq!(game.fighters[0].walker.position.0, destination.0);
        assert!(game.fighters[0].translocator.is_none());
        assert_eq!(game.flag_icon_states(), Some([0, 2]));
        assert_eq!(game.team_scores(), Some([0, 0]));

        // An obstructed destination consumes the module and leaves the player in place.
        map.collision
            .add_box(Vec3(-20.0, -20.0, 35.0), Vec3(20.0, 20.0, 100.0));
        game.time += 1.0;
        game.fighters[0].translocator = Some(Module::new(start, Vec3::default(), game.time));
        let safe = game.fighters[0].walker.position;
        game.fire(
            &map,
            0,
            Command {
                alternate: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert!(super::super::distance_squared(game.fighters[0].walker.position, safe) < 0.001);
        assert!(game.fighters[0].translocator.is_none());

        // Enemy fire disables the module, and using a disrupted module kills its owner.
        game.add_fighter(&map);
        game.fighters[1].team = Some(1);
        game.time += 1.0;
        let target = safe + Vec3(100.0, 0.0, 0.0);
        game.fighters[0].translocator = Some(Module::new(target, Vec3::default(), game.time));
        assert_eq!(
            game.translocator_hit(&map, 1, target - Vec3(30.0, 0.0, 0.0), Vec3(60.0, 0.0, 0.0))
                .map(|(id, _)| id),
            Some(0)
        );
        game.damage_translocator(&map, 0, 1, u16::MAX, Vec3::default());
        game.fire(
            &map,
            0,
            Command {
                alternate: true,
                ..Command::default()
            },
            &mut hits,
            &mut sounds,
        );
        assert!(
            hits.iter()
                .any(|hit| hit.victim == 0 && hit.owner == Some(1) && hit.damage == u16::MAX)
        );
    }
}
