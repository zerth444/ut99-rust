use crate::{
    collision::Collision,
    map::{Map, Vec3},
    mover::LiftStop,
};

use super::distance_squared;

const EDGE_LIMIT: f32 = 620.0;
pub(super) const WAYPOINT_RADIUS: f32 = 34.0;

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
    Teleport(Vec3),
    Lift(LiftRide),
}

#[derive(Clone, Copy)]
struct Edge {
    to: usize,
    cost: f32,
    step: Step,
}

pub(super) struct Navigation {
    points: Vec<Vec3>,
    edges: Vec<Vec<Edge>>,
}

impl Navigation {
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
        let mut edges = vec![Vec::new(); points.len()];
        for (index, &from) in points.iter().enumerate() {
            for (next, &to) in points.iter().enumerate().skip(index + 1) {
                let distance = distance_squared(from, to);
                if distance <= EDGE_LIMIT.powi(2) && walk_link(&map.collision, from, to) {
                    let cost = distance.sqrt();
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
                }
            }
        }
        for (source, edge) in transfers {
            edges[source].push(edge);
        }
        Self { points, edges }
    }

    pub(super) fn next_step(&self, world: &Collision, from: Vec3, to: Vec3) -> Option<Step> {
        if walk_link(world, from, to) {
            return Some(Step::Walk(to));
        }
        let mut costs = vec![f32::INFINITY; self.points.len()];
        let mut visited = vec![false; self.points.len()];
        let mut first = vec![None; self.points.len()];
        for (id, &point) in self.points.iter().enumerate() {
            let distance = distance_squared(from, point);
            if distance <= EDGE_LIMIT.powi(2) && walk_link(world, from, point) {
                costs[id] = distance.sqrt();
                if (from.2 - point.2).abs() > 48.0
                    || (from.0 - point.0).hypot(from.1 - point.1) >= WAYPOINT_RADIUS
                {
                    first[id] = Some(Step::Walk(point));
                }
            }
        }
        let mut best = f32::INFINITY;
        let mut waypoint = None;
        while let Some(id) = (0..costs.len())
            .filter(|&id| !visited[id])
            .min_by(|&a, &b| costs[a].total_cmp(&costs[b]))
        {
            if costs[id] >= best {
                break;
            }
            visited[id] = true;
            let point = self.points[id];
            let closing = distance_squared(point, to);
            if closing <= EDGE_LIMIT.powi(2)
                && costs[id] + closing.sqrt() < best
                && walk_link(world, point, to)
            {
                best = costs[id] + closing.sqrt();
                waypoint = first[id].or(Some(Step::Walk(to)));
            }
            for edge in &self.edges[id] {
                let length = costs[id] + edge.cost;
                if length < costs[edge.to] {
                    costs[edge.to] = length;
                    first[edge.to] = first[id].or(Some(edge.step));
                }
            }
        }
        waypoint
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

fn walk_link(world: &Collision, from: Vec3, to: Vec3) -> bool {
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
    let probes = (horizontal / 100.0).ceil().max(1.0) as u32;
    (1..=probes).all(|i| {
        let position = from + delta * (i as f32 / probes as f32);
        !world.regions.at(position).pain
            && world
                .ray(position + Vec3(0.0, 0.0, 8.0), Vec3(0.0, 0.0, -86.0))
                .is_some()
    })
}
