use crate::{
    assault::{AssaultMap, Turret},
    game::{Pose, Weapon, ballistics, decals, shock},
    map::{Map, Vec3},
    render::Camera,
    sound::{CONTACT_SOUND_RADIUS, SoundEvents},
};

use super::{Fighter, Hit, angle_delta, cylinder_hit, nearest_hit, unit};

pub(super) struct Defenses {
    pub guns: Vec<Gun>,
    pub shots: Vec<Shot>,
    shutdown_audio: Vec<usize>,
    smoke: Vec<Puff>,
}

pub(super) struct Gun {
    health: i32,
    disabled_until: f32,
    target: Option<Tracking>,
    pub yaw: f32,
    pitch: usize,
    active_since: f32,
    fired_at: f32,
    ready_at: f32,
    idle_since: f32,
    rng: u32,
    cadence: ballistics::Cadence,
    pub flash: Option<(Vec3, Vec3, f32)>,
    smoke_started: Option<(f32, usize)>,
}

pub(crate) struct Shot {
    pub turret: usize,
    pub position: Vec3,
    pub born: f32,
    velocity: Vec3,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct WarheadId {
    pub owner: usize,
    pub born: f32,
}

pub(super) struct Warhead {
    pub id: WarheadId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub collision: [f32; 2],
    pub warning: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Tracking {
    Pawn(usize),
    Warhead(WarheadId),
}

struct Puff {
    origin: Vec3,
    born: f32,
    variant: usize,
}

impl Gun {
    pub fn targetable(&self) -> bool {
        self.health >= 0
    }

    pub fn deployed(&self, turret: &Turret, now: f32) -> bool {
        self.target.is_some() && now >= self.active_since + turret.activate.duration()
    }

    pub fn pose<'a>(&self, turret: &'a Turret, now: f32) -> Option<Pose<'a>> {
        let duration = turret.activate.duration();
        if self.target.is_none() || now < self.active_since + duration {
            let progress = if self.target.is_none() {
                (1.0 - (now - self.idle_since).max(0.0) / 1.5).clamp(0.0, 1.0) * duration
            } else {
                now - self.active_since
            };
            return turret
                .activate
                .sample(progress.min((duration - 0.001).max(0.0)));
        }
        let fire = &turret.fire[self.pitch];
        fire.sample((now - self.fired_at).min((fire.duration() - 0.001).max(0.0)))
            .or_else(|| turret.activate.sample(0.0))
    }

    fn disable(&mut self, time: f32, until: f32) {
        self.health = -1;
        self.disabled_until = until;
        self.target = None;
        self.flash = None;
        self.idle_since = time;
        self.smoke_started = None;
    }
}

impl Defenses {
    pub fn new(map: &AssaultMap) -> Self {
        Self {
            guns: map
                .turrets
                .iter()
                .enumerate()
                .map(|(index, turret)| Gun {
                    health: turret.health,
                    disabled_until: f32::NEG_INFINITY,
                    target: None,
                    yaw: turret.yaw,
                    pitch: 0,
                    active_since: f32::NEG_INFINITY,
                    fired_at: f32::NEG_INFINITY,
                    ready_at: f32::NEG_INFINITY,
                    idle_since: f32::NEG_INFINITY,
                    rng: (index as u32).wrapping_mul(137).wrapping_add(1),
                    cadence: ballistics::Cadence::default(),
                    flash: None,
                    smoke_started: None,
                })
                .collect(),
            shots: Vec::new(),
            shutdown_audio: Vec::new(),
            smoke: Vec::new(),
        }
    }

