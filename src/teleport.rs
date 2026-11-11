use std::io;

use crate::map::Vec3;
use crate::mover::read_rotation;
use crate::package::{Export, Package};

pub struct Teleporter {
    pub position: Vec3,
    radius: f32,
    height: f32,
    yaw: f32,
    set_yaw: bool,
    enabled: bool,
    tag: String,
    url: String,
    target: Option<usize>,
}

pub fn load(package: &Package<'_>, actors: &[&Export]) -> io::Result<Vec<Teleporter>> {
    let mut portals = Vec::new();
    for actor in actors {
        let class = package.object_name(actor.class).unwrap_or_default();
        if !matches!(class, "Teleporter" | "VisibleTeleporter" | "UTTeleporter") {
            continue;
        }
        let mut portal = Teleporter {
            position: Vec3::default(),
            radius: 20.0,
            height: 40.0,
            yaw: 0.0,
            set_yaw: false,
            enabled: true,
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

fn overlaps(portal: &Teleporter, position: Vec3) -> bool {
    let offset = portal.position - position;
    let radius = portal.radius + 17.0;
    offset.0 * offset.0 + offset.1 * offset.1 <= radius * radius
        && offset.2.abs() <= portal.height + 39.0
}

pub fn touched(
    portals: &[Teleporter],
    position: Vec3,
    occupied: &mut Option<usize>,
) -> Option<(Vec3, Option<f32>)> {
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
            return Some((
                destination.position,
                destination.set_yaw.then_some(destination.yaw),
            ));
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
        let (position, _) = touched(&map.teleporters, source.position, &mut occupied).unwrap();
        let offset = position - expected.position;
        assert!(offset.dot(offset) < 0.01 && occupied == Some(target));
        assert!(touched(&map.teleporters, position, &mut occupied).is_none());
    }
}
