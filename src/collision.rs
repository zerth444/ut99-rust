use std::io;

use crate::map::Vec3;
use crate::movement::{Profile, Regions};
use crate::package::invalid;

pub(crate) const PLAYER_RADIUS: f32 = 17.0;
const PLAYER: Vec3 = Vec3(PLAYER_RADIUS, PLAYER_RADIUS, 39.0);
const SKIN: f32 = 0.15;
const UP: Vec3 = Vec3(0.0, 0.0, 1.0);
const FLOOR_NORMAL: f32 = 0.7;

#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
}

struct Hull {
    min: Vec3,
    max: Vec3,
    planes: Vec<Plane>,
}

pub struct Collision {
    hulls: Vec<Hull>,
    pub profile: Profile,
    pub regions: Regions,
}

struct Hit {
    fraction: f32,
    normal: Vec3,
}

impl Collision {
    pub fn read(
        planes: &[Plane],
        offsets: impl Iterator<Item = i32>,
        data: &[u32],
    ) -> io::Result<Self> {
        let mut hulls = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for offset in offsets.filter_map(|offset| usize::try_from(offset).ok()) {
            if !seen.insert(offset) {
                continue;
            }
            let entry = data
                .get(offset..)
                .ok_or_else(|| invalid("Invalid collision hull offset"))?;
            let end = entry
                .iter()
                .position(|&value| value == u32::MAX)
                .ok_or_else(|| invalid("Unterminated collision hull"))?;
            let bounds = entry
                .get(end + 1..end + 7)
                .ok_or_else(|| invalid("Missing collision hull bounds"))?;
            let min = Vec3(
                f32::from_bits(bounds[0]),
                f32::from_bits(bounds[1]),
                f32::from_bits(bounds[2]),
            );
            let max = Vec3(
                f32::from_bits(bounds[3]),
                f32::from_bits(bounds[4]),
                f32::from_bits(bounds[5]),
            );
            if [min.0, min.1, min.2, max.0, max.1, max.2]
                .iter()
                .any(|value| !value.is_finite())
            {
                return Err(invalid("Non-finite collision hull bounds"));
            }
            let mut hull_planes = Vec::new();
            for &reference in &entry[..end] {
                let mut plane = *planes
                    .get((reference & !0x4000_0000) as usize)
                    .ok_or_else(|| invalid("Invalid collision hull plane"))?;
                if reference & 0x4000_0000 != 0 {
                    plane.normal = plane.normal * -1.0;
                    plane.distance = -plane.distance;
                }
                hull_planes.push(plane);
            }
            hull_planes.extend(box_planes(min, max));
            hulls.push(Hull {
                min,
                max,
                planes: hull_planes,
            });
        }
        Ok(Self {
            hulls,
            profile: Profile::default(),
            regions: Regions::default(),
        })
    }

    pub fn add_box(&mut self, min: Vec3, max: Vec3) -> usize {
        let id = self.hulls.len();
        self.hulls.push(Hull {
            min,
            max,
            planes: box_planes(min, max).to_vec(),
        });
        id
    }

    pub fn set_box(&mut self, id: usize, min: Vec3, max: Vec3) {
        let hull = &mut self.hulls[id];
        hull.min = min;
        hull.max = max;
        hull.planes.copy_from_slice(&box_planes(min, max));
    }

    fn sweep(&self, start: Vec3, delta: Vec3, extent: Vec3) -> Option<Hit> {
        let end = start + delta;
        let lo = Vec3(start.0.min(end.0), start.1.min(end.1), start.2.min(end.2)) - extent;
        let hi = Vec3(start.0.max(end.0), start.1.max(end.1), start.2.max(end.2)) + extent;
        let mut best: Option<Hit> = None;
        for hull in &self.hulls {
            if hi.0 < hull.min.0
                || hi.1 < hull.min.1
                || hi.2 < hull.min.2
                || lo.0 > hull.max.0
                || lo.1 > hull.max.1
                || lo.2 > hull.max.2
            {
                continue;
            }
            if let Some(hit) = hull.sweep(start, delta, extent)
                && best
                    .as_ref()
                    .is_none_or(|best| hit.fraction < best.fraction)
            {
                best = Some(hit);
            }
        }
        best
    }

    pub fn ray(&self, start: Vec3, delta: Vec3) -> Option<Vec3> {
        self.ray_surface(start, delta).map(|(point, _)| point)
    }

    pub fn ray_surface(&self, start: Vec3, delta: Vec3) -> Option<(Vec3, Vec3)> {
        self.sweep(start, delta, Vec3::default())
            .map(|hit| (start + delta * hit.fraction, hit.normal))
    }

