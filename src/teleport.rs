use std::io;

use crate::collision::Walker;
use crate::map::{Map, Vec3};
use crate::mover::read_rotation;
use crate::package::{Export, Package};
use crate::sound::{CONTACT_SOUND_RADIUS, SoundEvents};

pub struct Teleporter {
    pub position: Vec3,
    radius: f32,
    height: f32,
    yaw: f32,
    set_yaw: bool,
    set_velocity: bool,
    velocity: Vec3,
    reverse: [bool; 3],
    enabled: bool,
    visible: bool,
    tag: String,
    url: String,
    target: Option<usize>,
}

pub fn load(package: &Package<'_>, actors: &[&Export]) -> io::Result<Vec<Teleporter>> {
    let mut portals = Vec::new();
    for actor in actors {
        let class = package.object_name(actor.class).unwrap_or_default();
        if !is_teleporter(class) {
            continue;
        }
        let mut portal = Teleporter {
            position: Vec3::default(),
            radius: 20.0,
            height: 40.0,
            yaw: 0.0,
            set_yaw: false,
            set_velocity: false,
            velocity: Vec3::default(),
            reverse: [false; 3],
            enabled: true,
            visible: class == "VisibleTeleporter",
            tag: class.to_owned(),
            url: String::new(),
            target: None,
        };
        let mut props = package.properties(actor)?;
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "Location" => portal.position = Vec3::read(&mut value)?,
                "Rotation" => portal.yaw = read_rotation(&mut value)?.1,
                "CollisionRadius" => portal.radius = value.f32()?.max(0.0),
                "CollisionHeight" => portal.height = value.f32()?.max(0.0),
                "bChangesYaw" => portal.set_yaw = prop.boolean.unwrap_or(false),
                "bChangesVelocity" => portal.set_velocity = prop.boolean.unwrap_or(false),
                "TargetVelocity" => portal.velocity = Vec3::read(&mut value)?,
                "bReversesX" => portal.reverse[0] = prop.boolean.unwrap_or(false),
                "bReversesY" => portal.reverse[1] = prop.boolean.unwrap_or(false),
                "bReversesZ" => portal.reverse[2] = prop.boolean.unwrap_or(false),
                "bEnabled" => portal.enabled = prop.boolean.unwrap_or(true),
                "Tag" => portal.tag = package.name(value.index()?)?.to_owned(),
                "URL" => portal.url = value.string(package.version)?,
                _ => {}
            }
        }
        portals.push(portal);
    }
    for index in 0..portals.len() {
        let portal = &portals[index];
        if !portal.enabled || portal.url.is_empty() || portal.url.contains(['/', '#', ':']) {
            continue;
        }
        portals[index].target = portals.iter().enumerate().find_map(|(target, other)| {
            (target != index && other.tag.eq_ignore_ascii_case(&portal.url)).then_some(target)
        });
    }
    Ok(portals)
}

pub(crate) fn is_teleporter(class: &str) -> bool {
    matches!(class, "Teleporter" | "VisibleTeleporter" | "UTTeleporter")
}

fn overlaps(portal: &Teleporter, position: Vec3) -> bool {
    let offset = portal.position - position;
    let radius = portal.radius + 17.0;
    offset.0 * offset.0 + offset.1 * offset.1 <= radius * radius
        && offset.2.abs() <= portal.height + 39.0
}

pub(crate) fn clear_radius(
    portals: &[Teleporter],
    occupied: Option<usize>,
    entry: Vec3,
) -> Option<f32> {
    let portal = portals.get(occupied?)?;
    let delta = portal.position - entry;
    (delta.dot(delta) < 1.0).then_some(portal.radius + crate::collision::PLAYER_RADIUS)
}

pub struct Arrival<'a> {
    source: &'a Teleporter,
    destination: &'a Teleporter,
}

