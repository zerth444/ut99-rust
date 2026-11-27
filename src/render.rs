use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

use crate::game::{
    Weapon,
    deathmatch::Marker,
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
    pub(crate) fn following_death(map: &Map, body: Vec3, yaw: f32, time_since_death: f32) -> Self {
        let age = time_since_death.max(0.0);
        let angle = yaw + 0.8 + age.min(1.6) * 0.45;
        let target = body - Vec3(0.0, 0.0, 12.0 + age.min(1.0) * 12.0);
        let desired = Vec3(-angle.cos() * 118.0, -angle.sin() * 118.0, 54.0);
        let clear_position = |delta| {
            map.collision
                .ray(target, delta)
                .map_or(target + delta, |point| {
                    point - crate::mover::unit(delta) * 5.0
                })
        };
        let mut position = clear_position(desired);
        if (target - position).dot(target - position) < 50.0_f32.powi(2) {
            for delta in [
                Vec3(-desired.1, desired.0, desired.2),
                Vec3(desired.1, -desired.0, desired.2),
                Vec3(-desired.0, -desired.1, desired.2),
            ] {
                let candidate = clear_position(delta);
                if (target - candidate).dot(target - candidate)
                    > (target - position).dot(target - position)
                {
                    position = candidate;
                }
            }
        }
        let look = target - position;
        Self {
            position,
            yaw: look.1.atan2(look.0),
            pitch: look.2.atan2(look.0.hypot(look.1)),
        }
    }
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

struct WeaponLighting<'a> {
    camera: &'a Camera,
    lights: Vec<&'a crate::light::Light>,
    ambient: [f32; 3],
}

