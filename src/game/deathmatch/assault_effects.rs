use crate::{
    assault::{Effects, Timer},
    game::{
        ballistics, decals,
        ordnance::{Kind, Visual},
        shock::Blast,
    },
    map::{Map, Vec3},
    mover::{rotate, unit},
    sound::{BLAST_SOUND_RADIUS, SoundEvents},
};

use super::{Fighter, Hit, nearest_hit};

struct Scheduled {
    next: f32,
    active: bool,
    attacker: bool,
}

struct GroundShake {
    remaining: f32,
    next: f32,
    initial: bool,
}

#[derive(Default)]
struct ViewShake {
    remaining: f32,
    magnitude: f32,
    roll: f32,
    increasing: bool,
    height: f32,
    height_timer: f32,
}

impl ViewShake {
    fn start(&mut self, duration: f32, magnitude: f32) {
        if magnitude > self.magnitude || duration >= self.remaining {
            self.remaining = duration;
            self.magnitude = magnitude;
            self.height_timer = 0.0;
            self.height = -1.1 * 0.015 * magnitude;
        }
    }

    fn step(&mut self, dt: f32, rng: &mut u32) {
        let roll_step = 10.0 * dt.min(0.1);
        if self.remaining <= 0.0 {
            self.height = 0.0;
            self.roll -=
                self.roll.signum() * (self.roll.abs().max(500.0) * roll_step).min(self.roll.abs());
            return;
        }
        self.remaining = (self.remaining - dt).max(0.0);
        if self.height_timer == 0.0 {
            self.height_timer = 0.1;
            self.height = -1.1 * 0.015 * self.magnitude;
        } else {
            self.height_timer -= dt;
            if self.height_timer < 0.0 {
                self.height_timer = 0.2 * roll(rng);
                self.height = ballistics::jitter(rng) * 0.015 * self.magnitude;
            }
        }

        let direction = if self.increasing { 1.0 } else { -1.0 };
        self.roll += direction * (roll_step * self.magnitude).trunc();
        let outward = self.roll * direction;
        let turn = (0.5 + roll(rng)) * self.magnitude;
        if outward > 1.3 * self.magnitude {
            self.roll = direction * 1.3 * self.magnitude;
            self.increasing = direction < 0.0;
        } else {
            self.increasing = if direction > 0.0 {
                outward < turn
            } else {
                outward > 0.0 && outward > turn
            };
            if roll(rng) < 3.0 * dt {
                self.increasing = !self.increasing;
            }
        }
    }
}

struct Shell {
    source: usize,
    position: Vec3,
    velocity: Vec3,
    born: f32,
    smoke_at: f32,
}

struct Explosion {
    center: Vec3,
    born: f32,
    damage: f32,
    radius: f32,
    momentum: f32,
    display: Vec3,
    scale: f32,
    mortar: bool,
}

pub(super) struct Environment {
    launches: Vec<f32>,
    charges: Vec<f32>,
    timers: Vec<Scheduled>,
    quakes: Vec<Option<GroundShake>>,
    view_shake: ViewShake,
    view_rng: u32,
    shells: Vec<Shell>,
    explosions: Vec<Explosion>,
    rng: u32,
}

impl Environment {
    pub fn new(data: &Effects, side: usize) -> Self {
        let mut rng = 0x6c24_11d3 ^ side as u32;
        Self {
            launches: data.mortars.iter().map(|gun| gun.interval).collect(),
            charges: vec![f32::INFINITY; data.charges.len()],
            timers: data
                .timers
                .iter()
                .map(|timer| {
                    let (next, active) = match timer {
                        Timer::Fixed { interval, .. } => (*interval, true),
                        Timer::Random {
                            min, max, always, ..
                        } => (delay(*min, *max, &mut rng), *always),
                    };
                    Scheduled {
                        next,
                        active,
                        attacker: false,
                    }
                })
                .collect(),
            quakes: data.quakes.iter().map(|_| None).collect(),
            view_shake: ViewShake::default(),
            view_rng: 0x78a3_194e ^ side as u32,
            shells: Vec::new(),
            explosions: Vec::new(),
            rng,
        }
    }

