use crate::{
    collision::Collision,
    map::{Map, Vec3},
};

use super::distance_squared;

const EDGE_LIMIT: f32 = 620.0;

pub(super) struct Navigation {
    points: Vec<Vec3>,
    edges: Vec<Vec<(usize, f32)>>,
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
        let mut edges = vec![Vec::new(); points.len()];
        for (index, &from) in points.iter().enumerate() {
            for (next, &to) in points.iter().enumerate().skip(index + 1) {
                let distance = distance_squared(from, to);
                if distance <= EDGE_LIMIT.powi(2) && walk_link(&map.collision, from, to) {
                    let cost = distance.sqrt();
                    edges[index].push((next, cost));
                    edges[next].push((index, cost));
                }
            }
        }
        Self { points, edges }
    }

    pub(super) fn next_step(&self, world: &Collision, from: Vec3, to: Vec3) -> Option<Vec3> {
        if walk_link(world, from, to) {
            return Some(to);
        }
        let mut costs = vec![f32::INFINITY; self.points.len()];
        let mut visited = vec![false; self.points.len()];
        let mut first = vec![None; self.points.len()];
        for (id, &point) in self.points.iter().enumerate() {
            let distance = distance_squared(from, point);
            if distance <= EDGE_LIMIT.powi(2) && walk_link(world, from, point) {
                costs[id] = distance.sqrt();
                first[id] = Some(id);
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
                waypoint = first[id].map(|first| self.points[first]);
            }
            for &(next, cost) in &self.edges[id] {
                let length = costs[id] + cost;
                if length < costs[next] {
                    costs[next] = length;
                    first[next] = first[id];
                }
            }
        }
        waypoint
    }
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