impl WeaponLighting<'_> {
    fn sample(&self, center: Vec3, normal: Vec3, time: f32) -> [f32; 3] {
        let axes = View::camera(self.camera);
        let world = |v: Vec3| axes.forward * v.0 + axes.right * v.1 + axes.up * v.2;
        let direct = crate::light::actor_lighting(
            self.lights.iter().copied(),
            self.camera.position + world(center) * (1.0 / 64.0),
            world(normal),
            time,
        );
        std::array::from_fn(|channel| (direct[channel] + self.ambient[channel]).min(1.8))
    }
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
        self.basis_model(origin, forward, right, up, scale)
    }

    fn basis_model(&self, origin: Vec3, forward: Vec3, right: Vec3, up: Vec3, scale: Vec3) -> Self {
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

    fn attach_muzzle(
        &self,
        map: &Map,
        muzzle: Option<(Weapon, Vec3)>,
        weapon: Weapon,
        point: Vec3,
        zoom: f32,
        age: Option<f32>,
    ) -> Vec3 {
        let Some((_, local)) = muzzle.filter(|&(held, local)| held == weapon && local.0 > NEAR)
        else {
            return point;
        };
        let weight = age.map_or(1.0, |age| (1.0 - age / 0.07).clamp(0.0, 1.0));
        let depth = self.point(point).2;
        if weight == 0.0 || !(NEAR..=160.0).contains(&depth) {
            return point;
        }
        let anchor = self.muzzle_at(local, depth, zoom);
        let corrected = point + (anchor - point) * weight;
        if map.collision.ray(self.eye, corrected - self.eye).is_some() {
            point
        } else {
            corrected
        }
    }

    fn muzzle_at(&self, local: Vec3, depth: f32, zoom: f32) -> Vec3 {
        // The local offset already includes the weapon projection. Keep its
        // screen ray under the visible muzzle at the world camera's FOV.
        let scale = depth / (local.0 * zoom);
        self.eye + self.forward * depth + (self.right * local.1 + self.up * local.2) * scale
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
    animated_lightmaps: HashMap<usize, CachedLightmap>,
}

struct CachedLightmap {
    gains: Vec<u8>,
    colors: Vec<[f32; 3]>,
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
            animated_lightmaps: HashMap::new(),
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
            animated_lightmaps: HashMap::new(),
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
        (flash, fog): (f32, Vec3),
    ) {
        // UE1 specifies the cinematic FOV across the viewport width.
        self.zoom = (self.width as f32 * 0.5 / focal_length(self.width, self.height))
            * (0.5 * fov.clamp(20.0, 150.0).to_radians()).tan().recip();
        self.roll = roll;
        self.draw(map, camera, time);
        self.roll = 0.0;
        self.zoom = 1.0;
        self.begin_ui(false);
        self.screen_flash(flash, fog);
    }

    fn screen_flash(&mut self, gain: f32, fog: Vec3) {
        let gain = gain.clamp(0.0, 1.0);
        if gain < 0.999 || fog.dot(fog) > 0.0 {
            if let Some(commands) = &mut self.commands {
                quad(
                    commands,
                    [self.width, self.height],
                    [0, 0, self.width as i32, self.height as i32],
                    [0.0, 0.0, 1.0, 1.0],
                    [fog.0, fog.1, fog.2, 1.0 - gain],
                    None,
                    Mode::Flash,
                    commands::WHITE,
                );
            } else {
                for pixel in &mut self.pixels {
                    let color = [16, 8, 0].map(|shift| ((*pixel >> shift) & 255) as f32);
                    *pixel = pack(
                        [
                            color[0] * gain + 255.0 * fog.0,
                            color[1] * gain + 255.0 * fog.1,
                            color[2] * gain + 255.0 * fog.2,
                        ],
                        1.0,
                    );
                }
            }
        }
    }

    pub fn draw(&mut self, map: &Map, camera: &Camera, time: f32) {
        self.draw_scene(map, camera, time, true);
        let view = self.camera_view(camera);
        self.draw_flags(
            map,
            &view,
            time,
            std::array::from_fn(|team| Some((map.flag_bases[team]?, map.flag_yaws[team], true))),
        );
        self.draw_control_points(map, &view, time, std::iter::empty());
    }

    fn draw_control_points(
        &mut self,
        map: &Map,
        view: &View,
        time: f32,
        owners: impl Iterator<Item = Option<usize>>,
    ) {
        for (point, owner) in map
            .control_points
            .iter()
            .zip(owners.chain(std::iter::repeat(None)))
        {
            if point.visible {
                let model = view.in_model(point.position, Vec3(1.0, 0.0, 0.0), 1.0);
                self.draw_lit_faces(
                    &map.objective_art.control[owner.unwrap_or(2)],
                    &model,
                    time,
                    false,
                    None,
                    None,
                    &map.scene_lights,
                    None,
                    None,
                );
            }
        }
    }

    fn draw_flags(
        &mut self,
        map: &Map,
        view: &View,
        time: f32,
        flags: [Option<(Vec3, f32, bool)>; 2],
    ) {
        self.layer(Layer::World);
        for (team, flag) in flags.into_iter().enumerate() {
            let Some((position, yaw, home)) = flag else {
                continue;
            };
            let motion = if home {
                &map.objective_art.base[team]
            } else {
                &map.objective_art.flag[team]
            };
            let Some(pose) = motion.sample_loop(time) else {
                continue;
            };
            let model = view.in_model(position, Vec3(yaw.cos(), yaw.sin(), 0.0), 1.0);
            self.draw_lit_faces(
                pose.current,
                &model,
                time,
                false,
                Some(pose),
                None,
                &map.scene_lights,
                None,
                None,
            );
        }
    }

    fn draw_scene(&mut self, map: &Map, camera: &Camera, time: f32, static_turrets: bool) {
        if let Some(commands) = &mut self.commands {
            commands.clear();
        }
        self.update_lighting(map, time);
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
            let view = self.mover_view(camera, mover);
            self.draw_lit_faces(
                &mover.faces,
                &view,
                time,
                false,
                None,
                None,
                &map.scene_lights,
                Some(mover),
                None,
            );
        }
        for prop in map.props.iter().filter(|prop| prop.visible) {
            let parent = prop.tag.and_then(|tag| {
                map.movers
                    .iter()
                    .find(|mover| mover.attach_tag == Some(tag))
            });
            let view = parent.map_or_else(
                || self.camera_view(camera),
                |mover| self.mover_view(camera, mover),
            );
            if view.point(prop.position).2 + prop.radius >= NEAR {
                let pose = prop.cinematic.as_ref().and_then(|actor| {
                    let time = time - actor.started?;
                    actor
                        .animation
                        .sample(time.min((actor.animation.duration() - 0.001).max(0.0)))
                });
                self.draw_lit_faces(
                    pose.map_or(prop.faces.as_slice(), |pose| pose.current),
                    &view,
                    time,
                    false,
                    pose,
                    None,
                    &map.scene_lights,
                    parent,
                    None,
                );
            }
        }
        let view = self.camera_view(camera);
        if static_turrets && let Some(assault) = &map.assault {
            for turret in &assault.turrets {
                self.draw_turret(
                    map,
                    &view,
                    time,
                    turret.volume.position,
                    Vec3(turret.yaw.cos(), turret.yaw.sin(), 0.0),
                    &turret.base,
                    turret.activate.sample(0.0),
                    0.0,
                );
            }
        }
        self.layer(Layer::Effects);
        for (position, texture, scale) in &map.objective_art.torch_sprites {
            self.world_sprite(&view, *position, texture.image(time), *scale, 1.0);
        }
        for (position, yaw) in crate::teleport::visuals(&map.teleporters) {
            let model = view.in_model(position, Vec3(yaw.cos(), yaw.sin(), 0.0), 1.0);
            for (layer, motion) in map.objective_art.portal.iter().enumerate() {
                if let Some(pose) = motion.sample_loop(time) {
                    let effect = view.in_model(position, Vec3(1.0, 0.0, 0.0), 1.0);
                    let model = if layer == 0 { &model } else { &effect };
                    self.draw_faces_in_view(
                        pose.current,
                        model,
                        time,
                        false,
                        Some(pose),
                        Some(1.0),
                    );
                }
            }
        }
        #[cfg(windows)]
        for (position, image, scale, glow) in
            map.smoke.sprites(time, &map.movers, &map.ballistics.smoke)
        {
            self.world_sprite(&view, position, image, scale, glow);
        }
        for corona in &map.coronas {
            let world = map
                .movers
                .iter()
                .find(|mover| {
                    corona.tag.is_some_and(|tag| mover.attach_tag == Some(tag))
                        || corona.attach_tag.is_some_and(|tag| mover.tag == Some(tag))
                })
                .map_or(corona.position, |mover| {
                    mover.transform_point(corona.position)
                });
            let position = view.point(world);
            if position.2 < NEAR
                || map
                    .collision
                    .ray(camera.position, world - camera.position)
                    .is_some_and(|hit| {
                        let gap = world - hit;
                        gap.dot(gap) > 16.0
                    })
            {
                continue;
            }
            let projected = project(
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
            let size = (0.8 * corona.scale * self.width as f32).round() as i32;
            let (w, h) = corona.texture.size();
            self.translucent_tile(
                corona.texture.image(time),
                [
                    projected.0.round() as i32 - size / 2,
                    projected.1.round() as i32 - size / 2,
                    size,
                    size,
                ],
                [0.0, 0.0, w, h],
                corona.color,
            );
        }
    }

    fn mover_view(&self, camera: &Camera, mover: &crate::mover::Mover) -> View {
        let mut view = self.camera_view(camera);
        view.eye = view.eye - mover.offset;
        if mover.is_rotated() {
            let [x, y, z] = [
                Vec3(1.0, 0.0, 0.0),
                Vec3(0.0, 1.0, 0.0),
                Vec3(0.0, 0.0, 1.0),
            ]
            .map(|axis| mover.transform_direction(axis));
            let inverse = |p: Vec3| Vec3(p.dot(x), p.dot(y), p.dot(z));
            view.eye = mover.pivot + inverse(view.eye - mover.pivot);
            view.forward = inverse(view.forward);
            view.up = inverse(view.up);
            view.right = inverse(view.right);
        }
        view
    }

    pub fn draw_deathmatch(
        &mut self,
        map: &Map,
        camera: &Camera,
        game: &crate::game::deathmatch::Match,
        fov: f32,
        body_blend: f32,
        scoreboard: bool,
    ) {
        let player = &game.fighters[0];
        let end_camera = game.assault_end_camera(map);
        let piloting = end_camera.is_none() && game.guidance_left().is_some();
        let dead = player.fell_at.is_some();
        let (roll, height) = game.view_shake(camera.position);
        let shaken = Camera {
            position: camera.position + Vec3(0.0, 0.0, height),
            ..*camera
        };
        let camera = if piloting || game.finished || dead || end_camera.is_some() {
            camera
        } else {
            &shaken
        };
        self.roll = if let Some(camera) = end_camera {
            camera.rotation.2
        } else if piloting {
            game.guided_roll()
        } else if game.finished || dead {
            0.0
        } else {
            roll
        };
        let fov = if player.zoomed() && !piloting && !game.finished && !dead && end_camera.is_none()
        {
            player.field_of_view(fov)
        } else {
            fov
        };
        self.zoom = (0.5 * fov.to_radians()).tan().recip();
        self.draw_scene(
            map,
            camera,
            game.time,
            game.mode() != crate::game::deathmatch::Mode::Assault,
        );
        self.layer(Layer::World);
        let view = self.camera_view(camera);
        let zoom = self.zoom;
        self.draw_teleport_bursts(map, &view, &game.teleports, game.time);
        let gun = (!piloting
            && !dead
            && !game.finished
            && end_camera.is_none()
            && !player.zoomed()
            && player.has_weapon())
        .then(|| map.arsenal.get(&player.weapon))
        .flatten()
        .map(|art| (art, player.weapon_pose(art, game.time)));
        let walk_bob = player.walker.walk_bob(game.weapon_bob);
        let muzzle = gun.and_then(|(art, pose)| {
            Some(self.projected_muzzle(
                player.weapon,
                art.muzzle_position(pose)? + weapon_bob(art, walk_bob),
            ))
        });
        let attach = |weapon, point, age| view.attach_muzzle(map, muzzle, weapon, point, zoom, age);
        self.draw_flags(map, &view, game.time, game.flag_models(map, body_blend));
        self.draw_control_points(map, &view, game.time, game.control_owners());
        for (position, direction, pose, launch) in game.translocator_modules(map) {
            let position = launch.map_or(position, |age| {
                attach(Weapon::Translocator, position, Some(age))
            });
            let model = view.in_model(position, direction, 1.0);
            self.draw_lit_faces(
                pose.current,
                &model,
                game.time,
                false,
                Some(pose),
                None,
                &map.scene_lights,
                None,
                None,
            );
        }
        for (barrel, offset) in game.training_barrels(map) {
            let model = View {
                eye: view.eye - offset,
                ..view
            };
            self.draw_faces_in_view(&barrel.faces, &model, game.time, false, None, None);
        }
        if let Some((position, pose)) = game.training_nali(map) {
            let model = view.in_model(position, Vec3(1.0, 0.0, 0.0), 1.0);
            self.draw_lit_faces(
                pose.current,
                &model,
                game.time,
                false,
                Some(pose),
                None,
                &map.scene_lights,
                None,
                None,
            );
        }
        for (origin, direction, base, pose, rise) in game.turret_models(map) {
            self.draw_turret(map, &view, game.time, origin, direction, base, pose, rise);
        }
        for drop in &game.drops {
            let template = &map.props[drop.prop];
            let model = View {
                eye: view.eye - (drop.position - template.position),
                ..view
            };
            if model.point(template.position).2 + template.radius >= NEAR {
                self.draw_lit_faces(
                    &template.faces,
                    &model,
                    game.time,
                    false,
                    None,
                    None,
                    &[],
                    None,
                    None,
                );
            }
        }
        self.layer(Layer::Effects);
        for (position, image, scale) in game.turret_projectiles(map) {
            self.world_sprite(&view, position, image, scale, 1.0);
        }
        for (position, image, scale) in game.turret_smoke(map) {
            self.world_sprite_mode(&view, position, image, scale, 1.0, Mode::Modulated);
        }
        for visual in game.assault_effects(map) {
            self.draw_world_effect(&view, game.time, visual);
        }
        for (origin, angles, scale, faces) in game
            .assault_debris(map)
            .chain(game.training_gibs(map))
            .chain(game.body_gibs(map))
        {
            self.draw_chunk(map, &view, game.time, origin, angles, scale, faces);
        }
        for (origin, direction, pose) in game.training_spurts(map).chain(game.body_spurts(map)) {
            let model = view.in_model(origin, direction, 1.0);
            self.draw_faces_in_view(pose.current, &model, game.time, false, Some(pose), None);
        }
        for (position, direction, faces) in game.turret_muzzles(map) {
            let model = view.in_model(position, direction, 1.0);
            self.draw_faces_in_view(faces, &model, game.time, false, None, Some(1.0));
        }
        game.rocket_trails
            .visuals(&map.ordnance, game.time, |visual| {
                self.draw_world_effect(&view, game.time, visual);
            });
        game.warhead_smoke
            .visuals(&map.ordnance, game.time, |visual| {
                self.draw_world_effect(&view, game.time, visual);
            });
        self.draw_shock(map, &view, &game.shock, game.time, |point, age| {
            attach(Weapon::Shock, point, age)
        });
        for &effect in &game.hammer_effects {
            self.effect(camera, effect);
        }
        for mut beam in game.pulse_beams(map, (!piloting && !dead).then_some(camera)) {
            if beam.local {
                beam.start = attach(Weapon::Pulse, beam.start, None);
            }
            self.draw_pulse_beam(map, &view, &beam, game.time);
        }
        if game.show_decals {
            for mark in game.decals.visible() {
                self.draw_faces_in_view(mark, &view, game.time, false, None, None);
            }
        }
        game.ballistics
            .visuals(&map.ballistics, game.time, |visual| {
                self.draw_world_effect(&view, game.time, visual);
            });
        for (id, fighter) in game.fighters.iter().enumerate().skip(usize::from(
            !piloting && !dead && game.result_camera(map).is_none(),
        )) {
            let animation_lag = (1.0 - body_blend) / 120.0;
            if let Some((pose, tween)) = fighter.rendered_pose(map, id, game.time, animation_lag) {
                let cloak = fighter.health > 0
                    && fighter.invis_left(game.time) > 0.0
                    && game.time - fighter.fired_at > 0.42;
                let skin = if cloak {
                    let Some(skin) = map.powerups.invis_skin.as_deref() else {
                        continue;
                    };
                    Some(skin)
                } else if fighter.health > 0 && fighter.amp_left(game.time) > 0.0 {
                    map.powerups.amp_skin.as_deref()
                } else {
                    None
                };
                let (position, yaw) = fighter.body_transform(body_blend);
                let model = view.in_model(position, Vec3(yaw.cos(), yaw.sin(), 0.0), 1.0);
                let palette = fighter.character(map).map(|(character, model)| {
                    (
                        model.slots.as_slice(),
                        &character.skins[fighter.team.unwrap_or(2)],
                    )
                });
                self.draw_model_faces(
                    pose.current,
                    &model,
                    game.time,
                    false,
                    Some(pose),
                    skin.map(|_| 1.0),
                    &[],
                    None,
                    skin,
                    palette,
                    None,
                    tween,
                );
            }
        }
        for trace in &game.traces {
            let art = &map.ordnance[trace.explosion as usize];
            let age = game.time - trace.born;
            if trace.draws_wave() {
                art.warhead_wave_visuals(trace.start, age, |visual| {
                    self.draw_world_effect(&view, game.time, visual);
                });
                continue;
            }
            if trace.explosion.warhead()
                || trace.explosion.bio()
                || matches!(
                    trace.explosion,
                    crate::game::ordnance::Kind::FlakShell
                        | crate::game::ordnance::Kind::Rocket
                        | crate::game::ordnance::Kind::Grenade
                )
            {
                let progress = age / (trace.expires - trace.born);
                if let Some(visual) = art.impact_sprite(trace.start, progress, trace.scale) {
                    self.draw_world_effect(&view, game.time, visual);
                    continue;
                }
            } else if let Some(sprite) = &art.explosion {
                self.world_sprite(
                    &view,
                    trace.start,
                    sprite.image(age),
                    (0.8 + age) * trace.scale,
                    1.0 - age,
                );
                continue;
            }
            self.effect(
                camera,
                crate::game::Effect {
                    origin: trace.start,
                    end: trace.start,
                    color: 0xf07130,
                    expires: game.time,
                },
            );
        }
        for (kind, mut position, direction, age, scale, surface_age, local) in game.bolts() {
            if local
                && surface_age.is_none()
                && let Some(weapon) = kind.launch_weapon()
            {
                position = attach(weapon, position, Some(age));
            }
            let art = &map.ordnance[kind as usize];
            let direction = if direction.dot(direction) > 0.01 {
                direction
            } else {
                Vec3(1.0, 0.0, 0.0)
            };
            let visual = if let Some(elapsed) = surface_age {
                art.bio_surface_visual(position, direction, elapsed, scale)
            } else {
                art.flight_visual(kind, position, direction, age, scale)
            };
            if let Some(visual) = visual {
                self.draw_world_effect(&view, game.time, visual);
                continue;
            }
            self.effect(
                camera,
                crate::game::Effect {
                    origin: position,
                    end: position,
                    color: 0xff9933,
                    expires: game.time,
                },
            );
        }
        if game.mode().team_based()
            && player.health > 0
            && !game.finished
            && !scoreboard
            && !piloting
        {
            self.layer(Layer::Hud);
            for (position, marker, team) in game.objective_markers(map) {
                if matches!(marker, Marker::Flag)
                    && map
                        .objective_art
                        .base
                        .iter()
                        .all(|motion| !motion.frames.is_empty())
                {
                    continue;
                }
                if matches!(marker, Marker::ControlPoint)
                    && !map.objective_art.control[team.unwrap_or(2)].is_empty()
                {
                    continue;
                }
                self.match_marker(map, &view, position + Vec3(0.0, 0.0, 34.0), team, marker);
            }
        }
        self.zoom = 1.0;
        self.roll = 0.0;
        if player.health > 0 && player.has_weapon() && !game.finished && end_camera.is_none() {
            if let Some((weapon, pose)) = gun {
                let skin = if player.amp_left(game.time) > 0.0 {
                    map.powerups.amp_skin.as_deref()
                } else if player.invis_left(game.time) > 0.0 {
                    map.powerups.invis_skin.as_deref()
                } else {
                    None
                };
                let muzzle = weapon
                    .flash
                    .as_ref()
                    .is_some_and(|flash| game.time - player.fired_at < flash.length)
                    .then_some(player.fired_count as usize);
                self.draw_weapon_view(
                    map,
                    camera,
                    player.weapon,
                    weapon,
                    pose,
                    game.time,
                    muzzle,
                    skin,
                    walk_bob,
                );
            }
        }
        self.layer(Layer::Hud);
        if game.finished || scoreboard || end_camera.is_some() {
            return;
        }
        if self.draw_guidance_hud(
            map,
            game.time,
            player.health > 0 && !game.finished,
            piloting,
            game.guidance_lost_until,
        ) {
            return;
        }
        self.draw_hit_tint(
            0.20 - (game.time - player.hurt_at),
            0.16 - (game.time - player.shield_at),
        );
        let hud = game.hud_state(map, game.hud_scale);
        map.hud.draw(self, &hud, &map.arsenal);
        if player.zoomed() && !piloting && player.health > 0 && !game.finished {
            self.draw_scope(map, player.weapon);
        } else if (hud.crosshair_visible && map.hud.crosshair.is_none()) || piloting {
            self.draw_crosshair();
        }
    }

    fn draw_teleport_bursts(
        &mut self,
        map: &Map,
        view: &View,
        bursts: &crate::teleport::Bursts,
        time: f32,
    ) {
        self.layer(Layer::Effects);
        let burst = &map.objective_art.teleport_burst;
        for (position, yaw, age) in bursts.visible(time) {
            let glow = 1.0 - age / crate::teleport::BURST_TIME;
            let swirl = &map.objective_art.portal[1];
            for (motion, point, angle, looping) in [
                (burst, position, yaw, false),
                (swirl, position, 0.0, true),
                (swirl, position - Vec3(0.0, 0.0, 10.0), yaw + 2.4, true),
            ] {
                let pose = if looping {
                    motion.sample_loop(age)
                } else {
                    motion.sample(age)
                };
                if let Some(pose) = pose {
                    let model = view.in_model(point, Vec3(angle.cos(), angle.sin(), 0.0), 1.0);
                    self.draw_faces_in_view(
                        pose.current,
                        &model,
                        time,
                        false,
                        Some(pose),
                        Some(glow),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_chunk(
        &mut self,
        map: &Map,
        view: &View,
        time: f32,
        origin: Vec3,
        angles: Vec3,
        scale: f32,
        faces: &[Face],
    ) {
        if view.point(origin).2 + 24.0 * scale < NEAR {
            return;
        }
        let axis = |vector| crate::mover::rotate(vector, angles);
        let model = view.basis_model(
            origin,
            axis(Vec3(1.0, 0.0, 0.0)),
            axis(Vec3(0.0, 1.0, 0.0)),
            axis(Vec3(0.0, 0.0, 1.0)),
            Vec3(scale, scale, scale),
        );
        self.draw_lit_faces(
            faces,
            &model,
            time,
            false,
            None,
            None,
            &map.scene_lights,
            None,
            None,
        );
    }

    fn projected_muzzle(&self, weapon: Weapon, Vec3(x, y, z): Vec3) -> (Weapon, Vec3) {
        let zoom = weapon_view_zoom(weapon, self.width, self.height);
        (weapon, Vec3(x, y * zoom, z * zoom))
    }

    fn weapon_ambient(&self, map: &Map, eye: Vec3) -> [f32; 3] {
        let ambient = map
            .collision
            .ray_surface(eye, Vec3(0.0, 0.0, -128.0))
            .and_then(|(floor, normal)| {
                map.faces.iter().find_map(|face| {
                    let lightmap = face.lightmap.as_ref()?;
                    let offset = floor - face.origin;
                    if face.sky
                        || face.normal.dot(normal) < 0.95
                        || offset.dot(face.normal).abs() > 1.0
                    {
                        return None;
                    }
                    let mut signs = [false; 2];
                    for (a, b) in face
                        .vertices
                        .iter()
                        .zip(face.vertices.iter().cycle().skip(1))
                    {
                        let side = (*b - *a).cross(floor - *a).dot(face.normal);
                        signs[0] |= side > 0.01;
                        signs[1] |= side < -0.01;
                    }
                    if signs == [true, true] {
                        return None;
                    }
                    let animated = self
                        .animated_lightmaps
                        .get(&(std::sync::Arc::as_ptr(lightmap) as usize))
                        .map(|cached| cached.colors.as_slice());
                    Some(lightmap.sample(
                        offset.dot(face.u_axis) + face.pan[0],
                        offset.dot(face.v_axis) + face.pan[1],
                        animated,
                    ))
                })
            });
        ambient.map_or([0.20; 3], |sample| {
            sample.map(|gain| (gain * 0.35).clamp(0.16, 0.50))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_weapon_view(
        &mut self,
        map: &Map,
        camera: &Camera,
        kind: Weapon,
        weapon: &crate::game::WeaponAssets,
        pose: crate::game::Pose<'_>,
        time: f32,
        flash_index: Option<usize>,
        skin: Option<&crate::texture::Texture>,
        walk_bob: Vec3,
    ) {
        self.zoom = weapon_view_zoom(kind, self.width, self.height);
        let view = Camera {
            position: weapon_bob(weapon, walk_bob) * -1.0,
            yaw: 0.0,
            pitch: 0.0,
        };
        if let Some(index) = flash_index
            && let Some(flash) = &weapon.flash
        {
            let texture = &flash.textures[index % flash.textures.len()];
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
                Mode::Screen,
                None,
            );
        }
        self.depth.fill(0.0);
        self.layer(Layer::Weapon);
        let lighting = WeaponLighting {
            camera,
            lights: map
                .scene_lights
                .iter()
                .filter(|light| light.reaches(camera.position))
                .collect(),
            ambient: self.weapon_ambient(map, camera.position),
        };
        self.draw_model_faces(
            pose.current,
            &View::camera(&view),
            time,
            false,
            Some(pose),
            skin.map(|_| 1.0),
            &[],
            None,
            skin,
            None,
            Some(&lighting),
            None,
        );
        self.zoom = 1.0;
    }

    pub fn draw_practice(
        &mut self,
        map: &Map,
        camera: &Camera,
        time: f32,
        game: &crate::game::Practice,
    ) {
        let death_camera = game.death.as_ref().map(|death| {
            Camera::following_death(map, death.position, death.yaw, game.time - death.started)
        });
        let guided_camera = game.ordnance.guided_view();
        let (shake_roll, shake_height) = game.ordnance.nuclear_shake(camera.position, game.time);
        let shaken = Camera {
            position: camera.position + Vec3(0.0, 0.0, shake_height),
            yaw: camera.yaw,
            pitch: camera.pitch,
        };
        let camera = death_camera
            .as_ref()
            .or(guided_camera.as_ref())
            .unwrap_or(&shaken);
        self.roll = if guided_camera.is_some() {
            game.ordnance.guided_roll
        } else {
            shake_roll
        };
        self.zoom = (0.5 * game.field_of_view().to_radians()).tan().recip();
        self.draw(map, camera, time);
        self.layer(Layer::Effects);
        let view = self.camera_view(camera);
        let zoom = self.zoom;
        self.draw_teleport_bursts(map, &view, &game.teleports, game.time);
        let gun = (guided_camera.is_none() && game.death.is_none() && !game.scoped())
            .then(|| map.arsenal.get(&game.weapon))
            .flatten()
            .map(|art| (art, game.pose(art)));
        let muzzle = gun.and_then(|(art, pose)| {
            Some(self.projected_muzzle(
                game.weapon,
                art.muzzle_position(pose)? + weapon_bob(art, game.walk_bob),
            ))
        });
        let attach = |weapon, point, age| view.attach_muzzle(map, muzzle, weapon, point, zoom, age);
        if game.show_decals {
            for mark in game.decals.visible() {
                self.draw_faces_in_view(mark, &view, time, false, None, None);
            }
        }
        for (position, rotation, scale, faces) in game.body_gibs(map) {
            self.draw_chunk(map, &view, time, position, rotation, scale, faces);
        }
        for (position, direction, pose) in game.body_spurts(map) {
            let model = view.in_model(position, direction, 1.0);
            self.draw_faces_in_view(pose.current, &model, time, false, Some(pose), None);
        }
        if let Some(target) = &game.target
            && !target.gibbed
        {
            let motion = if target.health == 0 {
                &map.player_deaths[0]
            } else {
                &map.player_idle
            };
            let age = if target.health == 0 {
                target
                    .fell_at
                    .map_or(0.0, |at| time - at)
                    .min((motion.duration() - 0.001).max(0.0))
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
        let mut draw_effect = |visual| self.draw_world_effect(&view, time, visual);
        game.ordnance.visuals(
            &map.ordnance,
            game.time,
            |kind, point, age| {
                kind.launch_weapon()
                    .map_or(point, |weapon| attach(weapon, point, Some(age)))
            },
            &mut draw_effect,
        );
        game.ballistics
            .visuals(&map.ballistics, game.time, &mut draw_effect);
        if let Some(death) = &game.death
            && !death.gibbed
        {
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
        self.draw_shock(map, &view, &game.shock, time, |point, age| {
            attach(Weapon::Shock, point, age)
        });
        if let Some(beam) = &game.pulse_beam {
            let mut beam = *beam;
            beam.start = attach(Weapon::Pulse, beam.start, None);
            self.draw_pulse_beam(map, &view, &beam, time);
        }
        self.zoom = 1.0;
        self.roll = 0.0;
        if let Some((weapon, pose)) = gun {
            let skin = if game.damage_amp_active() {
                map.powerups.amp_skin.as_deref()
            } else if game.invisible() {
                map.powerups.invis_skin.as_deref()
            } else {
                None
            };
            self.draw_weapon_view(
                map,
                camera,
                game.weapon,
                weapon,
                pose,
                time,
                (game.muzzle > game.time).then_some(game.shots as usize),
                skin,
                game.walk_bob,
            );
        }
        self.layer(Layer::Hud);
        if self.draw_guidance_hud(
            map,
            game.time,
            game.death.is_none(),
            guided_camera.is_some(),
            game.guidance_lost_until,
        ) {
            return;
        }
        self.draw_hit_tint(
            game.hit_flash_until - game.time,
            game.shield_flash_until - game.time,
        );
        map.hud.draw(
            self,
            &crate::hud_art::State::practice(game, &map.arsenal),
            &map.arsenal,
        );
        if game.death.is_some() {
            return;
        } else if game.scoped() {
            self.draw_scope(map, game.weapon);
        } else if map.hud.crosshair.is_none() {
            self.draw_crosshair();
        }
    }

    fn draw_shock(
        &mut self,
        map: &Map,
        view: &View,
        shots: &shock::Shots,
        time: f32,
        attach: impl Fn(Vec3, Option<f32>) -> Vec3,
    ) {
        if let Some(assets) = shock::assets(map) {
            shots.visuals(assets, time, attach, |visual| match visual {
                Visual::Sprite {
                    position,
                    image,
                    scale,
                    glow,
                } => self.world_sprite(view, position, image, scale, glow),
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
    }

    fn draw_pulse_beam(
        &mut self,
        map: &Map,
        view: &View,
        beam: &crate::game::PulseBeam,
        time: f32,
    ) {
        if !beam.blocked
            && let Some(art) = map.arsenal.get(&crate::game::Weapon::Pulse)
        {
            let delta = beam.end - beam.start;
            let length = delta.dot(delta).sqrt();
            if length > 0.0 {
                let forward = delta * length.recip();

                let age = time - beam.started;
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
                    self.world_sprite(view, beam.end - forward * inset, tip.image(age), 0.65, 1.0);
                }
            }
        }
    }

    fn draw_guidance_hud(
        &mut self,
        map: &Map,
        time: f32,
        alive: bool,
        guided: bool,
        lost_until: f32,
    ) -> bool {
        if let Some(weapon) = map.arsenal.get(&crate::game::Weapon::Redeemer) {
            let [reticle, readout, noise] = &weapon.guided_hud;
            if guided {
                self.screen_flash(0.8, Vec3(200.0 / 1000.0, 0.0, 0.0));
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
                    let offset = (time * 75.0) as i32 % h as i32;
                    for y in (0..self.height as i32 + h as i32).step_by(h as usize) {
                        self.translucent_tile(
                            readout.image(0.0),
                            [1, y - offset, w as i32, h as i32],
                            [0.0, 0.0, w, h],
                            [1.0; 3],
                        );
                    }
                }
                return true;
            }
            if alive
                && time < lost_until
                && let Some(noise) = noise
            {
                let (w, h) = noise.size();
                let scale = self.width.max(self.height) as f32 / 256.0;
                self.tile(
                    noise.image_once(1.0 - (lost_until - time) / 0.3),
                    [0, 0, (w * scale) as i32, (h * scale) as i32],
                    [0.0, 0.0, w, h],
                    [1.0; 3],
                );
                return true;
            }
        }
        false
    }

    fn draw_hit_tint(&mut self, red_left: f32, shield_left: f32) {
        let red = (red_left / 0.20).clamp(0.0, 1.0) * 0.22;
        let shield = (shield_left / 0.16).clamp(0.0, 1.0) * 0.18;
        if red > 0.0 || shield > 0.0 {
            let tint = [red.max(shield), shield, shield * 0.6];
            if let Some(commands) = &mut self.commands {
                quad(
                    commands,
                    [self.width, self.height],
                    [0, 0, self.width as i32, self.height as i32],
                    [0.0, 0.0, 1.0, 1.0],
                    [tint[0], tint[1], tint[2], 0.0],
                    None,
                    Mode::Screen,
                    commands::WHITE,
                );
            } else {
                let tint = pack(tint, 255.0);
                for pixel in &mut self.pixels {
                    *pixel = blend_light(tint, *pixel);
                }
            }
        }
    }

    fn draw_scope(&mut self, map: &Map, weapon: crate::game::Weapon) {
        let Some(scope) = map
            .arsenal
            .get(&weapon)
            .and_then(|weapon| weapon.scope.as_ref())
        else {
            self.draw_crosshair();
            return;
        };
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
    }

    fn draw_turret(
        &mut self,
        map: &Map,
        view: &View,
        time: f32,
        origin: Vec3,
        direction: Vec3,
        base: &[Face],
        pose: Option<crate::game::Pose<'_>>,
        rise: f32,
    ) {
        let model = view.in_model(origin, direction, 1.0);
        self.draw_lit_faces(
            base,
            &model,
            time,
            false,
            None,
            None,
            &map.scene_lights,
            None,
            None,
        );
        if let Some(pose) = pose {
            let model = view.in_model(origin + Vec3(0.0, 0.0, rise), direction, 1.0);
            self.draw_lit_faces(
                pose.current,
                &model,
                time,
                false,
                Some(pose),
                None,
                &map.scene_lights,
                None,
                None,
            );
        }
    }

    fn draw_world_effect(
        &mut self,
        view: &View,
        time: f32,
        visual: crate::game::ordnance::Visual<'_>,
    ) {
        match visual {
            crate::game::ordnance::Visual::Sprite(position, image, scale, glow) => {
                self.world_sprite(view, position, image, scale, glow)
            }
            crate::game::ordnance::Visual::Mesh(position, direction, pose, scale, glow) => {
                let model = view.in_model(position, direction, scale);
                self.draw_faces_in_view(pose.current, &model, time, false, Some(pose), glow);
            }
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
        self.world_sprite_mode(view, position, image, scale, glow, Mode::ScreenDepth);
    }

    fn world_sprite_mode(
        &mut self,
        view: &View,
        position: Vec3,
        image: &TextureImage,
        scale: f32,
        glow: f32,
        mode: Mode,
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
            mode,
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
        self.blit_tile(texture, rect, source, tint, Mode::Ui, None);
    }

    pub fn translucent_tile(
        &mut self,
        texture: &TextureImage,
        rect: [i32; 4],
        source: [f32; 4],
        tint: [f32; 3],
    ) {
        self.blit_tile(texture, rect, source, tint, Mode::Screen, None);
    }

    #[cfg(windows)]
    pub(crate) fn ink(&mut self, texture: &TextureImage, rect: [i32; 4], source: [f32; 4]) {
        let [x, y, w, h] = rect;
        if w <= 0 || h <= 0 {
            return;
        }
        let [u, v, uw, vh] = source;
        if let Some(commands) = &mut self.commands {
            let (id, [tw, th]) = commands.texture(Some(texture));
            quad(
                commands,
                [self.width, self.height],
                rect,
                [u / tw, v / th, uw / tw, vh / th],
                [0.0; 4],
                None,
                Mode::Ink,
                id,
            );
            return;
        }
        for py in y.max(0)..(y + h).min(self.height as i32) {
            for px in x.max(0)..(x + w).min(self.width as i32) {
                let u = u + ((px - x) as f32 + 0.5) / w as f32 * uw;
                let v = v + ((py - y) as f32 + 0.5) / h as f32 * vh;
                if let Some([coverage, ..]) = texture.sample(u, v, true) {
                    let pixel = &mut self.pixels[py as usize * self.width + px as usize];
                    let color = [16, 8, 0].map(|shift| ((*pixel >> shift) & 255) as f32);
                    *pixel = pack(color, 1.0 - coverage / 255.0);
                }
            }
        }
    }

    fn match_marker(
        &mut self,
        map: &Map,
        view: &View,
        position: Vec3,
        team: Option<usize>,
        marker: Marker,
    ) {
        let local = view.point(position);
        if local.2 < 75.0 || map.collision.ray(view.eye, position - view.eye).is_some() {
            return;
        }
        let Vec3(x, y, _) = project(
            Vertex {
                position: local,
                u: 0.0,
                v: 0.0,
            },
            self.width,
            self.height,
            self.zoom,
        )
        .position;
        if x < 8.0 || x >= self.width as f32 - 8.0 || y < 8.0 || y >= self.height as f32 - 8.0 {
            return;
        }
        let (x, y) = (x as i32, y as i32);
        let color = match team {
            Some(0) => [1.0, 0.29, 0.27],
            Some(_) => [0.40, 0.66, 1.0],
            None => [0.89, 0.89, 0.91],
        };
        match marker {
            Marker::Flag => {
                self.solid([x - 5, y - 5, 2, 13], color);
                self.solid([x - 3, y - 5, 10, 6], color);
            }
            Marker::ControlPoint => {
                self.solid([x - 5, y - 3, 11, 7], color);
                self.solid([x - 3, y - 5, 7, 11], color);
            }
        }
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

    #[cfg(windows)]
    pub(crate) fn shade(&mut self, rect: [i32; 4], color: [f32; 3], alpha: f32) {
        let color = color.map(|channel| channel * alpha);
        if let Some(commands) = &mut self.commands {
            quad(
                commands,
                [self.width, self.height],
                rect,
                [0.0, 0.0, 1.0, 1.0],
                [color[0], color[1], color[2], alpha],
                None,
                Mode::Flash,
                commands::WHITE,
            );
        } else {
            let [x, y, width, height] = rect;
            for py in y.max(0)..(y + height).min(self.height as i32) {
                for px in x.max(0)..(x + width).min(self.width as i32) {
                    let pixel = &mut self.pixels[py as usize * self.width + px as usize];
                    let under = [16, 8, 0].map(|shift| ((*pixel >> shift) & 255) as f32);
                    *pixel = pack(
                        std::array::from_fn(|i| under[i] * (1.0 - alpha) + 255.0 * color[i]),
                        1.0,
                    );
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
        mode: Mode,
        inverse_depth: Option<f32>,
    ) {
        let [x, y, w, h] = rect;
        let [u, v, uw, vh] = source;
        if w <= 0 || h <= 0 {
            return;
        }
        if let Some(commands) = &mut self.commands {
            let (id, [tw, th]) = commands.texture(Some(texture));
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
        for py in y.max(0)..(y + h).min(self.height as i32) {
            for px in x.max(0)..(x + w).min(self.width as i32) {
                let index = py as usize * self.width + px as usize;
                if inverse_depth.is_some_and(|depth| depth <= self.depth[index]) {
                    continue;
                }
                let u = u + (px - x) as f32 / w as f32 * uw;
                let v = v + (py - y) as f32 / h as f32 * vh;
                if let Some([r, g, b]) = texture.sample(u, v, true) {
                    let c = |value: f32, gain: f32| (value * gain).clamp(0.0, 255.0) as u32;
                    let color = (c(r, tint[0]) << 16) | (c(g, tint[1]) << 8) | c(b, tint[2]);
                    self.pixels[index] = match mode {
                        Mode::Screen | Mode::ScreenDepth => blend_light(color, self.pixels[index]),
                        Mode::Modulated => {
                            let destination = self.pixels[index];
                            let channel = |shift: u32| {
                                let source = (color >> shift) & 255;
                                let background = (destination >> shift) & 255;
                                ((2 * source * background / 255).min(255)) << shift
                            };
                            channel(16) | channel(8) | channel(0)
                        }
                        _ => color,
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
        self.draw_lit_faces(faces, view, time, sky, pose, glow, &[], None, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_lit_faces(
        &mut self,
        faces: &[Face],
        view: &View,
        time: f32,
        sky: bool,
        pose: Option<crate::game::Pose<'_>>,
        glow: Option<f32>,
        lights: &[crate::light::Light],
        mover: Option<&crate::mover::Mover>,
        skin: Option<&crate::texture::Texture>,
    ) {
        self.draw_model_faces(
            faces, view, time, sky, pose, glow, lights, mover, skin, None, None, None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_model_faces(
        &mut self,
        faces: &[Face],
        view: &View,
        time: f32,
        sky: bool,
        pose: Option<crate::game::Pose<'_>>,
        glow: Option<f32>,
        lights: &[crate::light::Light],
        mover: Option<&crate::mover::Mover>,
        skin: Option<&crate::texture::Texture>,
        palette: Option<(&[usize], &crate::props::characters::Skin)>,
        weapon_lighting: Option<&WeaponLighting<'_>>,
        actor_tween: Option<(crate::game::Pose<'_>, f32)>,
    ) {
        let mut transformed = Vec::new();
        let mut clipped = Vec::new();
        let vertex_key = |Vec3(x, y, z): Vec3| [x.to_bits(), y.to_bits(), z.to_bits()];
        let skin_normals = (skin.is_some() || faces.iter().any(|face| face.flags & 0x10 != 0))
            .then(|| {
                let mut normals = HashMap::<_, Vec3>::new();
                for face in faces {
                    for &vertex in &face.vertices {
                        let sum = normals.entry(vertex_key(vertex)).or_default();
                        *sum = *sum + face.normal;
                    }
                }
                normals
            });

        for face in faces.iter().filter(|face| face.sky == sky) {
            let assigned = palette.and_then(|(slots, textures)| {
                textures.get(*slots.get(face.mesh_triangle?)?)?.as_deref()
            });
            let texture = skin.or(assigned).or(face.texture.as_deref());
            let flags = face.flags | texture.map_or(0, |texture| texture.poly_flags);
            if flags & 1 != 0 || (sky && flags & 0x80 != 0) {
                continue;
            }
            let pose = pose
                .filter(|pose| pose.fraction > 0.0)
                .map(|pose| {
                    pose_face(pose.next, face.mesh_triangle)
                        .map_or((face, face, 0.0), |next| (face, next, pose.fraction))
                })
                .unwrap_or((face, face, 0.0));
            let previous = actor_tween.and_then(|(previous, blend)| {
                let current = pose_face(previous.current, face.mesh_triangle)?;
                let next = pose_face(previous.next, face.mesh_triangle).unwrap_or(current);
                Some(((current, next, previous.fraction), blend))
            });
            let position =
                |i: usize, _: Vec3| tween_face(pose, previous, |frame| frame.vertices[i]);
            let normal = tween_face(pose, previous, |frame| frame.normal);
            if flags & 0x100 == 0 && (view.eye - position(0, face.vertices[0])).dot(normal) < 0.0 {
                continue;
            }
            let uv_scale = assigned
                .filter(|_| skin.is_none())
                .map_or((1.0, 1.0), |texture| {
                    let (w, h) = texture.size();
                    let (base_w, base_h) = face
                        .texture
                        .as_ref()
                        .map_or((256.0, 256.0), |base| base.size());
                    (w / base_w, h / base_h)
                });
            transformed.clear();
            transformed.extend(face.vertices.iter().enumerate().map(|(i, &vertex)| {
                let point = position(i, vertex);
                let (u, v) = if let Some(texture) = texture
                    && let Some(normals) = &skin_normals
                    && (skin.is_some() || flags & 0x10 != 0)
                {
                    let (width, height) = texture.size();
                    let incident = crate::mover::unit(point - view.eye);
                    let normal = crate::mover::unit(
                        normals.get(&vertex_key(vertex)).copied().unwrap_or(normal),
                    );
                    let reflected = incident - normal * (2.0 * incident.dot(normal));
                    (
                        (0.5 + 0.5 * reflected.dot(view.right)) * width,
                        (0.5 + 0.5 * reflected.dot(view.up)) * height,
                    )
                } else {
                    let texture_point = vertex - face.origin;
                    (
                        texture_point.dot(face.u_axis) * uv_scale.0,
                        texture_point.dot(face.v_axis) * uv_scale.1,
                    )
                };
                Vertex {
                    position: view.point(point),
                    u,
                    v,
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
            let light = if flags & 0x0040_0000 != 0 {
                [1.0; 3]
            } else if let Some(lighting) = weapon_lighting {
                let center = face
                    .vertices
                    .iter()
                    .copied()
                    .fold(Vec3::default(), |sum, vertex| sum + vertex)
                    * (1.0 / face.vertices.len() as f32)
                    - view.eye;
                lighting.sample(center, normal, time)
            } else if lights.is_empty() {
                [0.93 + normal.dot(Vec3(-0.34, 0.43, 0.69)) * 0.32; 3]
            } else {
                let center = face
                    .vertices
                    .iter()
                    .enumerate()
                    .fold(Vec3::default(), |sum, (index, &vertex)| {
                        sum + position(index, vertex)
                    })
                    * (1.0 / face.vertices.len() as f32);
                let (center, normal) = mover.map_or((center, normal), |mover| {
                    (
                        mover.transform_point(center),
                        mover.transform_direction(normal),
                    )
                });
                crate::light::actor_lighting(lights, center, normal, time)
            };
            let u_pan = if flags & 0x200 != 0 { time * 64.0 } else { 0.0 };
            let v_pan = if flags & 0x400 != 0 { time * 64.0 } else { 0.0 };
            let pan = if skin.is_some() {
                [0.0; 2]
            } else {
                [
                    face.pan[0] * uv_scale.0 + u_pan,
                    face.pan[1] * uv_scale.1 + v_pan,
                ]
            };
            let texture = texture.map(|texture| texture.image(time));
            let a = project(clipped[0], self.width, self.height, self.zoom);
            for pair in clipped[1..].windows(2) {
                self.triangle(
                    a,
                    project(pair[0], self.width, self.height, self.zoom),
                    project(pair[1], self.width, self.height, self.zoom),
                    face,
                    flags,
                    (light, pan, texture),
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
        flags: u32,
        (light, [pan_u, pan_v], texture): ([f32; 3], [f32; 2], Option<&TextureImage>),
        glow: Option<f32>,
    ) {
        let [a, b, c] = [va.position, vb.position, vc.position];
        let area = edge(a, b, c.0, c.1);
        if area.abs() < 0.01 || !area.is_finite() {
            return;
        }
        let colors = face.lightmap.as_deref().and_then(|lightmap| {
            self.animated_lightmaps
                .get(&(lightmap as *const _ as usize))
                .map(|cached| cached.colors.as_slice())
        });
        let modulated = flags & MODULATED != 0;
        let translucent = flags & 4 != 0;
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
            let mode = if flags & 0x80 != 0 {
                Mode::SkyMask
            } else if modulated {
                Mode::Modulated
            } else if glow.is_some() || translucent {
                Mode::ScreenDepth
            } else {
                Mode::World
            };
            let mut bounds = [0.0; 4];
            let mut light_transform = [0.0; 4];
            let tint = if modulated {
                bounds[3] = -1.0;
                [1.0; 3]
            } else if let Some(glow) = glow {
                bounds[3] = -1.0;
                [glow; 3]
            } else if let Some((transform, limits)) = face
                .lightmap
                .as_deref()
                .and_then(|lm| commands.lighting(lm))
            {
                bounds = limits;
                light_transform = transform;
                [1.0; 3]
            } else {
                light
            };
            let base = if texture.is_some() {
                [1.0; 3]
            } else {
                face.color.map(|c| c / 255.0)
            };
            let [r, g, b] = std::array::from_fn(|i| base[i] * tint[i]);
            let mask = if flags & 2 != 0 { 0.25 } else { 0.0 };
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
                    if inverse_depth > self.depth[index] && flags & 0x80 != 0 {
                        self.depth[index] = inverse_depth;
                        self.pixels[index] = self.sky[index];
                    } else if inverse_depth > self.depth[index] {
                        let u = (w0 * va.u + w1 * vb.u + w2 * vc.u) / inverse_depth;
                        let v = (w0 * va.v + w1 * vb.v + w2 * vc.v) / inverse_depth;
                        let color = if let Some(texture) = texture {
                            texture.sample(u + pan_u, v + pan_v, flags & 2 != 0)
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
                            } else if let Some(glow) = glow.or(translucent.then_some(1.0)) {
                                self.pixels[index] =
                                    blend_light(pack(color, glow), self.pixels[index]);
                            } else {
                                self.depth[index] = inverse_depth;
                                let exposure = if flags & 0x0040_0000 != 0 {
                                    1.0
                                } else {
                                    0.62 + 0.38 * (inverse_depth * 680.0).min(1.0)
                                };
                                let color = if let Some(lightmap) = &face.lightmap {
                                    let lighting = lightmap.sample(u, v, colors);
                                    std::array::from_fn(|i| color[i] * lighting[i])
                                } else {
                                    std::array::from_fn(|i| color[i] * light[i] * exposure)
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

    fn update_lighting(&mut self, map: &Map, time: f32) {
        if map.animated_lightmaps.is_empty() {
            self.animated_lightmaps.clear();
            return;
        }
        let gains: HashMap<_, _> = map
            .scene_lights
            .iter()
            .filter_map(|light| Some((light.id, light.gain(time)?)))
            .collect();
        for lightmap in &map.animated_lightmaps {
            let id = std::sync::Arc::as_ptr(lightmap) as usize;
            let levels = || {
                lightmap
                    .animated_ids()
                    .map(|id| gains.get(&id).copied().unwrap_or(0))
            };
            if self
                .animated_lightmaps
                .get(&id)
                .is_some_and(|cached| levels().eq(cached.gains.iter().copied()))
            {
                continue;
            }
            let gains: Vec<_> = levels().collect();
            let colors = lightmap.combine(&gains);
            if let Some(commands) = &mut self.commands {
                commands.update_lightmap(lightmap, &colors);
            }
            self.animated_lightmaps
                .insert(id, CachedLightmap { gains, colors });
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

type FacePose<'a> = (&'a Face, &'a Face, f32);

fn tween_face(
    pose: FacePose<'_>,
    previous: Option<(FacePose<'_>, f32)>,
    sample: impl Fn(&Face) -> Vec3,
) -> Vec3 {
    let interpolate = |(a, b, fraction): FacePose<'_>| {
        let start = sample(a);
        start + (sample(b) - start) * fraction
    };
    let target = interpolate(pose);
    previous.map_or(target, |(from, fraction)| {
        let origin = interpolate(from);
        origin + (target - origin) * fraction
    })
}

fn pose_face(faces: &[Face], id: Option<usize>) -> Option<&Face> {
    let id = id?;
    let index = faces
        .binary_search_by_key(&Some(id), |face| face.mesh_triangle)
        .ok()?;
    faces.get(index)
}

fn weapon_bob(weapon: &crate::game::WeaponAssets, walk: Vec3) -> Vec3 {
    let damping = weapon.bob_damping.unwrap_or(0.96);
    // Preserve the relative Weapon.uc animation on the viewmodel once the
    // world camera's WalkBob is disabled. The viewmodel uses 64x UE1 units.
    Vec3(
        0.0,
        (damping - 1.0) * walk.1,
        (0.45 + 0.55 * damping - 1.0) * walk.2,
    ) * 64.0
}

fn weapon_view_zoom(weapon: Weapon, width: usize, height: usize) -> f32 {
    let vertical_focal = match weapon {
        Weapon::Flak => height as f32 * 0.52,
        Weapon::Translocator => height as f32 * 0.62,
        Weapon::Bio | Weapon::Rocket => height as f32 * (2.0 / 3.0),
        Weapon::Hammer => height as f32 * 0.80,
        // Match the 16:9 weapon view on wider displays.
        _ => focal_length(0, height),
    };
    0.84 * vertical_focal / focal_length(width, height)
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
    let bounds = [
        0.0,
        0.0,
        0.0,
        match mode {
            Mode::Flash => -2.0,
            Mode::Ink => -3.0,
            _ => -1.0,
        },
    ];
    let vertex = |x, y, u, v| GpuVertex {
        position: ndc(Vec3(x as f32, y as f32, z), dimensions),
        uv: [u * z, v * z],
        light_uv: [0.0; 2],
        bounds,
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

#[cfg(test)]
mod muzzle_tests {
    use super::*;
    use crate::game::deathmatch::{Match, Mode, Rules};
    use crate::package::Package;

    #[test]
    fn weapon_sway_keeps_world_muzzle_on_the_drawn_barrel() {
        let frame = Frame::new();
        let art = crate::game::WeaponAssets {
            bob_damping: Some(0.975),
            ..Default::default()
        };
        let camera = Camera {
            position: Vec3(20.0, -16.0, 70.0),
            yaw: 1.0,
            pitch: 0.3,
        };
        let world = View::camera(&camera);
        let muzzle = Vec3(225.0, 36.0, -24.0);
        let walk = Vec3(0.0, 3.5, -4.0);
        let offset = weapon_bob(&art, walk);
        assert!(offset.1 != 0.0 && offset.2 != 0.0);
        let kind = Weapon::Shock;
        let world_zoom = 0.92;
        let (_, projected) = frame.projected_muzzle(kind, muzzle + offset);
        let weapon = View::camera(&Camera {
            position: offset * -1.0,
            yaw: 0.0,
            pitch: 0.0,
        });
        let on_screen = |point, zoom| {
            project(
                Vertex {
                    position: point,
                    u: 0.0,
                    v: 0.0,
                },
                frame.width,
                frame.height,
                zoom,
            )
            .position
        };
        let model = on_screen(
            weapon.point(muzzle),
            weapon_view_zoom(kind, frame.width, frame.height),
        );
        let effect = on_screen(
            world.point(world.muzzle_at(projected, 64.0, world_zoom)),
            world_zoom,
        );
        assert!((model.0 - effect.0).abs() < 0.01 && (model.1 - effect.1).abs() < 0.01);
    }

    #[test]
    #[ignore = "requires the reference-client installation"]
    fn stock_weapon_muzzles_remain_in_the_viewport() {
        let path = Path::new("local/reference-client/Maps/CTF-Face.unr");
        let bytes = std::fs::read(path).unwrap();
        let map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        for weapon in Weapon::ALL {
            let art = &map.arsenal[&weapon];
            let rest = crate::game::Pose {
                current: &art.view,
                next: &art.view,
                fraction: 0.0,
            };
            for pose in [
                Some(rest),
                art.primary_motion.sample(0.0),
                art.alternate_motion.sample(0.0),
            ]
            .into_iter()
            .flatten()
            {
                let Vec3(x, y, z) = art.muzzle_position(pose).expect("stock weapon muzzle");
                for (width, height) in [(800, 600), (1920, 1080), (3440, 1440)] {
                    let point = project(
                        Vertex {
                            position: Vec3(y, z, x),
                            u: 0.0,
                            v: 0.0,
                        },
                        width,
                        height,
                        weapon_view_zoom(weapon, width, height),
                    )
                    .position;
                    assert!(
                        (0.0..width as f32).contains(&point.0)
                            && (0.0..height as f32).contains(&point.1),
                        "{weapon:?} muzzle at {point:?} in {width}x{height}",
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires the reference-client game and maps"]
    fn installed_modes_share_the_animated_muzzle_projection() {
        let maps = Path::new("local/reference-client/Maps");
        let mut paths = std::fs::read_dir(maps)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "unr"))
            .collect::<Vec<_>>();
        let reference = maps.join("CTF-Face.unr");
        let mut frame = Frame::new();
        paths.sort_by_key(|path| {
            let name = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            (path != &reference, !name.starts_with("ctf-"), name)
        });
        let mut configurations = 0;
        for path in paths {
            let name = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            let modes = Mode::ALL
                .into_iter()
                .filter(|mode| name.starts_with(mode.map_prefix()))
                .collect::<Vec<_>>();
            if modes.is_empty() {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let map = Map::load(&Package::read(&bytes).unwrap(), Some(&path)).unwrap();
            let camera = Camera {
                position: Vec3(0.0, 0.0, 25_000.0),
                yaw: 1.4,
                pitch: 0.3,
            };
            let view = View::camera(&camera);
            for mode in modes {
                let mut game = Match::new(
                    &map,
                    0,
                    Rules {
                        mode,
                        bots: 0,
                        ..Rules::default()
                    },
                );
                for weapon in Weapon::ALL {
                    let player = &mut game.fighters[0];
                    player.weapon = weapon;
                    let art = &map.arsenal[&weapon];
                    let idle = player.weapon_pose(art, game.time);
                    let poses = [
                        Some(idle),
                        art.primary_motion.sample(0.0),
                        art.alternate_motion.sample(0.0),
                    ];
                    for pose in poses.into_iter().flatten() {
                        let muzzle = art.muzzle_position(pose).unwrap_or_else(|| {
                            panic!("{name} {mode:?} {weapon:?} has no animated muzzle")
                        });
                        for (width, height) in [(800, 600), (1920, 1080), (2560, 1080)] {
                            (frame.width, frame.height) = (width, height);
                            let (_, offset) = frame.projected_muzzle(weapon, muzzle);
                            let local = project(
                                Vertex {
                                    position: Vec3(muzzle.1, muzzle.2, muzzle.0),
                                    u: 0.0,
                                    v: 0.0,
                                },
                                width,
                                height,
                                weapon_view_zoom(weapon, width, height),
                            )
                            .position;
                            for fov in [80.0_f32, 95.0, 110.0] {
                                let zoom = (0.5 * fov.to_radians()).tan().recip();
                                let visible = view.muzzle_at(offset, 20.0, zoom);
                                let p = project(
                                    Vertex {
                                        position: view.point(visible),
                                        u: 0.0,
                                        v: 0.0,
                                    },
                                    width,
                                    height,
                                    zoom,
                                )
                                .position;
                                assert!(
                                    (p.0 - local.0).abs() < 0.3 && (p.1 - local.1).abs() < 0.3,
                                    "{name} {mode:?} {weapon:?} muzzle mismatch at {width}x{height}"
                                );
                            }
                        }
                    }
                }
                configurations += 1;
            }
        }
        assert!(configurations >= 141);
        eprintln!(
            "Verified all weapon muzzle projections in {configurations} map/mode combinations"
        );
    }
}
