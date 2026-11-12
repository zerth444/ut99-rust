use std::{collections::VecDeque, sync::Arc};

use crate::{
    map::{Face, Vec3},
    texture::Texture,
};

pub const MODULATED: u32 = 0x40;

#[derive(Default)]
pub struct Marks {
    marks: VecDeque<Mark>,
}

struct Mark {
    expires: f32,
    faces: Vec<Face>,
}

impl Marks {
    pub fn project(
        &mut self,
        world: &[Face],
        point: Vec3,
        normal: Vec3,
        texture: &Arc<Texture>,
        scale: f32,
        expires: f32,
    ) {
        let (width, height) = texture.size();
        let spacing = width.min(height) * scale * 0.25;
        if self.marks.iter().any(|mark| {
            let face = &mark.faces[0];
            let delta = face.origin - point;
            face.normal.dot(normal) > 0.99
                && delta.dot(delta) < spacing * spacing
                && face
                    .texture
                    .as_ref()
                    .is_some_and(|existing| Arc::ptr_eq(existing, texture))
        }) {
            return;
        }
        let (right, up) = super::shock::axes(normal);
        let dx = right * (width * scale * 0.5);
        let dy = up * (height * scale * 0.5);
        let corners = [
            point - dx + dy,
            point + dx + dy,
            point + dx - dy,
            point - dx - dy,
        ];
        let mut faces = Vec::new();
        for surface in world.iter().filter(|face| {
            !face.sky && face.flags & (4 | 0x80) == 0 && face.normal.dot(normal) > 0.99
        }) {
            let distance = (surface.vertices[0] - point).dot(normal);
            if distance.abs() > 0.1 {
                continue;
            }
            let center = surface
                .vertices
                .iter()
                .copied()
                .fold(Vec3::default(), |sum, p| sum + p)
                * (1.0 / surface.vertices.len() as f32);
            let mut polygon = corners.to_vec();
            let mut clipped = Vec::with_capacity(8);
            for (&a, &b) in surface
                .vertices
                .iter()
                .zip(surface.vertices.iter().cycle().skip(1))
            {
                let mut inward = normal.cross(b - a);
                if inward.dot(center - a) < 0.0 {
                    inward = inward * -1.0;
                }
                clip_polygon(&polygon, &mut clipped, a, inward);
                std::mem::swap(&mut polygon, &mut clipped);
                if polygon.len() < 3 {
                    break;
                }
            }
            if polygon.len() >= 3 {
                let bias = normal * (distance + 0.04);
                for vertex in &mut polygon {
                    *vertex = *vertex + bias;
                }
                faces.push(Face {
                    vertices: polygon,
                    mesh_triangle: None,
                    normal,
                    color: [255.0; 3],
                    flags: MODULATED,
                    sky: false,
                    texture: Some(Arc::clone(texture)),
                    lightmap: None,
                    origin: point + bias,
                    u_axis: right * scale.recip(),
                    v_axis: up * -scale.recip(),
                    pan: [width * 0.5, height * 0.5],
                });
            }
        }
        if !faces.is_empty() {
            if self.marks.len() >= 128 {
                self.marks.pop_front();
            }
            self.marks.push_back(Mark { expires, faces });
        }
    }

    pub fn update(&mut self, time: f32) {
        self.marks.retain(|mark| mark.expires > time);
    }

    pub fn visible(&self) -> impl Iterator<Item = &[Face]> {
        self.marks.iter().map(|mark| mark.faces.as_slice())
    }
}

fn clip_polygon(polygon: &[Vec3], clipped: &mut Vec<Vec3>, edge: Vec3, inward: Vec3) {
    clipped.clear();
    let mut previous = polygon[polygon.len() - 1];
    let mut before = (previous - edge).dot(inward);
    for &current in polygon {
        let after = (current - edge).dot(inward);
        if (before >= 0.0) != (after >= 0.0) {
            clipped.push(previous + (current - previous) * (before / (before - after)));
        }
        if after >= 0.0 {
            clipped.push(current);
        }
        previous = current;
        before = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_a_mark_at_a_wall_edge() {
        let square = [
            Vec3(0.0, -2.0, -2.0),
            Vec3(0.0, 2.0, -2.0),
            Vec3(0.0, 2.0, 2.0),
            Vec3(0.0, -2.0, 2.0),
        ];
        let mut clipped = Vec::new();
        clip_polygon(
            &square,
            &mut clipped,
            Vec3(0.0, 1.0, 0.0),
            Vec3(0.0, -1.0, 0.0),
        );
        assert_eq!(clipped.len(), 4);
        assert_eq!(clipped.iter().filter(|p| p.1 == 1.0).count(), 2);
        assert!(clipped.iter().all(|p| p.1 <= 1.0));
    }
}
