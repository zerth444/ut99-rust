use crate::collision::{Collision, Plane};
use crate::light::{LightIndex, Lightmap, read_lights};
use crate::texture::{Texture, load_materials};
use std::ops::{Add, Mul, Sub};
use std::{io, path::Path, sync::Arc};

use crate::package::{Export, Package, Reader, invalid};

#[derive(Clone, Copy, Debug, Default)]
pub struct Vec3(pub f32, pub f32, pub f32);

impl Vec3 {
    pub fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self(reader.f32()?, reader.f32()?, reader.f32()?))
    }

    pub fn cross(self, other: Self) -> Self {
        Self(
            self.1 * other.2 - self.2 * other.1,
            self.2 * other.0 - self.0 * other.2,
            self.0 * other.1 - self.1 * other.0,
        )
    }

    pub fn dot(self, other: Self) -> f32 {
        self.0 * other.0 + self.1 * other.1 + self.2 * other.2
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0, self.1 + rhs.1, self.2 + rhs.2)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0, self.1 - rhs.1, self.2 - rhs.2)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs, self.1 * rhs, self.2 * rhs)
    }
}

pub struct Face {
    pub vertices: Vec<Vec3>,
    pub mesh_triangle: Option<usize>,
    pub normal: Vec3,
    pub color: [f32; 3],
    pub flags: u32,
    pub sky: bool,
    pub texture: Option<Arc<Texture>>,
    pub lightmap: Option<Arc<Lightmap>>,
    pub origin: Vec3,
    pub u_axis: Vec3,
    pub v_axis: Vec3,
    pub pan: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Start {
    pub position: Vec3,
    pub yaw: f32,
    pub enabled: bool,
}

pub(crate) struct LiftExit {
    pub position: Vec3,
    pub tag: usize,
}

pub struct Map {
    pub title: String,
    pub author: Option<String>,
    pub scene_lights: Vec<crate::light::Light>,
    pub(crate) animated_lightmaps: Vec<Arc<Lightmap>>,
    pub coronas: Vec<crate::light::Corona>,
    #[cfg(windows)]
    pub(crate) smoke: crate::frontend::smoke::Smoke,
    pub faces: Vec<Face>,
    pub starts: Vec<Start>,
    pub navigation_points: Vec<Vec3>,
    pub(crate) lift_exits: Vec<LiftExit>,
    pub collision: Collision,
    pub sky_origin: Option<Vec3>,
    pub ambient: Vec<crate::sound::Emitter>,
    pub music: Option<crate::music::Track>,
    pub movers: Vec<crate::mover::Mover>,
    pub props: Vec<crate::props::Prop>,
    pub hud: crate::hud_art::Art,
    pub arsenal: std::collections::HashMap<crate::game::Weapon, crate::game::WeaponAssets>,
    pub player_audio: crate::movement::PlayerAudio,
    pub match_announcer: [Option<Arc<crate::sound::Wave>>; 5],
    pub player_deaths: [crate::game::Motion; 3],
    pub player_idle: crate::game::Motion,
    pub player_clips: crate::game::ActorClips,
    pub ordnance: crate::game::ordnance::Arsenal,
    pub ballistics: crate::game::ballistics::Art,
    pub powerups: crate::game::PowerupArt,
    pub liquid_audio: Vec<[Option<Arc<crate::sound::Wave>>; 2]>,
    pub teleporters: Vec<crate::teleport::Teleporter>,
}

impl Map {
    pub fn load(package: &Package<'_>, path: Option<&Path>) -> io::Result<Self> {
        if package.version <= 61 {
            return Err(invalid("BSP preview requires package version 62 or later"));
        }
        let level = package
            .exports
            .iter()
            .find(|item| package.object_name(item.class) == Some("Level"))
            .ok_or_else(|| invalid("Package has no Level export"))?;
        let (model, actors) =
            level_data(package, level).map_err(|error| invalid(format!("Level: {error}")))?;
        let city_intro = path.filter(|path| {
            path.file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("CityIntro"))
        });
        let (mut faces, mut collision, sky_origin, zone_actors) =
            read_bsp(package, model, &actors, path, city_intro.is_some())
                .map_err(|error| invalid(format!("Model: {error}")))?;
        if faces.is_empty() {
            return Err(invalid("Level has no visible BSP faces"));
        }
        // Start with larger faces in the depth buffer when the scene has no spatial culling.
        faces.sort_by_key(|face| std::cmp::Reverse(face.vertices.len()));