impl Arrival<'_> {
    pub fn apply(
        self,
        map: &Map,
        walker: &mut Walker,
        yaw: &mut f32,
        local: bool,
        sounds: &mut SoundEvents,
    ) -> bool {
        let world = &map.collision;
        let destination = self.destination;
        let arrival = Walker::spawn(world, destination.position);
        if !world.player_fits(arrival.position) {
            return false;
        }
        let old = *yaw;
        if destination.set_yaw {
            *yaw = (destination.yaw + std::f32::consts::PI + old - self.source.yaw)
                .rem_euclid(std::f32::consts::TAU);
        }
        let mut velocity = walker.velocity();
        if destination.set_velocity {
            velocity = destination.velocity;
        } else {
            if destination.set_yaw {
                let before = Vec3(old.cos(), old.sin(), 0.0);
                let after = Vec3(yaw.cos(), yaw.sin(), 0.0);
                velocity = velocity + (after - before) * velocity.dot(before);
            }
            let reverse = destination
                .reverse
                .map(|axis| if axis { -1.0 } else { 1.0 });
            velocity = Vec3(
                velocity.0 * reverse[0],
                velocity.1 * reverse[1],
                velocity.2 * reverse[2],
            );
        }
        *walker = arrival;
        walker.set_teleport_velocity(velocity);
        let sound = map.objective_art.teleport_sound.iter().cloned();
        if local {
            sounds.extend(sound);
        } else {
            sounds.extend_at(sound, walker.position, CONTACT_SOUND_RADIUS, 1.0);
        }
        true
    }
}

pub fn touched<'a>(
    portals: &'a [Teleporter],
    position: Vec3,
    occupied: &mut Option<usize>,
) -> Option<Arrival<'a>> {
    if let Some(previous) = occupied.take()
        && overlaps(&portals[previous], position)
    {
        *occupied = Some(previous);
        return None;
    }
    for portal in portals {
        let Some(target) = portal.target else {
            continue;
        };
        if overlaps(portal, position) {
            *occupied = Some(target);
            let destination = &portals[target];
            return Some(Arrival {
                source: portal,
                destination,
            });
        }
    }
    None
}

pub fn link_count(portals: &[Teleporter]) -> usize {
    portals
        .iter()
        .filter(|portal| portal.target.is_some())
        .count()
}

pub(crate) fn links(portals: &[Teleporter]) -> impl Iterator<Item = (Vec3, Vec3)> + '_ {
    portals.iter().filter_map(|portal| {
        let destination = portals.get(portal.target?)?;
        Some((portal.position, destination.position))
    })
}

