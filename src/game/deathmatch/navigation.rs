use crate::{
    collision::{Collision, Walker},
    map::{Map, Vec3},
    mover::LiftStop,
};

use super::distance_squared;
use std::{cmp::Ordering, collections::BinaryHeap};

const EDGE_LIMIT: f32 = 620.0;
pub(super) const WAYPOINT_RADIUS: f32 = 22.0;
const JUMP_PROBE_SECONDS: f32 = 1.4;
const LAND_RADIUS: f32 = 48.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct LiftRide {
    pub mover: usize,
    pub from: LiftStop,
    pub entry: Vec3,
    pub exit: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    Walk(Vec3),
    Swim(Vec3),
    Teleport(Vec3),
    Lift(LiftRide),
    Jump(Jump),
}

impl Step {
    pub(super) fn movement_goal(self) -> Option<Vec3> {
        match self {
            Self::Walk(point) | Self::Swim(point) => Some(point),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Jump {
    pub start: Vec3,
    pub end: Vec3,
    direction: Vec3,
    runup: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum JumpPhase {
    #[default]
    Approach,
    Runup,
    Air,
}

#[derive(Default)]
pub(super) struct JumpProgress(JumpPhase);

pub(super) enum JumpControl {
    Drive { movement: Vec3, jump: bool },
    Landed,
    Failed,
}

impl Jump {
    pub(super) fn probe(world: &Collision, from: Vec3, to: Vec3) -> Option<Self> {
        let offset = to - from;
        if !(44.0..=330.0).contains(&offset.0.hypot(offset.1))
            || !(-95.0..=64.0).contains(&offset.2)
        {
            return None;
        }
        let arrival = Walker::spawn(world, to);
        let departure = Walker::spawn(world, from);
        if !departure.grounded
            || !arrival.grounded
            || !safe(world, departure.position)
            || !safe(world, arrival.position)
        {
            return None;
        }
        let start = departure.position;
        let end = arrival.position;
        let direction = unit_horizontal(end - start);
        for runup in [58.0, 33.0, 12.0, 0.0] {
            let jump = Self {
                start,
                end,
                direction,
                runup,
            };
            if jump.can_land(world, from, 60) && jump.can_land(world, from, 240) {
                return Some(jump);
            }
        }
        None
    }

    fn can_land(self, world: &Collision, from: Vec3, frequency: u32) -> bool {
        let mut walker = Walker::spawn(world, from);
        let mut progress = JumpProgress::default();
        let dt = 1.0 / frequency as f32;
        for _ in 0..(JUMP_PROBE_SECONDS * frequency as f32).ceil() as u32 {
            match progress.command(self, &walker) {
                JumpControl::Drive { movement, jump } => {
                    walker.advance(
                        world,
                        movement * world.profile.ground_speed,
                        jump.then_some(world.profile.jump_speed),
                        dt,
                    );
                    if !safe(world, walker.position) {
                        return false;
                    }
                }
                JumpControl::Landed => return true,
                JumpControl::Failed => return false,
            }
        }
        false
    }
}

impl JumpProgress {
    pub(super) fn command(&mut self, jump: Jump, walker: &Walker) -> JumpControl {
        let position = walker.position;
        let velocity = walker.velocity();
        if self.0 == JumpPhase::Approach {
            let offset = jump.start - position;
            let distance = offset.0.hypot(offset.1);
            if !walker.grounded
                || distance > 11.0
                || offset.2.abs() > 23.0
                || velocity.0.hypot(velocity.1) > 38.0
            {
                let movement = if distance <= 8.0 {
                    Vec3::default()
                } else {
                    unit_horizontal(offset) * (distance / 92.0).min(0.65)
                };
                return JumpControl::Drive {
                    movement,
                    jump: false,
                };
            }
            self.0 = JumpPhase::Runup;
        }
        match self.0 {
            JumpPhase::Runup => {
                if !walker.grounded {
                    return JumpControl::Failed;
                }
                if (position - jump.start).dot(jump.direction) < jump.runup - 3.0 {
                    return JumpControl::Drive {
                        movement: jump.direction,
                        jump: false,
                    };
                }
                self.0 = JumpPhase::Air;
                JumpControl::Drive {
                    movement: jump.direction,
                    jump: true,
                }
            }
            JumpPhase::Air if walker.grounded => {
                let offset = jump.end - position;
                if offset.0.hypot(offset.1) <= LAND_RADIUS && offset.2.abs() <= 24.0 {
                    JumpControl::Landed
                } else {
                    JumpControl::Failed
                }
            }
            JumpPhase::Air => {
                if position.2 < jump.end.2 - 32.0 && velocity.2 <= 0.0 {
                    return JumpControl::Failed;
                }
                let correction = (jump.end - position) * 7.0 - velocity * 0.36;
                JumpControl::Drive {
                    movement: unit_horizontal(correction),
                    jump: false,
                }
            }
            JumpPhase::Approach => {
                unreachable!("approach either returns a ground command or starts the runup")
            }
        }
    }
}

fn unit_horizontal(vector: Vec3) -> Vec3 {
    crate::mover::unit(Vec3(vector.0, vector.1, 0.0))
}

fn safe(world: &Collision, position: Vec3) -> bool {
    let zone = world.regions.at(position);
    !zone.water && !zone.pain
}

#[derive(Clone, Copy)]
struct Edge {
    to: usize,
    cost: f32,
    step: Step,
}

#[derive(Clone, Copy)]
struct RouteCandidate {
    id: usize,
    cost: f32,
}

impl PartialEq for RouteCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for RouteCandidate {}

impl PartialOrd for RouteCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RouteCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.id.cmp(&self.id))
    }
}

pub(super) struct Navigation {
    points: Vec<Vec3>,
    edges: Vec<Vec<Edge>>,
}

impl Navigation {
    #[cfg(test)]
    pub(super) fn jumps(&self) -> impl Iterator<Item = Jump> + '_ {
        self.edges
            .iter()
            .flatten()
            .filter_map(|edge| match edge.step {
                Step::Jump(jump) => Some(jump),
                _ => None,
            })
    }