    fn trace(&self, start: Vec3, delta: Vec3) -> Option<Hit> {
        self.sweep(start, delta, PLAYER)
    }

    fn slide(&self, mut position: Vec3, mut delta: Vec3) -> Vec3 {
        let mut normals = Vec::new();
        for _ in 0..4 {
            if delta.dot(delta) < 0.0001 {
                break;
            }
            let Some(hit) = self.trace(position, delta) else {
                return position + delta;
            };
            let approach = -delta.dot(hit.normal);
            let fraction = (hit.fraction - SKIN / approach.max(SKIN)).max(0.0);
            position = position + delta * fraction;
            delta = delta * (1.0 - fraction);
            normals.push(hit.normal);
            for &normal in &normals {
                delta = delta - normal * delta.dot(normal).min(0.0);
            }
            if normals.iter().any(|&normal| delta.dot(normal) < -0.001) {
                break;
            }
        }
        position
    }

    fn floor(&self, position: Vec3, distance: f32) -> Option<f32> {
        self.trace(position, UP * -distance)
            .filter(|hit| hit.normal.2 >= FLOOR_NORMAL)
            .map(|hit| (hit.fraction * distance - SKIN).max(0.0))
    }
}

fn box_planes(min: Vec3, max: Vec3) -> [Plane; 6] {
    [
        Plane {
            normal: Vec3(1.0, 0.0, 0.0),
            distance: max.0,
        },
        Plane {
            normal: Vec3(-1.0, 0.0, 0.0),
            distance: -min.0,
        },
        Plane {
            normal: Vec3(0.0, 1.0, 0.0),
            distance: max.1,
        },
        Plane {
            normal: Vec3(0.0, -1.0, 0.0),
            distance: -min.1,
        },
        Plane {
            normal: UP,
            distance: max.2,
        },
        Plane {
            normal: UP * -1.0,
            distance: -min.2,
        },
    ]
}

impl Hull {
    fn sweep(&self, start: Vec3, delta: Vec3, extent: Vec3) -> Option<Hit> {
        let mut enter = f32::NEG_INFINITY;
        let mut exit: f32 = 1.0;
        let mut normal = Vec3::default();
        for plane in &self.planes {
            let n = plane.normal;
            let extent = Vec3(n.0.abs(), n.1.abs(), n.2.abs()).dot(extent);
            let from = n.dot(start) - plane.distance - extent;
            let along = n.dot(delta);
            if along.abs() < 0.00001 {
                if from > 0.0 {
                    return None;
                }
            } else if along < 0.0 {
                let fraction = -from / along;
                if fraction > enter {
                    enter = fraction;
                    normal = n;
                }
            } else {
                exit = exit.min(-from / along);
            }
            if enter > exit {
                return None;
            }
        }
        // An already-overlapping hull must permit moving out of it.
        ((-0.0001..=1.0).contains(&enter) && exit > 0.0).then_some(Hit {
            fraction: enter.max(0.0),
            normal,
        })
    }
}

#[derive(Default)]
pub struct MotionEvents {
    pub jump: bool,
    pub landing: Option<f32>,
    pub footstep: Option<usize>,
}

pub struct Walker {
    pub position: Vec3,
    pub grounded: bool,
    velocity: Vec3,
    bob_time: f32,
    landing_bob: f32,
    applied_bob: f32,
    next_step: usize,
    dodging: bool,
    dodge_recovery: f32,
}

impl Walker {
    pub fn spawn(collision: &Collision, position: Vec3) -> Self {
        // Some map starts are authored for smaller pawns. Raise the hull before
        // settling it onto the nearest floor to avoid spawning inside a step.
        let raised = position + UP * 10.0;
        let floor = collision.floor(raised, 26.0);
        Self {
            position: floor.map_or(position, |distance| raised - UP * distance),
            grounded: floor.is_some(),
            velocity: Vec3::default(),
            bob_time: 0.0,
            landing_bob: 0.0,
            applied_bob: 0.0,
            next_step: 0,
            dodging: false,
            dodge_recovery: 0.0,
        }
    }

    pub fn touches_box(&self, min: Vec3, max: Vec3, margin: f32) -> bool {
        let padding = PLAYER + Vec3(margin, margin, margin);
        let lo = min - padding;
        let hi = max + padding;
        let Vec3(x, y, z) = self.position;
        x >= lo.0 && x <= hi.0 && y >= lo.1 && y <= hi.1 && z >= lo.2 && z <= hi.2
    }