        let ambient = path
            .map(|path| crate::sound::load(package, &actors, path))
            .transpose()?
            .unwrap_or_default();
        let music = path.and_then(|path| crate::music::load(package, &actors, path));
        let movers = crate::mover::Mover::load(package, &actors, path, &mut collision)?;
        let crate::props::Scene {
            props,
            arsenal,
            hud,
            player,
            player_audio,
            match_announcer,
            player_deaths,
            player_idle,
            player_clips,
            ordnance,
            ballistics,
            zones,
            powerups,
            liquid_audio,
        } = path
            .map(|path| {
                crate::props::load(
                    package,
                    &actors,
                    path,
                    &zone_actors,
                    collision.regions.level,
                )
            })
            .transpose()?
            .unwrap_or_default();
        collision.regions.zones = if zones.is_empty() {
            crate::movement::load_zones(
                package,
                &zone_actors,
                collision.regions.level,
                |actor, zone, properties| {
                    let mut zone =
                        crate::movement::stock_liquid_zone(package.object_name(actor.class), zone);
                    zone.apply(properties)?;
                    Ok(zone)
                },
            )?
        } else {
            zones
        };
        collision.profile = player;
        let teleporters = crate::teleport::load(package, &actors)?;
        let mut starts = Vec::new();
        let mut navigation_points = Vec::new();
        let mut lift_exits = Vec::new();
        for actor in actors {
            let class = package.object_name(actor.class);
            if matches!(
                class,
                Some("PathNode" | "LiftExit" | "InventorySpot" | "NavigationPoint")
            ) {
                let mut properties = package.properties(actor)?;
                let mut position = None;
                let mut tag = None;
                while let Some((name, mut value)) = properties.next()? {
                    match name {
                        "Location" => position = Some(Vec3::read(&mut value)?),
                        "LiftTag" if class == Some("LiftExit") => tag = Some(value.index()?),
                        _ => {}
                    }
                }
                if let Some(position) = position {
                    navigation_points.push(position);
                    if let Some(tag) = tag {
                        lift_exits.push(LiftExit { position, tag });
                    }
                }
            }
            if class != Some("PlayerStart") {
                continue;
            }
            let mut properties = package.properties(actor)?;
            let mut start = Start {
                position: Vec3::default(),
                yaw: 0.0,
                enabled: true,
            };
            while let Some(property) = properties.next_tag()? {
                let mut value = property.value;
                match property.name {
                    "Location" => start.position = Vec3::read(&mut value)?,
                    "Rotation" => {
                        value.u32()?;
                        start.yaw = (value.u32()? as i32) as f32 * std::f32::consts::TAU / 65_536.0;
                    }
                    "bEnabled" => start.enabled = property.boolean.unwrap_or(true),
                    _ => {}
                }
            }
            starts.push(start);
        }
        if starts.is_empty() {
            return Err(invalid("Level has no PlayerStart for the preview camera"));
        }
        let mut seen = std::collections::HashSet::new();
        let animated_lightmaps = faces
            .iter()
            .filter_map(|face| face.lightmap.as_ref())
            .filter(|map| map.animated_ids().next().is_some() && seen.insert(Arc::as_ptr(map)))
            .cloned()
            .collect();
        let mut title = path
            .and_then(Path::file_stem)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut author = None;
        if let Some(level) = package
            .exports
            .iter()
            .find(|actor| package.object_name(actor.class) == Some("LevelInfo"))
        {
            let mut properties = package.properties(level)?;
            while let Some((name, mut value)) = properties.next()? {
                if matches!(name, "Title" | "Author")
                    && let Ok(value) = value.string(package.version)
                    && !value.trim().is_empty()
                {
                    if name == "Title" {
                        title = value.trim().to_owned();
                    } else {
                        author = Some(value.trim().to_owned());
                    }
                }
            }
        }
        Ok(Self {
            title,
            author,
            animated_lightmaps,
            #[cfg(windows)]
            smoke: city_intro
                .map(|path| crate::frontend::smoke::Smoke::load(package, path, &movers))
                .transpose()?
                .unwrap_or_default(),
            coronas: city_intro
                .map(|path| crate::light::read_coronas(package, path))
                .transpose()?
                .unwrap_or_default(),
            scene_lights: if city_intro.is_some() {
                let refs: Vec<_> = package
                    .exports
                    .iter()
                    .enumerate()
                    .filter(|(_, actor)| {
                        matches!(
                            package.object_name(actor.class),
                            Some("Light" | "TriggerLight" | "Spotlight")
                        )
                    })
                    .map(|(index, _)| index as i32 + 1)
                    .collect();
                read_lights(package, &refs)?.into_values().collect()
            } else {
                Vec::new()
            },
            faces,
            starts,
            navigation_points,
            lift_exits,
            collision,
            sky_origin,
            ambient,
            music,
            movers,
            props,
            arsenal,
            hud,
            player_audio,
            match_announcer,
            player_deaths,
            player_idle,
            player_clips,
            ordnance,
            ballistics,
            powerups,
            liquid_audio,
            teleporters,
        })
    }
}

