use std::io::{self, Write};
use std::path::Path;

use crate::game::{
    decals::MODULATED,
    shock::{self, Visual},
};
use crate::map::{Face, Map, Start, Vec3};
use crate::texture::TextureImage;

pub mod commands;
use commands::{DrawList, Layer, Material, Mode, Vertex as GpuVertex};

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

struct View {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
}

impl View {
    fn camera(camera: &Camera) -> Self {
        let forward = camera.look_direction();
        let right = camera.right();
        Self {
            eye: camera.position,
            right,
            up: forward.cross(right),
            forward,
        }
    }

    fn in_model(&self, origin: Vec3, forward: Vec3, scale: f32) -> Self {
        self.scaled_model(origin, forward, Vec3(scale, scale, scale))
    }

    fn scaled_model(&self, origin: Vec3, forward: Vec3, scale: Vec3) -> Self {
        let (right, up) = shock::axes(forward);
        let local = |p: Vec3| Vec3(p.dot(forward), p.dot(right), p.dot(up));
        let product = crate::mover::product;
        Self {
            eye: product(
                local(self.eye - origin),
                Vec3(scale.0.recip(), scale.1.recip(), scale.2.recip()),
            ),
            right: product(local(self.right), scale),
            up: product(local(self.up), scale),
            forward: product(local(self.forward), scale),
        }
    }

    fn point(&self, position: Vec3) -> Vec3 {
        let offset = position - self.eye;
        Vec3(
            offset.dot(self.right),
            offset.dot(self.up),
            offset.dot(self.forward),
        )
    }
}