    pub fn on_box(&self, min: Vec3, max: Vec3) -> bool {
        let foot = self.position.2 - PLAYER.2;
        self.grounded && (foot - max.2).abs() < 1.0 && self.touches_box(min, max, 1.0)
    }

    pub fn carry(&mut self, world: &Collision, delta: Vec3) {
        self.position = world.slide(self.position, delta);
    }

    pub fn dodge(&mut self, profile: Profile, direction: Vec3) -> bool {
        if !self.grounded || self.dodge_recovery > 0.0 {
            return false;
        }
        let horizontal = Vec3(self.velocity.0, self.velocity.1, 0.0);
        let perpendicular = horizontal - direction * horizontal.dot(direction);
        self.velocity = direction * (1.5 * profile.ground_speed) + perpendicular + UP * 160.0;
        self.grounded = false;
        self.dodging = true;
        true
    }

    pub fn apply_self_damage_momentum(&mut self, mut momentum: Vec3, profile: Profile) {
        if momentum.dot(momentum) == 0.0 {
            return;
        }
        // Pawn.TakeDamage sets an upward component before applying the self-hit factor.
        if self.grounded {
            momentum.2 = momentum.2.max(0.4 * momentum.dot(momentum).sqrt());
            self.grounded = false;
        }
        let mut impulse = momentum * (0.6 / profile.mass.max(1.0));
        if self.velocity.2 > 380.0 && impulse.2 > 0.0 {
            impulse.2 *= 0.5;
        }
        self.velocity = self.velocity + impulse;
    }

    pub fn eye(&self, profile: Profile, right: Vec3) -> Vec3 {
        let speed = horizontal_length(self.velocity);
        let stride = if self.grounded && speed >= 10.0 {
            speed
        } else {
            0.0
        };
        let lateral = 0.4 * profile.bob * stride * (8.0 * self.bob_time).sin();
        let vertical = self.applied_bob + 0.3 * profile.bob * stride * (16.0 * self.bob_time).sin();
        let height = if self.grounded {
            profile.eye_height
        } else {
            profile.eye_height * 0.7
        };
        self.position + UP * (height + vertical) + right * lateral
    }