pub(crate) fn native_data<'a>(package: &Package<'a>, export: &Export) -> io::Result<Reader<'a>> {
    let mut properties = package.properties(export)?;
    while properties.next()?.is_some() {}
    Ok(properties.reader)
}

fn level_data<'p, 'a>(
    package: &'p Package<'a>,
    export: &Export,
) -> io::Result<(&'p Export, Vec<&'p Export>)> {
    let mut reader = native_data(package, export)?;
    let count = reader.u32()? as usize;
    reader.u32()?;
    if count > reader.remaining.len() {
        return Err(invalid("Level actor table exceeds export"));
    }
    let mut actors = Vec::with_capacity(count);
    for _ in 0..count {
        let reference = reader.index()?;
        if let Some(index) = reference.checked_sub(1) {
            actors.push(
                package
                    .exports
                    .get(index)
                    .ok_or_else(|| invalid("Invalid Level actor"))?,
            );
        }
    }
    for _ in 0..4 {
        skip_string(&mut reader)?;
    }
    let count = reader.count(1)?;
    for _ in 0..count {
        skip_string(&mut reader)?;
    }
    reader.take(8)?;
    let model = reader
        .index()?
        .checked_sub(1)
        .and_then(|index| package.exports.get(index))
        .filter(|item| package.object_name(item.class) == Some("Model"))
        .ok_or_else(|| invalid("Level does not reference a BSP Model"))?;
    Ok((model, actors))
}

fn skip_string(reader: &mut Reader<'_>) -> io::Result<()> {
    let length = reader.compact()?;
    let width = if length < 0 { 2 } else { 1 };
    reader.take(length.unsigned_abs() as usize * width)?;
    Ok(())
}

struct Node {
    first_vertex: i32,
    surface: i32,
    vertices: usize,
    plane: Plane,
    collision_bound: i32,
    zones: [i32; 2],
    children: [i32; 2],
}

struct Surface {
    material: i32,
    lightmap: i32,
    origin: i32,
    u_axis: i32,
    v_axis: i32,
    pan: [f32; 2],
    normal: usize,
    flags: u32,
    color: [f32; 3],
}

struct Model<'a> {
    vectors: Vec<Vec3>,
    points: Vec<Vec3>,
    nodes: Vec<Node>,
    surfaces: Vec<Surface>,
    pool: Vec<i32>,
    zones: Vec<i32>,
    polys: i32,
    reader: Reader<'a>,
}