    pub(super) fn new(map: &Map) -> Self {
        let mut points = Vec::new();
        for point in map
            .navigation_points
            .iter()
            .copied()
            .chain(map.starts.iter().map(|start| start.position))
        {
            if points
                .iter()
                .all(|&existing| distance_squared(point, existing) > 35.0_f32.powi(2))
            {
                points.push(point);
            }
        }
        let mut transfers = Vec::new();
        for (entry, exit) in crate::teleport::links(&map.teleporters) {
            let from = node_at(&mut points, entry);
            let to = node_at(&mut points, exit);
            transfers.push((
                from,
                Edge {
                    to,
                    cost: 80.0,
                    step: Step::Teleport(entry),
                },
            ));
        }
        for (index, mover) in map.movers.iter().enumerate() {
            let (Some(tag), Some(seconds)) = (mover.tag, mover.lift_ride_seconds()) else {
                continue;
            };
            let find_exit = |stop| {
                let board = mover.lift_board_position(stop);
                map.lift_exits
                    .iter()
                    .filter(|exit| exit.tag == tag)
                    .map(|exit| (exit.position, distance_squared(exit.position, board)))
                    .filter(|(exit, distance)| {
                        (exit.2 - board.2).abs() < 100.0 && *distance < 390.0_f32.powi(2)
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(exit, _)| exit)
            };
            let (Some(home), Some(away)) = (find_exit(LiftStop::Home), find_exit(LiftStop::Away))
            else {
                continue;
            };
            let home_node = node_at(&mut points, home);
            let away_node = node_at(&mut points, away);
            for (from, entry, exit, source, to) in [
                (LiftStop::Home, home, away, home_node, away_node),
                (LiftStop::Away, away, home, away_node, home_node),
            ] {
                transfers.push((
                    source,
                    Edge {
                        to,
                        cost: 290.0 + seconds * 245.0,
                        step: Step::Lift(LiftRide {
                            mover: index,
                            from,
                            entry,
                            exit,
                        }),
                    },
                ));
            }
        }
        if !map.navigation_points.is_empty() {
            for link in &map.reach_links {
                let (from, to) = (
                    node_at(&mut points, link.from),
                    node_at(&mut points, link.to),
                );
                if from == to {
                    continue;
                }
                let step = if link.swim {
                    Step::Swim(link.to)
                } else if link.jump && !walk_link(&map.collision, link.from, link.to) {
                    let Some(jump) = Jump::probe(&map.collision, link.from, link.to) else {
                        continue;
                    };
                    Step::Jump(jump)
                } else {
                    Step::Walk(link.to)
                };
                transfers.push((
                    from,
                    Edge {
                        to,
                        cost: link.distance,
                        step,
                    },
                ));
            }
        }
        let mut edges = vec![Vec::new(); points.len()];
        for (index, &from) in points.iter().enumerate() {
            for (next, &to) in points.iter().enumerate().skip(index + 1) {
                let distance = distance_squared(from, to);
                if distance > EDGE_LIMIT.powi(2) {
                    continue;
                }
                if walk_link(&map.collision, from, to) {
                    let cost = distance.sqrt() * 1.2;
                    edges[index].push(Edge {
                        to: next,
                        cost,
                        step: Step::Walk(to),
                    });
                    edges[next].push(Edge {
                        to: index,
                        cost,
                        step: Step::Walk(from),
                    });
                } else {
                    for (source, target) in [(index, next), (next, index)] {
                        if let Some(jump) =
                            Jump::probe(&map.collision, points[source], points[target])
                        {
                            edges[source].push(Edge {
                                to: target,
                                cost: distance.sqrt() + 190.0,
                                step: Step::Jump(jump),
                            });
                        }
                    }
                }
            }
        }
        for (source, edge) in transfers {
            edges[source].push(edge);
        }
        Self { points, edges }
    }

    pub(super) fn next_step(&self, world: &Collision, from: Vec3, to: Vec3) -> Option<Step> {
        self.next_step_avoiding(world, from, to, &[])
    }

    pub(super) fn next_step_avoiding(
        &self,
        world: &Collision,
        from: Vec3,
        to: Vec3,
        blocked: &[(Vec3, f32)],
    ) -> Option<Step> {
        let allowed = |point| {
            blocked
                .iter()
                .all(|&(failed, _)| distance_squared(point, failed) > 40.0_f32.powi(2))
        };
        // A route between lift landings needs the platform even when the
        // currently docked platform makes a straight approach look walkable.
        let lift = self
            .points
            .iter()
            .zip(&self.edges)
            .filter(|&(&point, _)| distance_squared(from, point) < 48.0_f32.powi(2))
            .flat_map(|(_, edges)| edges)
            .find_map(|edge| match edge.step {
                Step::Lift(ride)
                    if distance_squared(to, ride.exit) < WAYPOINT_RADIUS.powi(2)
                        && allowed(ride.exit) =>
                {
                    Some(edge.step)
                }
                _ => None,
            });
        if lift.is_some() {
            return lift;
        }
        if allowed(to) && walk_link(world, from, to) {
            return Some(Step::Walk(to));
        }
        let mut costs = vec![f32::INFINITY; self.points.len()];
        let mut queue = BinaryHeap::new();
        let mut first = vec![None; self.points.len()];
        for (id, &point) in self.points.iter().enumerate() {
            let distance = distance_squared(from, point);
            if distance <= EDGE_LIMIT.powi(2) && walk_link(world, from, point) {
                if (from.2 - point.2).abs() > 48.0
                    || (from.0 - point.0).hypot(from.1 - point.1) >= WAYPOINT_RADIUS
                {
                    if !allowed(point) {
                        continue;
                    }
                    first[id] = Some(Step::Walk(point));
                }
                costs[id] = distance.sqrt();
                queue.push(RouteCandidate {
                    id,
                    cost: costs[id],
                });
            }
        }
        let mut best = f32::INFINITY;
        let mut waypoint = None;
        while let Some(RouteCandidate { id, cost }) = queue.pop() {
            if cost >= best {
                break;
            }
            if cost > costs[id] {
                continue;
            }
            let point = self.points[id];
            let closing = distance_squared(point, to);
            if closing <= EDGE_LIMIT.powi(2)
                && costs[id] + closing.sqrt() < best
                && (first[id].is_some() || allowed(to))
                && walk_link(world, point, to)
            {
                best = costs[id] + closing.sqrt();
                waypoint = first[id].or(Some(Step::Walk(to)));
            }
            for edge in &self.edges[id] {
                if !allowed(self.points[edge.to]) {
                    continue;
                }
                let length = costs[id] + edge.cost;
                if length < costs[edge.to] {
                    costs[edge.to] = length;
                    first[edge.to] = first[id].or(Some(edge.step));
                    queue.push(RouteCandidate {
                        id: edge.to,
                        cost: length,
                    });
                }
            }
        }
        waypoint
    }

    pub(super) fn recovery(
        &self,
        world: &Collision,
        from: Vec3,
        objective: Option<Vec3>,
        blocked: &[(Vec3, f32)],
    ) -> Option<Step> {
        self.points
            .iter()
            .copied()
            .filter(|&point| {
                let distance = distance_squared(from, point);
                (65.0_f32.powi(2)..=300.0_f32.powi(2)).contains(&distance)
                    && blocked
                        .iter()
                        .all(|&(failed, _)| distance_squared(point, failed) > 48.0_f32.powi(2))
                    && walk_link(world, from, point)
            })
            .min_by(|&a, &b| {
                let score = |point| {
                    objective.map_or(distance_squared(from, point), |goal| {
                        distance_squared(goal, point)
                    })
                };
                score(a).total_cmp(&score(b))
            })
            .map(Step::Walk)
    }
}

fn node_at(points: &mut Vec<Vec3>, point: Vec3) -> usize {
    if let Some(index) = points
        .iter()
        .position(|&other| distance_squared(point, other) < 1.0)
    {
        return index;
    }
    points.push(point);
    points.len() - 1
}

pub(super) fn walk_link(world: &Collision, from: Vec3, to: Vec3) -> bool {
    let delta = to - from;
    let horizontal = delta.0.hypot(delta.1);
    if delta.2.abs() > 28.0 + horizontal * 0.43 {
        return false;
    }
    let side = crate::mover::unit(Vec3(-delta.1, delta.0, 0.0)) * 14.0;
    if [-1.0, 0.0, 1.0].into_iter().any(|offset| {
        world
            .ray(from + side * offset + Vec3(0.0, 0.0, 8.0), delta)
            .is_some()
    }) {
        return false;
    }
    // Probe several places along the ground so a clear line through a gap is not a walk edge.
    let probes = (horizontal / 48.0).ceil().max(1.0) as u32;
    (1..=probes).all(|i| {
        let position = from + delta * (i as f32 / probes as f32);
        !world.regions.at(position).pain
            && world
                .ray(position + Vec3(0.0, 0.0, 8.0), Vec3(0.0, 0.0, -86.0))
                .is_some()
    })
}