pub(crate) fn visuals(portals: &[Teleporter]) -> impl Iterator<Item = (Vec3, f32)> + '_ {
    portals
        .iter()
        .filter(|portal| portal.visible)
        .map(|portal| (portal.position, portal.yaw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Map;

    #[test]
    #[ignore = "requires the local GOTY maps"]
    fn facing_worlds_teleports_to_its_tagged_destination_without_bouncing() {
        let data = std::fs::read("local/goty/Maps/CTF-Face.unr").unwrap();
        let package = Package::read(&data).unwrap();
        let map = Map::load(&package, None).unwrap();
        assert_eq!(link_count(&map.teleporters), 12);
        let source = map
            .teleporters
            .iter()
            .find(|portal| portal.target.is_some())
            .unwrap();
        let target = source.target.unwrap();
        let expected = &map.teleporters[target];
        let mut occupied = None;
        let position = touched(&map.teleporters, source.position, &mut occupied)
            .unwrap()
            .destination
            .position;
        let offset = position - expected.position;
        assert!(offset.dot(offset) < 0.01 && occupied == Some(target));
        assert!(touched(&map.teleporters, position, &mut occupied).is_none());
    }

    #[test]
    #[ignore = "requires stock CTF map packages"]
    fn facing_worlds_loads_flag_skins_and_the_sequel_loads_animated_portals() {
        use std::path::Path;
        for (name, visual_count) in [("CTF-Face.unr", 0), ("CTF-Face][.unr", 8)] {
            let path = Path::new("local/reference-client/Maps").join(name);
            let data = std::fs::read(&path).unwrap();
            let package = Package::read(&data).unwrap();
            let map = Map::load(&package, Some(&path)).unwrap();
            for animations in [&map.objective_art.base, &map.objective_art.flag] {
                let textures = animations.each_ref().map(|motion| {
                    assert!(motion.frames.len() > 1, "{name}");
                    motion.frames[0]
                        .iter()
                        .find_map(|face| face.texture.as_ref())
                        .expect("flag skin")
                });
                assert!(!std::sync::Arc::ptr_eq(textures[0], textures[1]), "{name}");
            }
            assert_eq!(visuals(&map.teleporters).count(), visual_count);
            if visual_count > 0 {
                assert!(
                    map.objective_art
                        .portal
                        .iter()
                        .any(|motion| motion.frames.len() > 1
                            && motion.frames[0].iter().any(|face| face.texture.is_some()))
                );
            }
            let audio = map
                .objective_art
                .teleport_sound
                .as_ref()
                .expect("Botpack Resp2A");
            for (index, (position, destination)) in links(&map.teleporters).enumerate() {
                assert_eq!(map.collision.regions.at(position).gravity.2, -950.0);
                let mut walker = Walker::spawn(&map.collision, position);
                let mut yaw = 0.25;
                let mut occupied = None;
                let arrival =
                    touched(&map.teleporters, position, &mut occupied).expect("stock link");
                let mut sounds = SoundEvents::default();
                let local = index % 2 == 0;
                assert!(
                    arrival.apply(&map, &mut walker, &mut yaw, local, &mut sounds),
                    "{name} {position:?} -> {destination:?}"
                );
                let delta = walker.position - destination;
                assert!(delta.dot(delta) < 28.0_f32.powi(2));
                assert_eq!(sounds.len(), 1);
                assert!(
                    sounds
                        .waves()
                        .all(|wave| std::sync::Arc::ptr_eq(audio, wave))
                );
                if local {
                    assert!(sounds.position_of(audio).is_none());
                } else {
                    let offset = sounds.position_of(audio).unwrap() - walker.position;
                    assert!(offset.dot(offset) < 0.01);
                }
                assert!(touched(&map.teleporters, walker.position, &mut occupied).is_none());
            }
        }
    }

    #[test]
    #[ignore = "requires stock CTF map packages"]
    fn facing_worlds_draws_its_authored_portal_sprites() {
        use crate::render::{Camera, Frame};
        let path = std::path::Path::new("local/reference-client/Maps/CTF-Face.unr");
        let data = std::fs::read(path).unwrap();
        let package = Package::read(&data).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        assert_eq!(map.objective_art.torch_sprites.len(), 12);
        for (position, _) in links(&map.teleporters) {
            assert!(
                map.objective_art
                    .torch_sprites
                    .iter()
                    .any(|&(sprite, _, _)| {
                        let offset = sprite - position;
                        offset.dot(offset) < 36.0_f32.powi(2)
                    })
            );
        }
        let (target, texture, _) = &map.objective_art.torch_sprites[0];
        assert_ne!(texture.image(0.0).bgra(), texture.image(0.19).bgra());
        let camera = [180.0, 125.0, 90.0]
            .into_iter()
            .find_map(|distance| {
                (0..24).find_map(|direction| {
                    let yaw = direction as f32 * std::f32::consts::TAU / 24.0;
                    let position = *target - Vec3(yaw.cos() * distance, yaw.sin() * distance, -9.0);
                    (map.collision.player_fits(position - Vec3(0.0, 0.0, 21.0))
                        && map.collision.ray(position, *target - position).is_none())
                    .then_some(Camera {
                        position,
                        yaw,
                        pitch: (-9.0_f32).atan2(distance),
                    })
                })
            })
            .expect("visible approach to the portal");
        let mut with_portal = Frame::new();
        with_portal.draw(&map, &camera, 0.3);
        let sprite_vertices = |map: &Map| {
            let mut frame = Frame::hardware(map);
            frame.draw(map, &camera, 0.3);
            frame.commands.as_ref().unwrap().passes[2]
                .batches()
                .filter(|(material, _)| material.mode == crate::render::commands::Mode::ScreenDepth)
                .map(|(_, vertices)| vertices.len())
                .sum::<usize>()
        };
        let gpu_with = sprite_vertices(&map);
        map.objective_art.torch_sprites.clear();
        let mut without = Frame::new();
        without.draw(&map, &camera, 0.3);
        assert_ne!(with_portal.pixels, without.pixels);
        assert!(gpu_with > sprite_vertices(&map));
    }
}