fn read_model<'a>(package: &Package<'a>, export: &Export) -> io::Result<Model<'a>> {
    let mut reader = native_data(package, export)?;
    reader.take(41)?; // UPrimitive bounds (FBox and FSphere).
    let vectors = read_vectors(&mut reader)?;
    let points = read_vectors(&mut reader)?;
    let mut nodes = Vec::new();
    for _ in 0..reader.count(43)? {
        let plane = Plane {
            normal: Vec3::read(&mut reader)?,
            distance: reader.f32()?,
        };
        reader.take(9)?; // Zone mask and node flags.
        let first_vertex = reader.compact()?;
        let surface = reader.compact()?;
        let children = [reader.compact()?, reader.compact()?];
        reader.compact()?; // Coplanar node.
        let collision_bound = reader.compact()?;
        reader.compact()?; // Render bound.
        let zones = [reader.compact()?, reader.compact()?];
        let vertices = reader.byte()? as usize;
        reader.take(8)?; // Leaves on both sides of the plane.
        nodes.push(Node {
            first_vertex,
            plane,
            collision_bound,
            children,
            zones,
            surface,
            vertices,
        });
    }
    let mut surfaces = Vec::new();
    for _ in 0..reader.count(16)? {
        let material = reader.compact()?;
        let flags = reader.u32()?;
        let origin = reader.compact()?;
        let normal = reader.index()?;
        let u_axis = reader.compact()?;
        let v_axis = reader.compact()?;
        let lightmap = reader.compact()?;
        reader.compact()?; // Brush polygon.
        let pan = [reader.u16()? as i16 as f32, reader.u16()? as i16 as f32];
        reader.compact()?; // Brush actor.
        surfaces.push(Surface {
            material,
            lightmap,
            origin,
            u_axis,
            v_axis,
            pan,

            normal,
            flags,
            color: material_color(package.object_name(material)),
        });
    }
    let mut pool = Vec::new();
    for _ in 0..reader.count(2)? {
        pool.push(reader.compact()?);
        reader.compact()?; // Shared edge.
    }

    reader.u32()?; // Shared sides.
    let zone_count = reader.u32()? as usize;
    if zone_count > 64 {
        return Err(invalid("Invalid BSP zone count"));
    }
    let mut zones = Vec::new();
    for _ in 0..zone_count {
        zones.push(reader.compact()?);
        reader.take(16)?;
    }
    let polys = reader.compact()?;
    Ok(Model {
        vectors,
        points,
        nodes,
        surfaces,
        pool,
        zones,
        polys,
        reader,
    })
}

pub(crate) fn brush_polys<'a>(
    package: &Package<'a>,
    brush: &Export,
) -> io::Result<Option<Reader<'a>>> {
    let id = read_model(package, brush)?.polys;
    usize::try_from(id)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| package.exports.get(index))
        .map(|export| native_data(package, export))
        .transpose()
}