    pub fn advance(
        &mut self,
        world: &Collision,
        desired: Vec3,
        jump_speed: Option<f32>,
        dt: f32,
    ) -> MotionEvents {
        let mut events = MotionEvents::default();
        if dt <= 0.0 {
            return events;
        }
        self.dodge_recovery = (self.dodge_recovery - dt).max(0.0);
        let profile = world.profile;
        let steps = (dt * 120.0).ceil().max(1.0) as usize;
        let step = dt / steps as f32;
        if self.grounded
            && let Some(jump_speed) = jump_speed
        {
            self.velocity.2 = jump_speed;
            self.grounded = false;
            events.jump = true;
        }
        let requested = horizontal_length(desired).min(profile.ground_speed);
        let heading = if requested > 0.0 {
            desired * horizontal_length(desired).recip()
        } else {
            Vec3::default()
        };
        for _ in 0..steps {
            let zone = world.regions.at(self.position);
            let speed = horizontal_length(self.velocity);
            let mut horizontal = Vec3(self.velocity.0, self.velocity.1, 0.0);
            if self.grounded {
                self.velocity.2 = 0.0;
                let friction = zone.friction * step;
                if requested > 0.0 {
                    horizontal = horizontal + (heading * speed - horizontal) * friction.min(1.0);
                    let acceleration =
                        profile.acceleration * requested / profile.ground_speed.max(1.0);
                    horizontal = limit(horizontal + heading * (acceleration * step), requested);
                } else {
                    horizontal = horizontal * (1.0 - 2.0 * friction).max(0.0);
                }
            } else {
                let foot_zone = world.regions.at(self.position - UP * PLAYER.2);
                let fluid = if foot_zone.water && self.velocity.2 < 0.0 {
                    foot_zone.fluid_friction
                } else {
                    0.0
                };
                let retained = (1.0 - fluid * step).max(0.0);
                let control = heading * (0.75 * profile.air_control * profile.acceleration * step);
                horizontal = horizontal * retained
                    + control
                    + Vec3(zone.gravity.0, zone.gravity.1, 0.0) * step;
                horizontal = limit(horizontal, profile.ground_speed.max(speed));
                self.velocity.2 = self.velocity.2 * retained + zone.gravity.2 * step;
                let velocity = limit(horizontal + UP * self.velocity.2, zone.terminal);
                horizontal = Vec3(velocity.0, velocity.1, 0.0);
                self.velocity.2 = velocity.2;
            }
            self.velocity.0 = horizontal.0;
            self.velocity.1 = horizontal.1;

            let flow = zone.flow * (step * 25.0);
            let delta = (horizontal + Vec3(flow.0, flow.1, 0.0)) * step;
            let origin = self.position;
            self.position = world.slide(origin, delta);
            let moved = self.position - origin;
            let blocked = moved.dot(delta) < delta.dot(delta) * 0.95;
            if self.grounded && blocked {
                let up = UP * profile.step_height;
                if world.trace(origin, up).is_none() {
                    let raised = world.slide(origin + up, delta);
                    let stepped = raised - origin - up;
                    if stepped.dot(delta) > moved.dot(delta)
                        && let Some(floor) = world.floor(raised, profile.step_height + 3.0)
                    {
                        self.position = raised - UP * floor;
                    }
                }
            }
            if blocked {
                let actual = (self.position - origin) * step.recip();
                self.velocity.0 = actual.0 - flow.0;
                self.velocity.1 = actual.1 - flow.1;
            }
            if self.grounded
                && let Some(floor) = world.floor(self.position, 5.0)
            {
                self.position = self.position - UP * floor;
                continue;
            }
            if self.grounded {
                self.grounded = false;
                self.velocity.2 = zone.gravity.2 * step;
            }
            let vertical = UP * ((self.velocity.2 + flow.2) * step);
            if let Some(hit) = world.trace(self.position, vertical) {
                let distance = (vertical.2.abs() * hit.fraction - SKIN).max(0.0);
                self.position = self.position + UP * (distance * vertical.2.signum());
                self.grounded = self.velocity.2 < 0.0 && hit.normal.2 >= FLOOR_NORMAL;
                if self.grounded {
                    events.landing = Some(-self.velocity.2);
                    if self.dodging {
                        self.dodging = false;
                        self.dodge_recovery = 0.35;
                    }
                    self.landing_bob = (0.055 * self.velocity.2).max(-50.0);
                }
                self.velocity.2 = 0.0;
            } else {
                self.position = self.position + vertical;
            }
        }

        let speed = if self.grounded {
            horizontal_length(self.velocity)
        } else {
            0.0
        };
        let previous = self.bob_time;
        self.bob_time += dt
            * if speed < 10.0 {
                0.2
            } else {
                0.3 + 0.7 * speed / profile.ground_speed.max(1.0)
            };
        let weight = (16.0 * dt).min(1.0);
        self.applied_bob += (self.landing_bob - self.applied_bob) * weight;
        self.landing_bob *= (1.0 - 8.0 * dt).max(0.0);
        let foot_beat =
            |time| (0.5 * std::f32::consts::PI + 9.0 * time / std::f32::consts::PI) as i32;
        if speed >= 10.0
            && (requested == 0.0 || requested > profile.ground_speed * 0.31)
            && foot_beat(previous) != foot_beat(self.bob_time)
        {
            events.footstep = Some(self.next_step);
            self.next_step = (self.next_step + 1) % 3;
        }
        events
    }
}

fn horizontal_length(velocity: Vec3) -> f32 {
    velocity.0.hypot(velocity.1)
}

