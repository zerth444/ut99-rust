use std::io::{self, Write};
use std::path::Path;

use crate::map::{Face, Map, Start, Vec3};
use crate::texture::TextureImage;

pub const WIDTH: usize = 800;
pub const HEIGHT: usize = 450;
const NEAR: f32 = 4.0;

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    pub fn from_start(start: Start) -> Self {
        Self {
            position: start.position + Vec3(0.0, 0.0, 24.0),
            yaw: start.yaw,
            pitch: 0.0,
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    pub fn look_direction(&self) -> Vec3 {
        let (sin, cos) = self.pitch.sin_cos();
        self.forward() * cos + Vec3(0.0, 0.0, sin)
    }

    pub fn right(&self) -> Vec3 {
        Vec3(-self.yaw.sin(), self.yaw.cos(), 0.0)
    }
}

#[derive(Clone, Copy)]
struct Vertex {
    position: Vec3,
    u: f32,
    v: f32,
}

pub struct Frame {
    pub pixels: Vec<u32>,
    depth: Vec<f32>,
    sky: Vec<u32>,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            depth: vec![0.0; WIDTH * HEIGHT],
            sky: vec![0x0d1520; WIDTH * HEIGHT],
        }
    }

    pub fn draw(&mut self, map: &Map, camera: &Camera, time: f32) {
        if let Some(position) = map.sky_origin {
            let sky_camera = Camera {
                position,
                yaw: camera.yaw,
                pitch: camera.pitch,
            };
            self.clear();
            self.draw_faces(&map.faces, &sky_camera, time, true);
            std::mem::swap(&mut self.sky, &mut self.pixels);
        } else {
            self.sky.fill(0x0d1520);
        }
        self.clear();
        self.draw_faces(&map.faces, camera, time, false);
        for mover in &map.movers {
            let local_camera = Camera {
                position: camera.position - mover.offset,
                yaw: camera.yaw,
                pitch: camera.pitch,
            };
            self.draw_faces(&mover.faces, &local_camera, time, false);
        }
        let (sin, cos) = camera.pitch.sin_cos();
        let forward = camera.forward() * cos + Vec3(0.0, 0.0, sin);
        for prop in map.props.iter().filter(|prop| prop.visible) {
            let offset = prop.position - camera.position;
            if offset.dot(forward) + prop.radius >= NEAR {
                self.draw_faces(&prop.faces, camera, time, false);
            }
        }
        if map.hud.crosshair.is_none() {
            self.draw_crosshair();
        }
    }

    pub fn draw_practice(
        &mut self,
        map: &Map,
        camera: &Camera,
        time: f32,
        game: &crate::game::Practice,
    ) {
        self.draw(map, camera, time);
        for effect in game.visible_effects() {
            self.effect(camera, effect);
        }
        if let Some(weapon) = map.arsenal.get(&game.weapon) {
            if game.muzzle > game.time
                && let Some(flash) = &weapon.flash
            {
                let texture = &flash.textures[game.shots as usize % flash.textures.len()];
                let (w, h) = texture.size();
                let scale = flash.scale * WIDTH as f32 / 640.0;
                let mut x = flash.x;
                let mut y = flash.y;
                if game.weapon == crate::game::Weapon::Enforcer && game.alternate_flash() {
                    x *= 0.3;
                    y *= 2.5;
                }
                self.blit_tile(
                    texture.image(0.0),
                    [
                        ((0.5 + x) * WIDTH as f32 - w * scale * 0.5).round() as i32,
                        ((0.5 + y) * HEIGHT as f32 - h * scale * 0.5).round() as i32,
                        (w * scale).round() as i32,
                        (h * scale).round() as i32,
                    ],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                    true,
                );
            }
            let view = Camera {
                position: Vec3::default(),
                yaw: 0.0,
                pitch: 0.0,
            };
            self.depth.fill(0.0);
            let pose = game.pose(weapon);
            self.draw_faces_blended(pose.current, &view, time, false, Some(pose));
        }
        map.hud.draw(self, game);
        if map.hud.crosshair.is_none() {
            self.draw_crosshair();
        }
    }

    pub fn tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
    ) {
        self.blit_tile(texture, rect, source, tint, false);
    }

    fn blit_tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
        translucent: bool,
    ) {
        let [x, y, w, h] = rect;
        let [u, v, uw, vh] = source;
        if w <= 0 || h <= 0 {
            return;
        }
        for py in y.max(0)..(y + h).min(HEIGHT as i32) {
            for px in x.max(0)..(x + w).min(WIDTH as i32) {
                let u = u + (px - x) as f32 / w as f32 * uw;
                let v = v + (py - y) as f32 / h as f32 * vh;
                if let Some([r, g, b]) = texture.sample(u, v, true) {
                    let c = |value: f32, gain: f32| (value * gain).clamp(0.0, 255.0) as u32;
                    let pixel = &mut self.pixels[py as usize * WIDTH + px as usize];
                    let mix = |src: u32, shift: u32| {
                        let dst = (*pixel >> shift) & 255;
                        if translucent {
                            src + dst * (255 - src) / 255
                        } else {
                            src
                        }
                    };
                    *pixel = (mix(c(r, tint[0]), 16) << 16)
                        | (mix(c(g, tint[1]), 8) << 8)
                        | mix(c(b, tint[2]), 0);
                }
            }
        }
    }

    fn effect(&mut self, camera: &Camera, effect: crate::game::Effect) {
        let forward = camera.look_direction();
        let right = camera.right();
        let up = forward.cross(right);
        let view = |p| {
            let offset: Vec3 = p - camera.position;
            Vec3(offset.dot(right), offset.dot(up), offset.dot(forward))
        };
        let mut a = view(effect.origin);
        let mut b = view(effect.end);
        if a.2 < NEAR && b.2 < NEAR {
            return;
        }
        if a.2 < NEAR {
            a = a + (b - a) * ((NEAR - a.2) / (b.2 - a.2));
        } else if b.2 < NEAR {
            b = b + (a - b) * ((NEAR - b.2) / (a.2 - b.2));
        }
        let to_screen = |position| {
            project(Vertex {
                position,
                u: 0.0,
                v: 0.0,
            })
            .position
        };
        let a = to_screen(a);
        let b = to_screen(b);
        let delta = b - a;
        let length = delta.0.abs().max(delta.1.abs());
        if length < 1.0 {
            let radius = (b.2 * 960.0).clamp(3.0, 13.0) as i32;
            self.spark(b.0 as i32, b.1 as i32, radius, effect.color, b.2);
        } else {
            let steps = length.ceil().min(1700.0) as i32;
            for step in 0..=steps {
                let point = a + delta * (step as f32 / steps as f32);
                self.spark(point.0 as i32, point.1 as i32, 1, effect.color, point.2);
            }
        }
    }

    fn spark(&mut self, x: i32, y: i32, radius: i32, color: u32, depth: f32) {
        if x < -radius || x >= WIDTH as i32 + radius || y < -radius || y >= HEIGHT as i32 + radius {
            return;
        }
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (px, py) = (x + dx, y + dy);
                if (0..WIDTH as i32).contains(&px)
                    && (0..HEIGHT as i32).contains(&py)
                    && dx.abs() + dy.abs() <= radius
                {
                    let index = py as usize * WIDTH + px as usize;
                    if depth >= self.depth[index] * 0.99 {
                        self.pixels[index] = if dx == 0 && dy == 0 { 0xffffeb } else { color };
                    }
                }
            }
        }
    }

    fn clear(&mut self) {
        self.pixels.fill(0x0d1520);
        self.depth.fill(0.0);
    }

    fn draw_faces(&mut self, faces: &[Face], camera: &Camera, time: f32, sky: bool) {
        self.draw_faces_blended(faces, camera, time, sky, None);
    }

    fn draw_faces_blended(
        &mut self,
        faces: &[Face],
        camera: &Camera,
        time: f32,
        sky: bool,
        pose: Option<crate::game::Pose<'_>>,
    ) {
        let flat_forward = camera.forward();
        let right = camera.right();
        let up = Vec3(0.0, 0.0, 1.0);
        let (sine, cosine) = camera.pitch.sin_cos();
        let forward = flat_forward * cosine + up * sine;
        let up = up * cosine - flat_forward * sine;
        let mut transformed = Vec::new();
        let mut clipped = Vec::new();

        for face in faces.iter().filter(|face| face.sky == sky) {
            if sky && face.flags & 0x80 != 0 {
                continue;
            }
            let next = pose.filter(|pose| pose.fraction > 0.0).and_then(|pose| {
                let id = face.mesh_triangle?;
                let index = pose
                    .next
                    .binary_search_by_key(&Some(id), |face| face.mesh_triangle)
                    .ok()?;
                Some((&pose.next[index], pose.fraction))
            });
            let position =
                |i: usize, p: Vec3| next.map_or(p, |(face, t)| p + (face.vertices[i] - p) * t);
            let normal = next.map_or(face.normal, |(next, t)| {
                face.normal + (next.normal - face.normal) * t
            });
            if face.flags & 0x100 == 0
                && (camera.position - position(0, face.vertices[0])).dot(normal) < 0.0
            {
                continue;
            }
            transformed.clear();
            transformed.extend(face.vertices.iter().enumerate().map(|(i, &vertex)| {
                let offset = position(i, vertex) - camera.position;
                let texture_point = vertex - face.origin;
                Vertex {
                    position: Vec3(offset.dot(right), offset.dot(up), offset.dot(forward)),
                    u: texture_point.dot(face.u_axis),
                    v: texture_point.dot(face.v_axis),
                }
            }));
            if transformed.iter().all(|p| p.position.2 < NEAR) {
                continue;
            }
            clipped.clear();
            clip_near(&transformed, &mut clipped);
            if clipped.len() < 3 {
                continue;
            }
            let light = 0.93 + normal.dot(Vec3(-0.34, 0.43, 0.69)) * 0.32;
            let u_pan = if face.flags & 0x200 != 0 {
                time * 64.0
            } else {
                0.0
            };
            let v_pan = if face.flags & 0x400 != 0 {
                time * 64.0
            } else {
                0.0
            };
            let pan = [face.pan[0] + u_pan, face.pan[1] + v_pan];
            let texture = face.texture.as_ref().map(|texture| texture.image(time));
            let a = project(clipped[0]);
            for pair in clipped[1..].windows(2) {
                self.triangle(
                    a,
                    project(pair[0]),
                    project(pair[1]),
                    face,
                    ([light, pan[0], pan[1]], texture),
                );
            }
        }
    }

    fn draw_crosshair(&mut self) {
        let center = (HEIGHT / 2) * WIDTH + WIDTH / 2;
        for offset in [3, 4, 5, 6] {
            for pixel in [
                center - offset,
                center + offset,
                center - offset * WIDTH,
                center + offset * WIDTH,
            ] {
                self.pixels[pixel] = 0xcfe0e9;
            }
        }
    }

    fn triangle(
        &mut self,
        va: Vertex,
        vb: Vertex,
        vc: Vertex,
        face: &Face,
        ([light, pan_u, pan_v], texture): ([f32; 3], Option<&TextureImage>),
    ) {
        let [a, b, c] = [va.position, vb.position, vc.position];
        let area = edge(a, b, c.0, c.1);
        if area.abs() < 0.01 || !area.is_finite() {
            return;
        }
        let left = a.0.min(b.0).min(c.0).floor().max(0.0) as usize;
        let right = (a.0.max(b.0).max(c.0).ceil() as usize).min(WIDTH);
        let top = a.1.min(b.1).min(c.1).floor().max(0.0) as usize;
        let bottom = (a.1.max(b.1).max(c.1).ceil() as usize).min(HEIGHT);
        let inverse_area = area.recip();
        let d_x = [
            (b.1 - c.1) * inverse_area,
            (c.1 - a.1) * inverse_area,
            (a.1 - b.1) * inverse_area,
        ];
        for y in top..bottom {
            let x0 = left as f32 + 0.5;
            let py = y as f32 + 0.5;
            let mut w0 = edge(b, c, x0, py) * inverse_area;
            let mut w1 = edge(c, a, x0, py) * inverse_area;
            let mut w2 = edge(a, b, x0, py) * inverse_area;
            for x in left..right {
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let inverse_depth = w0 * a.2 + w1 * b.2 + w2 * c.2;
                    let index = y * WIDTH + x;
                    if inverse_depth > self.depth[index] && face.flags & 0x80 != 0 {
                        self.depth[index] = inverse_depth;
                        self.pixels[index] = self.sky[index];
                    } else if inverse_depth > self.depth[index] {
                        let u = (w0 * va.u + w1 * vb.u + w2 * vc.u) / inverse_depth;
                        let v = (w0 * va.v + w1 * vb.v + w2 * vc.v) / inverse_depth;
                        let color = if let Some(texture) = texture {
                            texture.sample(u + pan_u, v + pan_v, face.flags & 2 != 0)
                        } else {
                            Some(face.color)
                        };
                        if let Some(color) = color {
                            self.depth[index] = inverse_depth;
                            let exposure = light * (0.62 + 0.38 * (inverse_depth * 680.0).min(1.0));
                            let color = if let Some(lightmap) = &face.lightmap {
                                let lighting = lightmap.sample(u, v);
                                std::array::from_fn(|i| color[i] * lighting[i])
                            } else {
                                color.map(|channel| channel * exposure)
                            };
                            self.pixels[index] = pack(color, 1.0);
                        }
                    }
                }
                w0 += d_x[0];
                w1 += d_x[1];
                w2 += d_x[2];
            }
        }
    }

    pub fn save_bmp(&self, path: &Path) -> io::Result<()> {
        let mut file = io::BufWriter::new(std::fs::File::create(path)?);
        let image_size = (WIDTH * HEIGHT * 4) as u32;
        file.write_all(b"BM")?;
        for value in [
            54 + image_size,
            0,
            54,
            40,
            WIDTH as u32,
            -(HEIGHT as i32) as u32,
        ] {
            file.write_all(&value.to_le_bytes())?;
        }
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&32u16.to_le_bytes())?;
        for value in [0, image_size, 2835, 2835, 0, 0] {
            file.write_all(&value.to_le_bytes())?;
        }
        for pixel in &self.pixels {
            file.write_all(&pixel.to_le_bytes())?;
        }
        file.flush()
    }
}