fn read_bsp(
    package: &Package<'_>,
    export: &Export,
    actors: &[&Export],
    path: Option<&Path>,
    use_zone_ambient: bool,
) -> io::Result<(Vec<Face>, Collision, Option<Vec3>, Vec<i32>)> {
    let Model {
        vectors,
        points,
        nodes,
        surfaces,
        pool,
        zones,
        mut reader,
        ..
    } = read_model(package, export)?;
    let mut sky_zones = [false; 64];
    let mut ambient = [if use_zone_ambient {
        [0.0; 3]
    } else {
        [0.30; 3]
    }; 64];
    let mut sky_origin = None;
    for (sky, actor) in sky_zones.iter_mut().zip(zones.iter().copied()) {
        if let Some(export) = usize::try_from(actor)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| package.exports.get(i))
            && package.object_name(export.class) == Some("SkyZoneInfo")
        {
            let mut props = package.properties(export)?;
            while let Some((name, mut value)) = props.next()? {
                if name == "Location" {
                    sky_origin.get_or_insert(Vec3::read(&mut value)?);
                    *sky = true;
                }
            }
        }
    }
    if use_zone_ambient {
        for (color, &actor) in ambient.iter_mut().zip(&zones) {
            *color = crate::light::zone_ambient(package, actor)?;
        }
    }
    let light_indices = (0..reader.count(30)?)
        .map(|_| LightIndex::read(&mut reader))
        .collect::<io::Result<Vec<_>>>()?;
    let light_bytes = reader.count(1)?;
    let light_masks = reader.take(light_bytes)?;
    let bounds = reader.count(25)?;
    reader.take(bounds * 25)?;
    let hulls = (0..reader.count(4)?)
        .map(|_| reader.u32())
        .collect::<io::Result<Vec<_>>>()?;
    for _ in 0..reader.count(11)? {
        reader.compact()?;
        reader.compact()?;
        reader.compact()?;
        reader.take(8)?;
    }
    let light_refs = (0..reader.count(1)?)
        .map(|_| reader.compact())
        .collect::<io::Result<Vec<_>>>()?;
    let lights = if path.is_some() {
        read_lights(package, &light_refs)?
    } else {
        Default::default()
    };
    let planes: Vec<_> = nodes.iter().map(|node| node.plane).collect();
    let mut collision = Collision::read(
        &planes,
        nodes.iter().map(|node| node.collision_bound),
        &hulls,
    )?;

    let mut level_zone = crate::movement::Zone::default();
    if let Some(level) = actors
        .iter()
        .find(|actor| package.object_name(actor.class) == Some("LevelInfo"))
    {
        level_zone.apply(package.properties(level)?)?;
    }
    collision.regions = crate::movement::Regions {
        nodes: nodes
            .iter()
            .map(|node| crate::movement::RegionNode {
                plane: node.plane,
                children: node.children,
                zones: node.zones,
            })
            .collect(),
        zones: Vec::new(),
        level: level_zone,
    };

    let textures = path
        .map(|path| {
            load_materials(
                package,
                path,
                surfaces.iter().map(|surface| surface.material),
            )
        })
        .unwrap_or_default();
    let mut faces = Vec::new();
    let mut lightmaps = std::collections::HashMap::new();
    for node in nodes {
        if node.vertices < 3 {
            continue;
        }
        let surface = usize::try_from(node.surface)
            .ok()
            .and_then(|index| surfaces.get(index))
            .ok_or_else(|| invalid("Invalid BSP surface index"))?;
        // Invisible faces include portals and collision-only geometry.
        if surface.flags & 1 != 0 {
            continue;
        }
        let normal = *vectors
            .get(surface.normal)
            .ok_or_else(|| invalid("Invalid BSP normal"))?;
        let first =
            usize::try_from(node.first_vertex).map_err(|_| invalid("Invalid BSP vertex pool"))?;
        let vertices = pool
            .get(first..first + node.vertices)
            .ok_or_else(|| invalid("BSP node exceeds vertex pool"))?
            .iter()
            .map(|&index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| points.get(index))
                    .copied()
                    .ok_or_else(|| invalid("Invalid BSP point"))
            })
            .collect::<io::Result<_>>()?;
        let texture_vector = |array: &[Vec3], index: i32| {
            usize::try_from(index)
                .ok()
                .and_then(|index| array.get(index))
                .copied()
                .unwrap_or_default()
        };
        let mut face = Face {
            mesh_triangle: None,
            flags: surface.flags,
            sky: node.zones.into_iter().any(|zone| {
                usize::try_from(zone)
                    .ok()
                    .and_then(|zone| sky_zones.get(zone))
                    .copied()
                    == Some(true)
            }),
            texture: textures.get(&surface.material).cloned(),
            lightmap: None,
            origin: texture_vector(&points, surface.origin),
            u_axis: texture_vector(&vectors, surface.u_axis),
            v_axis: texture_vector(&vectors, surface.v_axis),
            pan: surface.pan,

            vertices,
            normal,
            color: surface.color,
        };
        if path.is_some()
            && face.flags & 0x0040_0000 == 0
            && let Some(index) = usize::try_from(surface.lightmap)
                .ok()
                .and_then(|index| light_indices.get(index))
        {
            let zone = if use_zone_ambient {
                node.zones[usize::from(node.plane.normal.dot(normal) >= 0.0)]
            } else {
                0
            };
            let ambient = ambient.get(zone as usize).copied().unwrap_or_default();
            face.lightmap = lightmaps
                .entry((node.surface, zone))
                .or_insert_with(|| {
                    Lightmap::bake(
                        &face,
                        index,
                        light_masks,
                        &light_refs,
                        &lights,
                        ambient,
                        use_zone_ambient,
                    )
                    .map(Arc::new)
                })
                .clone();
        }
        faces.push(face);
    }
    Ok((faces, collision, sky_origin, zones))
}

fn read_vectors(reader: &mut Reader<'_>) -> io::Result<Vec<Vec3>> {
    (0..reader.count(12)?).map(|_| Vec3::read(reader)).collect()
}