    pub fn timers(&mut self, data: &Effects, time: f32) -> Vec<(f32, usize, bool)> {
        let mut due = Vec::new();
        for (timer, state) in data.timers.iter().zip(&mut self.timers) {
            if state.next > time {
                continue;
            }
            let at = state.next;
            match timer {
                Timer::Fixed {
                    event,
                    interval,
                    repeat,
                } => {
                    due.push((at, *event, false));
                    state.next = if *repeat {
                        at + interval
                    } else {
                        f32::INFINITY
                    };
                }
                Timer::Random {
                    events,
                    min,
                    max,
                    probability,
                    ..
                } => {
                    if state.active && roll(&mut self.rng) <= *probability {
                        let index = (roll(&mut self.rng) * events.len() as f32) as usize;
                        due.push((at, events[index.min(events.len() - 1)], state.attacker));
                    }
                    state.next = at + delay(*min, *max, &mut self.rng);
                }
            }
        }
        due
    }

    pub fn signal(&mut self, data: &Effects, event: usize, time: f32, attacker: bool) {
        self.set_random(data, event, true, attacker);
        for (quake, active) in data.quakes.iter().zip(&mut self.quakes) {
            if quake.tag == event {
                *active = Some(GroundShake {
                    remaining: quake.duration.unwrap_or_default(),
                    next: time,
                    initial: true,
                });
            }
        }
        for (index, charge) in data.charges.iter().enumerate() {
            if charge.tag == Some(event) {
                self.arm(data, index, time);
            }
        }
    }

    pub fn release(&mut self, data: &Effects, event: usize) {
        self.set_random(data, event, false, false);
    }

    fn set_random(&mut self, data: &Effects, event: usize, active: bool, attacker: bool) {
        for (timer, state) in data.timers.iter().zip(&mut self.timers) {
            if let Timer::Random {
                tag: Some(tag),
                always: false,
                ..
            } = timer
                && *tag == event
            {
                state.active = active;
                state.attacker = attacker;
            }
        }
    }

    pub fn targetable(&self, data: &Effects, index: usize) -> bool {
        self.charges[index] == f32::INFINITY && !data.charges[index].trigger_only
    }

    pub fn arm(&mut self, data: &Effects, index: usize, time: f32) {
        if self.charges[index] == f32::INFINITY {
            let interval = data.charges[index].delay;
            self.charges[index] = time + delay(interval, 3.0 * interval, &mut self.rng);
        }
    }

    pub fn tick_quakes(&mut self, data: &Effects, player: &Fighter, time: f32, dt: f32) -> Vec3 {
        let mut impulse = Vec3::default();
        for (quake, active) in data.quakes.iter().zip(&mut self.quakes) {
            let Some(state) = active else {
                continue;
            };
            if time < state.next {
                continue;
            }
            let magnitude = quake.magnitude.unwrap_or_default();
            let toss = quake.toss.unwrap_or(false);
            let delta = player.walker.position - quake.position;
            if player.health > 0 && delta.dot(delta) < quake.radius.unwrap_or_default().powi(2) {
                self.view_shake.start(state.remaining, magnitude);
                if toss && player.walker.grounded {
                    let strength = magnitude * if state.initial { 0.18 } else { 0.15 };
                    let angle = roll(&mut self.rng) * std::f32::consts::TAU;
                    let pitch = ballistics::jitter(&mut self.rng);
                    let horizontal = strength * (1.0 - pitch * pitch).max(0.0).sqrt();
                    impulse = impulse
                        + Vec3(
                            horizontal * angle.cos(),
                            horizontal * angle.sin(),
                            (strength * pitch).abs().max(120.0),
                        );
                }
            }
            if toss && state.remaining > 0.5 {
                state.next += 0.5;
                state.remaining -= 0.5;
                state.initial = false;
            } else {
                *active = None;
            }
        }
        self.view_shake.step(dt, &mut self.view_rng);
        impulse
    }