pub struct Frame {
    pub pixels: Vec<u32>,
    depth: Vec<f32>,
    sky: Vec<u32>,
    pub width: usize,
    pub height: usize,
    pub commands: Option<DrawList>,
    zoom: f32,
    roll: f32,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            depth: vec![0.0; WIDTH * HEIGHT],
            sky: vec![0x0d1520; WIDTH * HEIGHT],
            width: WIDTH,
            height: HEIGHT,
            commands: None,
            zoom: 1.0,
            roll: 0.0,
        }
    }

    pub fn hardware(map: &Map) -> Self {
        Self {
            pixels: Vec::new(),
            depth: Vec::new(),
            sky: Vec::new(),
            width: WIDTH,
            height: HEIGHT,
            commands: Some(DrawList::new(map)),
            zoom: 1.0,
            roll: 0.0,
        }
    }

    fn layer(&mut self, layer: Layer) {
        if let Some(commands) = &mut self.commands {
            commands.layer = layer;
        }
    }

    #[cfg(windows)]
    pub fn begin_ui(&mut self, clear: bool) {
        if clear && let Some(commands) = &mut self.commands {
            commands.clear();
        }
        self.layer(Layer::Hud);
    }

    #[cfg(windows)]
    pub fn draw_cinematic(
        &mut self,
        map: &Map,
        camera: &Camera,
        time: f32,
        fov: f32,
        roll: f32,
        flash: f32,
    ) {
        self.zoom = (0.5 * fov.clamp(20.0, 150.0).to_radians()).tan().recip();
        self.roll = roll;
        self.draw(map, camera, time);
        self.roll = 0.0;
        self.zoom = 1.0;
        self.begin_ui(false);
        if flash < 0.999 {
            if let Some(commands) = &mut self.commands {
                let gain = flash.clamp(0.0, 1.0) * 0.5;
                quad(
                    commands,
                    [self.width, self.height],
                    [0, 0, self.width as i32, self.height as i32],
                    [0.0, 0.0, 1.0, 1.0],
                    [gain, gain, gain, 0.0],
                    None,
                    Mode::ModulatedScreen,
                    commands::WHITE,
                );
            } else {
                for pixel in &mut self.pixels {
                    let color = [16, 8, 0].map(|shift| ((*pixel >> shift) & 255) as f32);
                    *pixel = pack(color, flash);
                }
            }
        }
    }

    pub fn draw(&mut self, map: &Map, camera: &Camera, time: f32) {
        if let Some(commands) = &mut self.commands {
            commands.clear();
        }
        if let Some(position) = map.sky_origin {
            self.layer(Layer::Sky);
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
        self.layer(Layer::World);
        self.clear();
        self.draw_faces(&map.faces, camera, time, false);
        for mover in &map.movers {
            let local_camera = Camera {
                position: camera.position - mover.offset,
                yaw: camera.yaw,
                pitch: camera.pitch,
            };
            if mover.scene_rotation.dot(mover.scene_rotation) > 0.0 {
                let mut view = self.camera_view(&local_camera);
                let [x, y, z] = [
                    Vec3(1.0, 0.0, 0.0),
                    Vec3(0.0, 1.0, 0.0),
                    Vec3(0.0, 0.0, 1.0),
                ]
                .map(|axis| crate::mover::rotate(axis, mover.scene_rotation));
                let inverse = |p: Vec3| Vec3(p.dot(x), p.dot(y), p.dot(z));
                view.eye = mover.pivot + inverse(view.eye - mover.pivot);
                view.forward = inverse(view.forward);
                view.up = inverse(view.up);
                view.right = inverse(view.right);
                self.draw_faces_in_view(&mover.faces, &view, time, false, None, None);
            } else {
                self.draw_faces(&mover.faces, &local_camera, time, false);
            }
        }
        let (sin, cos) = camera.pitch.sin_cos();
        let forward = camera.forward() * cos + Vec3(0.0, 0.0, sin);
        for prop in map.props.iter().filter(|prop| prop.visible) {
            let offset = prop.position - camera.position;
            if offset.dot(forward) + prop.radius >= NEAR {
                let pose = prop.cinematic.as_ref().and_then(|actor| {
                    let time = time - actor.started?;
                    actor
                        .animation
                        .sample(time.min((actor.animation.duration() - 0.001).max(0.0)))
                });
                self.draw_faces_blended(
                    pose.map_or(prop.faces.as_slice(), |pose| pose.current),
                    camera,
                    time,
                    false,
                    pose,
                );
            }
        }
    }

    pub fn draw_practice(
        &mut self,
        map: &Map,
        camera: &Camera,
        time: f32,
        game: &crate::game::Practice,
    ) {
        let death_camera = game.death.as_ref().map(|death| {
            let age = (game.time - death.started).max(0.0);
            let angle = death.yaw + 0.8 + age.min(1.6) * 0.45;
            let up = Vec3(0.0, 0.0, 1.0);
            let target = death.position - up * (12.0 + age.min(1.0) * 12.0);
            let delta = Vec3(-angle.cos() * 118.0, -angle.sin() * 118.0, 54.0);
            let position = map
                .collision
                .ray(target, delta)
                .map_or(target + delta, |point| {
                    point - crate::mover::unit(delta) * 5.0
                });
            let look = target - position;
            Camera {
                position,
                yaw: look.1.atan2(look.0),
                pitch: look.2.atan2(look.0.hypot(look.1)),
            }
        });
        let guided_camera = game.ordnance.guided_view();
        let camera = death_camera
            .as_ref()
            .or(guided_camera.as_ref())
            .unwrap_or(camera);
        self.zoom = (0.5 * game.field_of_view().to_radians()).tan().recip();
        self.draw(map, camera, time);
        self.layer(Layer::Effects);
        let view = View::camera(camera);
        for mark in game.decals.visible() {
            self.draw_faces_in_view(mark, &view, time, false, None, None);
        }
        if let Some(target) = &game.target {
            let motion = if target.health == 0 {
                &map.player_deaths[0]
            } else {
                &map.player_idle
            };
            let age = if target.health == 0 {
                (motion.duration() - 0.001).max(0.0)
            } else if motion.duration() > 0.0 {
                game.time.rem_euclid(motion.duration())
            } else {
                0.0
            };
            if let Some(pose) = motion.sample(age) {
                let model = view.in_model(
                    target.position,
                    Vec3(target.yaw.cos(), target.yaw.sin(), 0.0),
                    1.0,
                );
                self.draw_faces_in_view(pose.current, &model, time, false, Some(pose), None);
            }
        }
        let mut draw_effect = |visual| match visual {
            crate::game::ordnance::Visual::Sprite(position, image, scale, glow) => {
                self.world_sprite(&view, position, image, scale, glow)
            }
            crate::game::ordnance::Visual::Mesh(position, direction, pose, scale, glow) => {
                let model = view.in_model(position, direction, scale);
                self.draw_faces_in_view(pose.current, &model, time, false, Some(pose), glow);
            }
        };
        game.ordnance
            .visuals(&map.ordnance, game.time, &mut draw_effect);
        game.ballistics
            .visuals(&map.ballistics, game.time, &mut draw_effect);
        if let Some(death) = &game.death {
            let motion = &map.player_deaths[death.animation];
            let age = (game.time - death.started).min((motion.duration() - 0.001).max(0.0));
            if let Some(pose) = motion.sample(age) {
                let model = view.in_model(
                    death.position,
                    Vec3(death.yaw.cos(), death.yaw.sin(), 0.0),
                    1.0,
                );
                self.draw_faces_in_view(pose.current, &model, time, false, Some(pose), None);
            }
        }
        for effect in game.visible_effects() {
            self.effect(camera, effect);
        }
        if let Some(assets) = shock::assets(map) {
            let view = View::camera(camera);
            game.shock
                .visuals(assets, game.time, |visual| match visual {
                    Visual::Sprite {
                        position,
                        image,
                        scale,
                        glow,
                    } => self.world_sprite(&view, position, image, scale, glow),
                    Visual::Ring {
                        position,
                        direction,
                        pose,
                        scale,
                        glow,
                    } => {
                        let model = view.in_model(position, direction, scale);
                        self.draw_faces_in_view(
                            pose.current,
                            &model,
                            time,
                            false,
                            Some(pose),
                            Some(glow),
                        );
                    }
                });
        }
        if let Some(beam) = &game.pulse_beam
            && !beam.blocked
            && let Some(art) = map.arsenal.get(&crate::game::Weapon::Pulse)
        {
            let delta = beam.end - beam.start;
            let length = delta.dot(delta).sqrt();
            if length > 0.0 {
                let forward = delta * length.recip();
                let view = View::camera(camera);
                let age = game.time - beam.started;
                let animation = (age / 0.05) as usize;
                for segment in 0..((length / 81.0).ceil() as usize).min(10) {
                    let frames = if segment == 0 {
                        &art.pulse_starter
                    } else {
                        &art.pulse_beam
                    };
                    if !frames.is_empty() {
                        let faces = &frames[animation % frames.len()];
                        let model = view.in_model(
                            beam.start + forward * (segment as f32 * 81.0),
                            forward,
                            1.0,
                        );
                        self.draw_faces_in_view(faces, &model, time, false, None, Some(1.0));
                    }
                }
                if !beam.hit_player
                    && let Some(tip) = &art.pulse_tip[usize::from(beam.wall.is_some())]
                {
                    let inset = if beam.wall.is_some() { 5.0 } else { 4.0 };
                    self.world_sprite(&view, beam.end - forward * inset, tip.image(age), 0.65, 1.0);
                }
            }
        }
        self.zoom = 1.0;
        if guided_camera.is_none()
            && game.death.is_none()
            && !game.scoped()
            && let Some(weapon) = map.arsenal.get(&game.weapon)
        {
            let view = Camera {
                position: Vec3::default(),
                yaw: 0.0,
                pitch: 0.0,
            };
            let pose = game.pose(weapon);
            if game.muzzle > game.time
                && let Some(flash) = &weapon.flash
            {
                let texture = &flash.textures[game.shots as usize % flash.textures.len()];
                let (w, h) = texture.size();
                let scale = flash.scale * self.width as f32 / 640.0;
                let [x, y] = weapon.muzzle_position(pose).map_or_else(
                    || {
                        [
                            (0.5 + flash.x) * self.width as f32,
                            (0.5 + flash.y) * self.height as f32,
                        ]
                    },
                    |position| {
                        let muzzle = project(
                            Vertex {
                                position: View::camera(&view).point(position),
                                u: 0.0,
                                v: 0.0,
                            },
                            self.width,
                            self.height,
                            self.zoom,
                        )
                        .position;
                        [muzzle.0, muzzle.1]
                    },
                );
                let offset = if flash.half_offset > 0.0 {
                    [flash.half_offset; 2]
                } else {
                    [w * 0.5, h * 0.5]
                };
                self.blit_tile(
                    texture.image(0.0),
                    [
                        (x - offset[0] * scale).round() as i32,
                        (y - offset[1] * scale).round() as i32,
                        (w * scale).round() as i32,
                        (h * scale).round() as i32,
                    ],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                    true,
                    None,
                );
            }
            self.depth.fill(0.0);
            self.layer(Layer::Weapon);
            self.draw_faces_blended(pose.current, &view, time, false, Some(pose));
        }
        self.layer(Layer::Hud);
        if let Some(weapon) = map.arsenal.get(&crate::game::Weapon::Redeemer) {
            let [reticle, readout, noise] = &weapon.guided_hud;
            if guided_camera.is_some() && game.death.is_none() {
                if let Some(reticle) = reticle {
                    let (w, h) = reticle.size();
                    let scale = (self.width as f32 / 640.0).floor().max(0.5);
                    let (dw, dh) = ((w * scale) as i32, (h * scale) as i32);
                    self.translucent_tile(
                        reticle.image(0.0),
                        [
                            (self.width as i32 - dw) / 2,
                            (self.height as i32 - dh) / 2,
                            dw,
                            dh,
                        ],
                        [0.0, 0.0, w, h],
                        [1.0; 3],
                    );
                }
                if let Some(readout) = readout {
                    let (w, h) = readout.size();
                    let offset = (game.time * 75.0) as i32 % h as i32;
                    for y in (0..self.height as i32 + h as i32).step_by(h as usize) {
                        self.translucent_tile(
                            readout.image(0.0),
                            [1, y - offset, w as i32, h as i32],
                            [0.0, 0.0, w, h],
                            [1.0; 3],
                        );
                    }
                }
                return;
            }
            if game.death.is_none()
                && game.time < game.guidance_lost_until
                && let Some(noise) = noise
            {
                let (w, h) = noise.size();
                self.tile(
                    noise.image(time),
                    [0, 0, self.width as i32, self.height as i32],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                );
                return;
            }
        }
        let flash = ((game.hit_flash_until - game.time) / 0.20).clamp(0.0, 1.0) * 0.22;
        if flash > 0.0 {
            if let Some(commands) = &mut self.commands {
                quad(
                    commands,
                    [self.width, self.height],
                    [0, 0, self.width as i32, self.height as i32],
                    [0.0, 0.0, 1.0, 1.0],
                    [flash, 0.0, 0.0, 0.0],
                    None,
                    Mode::Screen,
                    commands::WHITE,
                );
            } else {
                let tint = pack([flash, 0.0, 0.0], 255.0);
                for pixel in &mut self.pixels {
                    *pixel = blend_light(tint, *pixel);
                }
            }
        }
        map.hud.draw(self, game, &map.arsenal);
        if game.death.is_some() {
            return;
        } else if game.scoped() {
            if let Some(scope) = map
                .arsenal
                .get(&game.weapon)
                .and_then(|weapon| weapon.scope.as_ref())
            {
                let (w, h) = scope.size();
                let scale = (self.width as f32 / 640.0).min(self.height as f32 / 256.0);
                let (dw, dh) = ((w * scale).round() as i32, (h * scale).round() as i32);
                self.translucent_tile(
                    scope.image(0.0),
                    [
                        (self.width as i32 - dw) / 2,
                        (self.height as i32 - dh) / 2,
                        dw,
                        dh,
                    ],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                );
            } else {
                self.draw_crosshair();
            }
        } else if map.hud.crosshair.is_none() {
            self.draw_crosshair();
        }
    }

    fn world_sprite(
        &mut self,
        view: &View,
        position: Vec3,
        image: &TextureImage,
        scale: f32,
        glow: f32,
    ) {
        let position = view.point(position);
        if position.2 < NEAR {
            return;
        }
        let screen = project(
            Vertex {
                position,
                u: 0.0,
                v: 0.0,
            },
            self.width,
            self.height,
            self.zoom,
        )
        .position;
        let (w, h) = image.size();
        let size = self.focal_length() * screen.2 * scale;
        let width = (w * size).round() as i32;
        let height = (h * size).round() as i32;
        self.blit_tile(
            image,
            [
                screen.0.round() as i32 - width / 2,
                screen.1.round() as i32 - height / 2,
                width,
                height,
            ],
            [0.0, 0.0, w, h],
            [glow; 3],
            true,
            Some(screen.2),
        );
    }

    pub fn tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
    ) {
        self.blit_tile(texture, rect, source, tint, false, None);
    }

    pub fn translucent_tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
    ) {
        self.blit_tile(texture, rect, source, tint, true, None);
    }

    pub fn solid(&mut self, rect: [i32; 4], color: [f32; 3]) {
        if let Some(commands) = &mut self.commands {
            quad(
                commands,
                [self.width, self.height],
                rect,
                [0.0, 0.0, 1.0, 1.0],
                [color[0], color[1], color[2], 0.0],
                None,
                Mode::Ui,
                commands::WHITE,
            );
        } else {
            let [x, y, w, h] = rect;
            let color = pack(color, 255.0);
            for py in y.max(0)..(y + h).min(self.height as i32) {
                for px in x.max(0)..(x + w).min(self.width as i32) {
                    self.pixels[py as usize * self.width + px as usize] = color;
                }
            }
        }
    }

    fn blit_tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
        translucent: bool,
        inverse_depth: Option<f32>,
    ) {
        let [x, y, w, h] = rect;
        let [u, v, uw, vh] = source;
        if w <= 0 || h <= 0 {
            return;
        }
        if let Some(commands) = &mut self.commands {
            let (id, [tw, th]) = commands.texture(Some(texture));
            let mode = match (translucent, inverse_depth) {
                (true, Some(_)) => Mode::ScreenDepth,
                (true, None) => Mode::Screen,
                _ => Mode::Ui,
            };
            quad(
                commands,
                [self.width, self.height],
                rect,
                [u / tw, v / th, uw / tw, vh / th],
                [tint[0], tint[1], tint[2], 0.25],
                inverse_depth,
                mode,
                id,
            );
            return;
        }
        for py in y.max(0)..(y + h).min(HEIGHT as i32) {
            for px in x.max(0)..(x + w).min(WIDTH as i32) {
                let index = py as usize * WIDTH + px as usize;
                if inverse_depth.is_some_and(|depth| depth <= self.depth[index]) {
                    continue;
                }
                let u = u + (px - x) as f32 / w as f32 * uw;
                let v = v + (py - y) as f32 / h as f32 * vh;
                if let Some([r, g, b]) = texture.sample(u, v, true) {
                    let c = |value: f32, gain: f32| (value * gain).clamp(0.0, 255.0) as u32;
                    let color = (c(r, tint[0]) << 16) | (c(g, tint[1]) << 8) | c(b, tint[2]);
                    self.pixels[index] = if translucent {
                        blend_light(color, self.pixels[index])
                    } else {
                        color
                    };
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
        let dimensions = (self.width, self.height);
        let to_screen = |position| {
            project(
                Vertex {
                    position,
                    u: 0.0,
                    v: 0.0,
                },
                dimensions.0,
                dimensions.1,
                self.zoom,
            )
            .position
        };
        let a = to_screen(a);
        let b = to_screen(b);
        if let Some(commands) = &mut self.commands {
            effect_geometry(commands, dimensions, a, b, effect.color);
            return;
        }
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
        self.draw_faces_in_view(faces, &self.camera_view(camera), time, sky, pose, None);
    }

    fn camera_view(&self, camera: &Camera) -> View {
        let mut view = View::camera(camera);
        if self.roll != 0.0 {
            let (sin, cos) = self.roll.sin_cos();
            (view.right, view.up) = (
                view.right * cos + view.up * sin,
                view.up * cos - view.right * sin,
            );
        }
        view
    }

    fn draw_faces_in_view(
        &mut self,
        faces: &[Face],
        view: &View,
        time: f32,
        sky: bool,
        pose: Option<crate::game::Pose<'_>>,
        glow: Option<f32>,
    ) {
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
                && (view.eye - position(0, face.vertices[0])).dot(normal) < 0.0
            {
                continue;
            }
            transformed.clear();
            transformed.extend(face.vertices.iter().enumerate().map(|(i, &vertex)| {
                let texture_point = vertex - face.origin;
                Vertex {
                    position: view.point(position(i, vertex)),
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
            let a = project(clipped[0], self.width, self.height, self.zoom);
            for pair in clipped[1..].windows(2) {
                self.triangle(
                    a,
                    project(pair[0], self.width, self.height, self.zoom),
                    project(pair[1], self.width, self.height, self.zoom),
                    face,
                    ([light, pan[0], pan[1]], texture),
                    glow,
                );
            }
        }
    }

    fn draw_crosshair(&mut self) {
        if self.commands.is_some() {
            let (x, y) = (self.width as i32 / 2, self.height as i32 / 2);
            let color = [0.81, 0.88, 0.91];
            for rect in [
                [x - 6, y, 4, 1],
                [x + 3, y, 4, 1],
                [x, y - 6, 1, 4],
                [x, y + 3, 1, 4],
            ] {
                self.solid(rect, color);
            }
            return;
        }
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
        glow: Option<f32>,
    ) {
        let [a, b, c] = [va.position, vb.position, vc.position];
        let area = edge(a, b, c.0, c.1);
        if area.abs() < 0.01 || !area.is_finite() {
            return;
        }
        let modulated = face.flags & MODULATED != 0;
        if let Some(commands) = &mut self.commands {
            let dimensions = [self.width, self.height];
            if [a, b, c].iter().all(|p| p.0 < 0.0)
                || [a, b, c].iter().all(|p| p.1 < 0.0)
                || [a, b, c].iter().all(|p| p.0 > dimensions[0] as f32)
                || [a, b, c].iter().all(|p| p.1 > dimensions[1] as f32)
            {
                return;
            }
            let (id, [tw, th]) = commands.texture(texture);
            let mode = if face.flags & 0x80 != 0 {
                Mode::SkyMask
            } else if modulated {
                Mode::Modulated
            } else if glow.is_some() || face.flags & 4 != 0 {
                Mode::ScreenDepth
            } else {
                Mode::World
            };
            let mut bounds = [0.0; 4];
            let mut light_transform = [0.0; 4];
            let tint = if modulated {
                bounds[3] = -1.0;
                1.0
            } else if let Some(glow) = glow {
                bounds[3] = -1.0;
                glow
            } else if let Some((transform, limits)) = face
                .lightmap
                .as_deref()
                .and_then(|lm| commands.lighting(lm))
            {
                bounds = limits;
                light_transform = transform;
                1.0
            } else {
                light
            };
            let base = if texture.is_some() {
                [1.0; 3]
            } else {
                face.color.map(|c| c / 255.0)
            };
            let [r, g, b] = base.map(|c| c * tint);
            let mask = if face.flags & 2 != 0 { 0.25 } else { 0.0 };
            let vertices = [va, vb, vc].map(|v| {
                let z = v.position.2;
                let [du, dv, ou, ov] = light_transform;
                GpuVertex {
                    position: ndc(v.position, dimensions),
                    uv: [(v.u + pan_u * z) / tw, (v.v + pan_v * z) / th],
                    light_uv: [v.u * du + ou * z, v.v * dv + ov * z],
                    bounds,
                    color: [r, g, b, mask],
                }
            });
            commands.push(Material { texture: id, mode }, &vertices);
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
                            if modulated {
                                let dst = self.pixels[index];
                                let [r, g, b] =
                                    [16, 8, 0].map(|shift| ((dst >> shift) & 255) as f32);
                                self.pixels[index] =
                                    pack([r * color[0], g * color[1], b * color[2]], 2.0 / 255.0);
                            } else if let Some(glow) = glow {
                                self.pixels[index] =
                                    blend_light(pack(color, glow), self.pixels[index]);
                            } else {
                                self.depth[index] = inverse_depth;
                                let exposure =
                                    light * (0.62 + 0.38 * (inverse_depth * 680.0).min(1.0));
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

    fn focal_length(&self) -> f32 {
        focal_length(self.width, self.height) * self.zoom
    }
}

fn edge(a: Vec3, b: Vec3, x: f32, y: f32) -> f32 {
    (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)
}

fn pack(color: [f32; 3], light: f32) -> u32 {
    let [r, g, b] = color.map(|channel| (channel * light).clamp(0.0, 255.0) as u32);
    (r << 16) | (g << 8) | b
}

fn blend_light(source: u32, destination: u32) -> u32 {
    let channel = |shift: u32| -> u32 {
        let src = (source >> shift) & 255u32;
        let dst = (destination >> shift) & 255u32;
        src + dst * (255 - src) / 255
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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

fn focal_length(width: usize, height: usize) -> f32 {
    (width as f32).max(height as f32 * (16.0 / 9.0)) * 0.48
}

fn ndc(p: Vec3, [w, h]: [usize; 2]) -> [f32; 3] {
    [p.0 * 2.0 / w as f32 - 1.0, 1.0 - p.1 * 2.0 / h as f32, p.2]
}

fn quad(
    commands: &mut DrawList,
    dimensions: [usize; 2],
    [x, y, w, h]: [i32; 4],
    [u, v, uw, vh]: [f32; 4],
    color: [f32; 4],
    depth: Option<f32>,
    mode: Mode,
    texture: usize,
) {
    if w <= 0
        || h <= 0
        || x >= dimensions[0] as i32
        || y >= dimensions[1] as i32
        || x + w <= 0
        || y + h <= 0
    {
        return;
    }
    let z = depth.unwrap_or(0.25);
    let vertex = |x, y, u, v| GpuVertex {
        position: ndc(Vec3(x as f32, y as f32, z), dimensions),
        uv: [u * z, v * z],
        light_uv: [0.0; 2],
        bounds: [0.0, 0.0, 0.0, -1.0],
        color,
    };
    let a = vertex(x, y, u, v);
    let b = vertex(x + w, y, u + uw, v);
    let c = vertex(x + w, y + h, u + uw, v + vh);
    let d = vertex(x, y + h, u, v + vh);
    commands.push(Material { texture, mode }, &[a, b, c, a, c, d]);
}

fn effect_geometry(commands: &mut DrawList, (w, h): (usize, usize), a: Vec3, b: Vec3, color: u32) {
    let delta = b - a;
    let length = delta.0.hypot(delta.1);
    let color = [
        ((color >> 16) & 255) as f32 / 255.0,
        ((color >> 8) & 255) as f32 / 255.0,
        (color & 255) as f32 / 255.0,
        0.0,
    ];
    let pixel_scale = h as f32 / HEIGHT as f32;
    let corners = if length < 1.0 {
        let size = (b.2 * 960.0).clamp(3.0, 13.0) * pixel_scale;
        [
            b + Vec3(-size, 0.0, 0.0),
            b + Vec3(0.0, -size, 0.0),
            b + Vec3(size, 0.0, 0.0),
            b + Vec3(0.0, size, 0.0),
        ]
    } else {
        let cross = Vec3(-delta.1, delta.0, 0.0) * (pixel_scale.max(1.0) / length);
        [a - cross, a + cross, b + cross, b - cross]
    };
    let [a, b, c, d] = corners.map(|p| GpuVertex {
        position: ndc(p, [w, h]),
        uv: [0.0; 2],
        light_uv: [0.0; 2],
        bounds: [0.0, 0.0, 0.0, -1.0],
        color,
    });
    commands.push(
        Material {
            texture: commands::WHITE,
            mode: Mode::ScreenDepth,
        },
        &[a, b, c, a, c, d],
    );
}

fn project(vertex: Vertex, width: usize, height: usize, zoom: f32) -> Vertex {
    let p = vertex.position;
    let inverse_depth = p.2.recip();
    let scale = focal_length(width, height) * inverse_depth * zoom;
    Vertex {
        position: Vec3(
            width as f32 * 0.5 + p.0 * scale,
            height as f32 * 0.5 - p.1 * scale,
            inverse_depth,
        ),
        u: vertex.u * inverse_depth,
        v: vertex.v * inverse_depth,
    }
}