fn edge(a: Vec3, b: Vec3, x: f32, y: f32) -> f32 {
    (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)
}

fn pack(color: [f32; 3], light: f32) -> u32 {
    let [r, g, b] = color.map(|channel| (channel * light).clamp(0.0, 255.0) as u32);
    (r << 16) | (g << 8) | b
}

fn clip_near(vertices: &[Vertex], clipped: &mut Vec<Vertex>) {
    let mut previous = vertices[vertices.len() - 1];
    for &current in vertices {
        let z = current.position.2;
        let prev_z = previous.position.2;
        if (prev_z >= NEAR) != (z >= NEAR) {
            let fraction = (NEAR - prev_z) / (z - prev_z);
            clipped.push(Vertex {
                position: previous.position + (current.position - previous.position) * fraction,
                u: previous.u + (current.u - previous.u) * fraction,
                v: previous.v + (current.v - previous.v) * fraction,
            });
        }
        if z >= NEAR {
            clipped.push(current);
        }
        previous = current;
    }
}

fn project(vertex: Vertex) -> Vertex {
    let p = vertex.position;
    let inverse_depth = p.2.recip();
    let scale = WIDTH as f32 * 0.48 * inverse_depth;
    Vertex {
        position: Vec3(
            WIDTH as f32 * 0.5 + p.0 * scale,
            HEIGHT as f32 * 0.5 - p.1 * scale,
            inverse_depth,
        ),
        u: vertex.u * inverse_depth,
        v: vertex.v * inverse_depth,
    }
}