fn limit(velocity: Vec3, maximum: f32) -> Vec3 {
    let squared = velocity.dot(velocity);
    if squared > maximum * maximum {
        velocity * (maximum / squared.sqrt())
    } else {
        velocity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_damage_momentum_lifts_grounded_players_and_limits_extra_upward_boost() {
        let mut world = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        world.add_box(Vec3(-100.0, -100.0, -64.0), Vec3(100.0, 100.0, 0.0));
        let mut walker = Walker::spawn(&world, UP * 39.0);
        assert!(walker.grounded);
        walker.apply_self_damage_momentum(Vec3(70_000.0, 0.0, 0.0), world.profile);
        assert!(!walker.grounded);
        assert!((walker.velocity.0 - 420.0).abs() < 0.01);
        assert!((walker.velocity.2 - 168.0).abs() < 0.01);

        walker.velocity.2 = 400.0;
        walker.apply_self_damage_momentum(Vec3(0.0, 0.0, 70_000.0), world.profile);
        assert!((walker.velocity.2 - 610.0).abs() < 0.01);
    }

    #[test]
    fn stock_acceleration_braking_and_dodge_preserve_momentum() {
        let mut world = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        world.add_box(Vec3(-4000.0, -4000.0, -64.0), Vec3(4000.0, 4000.0, 0.0));
        let forward = Vec3(1.0, 0.0, 0.0);
        let desired = forward * world.profile.ground_speed;
        let mut walker = Walker::spawn(&world, UP * 39.0);
        walker.advance(&world, desired, None, 1.0 / 120.0);
        assert!((walker.velocity.0 - 2048.0 / 120.0).abs() < 0.01);
        let mut heard_step = false;
        for _ in 0..55 {
            heard_step |= walker
                .advance(&world, desired, None, 1.0 / 120.0)
                .footstep
                .is_some();
        }
        assert_eq!(walker.velocity.0, 400.0);
        assert!(heard_step);
        walker.advance(&world, Vec3::default(), None, 1.0 / 120.0);
        assert!((walker.velocity.0 - 400.0 * (1.0 - 16.0 / 120.0)).abs() < 0.01);
        assert!(walker.dodge(world.profile, forward));
        assert_eq!(walker.velocity.0, 600.0);
        assert_eq!(walker.velocity.2, 160.0);
        assert!(!walker.dodge(world.profile, forward));
        walker.advance(&world, Vec3(0.0, 400.0, 0.0), None, 1.0 / 120.0);
        assert!(walker.velocity.1 > 0.0 && walker.velocity.1 < 6.0);
        assert!((horizontal_length(walker.velocity) - 600.0).abs() < 0.01);
        while !walker.grounded {
            walker.advance(&world, Vec3::default(), None, 1.0 / 120.0);
            assert!(walker.position.2 >= 39.0);
        }
        assert!(
            !walker.dodge(world.profile, forward),
            "landing starts dodge recovery"
        );
    }

    #[test]
    fn bsp_zone_changes_jump_apex() {
        let mut world = Collision::read(&[], std::iter::empty(), &[]).unwrap();
        world.add_box(Vec3(-4000.0, -4000.0, -64.0), Vec3(4000.0, 4000.0, 0.0));
        world.regions.nodes.push(crate::movement::RegionNode {
            plane: Plane {
                normal: Vec3(1.0, 0.0, 0.0),
                distance: 0.0,
            },
            children: [-1, -1],
            zones: [0, 1],
        });
        world.regions.zones = vec![
            crate::movement::Zone::default(),
            crate::movement::Zone {
                gravity: Vec3(0.0, 0.0, -475.0),
                ..Default::default()
            },
        ];
        let jump = |x| {
            let mut walker = Walker::spawn(&world, Vec3(x, 0.0, 39.0));
            let base = walker.position.2;
            walker.advance(
                &world,
                Vec3::default(),
                Some(world.profile.jump_speed),
                1.0 / 120.0,
            );
            let mut peak = walker.position.2;
            for _ in 0..180 {
                walker.advance(&world, Vec3::default(), None, 1.0 / 120.0);
                peak = peak.max(walker.position.2);
                if walker.grounded {
                    break;
                }
            }
            peak - base
        };
        let standard = jump(-100.0);
        let low_gravity = jump(100.0);
        assert!((53.0..56.0).contains(&standard), "{standard}");
        assert!((108.0..112.0).contains(&low_gravity), "{low_gravity}");
    }

    #[test]
    fn walker_jumps_lands_and_slides_without_crossing_wall() {
        let mut planes = Vec::new();
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for (min, max) in [
            (Vec3(-1000.0, -1000.0, -64.0), Vec3(1000.0, 1000.0, 0.0)),
            (Vec3(100.0, -1000.0, 0.0), Vec3(105.0, 1000.0, 1000.0)),
        ] {
            offsets.push(data.len() as i32);
            data.push(planes.len() as u32);
            planes.push(Plane {
                normal: UP,
                distance: max.2,
            });
            data.push(u32::MAX);
            data.extend([min.0, min.1, min.2, max.0, max.1, max.2].map(f32::to_bits));
        }
        let collision = Collision::read(&planes, offsets.into_iter(), &data).unwrap();
        let mut walker = Walker::spawn(&collision, Vec3(0.0, 0.0, 39.0));
        assert!(walker.grounded);
        let floor = walker.position.2;
        walker.advance(&collision, Vec3::default(), Some(325.0), 1.0 / 60.0);
        assert!(!walker.grounded && walker.position.2 > floor + 4.0);
        for _ in 0..60 {
            walker.advance(&collision, Vec3(280.0, 180.0, 0.0), None, 1.0 / 60.0);
        }
        assert!(walker.grounded && (walker.position.2 - floor).abs() < 0.01);
        assert!(
            (82.0..83.0).contains(&walker.position.0),
            "{:?}",
            walker.position
        );
        assert!(walker.position.1 > 120.0, "{:?}", walker.position);
    }
}