    pub fn shake(&self) -> (f32, f32) {
        (
            self.view_shake.roll * std::f32::consts::TAU / 65_536.0,
            self.view_shake.height,
        )
    }

    pub fn tick(
        &mut self,
        map: &Map,
        fighters: &[Fighter],
        time: f32,
        dt: f32,
        ballistic: &mut ballistics::Effects,
        marks: &mut decals::Marks,
        sounds: &mut SoundEvents,
        debris: &mut super::breakables::Debris,
    ) -> (Vec<Hit>, Vec<usize>) {
        let Some(assault) = &map.assault else {
            return (Vec::new(), Vec::new());
        };
        let data = &assault.effects;
        let art = &map.ordnance[Kind::FlakShell as usize];
        self.explosions.retain(|effect| time - effect.born < 0.65);
        for (index, (gun, next)) in data.mortars.iter().zip(&mut self.launches).enumerate() {
            if *next > time {
                continue;
            }
            *next += gun.interval;
            let angle = |seed: &mut u32| ballistics::jitter(seed) * gun.deviation;
            let rotation = gun.rotation + Vec3(angle(&mut self.rng), angle(&mut self.rng), 0.0);
            let direction = rotate(Vec3(1.0, 0.0, 0.0), rotation);
            let position = gun.location + rotate(Vec3(20.0, 0.0, 0.0), gun.rotation);
            self.shells.push(Shell {
                source: index,
                position,
                // The inherited shell initializes its velocity before the launcher assigns ShellSpeed.
                velocity: direction * art.speed.unwrap_or(gun.speed) + Vec3(0.0, 0.0, 200.0),
                born: time,
                smoke_at: time + 0.05,
            });
            sounds.extend_at(
                gun.sound.iter().cloned(),
                gun.location,
                BLAST_SOUND_RADIUS,
                1.0,
            );
        }
        let mut blasts = Vec::new();
        self.shells.retain_mut(|shell| {
            if time - shell.born >= art.lifetime.unwrap_or(6.0) {
                return false;
            }
            if time <= shell.born {
                return true;
            }
            let gravity = map.collision.regions.at(shell.position).gravity;
            let delta = (shell.velocity + gravity * (0.5 * dt)) * dt;
            shell.velocity = shell.velocity + gravity * dt;
            let wall = map.collision.projectile_contact(
                shell.position,
                delta,
                art.collision_radius,
                art.collision_height,
            );
            let path = wall.map_or(delta, |(point, _, _)| point - shell.position);
            let pawn = nearest_hit(
                fighters,
                usize::MAX,
                shell.position,
                path,
                [art.collision_radius, art.collision_height],
            );
            let impact = if let Some((victim, at)) = pawn {
                let point = shell.position + path * at;
                Some((point, unit(point - fighters[victim].walker.position)))
            } else if let Some((point, normal, hull)) = wall {
                if let Some((texture, scale, life)) = &data.mortar_mark {
                    let surface = map
                        .collision
                        .ray(point + normal, normal * -32.0)
                        .unwrap_or(point);
                    if let Some(mover) = map
                        .movers
                        .iter()
                        .position(|mover| mover.collision_hull() == Some(hull))
                    {
                        marks.project_mover(
                            &map.movers,
                            mover,
                            surface,
                            normal,
                            texture,
                            *scale,
                            time + *life,
                        );
                    } else {
                        marks.project(&map.faces, surface, normal, texture, *scale, time + *life);
                    }
                }
                Some((point, normal))
            } else {
                None
            };
            if let Some((point, normal)) = impact {
                let gun = &data.mortars[shell.source];
                blasts.push(Explosion {
                    center: point + normal,
                    born: time,
                    damage: gun.damage,
                    radius: gun.radius,
                    momentum: gun.momentum,
                    display: point + normal * 10.0,
                    scale: 2.5,
                    mortar: true,
                });
                sounds.extend_at(art.sound.iter().cloned(), point, BLAST_SOUND_RADIUS, 1.0);
                return false;
            }
            shell.position = shell.position + path;
            if time >= shell.smoke_at {
                ballistic.smoke.push((shell.position, time, 0));
                shell.smoke_at += 0.05;
            }
            true
        });
        for (index, charge) in data.charges.iter().enumerate() {
            let due = self.charges[index];
            if due == f32::NEG_INFINITY || due > time {
                continue;
            }
            self.charges[index] = f32::NEG_INFINITY;
            let center = charge.volume.position;
            blasts.push(Explosion {
                center,
                born: time,
                damage: charge.damage,
                radius: charge.damage + 100.0,
                momentum: charge.momentum,
                display: center + Vec3(0.0, 0.0, 16.0),
                scale: (charge.damage / 100.0 + 0.4 + 0.5 * roll(&mut self.rng)) * charge.scale,
                mortar: false,
            });
            sounds.extend_at(
                data.charge_sound.iter().cloned(),
                center,
                BLAST_SOUND_RADIUS,
                1.0,
            );
        }
        let mut hits = Vec::new();
        let mut wall_events = Vec::new();
        for explosion in &blasts {
            let blast = Blast::radial(
                explosion.center,
                explosion.radius,
                explosion.damage,
                explosion.momentum,
            );
            let damage_type = if explosion.mortar {
                "mortared"
            } else {
                "detonated"
            };
            wall_events.extend(debris.blast(map, data, &blast, damage_type, 1, time));
            for (victim, fighter) in fighters
                .iter()
                .enumerate()
                .filter(|(_, fighter)| fighter.health > 0)
            {
                if let Some((damage, impulse)) =
                    blast.hit_player(&map.collision, fighter.walker.position)
                {
                    hits.push(Hit {
                        victim,
                        owner: None,
                        bypass_armor: false,
                        damage,
                        impulse: impulse * map.collision.profile.mass.max(1.0).recip(),
                    });
                }
            }
            for (index, charge) in data.charges.iter().enumerate() {
                if self.targetable(data, index)
                    && blast
                        .hit_actor(&map.collision, charge.volume.position, charge.volume.radius)
                        .is_some()
                {
                    self.arm(data, index, time);
                }
            }
        }
        self.explosions.extend(blasts);
        (hits, wall_events)
    }

