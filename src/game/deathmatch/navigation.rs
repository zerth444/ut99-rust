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
        self.route_avoiding(world, from, to, blocked)
            .map(|(step, _)| step)
    }

    pub(super) fn route_avoiding(
        &self,
        world: &Collision,
        from: Vec3,
        to: Vec3,
        blocked: &[(Vec3, f32)],
    ) -> Option<(Step, f32)> {
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
            return lift.map(|step| (step, distance_squared(from, to).sqrt()));
        }
        if allowed(to)
            && let Some(step) = approach_step(world, from, to)
        {
            return Some((
                step,
                approach_cost(world, step, distance_squared(from, to).sqrt()),
            ));
        }
        let mut costs = vec![f32::INFINITY; self.points.len()];
        let mut queue = BinaryHeap::new();
        let mut first = vec![None; self.points.len()];
        for (id, &point) in self.points.iter().enumerate() {
            let distance = distance_squared(from, point);
            if distance <= EDGE_LIMIT.powi(2)
                && let Some(step) = approach_step(world, from, point)
            {
                if (from.2 - point.2).abs() > 48.0
                    || (from.0 - point.0).hypot(from.1 - point.1) >= WAYPOINT_RADIUS
                {
                    if !allowed(point) {
                        continue;
                    }
                    first[id] = Some(step);
                }
                costs[id] = approach_cost(world, step, distance.sqrt());
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
                && let Some(step) = approach_step(world, point, to)
            {
                let total = costs[id] + approach_cost(world, step, closing.sqrt());
                if total < best {
                    best = total;
                    waypoint = first[id].or(Some(step));
                }
            }
            for edge in &self.edges[id] {
                if !allowed(self.points[edge.to])
                    || matches!(edge.step, Step::Teleport(entry) if !allowed(entry))
                {
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
        waypoint.map(|step| (step, best))
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
            })
            .filter_map(|point| Some((point, approach_step(world, from, point)?)))
            .min_by(|&(a, _), &(b, _)| {
                let score = |point| {
                    objective.map_or(distance_squared(from, point), |goal| {
                        distance_squared(goal, point)
                    })
                };
                score(a).total_cmp(&score(b))
            })
            .map(|(_, step)| step)
    }
}

fn approach_step(world: &Collision, from: Vec3, to: Vec3) -> Option<Step> {
    use crate::collision::{PLAYER_HEIGHT, PLAYER_RADIUS};
    if world.regions.at(from).water {
        let delta = to - from;
        if !world.regions.at(to).pain
            && world
                .projectile_surface(from, delta, PLAYER_RADIUS, PLAYER_HEIGHT)
                .is_none()
        {
            return Some(Step::Swim(to));
        }
        return None;
    }
    walk_link(world, from, to).then_some(Step::Walk(to))
}