    pub fn signal(&mut self, map: &AssaultMap, event: usize, time: f32, sounds: &mut SoundEvents) {
        for (turret, gun) in map.turrets.iter().zip(&mut self.guns) {
            if turret.tag == Some(event) && gun.disabled_until != f32::INFINITY {
                gun.disable(time, f32::INFINITY);
                sounds.extend_at(
                    turret.audio[1].iter().cloned(),
                    turret.volume.position,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            }
        }
    }

    pub fn damage(&mut self, index: usize, damage: u16, time: f32) {
        let gun = &mut self.guns[index];
        if !gun.targetable() {
            return;
        }
        gun.health = gun.health.saturating_sub(i32::from(damage));
        if gun.health < 0 {
            gun.disable(time, time + 16.5);
            gun.smoke_started = Some((time, 0));
            self.shutdown_audio.push(index);
        }
    }

    fn emit_smoke(&mut self, map: &AssaultMap, time: f32) {
        let Some(art) = &map.turret_smoke else {
            return;
        };
        self.smoke.retain(|puff| time < puff.born + art.lifetime);
        for (turret, gun) in map.turrets.iter().zip(&mut self.guns) {
            let Some((start, emitted)) = gun.smoke_started else {
                continue;
            };
            let offsets = [0.0, 1.5, 2.5, 3.5];
            let mut count = emitted;
            for &offset in &offsets[emitted..] {
                let born = start + offset;
                if born > time {
                    break;
                }
                count += 1;
                if time < born + art.lifetime {
                    let variant = ((ballistics::jitter(&mut gun.rng) + 1.0)
                        * 0.5
                        * art.variants.len() as f32) as usize;
                    self.smoke.push(Puff {
                        origin: turret.volume.position,
                        born,
                        variant: variant.min(art.variants.len() - 1),
                    });
                }
            }
            gun.smoke_started = (count < offsets.len()).then_some((start, count));
        }
    }

    pub fn smoke<'a>(
        &'a self,
        data: &'a AssaultMap,
        time: f32,
    ) -> impl Iterator<Item = (Vec3, &'a crate::texture::TextureImage, f32)> + 'a {
        self.smoke.iter().filter_map(move |puff| {
            let art = data.turret_smoke.as_ref()?;
            let age = time - puff.born;
            if age >= art.lifetime {
                return None;
            }
            let texture = art.variants.get(puff.variant)?;
            Some((
                puff.origin + Vec3(0.0, 0.0, art.rise * age),
                texture.image_once(age / art.lifetime),
                art.scale,
            ))
        })
    }

    pub fn tick(
        &mut self,
        map: &Map,
        fighters: &[Fighter],
        attack_team: usize,
        time: f32,
        dt: f32,
        damage_scale: f32,
        warheads: &[Warhead],
        plasma: &mut shock::Shots,
        effects: &mut ballistics::Effects,
        marks: &mut decals::Marks,
        sounds: &mut SoundEvents,
    ) -> (Vec<Hit>, Vec<WarheadId>) {
        let Some(data) = &map.assault else {
            return (Vec::new(), Vec::new());
        };
        self.emit_smoke(data, time);
        let mut intercepted = Vec::new();
        for index in self.shutdown_audio.drain(..) {
            let turret = &data.turrets[index];
            sounds.extend_at(
                turret.audio[2].iter().cloned(),
                turret.volume.position,
                CONTACT_SOUND_RADIUS,
                1.0,
            );
        }
        let mut hits = Vec::new();
        self.shots.retain_mut(|shot| {
            let projectile = data.turrets[shot.turret].projectile.as_ref().unwrap();
            if time - shot.born >= projectile.lifetime {
                return false;
            }
            let delta = shot.velocity * dt;
            let wall = map.collision.projectile_surface(
                shot.position,
                delta,
                projectile.radius,
                projectile.height,
            );
            let travel = wall.map_or(delta, |(point, _)| point - shot.position);
            let contact = nearest_hit(
                fighters,
                usize::MAX,
                shot.position,
                travel,
                [projectile.radius, projectile.height],
            );
            let missile = trace_warheads(
                warheads,
                &intercepted,
                shot.position,
                travel,
                [projectile.radius, projectile.height],
                dt,
            )
            .filter(|&(_, at)| contact.is_none_or(|(_, other)| at < other));
            let contact = contact.filter(|_| missile.is_none());
            if contact.is_some() || missile.is_some() || wall.is_some() {
                let direction = unit(shot.velocity);
                let at = missile
                    .map(|(_, at)| at)
                    .or(contact.map(|(_, at)| at))
                    .unwrap_or(1.0);
                let impact = shot.position + travel * at;
                if let Some((id, _)) = missile
                    && (f32::from(projectile.damage) * damage_scale) as u16 > 5
                {
                    intercepted.push(id);
                }
                if projectile.shock
                    && let Some(assets) = shock::assets(map)
                {
                    let offset = wall
                        .filter(|_| contact.is_none() && missile.is_none())
                        .map_or(direction * -8.0, |(_, normal)| normal * 8.0);
                    let blast = plasma.bolt_impact(assets, impact, offset, direction, time, sounds);
                    for (victim, fighter) in fighters
                        .iter()
                        .enumerate()
                        .filter(|(_, fighter)| fighter.team == Some(attack_team))
                    {
                        if let Some((damage, impulse)) =
                            blast.hit_player(&map.collision, fighter.walker.position)
                        {
                            hits.push(Hit {
                                victim,
                                owner: None,
                                bypass_armor: false,
                                damage: (f32::from(damage) * damage_scale) as u16,
                                impulse: impulse * map.collision.profile.mass.max(1.0).recip(),
                            });
                        }
                    }
                } else {
                    if let Some((victim, _)) =
                        contact.filter(|&(victim, _)| fighters[victim].team == Some(attack_team))
                    {
                        hits.push(gun_hit(
                            victim,
                            (f32::from(projectile.damage) * damage_scale) as u16,
                            direction,
                            map,
                        ));
                    }
                    sounds.extend_at(
                        projectile.impact_audio.iter().cloned(),
                        impact,
                        CONTACT_SOUND_RADIUS,
                        1.0,
                    );
                }
                return false;
            }
            shot.position = shot.position + travel;
            true
        });
        for (index, (turret, gun)) in data.turrets.iter().zip(&mut self.guns).enumerate() {
            if !gun.targetable() {
                if time < gun.disabled_until {
                    continue;
                }
                gun.health = turret.health;
            }
            let mount = turret.volume.position;
            let pivot = mount + Vec3(0.0, 0.0, if turret.minigun { 40.0 } else { -12.0 });
            let visible = |id: usize| {
                let fighter = &fighters[id];
                let delta = fighter.walker.position - pivot;
                fighter.team == Some(attack_team)
                    && fighter.health > 0
                    && time >= fighter.protected_until
                    && delta.dot(delta) <= turret.sight * turret.sight
                    && (fighter.invis_left(time) <= 0.0 || time - fighter.fired_at < 0.42)
                    && map.collision.ray(pivot, delta).is_none()
            };
            let missile = gun
                .target
                .and_then(|target| {
                    let Tracking::Warhead(id) = target else {
                        return None;
                    };
                    warheads
                        .iter()
                        .find(|warhead| warhead.id == id && !intercepted.contains(&id))
                })
                .or_else(|| {
                    warheads.iter().find(|warhead| {
                        warhead.warning
                            && !intercepted.contains(&warhead.id)
                            && map.collision.ray(pivot, warhead.position - pivot).is_none()
                    })
                });
            let was_tracking = matches!(gun.target, Some(Tracking::Warhead(_)));
            let target = missile
                .map(|missile| Tracking::Warhead(missile.id))
                .or_else(|| {
                    let pawn = gun.target.and_then(|target| match target {
                        Tracking::Pawn(id) if visible(id) => Some(id),
                        _ => None,
                    });
                    pawn.or_else(|| {
                        let facing = Vec3(gun.yaw.cos(), gun.yaw.sin(), 0.0);
                        (0..fighters.len())
                            .filter(|&id| {
                                let offset = fighters[id].walker.position - pivot;
                                (was_tracking
                                    || facing.dot(offset) >= offset.0.hypot(offset.1) * turret.fov)
                                    && visible(id)
                            })
                            .min_by(|&a, &b| {
                                super::distance_squared(pivot, fighters[a].walker.position)
                                    .total_cmp(&super::distance_squared(
                                        pivot,
                                        fighters[b].walker.position,
                                    ))
                            })
                    })
                    .map(Tracking::Pawn)
                });
            let Some(target) = target else {
                if gun.target.take().is_some() {
                    gun.idle_since = time + 5.0;
                }
                let turn = angle_delta(turret.yaw, gun.yaw);
                if time >= gun.idle_since {
                    gun.yaw += turn.clamp(-dt * turret.tracking, dt * turret.tracking);
                }
                continue;
            };
            let previous = gun.target.replace(target);
            if previous.is_none() || (missile.is_some() && previous != Some(target)) {
                gun.active_since = time;
                gun.ready_at = time + turret.activate.duration() + turret.sample;
                sounds.extend_at(
                    turret.audio[1].iter().cloned(),
                    mount,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            }
            let (center, velocity) = match target {
                Tracking::Pawn(id) => {
                    (fighters[id].walker.position, fighters[id].walker.velocity())
                }
                Tracking::Warhead(_) => {
                    let warhead =
                        missile.expect("warhead target was acquired from the live snapshot");
                    (warhead.position, warhead.velocity)
                }
            };
            let offset = center - pivot;
            let yaw = offset.1.atan2(offset.0);
            let pitch = offset.2.atan2(offset.0.hypot(offset.1));
            let turn = angle_delta(yaw, gun.yaw);
            gun.yaw += turn.clamp(-dt * turret.tracking, dt * turret.tracking);
            if time < gun.ready_at
                || turn.abs()
                    > if missile.is_some() {
                        std::f32::consts::TAU * 2000.0 / 65536.0
                    } else {
                        0.12
                    }
            {
                continue;
            }
            let pitch_units = pitch * 65_536.0 / std::f32::consts::TAU;
            gun.pitch = if turret.minigun {
                if pitch_units < -4000.0 {
                    2
                } else if pitch_units < -1000.0 {
                    1
                } else if pitch_units > 4000.0 {
                    4
                } else if pitch_units > 1000.0 {
                    3
                } else {
                    0
                }
            } else {
                [-1000.0, -4200.0, -7400.0, -10600.0, -13400.0]
                    .iter()
                    .take_while(|&&threshold| pitch_units < threshold)
                    .count()
            };
            if (turret.minigun && pitch_units < -10000.0)
                || (!turret.minigun && !(-20000.0..2000.0).contains(&pitch_units))
            {
                continue;
            }
            let camera = Camera {
                position: pivot,
                yaw: gun.yaw,
                pitch,
            };
            let forward = camera.look_direction();
            let right = camera.right();
            let muzzle = if turret.minigun {
                pivot
                    + Vec3(gun.yaw.cos(), gun.yaw.sin(), 0.0) * 20.0
                    + right * 12.0
                    + Vec3(0.0, 0.0, 16.0)
            } else {
                mount + forward * 100.0 - Vec3(0.0, 0.0, 10.0 + gun.pitch as f32 * 5.0)
            };
            if map.collision.ray(pivot, muzzle - pivot).is_some() {
                continue;
            }
            gun.ready_at = time
                + if turret.minigun {
                    2.0 * turret.sample
                } else {
                    turret.sample + 0.05
                };
            gun.fired_at = time;
            if !turret.minigun
                || map
                    .arsenal
                    .get(&Weapon::Minigun)
                    .and_then(|art| art.fire.as_ref())
                    .is_none()
            {
                sounds.extend_at(
                    turret.audio[0].iter().cloned(),
                    muzzle,
                    CONTACT_SOUND_RADIUS,
                    1.0,
                );
            }
            if turret.minigun {
                let rng = &mut gun.rng;
                let mut jitter = || {
                    *rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                    (*rng >> 8) as f32 / 8_388_608.0 - 1.0
                };
                let offset = center - muzzle;
                let shot = Camera {
                    position: muzzle,
                    yaw: offset.1.atan2(offset.0) + jitter() * 0.098,
                    pitch: offset.2.atan2(offset.0.hypot(offset.1)) + jitter() * 0.025,
                };
                let delta = shot.look_direction() * 10_000.0;
                gun.flash = Some((muzzle, shot.look_direction(), time + 0.055));
                let wall = map.collision.ray_surface(muzzle, delta);
                let clipped = wall.map_or(delta, |(point, _)| point - muzzle);
                let target = nearest_hit(fighters, usize::MAX, muzzle, clipped, [0.0; 2]);
                let warhead =
                    trace_warheads(warheads, &intercepted, muzzle, clipped, [0.0; 2], 0.0)
                        .filter(|&(_, at)| target.is_none_or(|(_, other)| at < other));
                let target = target.filter(|_| warhead.is_none());
                if let Some((id, _)) = warhead {
                    let damage = 5 + ((jitter() + 1.0) * 2.0) as u16;
                    if (f32::from(damage) * damage_scale) as u16 > 5 {
                        intercepted.push(id);
                    }
                }
                if let Some((victim, _)) = target
                    && fighters[victim].team == Some(attack_team)
                {
                    let damage = 5 + ((jitter() + 1.0) * 2.0) as u16;
                    hits.push(gun_hit(
                        victim,
                        (f32::from(damage) * damage_scale) as u16,
                        shot.look_direction(),
                        map,
                    ));
                }
                effects.bullet(
                    map,
                    marks,
                    &shot,
                    Weapon::Minigun,
                    time,
                    ballistics::Bullet {
                        start: muzzle,
                        delta,
                        visible: clipped
                            * warhead
                                .map(|(_, at)| at)
                                .or(target.map(|(_, at)| at))
                                .unwrap_or(1.0),
                        wall: wall.filter(|_| target.is_none() && warhead.is_none()),
                        case_origin: None,
                    },
                    &mut gun.cadence,
                    sounds,
                    jitter,
                );
            } else if let Some(projectile) = &turret.projectile {
                let aim = if turret.lead {
                    let to = center - muzzle;
                    let predicted = to + velocity * (to.dot(to).sqrt() / projectile.speed);
                    unit(if map.collision.ray(muzzle, predicted).is_none() {
                        predicted
                    } else {
                        (predicted + to) * 0.5
                    })
                } else {
                    forward
                };
                self.shots.push(Shot {
                    turret: index,
                    position: muzzle,
                    born: time,
                    velocity: aim * projectile.speed * if missile.is_some() { 2.0 } else { 1.0 },
                });
            }
        }
        (hits, intercepted)
    }
}

fn trace_warheads(
    warheads: &[Warhead],
    intercepted: &[WarheadId],
    start: Vec3,
    delta: Vec3,
    extent: [f32; 2],
    dt: f32,
) -> Option<(WarheadId, f32)> {
    warheads
        .iter()
        .filter(|warhead| !intercepted.contains(&warhead.id))
        .filter_map(|warhead| {
            let motion = warhead.velocity * dt;
            let at = cylinder_hit(
                start - (warhead.position - motion),
                delta - motion,
                warhead.collision[0] + extent[0],
                warhead.collision[1] + extent[1],
            )?;
            (at < 1.0).then_some((warhead.id, at))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

fn gun_hit(victim: usize, damage: u16, direction: Vec3, map: &Map) -> Hit {
    Hit {
        victim,
        owner: None,
        bypass_armor: false,
        damage,
        impulse: direction * (f32::from(damage) * 500.0 / map.collision.profile.mass.max(1.0)),
    }
}