pub(crate) fn material_color(name: Option<&str>) -> [f32; 3] {
    let hash = name
        .unwrap_or_default()
        .bytes()
        .fold(0x811c9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
        });
    [
        84.0 + (hash & 63) as f32,
        94.0 + ((hash >> 8) & 63) as f32,
        102.0 + ((hash >> 16) & 63) as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::Walker;

    #[test]
    #[ignore = "requires the local GOTY map files"]
    fn deck16_swimmer_can_surface_and_dive() {
        let path = Path::new("local/goty/Maps/DM-Deck16][.unr");
        let bytes = std::fs::read(path).unwrap();
        let mut map = Map::load(&Package::read(&bytes).unwrap(), Some(path)).unwrap();
        let world = &map.collision;
        let up = Vec3(0.0, 0.0, 1.0);
        let location = map
            .faces
            .iter()
            .filter(|face| face.normal.2.abs() > 0.95)
            .map(|face| {
                face.vertices
                    .iter()
                    .copied()
                    .fold(Vec3::default(), |sum, point| sum + point)
                    * (1.0 / face.vertices.len() as f32)
            })
            .find(|&point| {
                world.regions.at(point - up * 80.0).water
                    && !world.regions.at(point + up * 8.0).water
                    && world.ray(point - up * 80.0, up * 160.0).is_none()
            })
            .expect("open water surface on Deck16");
        let start = location - up * 80.0;
        let slime = world.regions.at(start);
        assert!(slime.pain);
        assert_eq!(slime.damage_per_second, 40);
        let sounds = &map.liquid_audio[slime.liquid_audio.expect("slime zone sounds")];
        assert!(sounds.iter().all(Option::is_some));
        let mut lower = start.2;
        let mut upper = location.2 + 8.0;
        for _ in 0..12 {
            let middle = (lower + upper) * 0.5;
            if world.regions.at(Vec3(start.0, start.1, middle)).water {
                lower = middle;
            } else {
                upper = middle;
            }
        }
        let mut walker = Walker::spawn(world, start);
        let dt = 1.0 / 240.0;
        let mut surfaced = false;
        for _ in 0..240 {
            let events = walker.advance_with_swim(world, Vec3::default(), up * 0.5, None, dt);
            assert!(events.landing.is_none());
            surfaced |= !world.regions.at(walker.position).water;
        }
        assert!(surfaced, "surface z={upper} {:?}", walker.position);
        for _ in 0..240 {
            walker.advance_with_swim(world, Vec3::default(), up * -1.0, None, dt);
        }
        assert!(world.regions.at(walker.position).water);
        assert!(walker.position.2 < upper - 20.0);

        let mut swimmer = Walker::spawn(world, Vec3(-1160.0, -1230.0, upper - 20.0));
        assert!(world.regions.at(swimmer.position).water);
        let reach_ramp = Vec3(1.0, 0.0, 0.0);
        for _ in 0..220 {
            swimmer.advance_with_swim(
                world,
                reach_ramp * world.profile.ground_speed,
                reach_ramp + up * 0.65,
                None,
                dt,
            );
            if swimmer.grounded && swimmer.position.0 > -1100.0 {
                break;
            }
        }
        assert!(
            swimmer.grounded && swimmer.position.0 > -1100.0,
            "{:?}",
            swimmer.position
        );
        let up_ramp = Vec3(0.0, -1.0, 0.0);
        for _ in 0..500 {
            swimmer.advance(world, up_ramp * world.profile.ground_speed, None, dt);
            if swimmer.grounded && swimmer.position.2 > upper + 130.0 {
                break;
            }
        }
        assert!(
            swimmer.grounded && swimmer.position.2 > upper + 130.0,
            "{:?}",
            swimmer.position
        );

        let mut game = crate::game::Practice::new(&map);
        let mut camera = crate::render::Camera::from_start(map.starts[0]);
        let mut cross = |position: Vec3| {
            walker.position = position;
            camera.position = position + up * 27.0;
            game.update(
                &mut map,
                &camera,
                0.3,
                crate::game::Input {
                    primary: false,
                    alternate: false,
                    slot: None,
                    cycle: 0,
                },
                false,
                Some(&mut walker),
            )
        };
        cross(location + up * 64.0);
        let feet_in_slime = start + up * crate::collision::PLAYER_HEIGHT;
        assert!(
            !cross(feet_in_slime).is_empty(),
            "entering the slime plays a sound"
        );
        assert!(
            !cross(location + up * 64.0).is_empty(),
            "leaving the slime plays a sound"
        );
    }

    #[test]
    #[ignore = "requires the local GOTY map files"]
    fn stock_spawn_points_have_solid_ground() {
        for name in ["DM-Deck16][.unr", "DM-Liandri.unr", "CTF-Face.unr"] {
            let path = Path::new("local/goty/Maps").join(name);
            let bytes = std::fs::read(path).unwrap();
            let package = Package::read(&bytes).unwrap();
            let map = Map::load(&package, None).unwrap();
            for (index, start) in map.starts.iter().enumerate() {
                let mut walker = Walker::spawn(&map.collision, start.position);
                for _ in 0..30 {
                    walker.advance(&map.collision, Vec3::default(), None, 1.0 / 60.0);
                }
                assert!(
                    walker.grounded,
                    "{name} #{} {:?} -> {:?}",
                    index + 1,
                    start.position,
                    walker.position
                );
            }
        }
    }
}