    pub fn visuals<'a>(&'a self, map: &'a Map, time: f32) -> impl Iterator<Item = Visual<'a>> + 'a {
        let art = &map.ordnance[Kind::FlakShell as usize];
        self.shells
            .iter()
            .filter_map(move |shell| {
                art.flight_visual(
                    Kind::FlakShell,
                    shell.position,
                    unit(shell.velocity),
                    time - shell.born,
                    1.0,
                )
            })
            .chain(self.explosions.iter().filter_map(move |effect| {
                let progress = ((time - effect.born) / 0.65).clamp(0.0, 0.999);
                if effect.mortar {
                    art.impact_sprite(effect.display, progress, effect.scale)
                } else {
                    let texture = map.assault.as_ref()?.effects.charge_image.as_ref()?;
                    Some(Visual::Sprite(
                        effect.display,
                        texture.image_once(progress),
                        effect.scale,
                        1.0,
                    ))
                }
            }))
    }
}

fn roll(seed: &mut u32) -> f32 {
    (ballistics::jitter(seed) + 1.0) * 0.5
}

fn delay(min: f32, max: f32, seed: &mut u32) -> f32 {
    min + (max - min) * roll(seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::deathmatch::{Command, Match, Mode, Rules},
        package::Package,
    };
    use std::path::Path;

    #[test]
    fn view_shake_keeps_the_stronger_active_pulse_and_levels_afterward() {
        let mut shake = ViewShake::default();
        let mut rng = 0x1ce9_fa3d;
        shake.start(1.5, 1000.0);
        shake.step(0.025, &mut rng);
        assert_eq!(shake.roll, -250.0);
        assert!((shake.height + 16.5).abs() < 0.001);
        shake.start(0.5, 500.0);
        assert_eq!(shake.magnitude, 1000.0);
        for _ in 0..60 {
            shake.step(0.025, &mut rng);
            assert!(shake.roll.abs() <= 1300.0);
        }
        for _ in 0..20 {
            shake.step(0.025, &mut rng);
        }
        assert_eq!((shake.roll, shake.height), (0.0, 0.0));
    }

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn overlord_launches_its_mortar_on_the_map_timer_and_reaches_the_bsp() {
        let path = Path::new("local/reference-client/Maps/AS-Overlord.unr");
        let bytes = std::fs::read(path).unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        let data = &map.assault.as_ref().unwrap().effects;
        assert_eq!(data.mortars.len(), 1);
        let mortar = &data.mortars[0];
        assert_eq!(
            (mortar.interval, mortar.damage, mortar.radius),
            (4.0, 500.0, 400.0)
        );
        assert!(mortar.sound.is_some() && data.mortar_mark.is_some());
        assert!(!data.timers.is_empty() && !data.quakes.is_empty());
        let mut game = Match::new(
            &map,
            0,
            Rules {
                mode: Mode::Assault,
                bots: 0,
                ..Rules::default()
            },
        );
        let mut first_launch = None;
        let mut flight = false;
        let mut impact = false;
        for _ in 0..750 {
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            let environment = &game.assault.as_ref().unwrap().environment;
            if let Some(shell) = environment.shells.first() {
                first_launch.get_or_insert(shell.born);
                flight |= game
                    .assault_effects(&map)
                    .any(|visual| matches!(visual, Visual::Mesh(..)));
            }
            impact |= environment.explosions.iter().any(|effect| effect.mortar);
            if impact {
                break;
            }
        }
        assert!((first_launch.unwrap() - 4.0).abs() < 0.03);
        assert!(
            flight && impact,
            "expected the shell to travel and hit the map"
        );
    }

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn guardia_dispatcher_arms_placed_explosive_charges() {
        let path = Path::new("local/reference-client/Maps/AS-Guardia.unr");
        let bytes = std::fs::read(path).unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        let data = &map.assault.as_ref().unwrap().effects;
        assert!(data.charge_sound.is_some() && data.charge_image.is_some());
        let tag = |name: &str| {
            package
                .names
                .iter()
                .position(|entry| entry.eq_ignore_ascii_case(name))
                .unwrap()
        };
        let charges = tag("Wallholio");
        let tremor = data
            .quakes
            .first()
            .expect("Guardia has a placed Earthquake");
        let tremor_center = tremor.position;
        let tremor_tag = tremor.tag;
        assert!(
            tremor.duration.unwrap_or_default() > 0.0 && tremor.radius.unwrap_or_default() > 0.0
        );
        assert!(
            data.charges
                .iter()
                .filter(|charge| charge.tag == Some(charges))
                .count()
                > 1
        );
        let mut game = Match::new(
            &map,
            0,
            Rules {
                mode: Mode::Assault,
                bots: 0,
                ..Rules::default()
            },
        );
        game.assault
            .as_mut()
            .unwrap()
            .emit(0.0, tag("byebyebirdie"), true);
        game.assault.as_mut().unwrap().emit(3.9, tremor_tag, true);
        let mut spent = 0;
        let mut shook = false;
        for _ in 0..630 {
            if (3.85..4.0).contains(&game.time) {
                game.fighters[0].walker.position = tremor_center;
            }
            game.step(&mut map, Command::default(), 1.0 / 60.0);
            let environment = &game.assault.as_ref().unwrap().environment;
            spent = environment
                .charges
                .iter()
                .filter(|&&stage| stage == f32::NEG_INFINITY)
                .count();
            let (roll, height) = game.view_shake(tremor_center);
            shook |= roll != 0.0 || height != 0.0;
            if spent > 1 && shook {
                break;
            }
        }
        assert!(spent > 1, "expected the scheduled explosion chain to fire");
        assert!(shook, "expected the quake event to shake the camera");
        assert!(
            game.assault_debris(&map)
                .any(|(_, _, _, faces)| faces.iter().any(|face| face.texture.is_some())),
            "the Wallholio event should also release textured wall debris"
        );
        assert!(
            game.assault_effects(&map)
                .any(|visual| matches!(visual, Visual::Sprite(..)))
        );
    }
}