fn approach_cost(world: &Collision, step: Step, distance: f32) -> f32 {
    if matches!(step, Step::Swim(_)) {
        distance * (world.profile.ground_speed / world.profile.water_speed.max(1.0)).max(1.0)
    } else {
        distance
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

pub(super) fn portal_clearance(
    world: &Collision,
    from: Vec3,
    portal: Vec3,
    radius: f32,
) -> Option<Vec3> {
    let offset = from - portal;
    let angle = offset.1.atan2(offset.0);
    (0_usize..16)
        .map(|index| {
            let turn = index.div_ceil(2) as f32 * std::f32::consts::TAU / 16.0;
            let (sin, cos) = (angle + if index % 2 == 0 { turn } else { -turn }).sin_cos();
            Vec3(portal.0, portal.1, from.2) + Vec3(cos, sin, 0.0) * (radius + 24.0)
        })
        .find(|&point| {
            world
                .projectile_surface(
                    from,
                    point - from,
                    crate::collision::PLAYER_RADIUS,
                    crate::collision::PLAYER_HEIGHT,
                )
                .is_none()
                && walk_link(world, from, point)
        })
}

pub(super) fn swim_to_surface(
    world: &Collision,
    from: Vec3,
    blocked: &[(Vec3, f32)],
) -> Option<Step> {
    if !world.regions.at(from).water {
        return None;
    }
    // Look up and around an overhead ledge when no authored route is accessible.
    let vertical = Vec3(0.0, 0.0, 112.0);
    std::iter::once(from + vertical)
        .chain((0..8).map(|index| {
            let (sin, cos) = (index as f32 * std::f32::consts::TAU / 8.0).sin_cos();
            from + vertical + Vec3(cos, sin, 0.0) * 96.0
        }))
        .filter(|&point| {
            blocked
                .iter()
                .all(|&(failed, _)| distance_squared(point, failed) > 40.0_f32.powi(2))
        })
        .filter_map(|point| Some((point, approach_step(world, from, point)?)))
        .min_by_key(|(point, _)| world.regions.at(*point).water)
        .map(|(_, step)| step)
}

pub(super) fn walk_around_wall(world: &Collision, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
    use crate::collision::{PLAYER_HEIGHT, PLAYER_RADIUS};
    const SIDE: usize = 29;
    const SPACING: f32 = 24.0;
    let delta = to - from;
    let horizontal = Vec3(delta.0, delta.1, 0.0);
    if delta.2.abs() > world.profile.step_height + 16.0
        || !(30.0_f32.powi(2)..=440.0_f32.powi(2)).contains(&horizontal.dot(horizontal))
    {
        return None;
    }
    let (hit, normal, _) =
        world.projectile_contact(from, horizontal, PLAYER_RADIUS, PLAYER_HEIGHT)?;
    if normal.2.abs() >= 0.25 || distance_squared(from, hit) > 70.0_f32.powi(2) {
        return None;
    }
    let clear = |a: Vec3, b: Vec3| {
        let delta = b - a;
        world
            .projectile_surface(a, Vec3(delta.0, delta.1, 0.0), PLAYER_RADIUS, PLAYER_HEIGHT)
            .is_none()
            && walk_link(world, a, b)
    };
    let point = |id: usize| {
        let middle = (SIDE / 2) as f32;
        from + Vec3(
            (id % SIDE) as f32 - middle,
            (id / SIDE) as f32 - middle,
            0.0,
        ) * SPACING
    };
    let start = (SIDE * SIDE) / 2;
    let mut costs = [f32::INFINITY; SIDE * SIDE];
    let mut parents = [None; SIDE * SIDE];
    let mut queue = BinaryHeap::new();
    costs[start] = 0.0;
    queue.push(RouteCandidate {
        id: start,
        cost: distance_squared(from, to).sqrt(),
    });
    while let Some(RouteCandidate { id, cost }) = queue.pop() {
        let here = point(id);
        let remaining = distance_squared(here, to).sqrt();
        if cost > costs[id] + remaining {
            continue;
        }
        if id != start && remaining <= 112.0 && clear(here, to) {
            let mut path = vec![to, here];
            let mut parent = parents[id];
            while let Some(index) = parent.filter(|&index| index != start) {
                path.push(point(index));
                parent = parents[index];
            }
            let mut route = Vec::new();
            let mut origin = from;
            while let Some(next) = path.pop() {
                if path.last().is_some_and(|&after| clear(origin, after)) {
                    continue;
                }
                route.push(next);
                origin = next;
            }
            return Some(route);
        }
        let (x, y) = (id % SIDE, id / SIDE);
        for (dx, dy) in [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            let (Some(x), Some(y)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) else {
                continue;
            };
            if x >= SIDE || y >= SIDE {
                continue;
            }
            let next = y * SIDE + x;
            let there = point(next);
            let distance = costs[id] + distance_squared(here, there).sqrt();
            if distance < costs[next] && world.player_fits(there) && clear(here, there) {
                costs[next] = distance;
                parents[next] = Some(id);
                queue.push(RouteCandidate {
                    id: next,
                    cost: distance + distance_squared(there, to).sqrt(),
                });
            }
        }
    }
    None
}
