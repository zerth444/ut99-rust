use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
    sync::Arc,
};

use crate::game::ordnance::{self, Kind};
use crate::game::shock;
use crate::game::{Flash, Item, Motion, Weapon, WeaponAssets};
use crate::map::{Face, Vec3, material_color, native_data};
use crate::mover::{product, read_rotation, rotate, unit};
use crate::package::{Export, Package, Properties, Reader, invalid};
use crate::sound::Wave;
use crate::texture::{Texture, export_path, import_path};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Object {
    package: usize,
    export: usize,
}

struct Assets<'a> {
    packages: Vec<Package<'a>>,
    names: HashMap<String, usize>,
    references: HashMap<(usize, i32), Option<Object>>,
}

impl<'a> Assets<'a> {
    fn object(&self, object: Object) -> (&Package<'a>, &Export) {
        let package = &self.packages[object.package];
        (package, &package.exports[object.export])
    }

    fn resolve(&mut self, package: usize, reference: i32) -> Option<Object> {
        if reference > 0 {
            let export = reference as usize - 1;
            return self.packages[package]
                .exports
                .get(export)
                .map(|_| Object { package, export });
        }
        if reference == 0 {
            return None;
        }
        *self
            .references
            .entry((package, reference))
            .or_insert_with(|| {
                let source = &self.packages[package];
                let import = source.imports.get(reference.unsigned_abs() as usize - 1)?;
                let class = source.name(import.class).ok()?;
                let (name, path) = import_path(source, reference)?;
                let package = *self.names.get(&name)?;
                let target = &self.packages[package];
                let export = target.exports.iter().position(|item| {
                    let kind = target.object_name(item.class).unwrap_or("Class");
                    kind.eq_ignore_ascii_case(class)
                        && export_path(target, item).as_ref() == Some(&path)
                })?;
                Some(Object { package, export })
            })
    }
}

#[derive(Clone, Copy, Default)]
struct Appearance {
    hidden: bool,
    draw_type: u8,
    mesh: Option<Object>,
    player_mesh: Option<Object>,
    status_icon: Option<Object>,
    player_offset: Option<Vec3>,
    player_scale: Option<f32>,
    fire_sound: Option<Object>,
    alt_sound: Option<Object>,
    pickup_sound: Option<Object>,
    select_sound: Option<Object>,
    flash_texture: Option<Object>,
    flash_variations: [Option<Object>; 5],
    flash_enabled: Option<bool>,
    flash_scale: Option<f32>,
    flash_y: Option<f32>,
    flash_o: Option<f32>,
    flash_length: Option<f32>,
    flash_size: Option<u32>,
    fire_offset: Option<Vec3>,
    ammo_class: Option<Object>,
    pickup_ammo: Option<u16>,
    ammo_amount: Option<u16>,
    max_ammo: Option<u16>,
    respawn_time: Option<f32>,
    lifetime: Option<f32>,
    skin: Option<Object>,
    texture: Option<Object>,
    multiskins: [Option<Object>; 8],
    scale: f32,
}

struct VisualProperties {
    hidden: Option<bool>,
    draw_type: Option<u8>,
    mesh: Option<i32>,
    player_mesh: Option<i32>,
    status_icon: Option<i32>,
    player_offset: Option<Vec3>,
    player_scale: Option<f32>,
    fire_sound: Option<i32>,
    alt_sound: Option<i32>,
    pickup_sound: Option<i32>,
    select_sound: Option<i32>,
    flash_texture: Option<i32>,
    flash_variations: [Option<i32>; 5],
    flash_enabled: Option<bool>,
    flash_scale: Option<f32>,
    flash_y: Option<f32>,
    flash_o: Option<f32>,
    flash_length: Option<f32>,
    flash_size: Option<u32>,
    fire_offset: Option<Vec3>,
    ammo_class: Option<i32>,
    pickup_ammo: Option<u16>,
    ammo_amount: Option<u16>,
    max_ammo: Option<u16>,
    respawn_time: Option<f32>,
    lifetime: Option<f32>,
    skin: Option<i32>,
    texture: Option<i32>,
    multiskins: [Option<i32>; 8],
    scale: Option<f32>,
    location: Vec3,
    rotation: Vec3,
}

impl VisualProperties {
    fn read(mut props: Properties<'_, '_>) -> io::Result<Self> {
        let mut props_out = Self {
            hidden: None,
            draw_type: None,
            mesh: None,
            player_mesh: None,
            status_icon: None,
            player_offset: None,
            player_scale: None,
            fire_sound: None,
            alt_sound: None,
            pickup_sound: None,
            select_sound: None,
            flash_texture: None,
            flash_variations: [None; 5],
            flash_enabled: None,
            flash_scale: None,
            flash_y: None,
            flash_o: None,
            flash_length: None,
            flash_size: None,
            fire_offset: None,
            ammo_class: None,
            pickup_ammo: None,
            ammo_amount: None,
            max_ammo: None,
            respawn_time: None,
            lifetime: None,
            skin: None,
            texture: None,
            multiskins: [None; 8],
            scale: None,
            location: Vec3::default(),
            rotation: Vec3::default(),
        };
        while let Some(prop) = props.next_tag()? {
            let mut value = prop.value;
            match prop.name {
                "bHidden" => props_out.hidden = prop.boolean,
                "DrawType" => props_out.draw_type = Some(value.byte()?),
                "Mesh" => props_out.mesh = Some(value.compact()?),
                "PlayerViewMesh" => props_out.player_mesh = Some(value.compact()?),
                "StatusIcon" => props_out.status_icon = Some(value.compact()?),
                "PlayerViewOffset" => props_out.player_offset = Some(Vec3::read(&mut value)?),
                "PlayerViewScale" => props_out.player_scale = Some(value.f32()?),
                "FireSound" => props_out.fire_sound = Some(value.compact()?),
                "AltFireSound" => props_out.alt_sound = Some(value.compact()?),
                "PickupSound" => props_out.pickup_sound = Some(value.compact()?),
                "SelectSound" => props_out.select_sound = Some(value.compact()?),
                "MFTexture" => props_out.flash_texture = Some(value.compact()?),
                "MuzzleFlashVariations" if prop.array_index < props_out.flash_variations.len() => {
                    props_out.flash_variations[prop.array_index] = Some(value.compact()?);
                }
                "bDrawMuzzleFlash" => props_out.flash_enabled = prop.boolean,
                "MuzzleScale" => props_out.flash_scale = Some(value.f32()?),
                "FlashY" => props_out.flash_y = Some(value.f32()?),
                "FlashO" => props_out.flash_o = Some(value.f32()?),
                "FlashLength" => props_out.flash_length = Some(value.f32()?),
                "FlashS" => props_out.flash_size = Some(value.u32()?),
                "FireOffset" => props_out.fire_offset = Some(Vec3::read(&mut value)?),
                "AmmoName" => props_out.ammo_class = Some(value.compact()?),
                "PickupAmmoCount" => props_out.pickup_ammo = u16::try_from(value.u32()?).ok(),
                "AmmoAmount" => props_out.ammo_amount = u16::try_from(value.u32()?).ok(),
                "MaxAmmo" => props_out.max_ammo = u16::try_from(value.u32()?).ok(),
                "RespawnTime" => props_out.respawn_time = Some(value.f32()?),
                "LifeSpan" => props_out.lifetime = Some(value.f32()?),
                "Skin" => props_out.skin = Some(value.compact()?),
                "Texture" => props_out.texture = Some(value.compact()?),
                "MultiSkins" if prop.array_index < props_out.multiskins.len() => {
                    props_out.multiskins[prop.array_index] = Some(value.compact()?);
                }
                "DrawScale" => props_out.scale = Some(value.f32()?),
                "Location" => props_out.location = Vec3::read(&mut value)?,
                "Rotation" => props_out.rotation = read_rotation(&mut value)?,
                _ => {}
            }
        }
        Ok(props_out)
    }

    fn apply(&self, assets: &mut Assets<'_>, package: usize, mut parent: Appearance) -> Appearance {
        if let Some(value) = self.hidden {
            parent.hidden = value;
        }
        if let Some(value) = self.draw_type {
            parent.draw_type = value;
        }
        if let Some(value) = self.mesh {
            parent.mesh = assets.resolve(package, value);
        }
        for (slot, value) in [
            (&mut parent.player_mesh, self.player_mesh),
            (&mut parent.status_icon, self.status_icon),
            (&mut parent.fire_sound, self.fire_sound),
            (&mut parent.alt_sound, self.alt_sound),
            (&mut parent.pickup_sound, self.pickup_sound),
            (&mut parent.select_sound, self.select_sound),
            (&mut parent.flash_texture, self.flash_texture),
            (&mut parent.ammo_class, self.ammo_class),
        ] {
            if let Some(value) = value {
                *slot = assets.resolve(package, value);
            }
        }
        if let Some(value) = self.skin {
            parent.skin = assets.resolve(package, value);
        }
        if let Some(value) = self.texture {
            parent.texture = assets.resolve(package, value);
        }
        for (slot, value) in parent.multiskins.iter_mut().zip(self.multiskins) {
            if let Some(value) = value {
                *slot = assets.resolve(package, value);
            }
        }
        for (slot, value) in parent
            .flash_variations
            .iter_mut()
            .zip(self.flash_variations)
        {
            if let Some(value) = value {
                *slot = assets.resolve(package, value);
            }
        }
        parent.flash_enabled = self.flash_enabled.or(parent.flash_enabled);
        parent.flash_scale = self.flash_scale.or(parent.flash_scale);
        parent.flash_y = self.flash_y.or(parent.flash_y);
        parent.flash_o = self.flash_o.or(parent.flash_o);
        parent.flash_length = self.flash_length.or(parent.flash_length);
        parent.flash_size = self.flash_size.or(parent.flash_size);
        parent.fire_offset = self.fire_offset.or(parent.fire_offset);
        parent.player_offset = self.player_offset.or(parent.player_offset);
        parent.player_scale = self.player_scale.or(parent.player_scale);
        parent.pickup_ammo = self.pickup_ammo.or(parent.pickup_ammo);
        parent.ammo_amount = self.ammo_amount.or(parent.ammo_amount);
        parent.max_ammo = self.max_ammo.or(parent.max_ammo);
        parent.respawn_time = self.respawn_time.or(parent.respawn_time);
        parent.lifetime = self.lifetime.or(parent.lifetime);
        if let Some(value) = self.scale {
            parent.scale = value;
        }
        parent
    }
}

fn class_properties<'p, 'a>(
    package: &'p Package<'a>,
    class: &Export,
) -> io::Result<Properties<'p, 'a>> {
    let mut reader = Reader::at(&package.data[class.data.clone()], 0)?;
    for _ in 0..5 {
        reader.compact()?;
    } // UField and UStruct references.
    reader.take(8)?; // Source positions.
    let script_size = reader.u32()?;
    crate::script::skip(package, &mut reader, script_size)?;
    reader.take(22)?; // UState.
    if package.version <= 61 {
        reader.u32()?;
    }
    reader.take(20)?; // Class flags and GUID.
    for _ in 0..reader.count(9)? {
        reader.compact()?;
        reader.take(8)?;
    }
    for _ in 0..reader.count(1)? {
        reader.compact()?;
    }
    if package.version >= 62 {
        reader.compact()?;
        reader.compact()?;
    }
    Ok(package.properties_at(reader))
}

fn appearance(
    assets: &mut Assets<'_>,
    cache: &mut HashMap<Object, Appearance>,
    mut object: Option<Object>,
) -> Appearance {
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    let mut defaults = Appearance {
        scale: 1.0,
        ..Default::default()
    };
    while let Some(class) = object.filter(|class| visited.insert(*class)) {
        if let Some(&parent) = cache.get(&class) {
            defaults = parent;
            break;
        }
        let (package, export) = assets.object(class);
        let super_id = export.superclass;
        let props = class_properties(package, export)
            .and_then(VisualProperties::read)
            .ok();
        chain.push((class, props));
        object = assets.resolve(class.package, super_id);
    }
    for (class, props) in chain.into_iter().rev() {
        if let Some(props) = props {
            defaults = props.apply(assets, class.package, defaults);
        }
        cache.insert(class, defaults);
    }
    defaults
}

pub struct Prop {
    pub faces: Vec<Face>,
    pub position: Vec3,
    pub radius: f32,
    pub item: Option<Item>,
    pub visible: bool,
    pub pickup_sound: Option<Arc<Wave>>,
    pub respawn_time: Option<f32>,
    pub cinematic: Option<CinematicMotion>,
}

pub struct CinematicMotion {
    pub event: usize,
    pub animation: Motion,
    pub started: Option<f32>,
}

#[derive(Default)]
pub struct Scene {
    pub props: Vec<Prop>,
    pub arsenal: HashMap<Weapon, WeaponAssets>,
    pub hud: crate::hud_art::Art,
    pub player: crate::movement::Profile,
    pub player_audio: crate::movement::PlayerAudio,
    pub player_deaths: [Motion; 3],
    pub player_idle: Motion,
    pub ordnance: ordnance::Arsenal,
    pub ballistics: crate::game::ballistics::Art,
    pub zones: Vec<crate::movement::Zone>,
}

pub fn load(
    map: &Package<'_>,
    actors: &[&Export],
    path: &Path,
    zone_actors: &[i32],
    level: crate::movement::Zone,
) -> io::Result<Scene> {
    let Some(folder) = path.parent() else {
        return Ok(Scene::default());
    };
    let mut packages: Vec<_> = [
        ("../System", "u"),
        ("../Textures", "utx"),
        ("../Sounds", "uax"),
    ]
    .into_iter()
    .flat_map(|(directory, extension)| {
        fs::read_dir(folder.join(directory))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(move |entry| (entry.path(), extension))
    })
    .filter_map(|(path, extension)| {
        path.extension()?
            .to_str()?
            .eq_ignore_ascii_case(extension)
            .then_some(())?;
        let name = path.file_stem()?.to_str()?.to_ascii_lowercase();
        // Only packages directly referenced by this map and the engine's base classes.
        if !matches!(
            name.as_str(),
            "engine" | "core" | "botpack" | "dmatch" | "unrealshare" | "unreali" | "soldierskins"
        ) && !map
            .imports
            .iter()
            .any(|import| import.outer == 0 && map.names[import.name].eq_ignore_ascii_case(&name))
        {
            return None;
        }
        Some((name, fs::read(path).ok()?))
    })
    .collect();
    packages.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut assets = Assets {
        packages: vec![Package::read(map.data)?],
        names: HashMap::new(),
        references: HashMap::new(),
    };
    for (name, data) in &packages {
        if let Ok(package) = Package::read(data) {
            assets.names.insert(name.clone(), assets.packages.len());
            assets.packages.push(package);
        }
    }
    let mut class_cache = HashMap::new();
    let mut meshes = HashMap::new();
    let mut textures = HashMap::<Object, Option<Arc<Texture>>>::new();
    let mut waves = HashMap::new();
    let mut props = Vec::new();
    for &actor in actors {
        let item = map.object_name(actor.class).and_then(Item::from_class);
        let class = assets.resolve(0, actor.class);
        let defaults = appearance(&mut assets, &mut class_cache, class);
        let placement = VisualProperties::read(map.properties(actor)?)?;
        let visual = placement.apply(&mut assets, 0, defaults);
        let Some(object) = visual
            .mesh
            .filter(|_| !visual.hidden && visual.draw_type == 2 && visual.scale > 0.0)
        else {
            continue;
        };
        let mesh = meshes.entry(object).or_insert_with(|| {
            let (package, export) = assets.object(object);
            Mesh::read(package, export).ok()
        });
        let Some(mesh) = mesh else { continue };
        let placed_frame = |index| -> Vec<Vec3> {
            mesh.frame(index)
                .unwrap_or_default()
                .iter()
                .map(|&point| {
                    rotate(
                        rotate(product(point - mesh.origin, mesh.scale), mesh.rotation)
                            * visual.scale,
                        placement.rotation,
                    ) + placement.location
                })
                .collect()
        };
        let vertices = placed_frame(0);
        let radius = vertices
            .iter()
            .map(|&point| {
                let offset = point - placement.location;
                offset.dot(offset)
            })
            .fold(0.0_f32, f32::max)
            .sqrt();
        let faces = mesh_faces(&mut assets, &mut textures, object, visual, mesh, &vertices);
        if !faces.is_empty() {
            let clip = match map.object_name(actor.class) {
                Some("IntroDude") => mesh.sequence(&["Shake"]).map(|clip| (clip, 1.0)),
                Some("IntroBoss") => mesh.sequence(&["Wave"]).map(|clip| (clip, 0.5)),
                _ => None,
            };
            let cinematic = clip.and_then(|(clip, rate)| {
                let mut props = map.properties(actor).ok()?;
                let mut tag = None;
                while let Some((name, mut value)) = props.next().ok()? {
                    if name == "Tag" {
                        tag = value.index().ok();
                        break;
                    }
                }
                Some(CinematicMotion {
                    event: tag?,
                    started: None,
                    animation: Motion {
                        rate: clip.rate * rate,
                        frames: (clip.start..clip.end)
                            .map(|index| {
                                mesh_faces(
                                    &mut assets,
                                    &mut textures,
                                    object,
                                    visual,
                                    mesh,
                                    &placed_frame(index),
                                )
                            })
                            .collect(),
                    },
                })
            });
            props.push(Prop {
                faces,
                position: placement.location,
                radius,
                item: item.map(|item| match item {
                    Item::Ammo(weapon, count) => {
                        Item::Ammo(weapon, visual.ammo_amount.unwrap_or(count))
                    }
                    _ => item,
                }),
                respawn_time: visual.respawn_time.filter(|value| *value > 0.0),
                cinematic,
                visible: true,
                pickup_sound: cached_wave(&assets, &mut waves, visual.pickup_sound).or_else(|| {
                    cached_wave(
                        &assets,
                        &mut waves,
                        find_export(&assets, "dmatch", "PickupSound", "Sound"),
                    )
                }),
            });
        }
    }
    let mut arsenal = HashMap::new();
    for weapon in Weapon::ALL {
        let Some(class) = find_export(&assets, "botpack", weapon.class(), "Class") else {
            continue;
        };
        let visual = appearance(&mut assets, &mut class_cache, Some(class));
        let ammo = appearance(&mut assets, &mut class_cache, visual.ammo_class);
        let mut weapon_assets = WeaponAssets {
            pickup_ammo: visual.pickup_ammo,
            max_ammo: ammo.max_ammo,
            status_icon: visual
                .status_icon
                .and_then(|id| cached_texture(&assets, &mut textures, id)),
            fire: cached_wave(&assets, &mut waves, visual.fire_sound),
            select_sound: cached_wave(&assets, &mut waves, visual.select_sound),
            alternate: cached_wave(&assets, &mut waves, visual.alt_sound),
            ..Default::default()
        };
        if weapon == Weapon::Shock {
            weapon_assets.shock = Some(read_shock(
                &mut assets,
                &mut class_cache,
                &mut textures,
                &mut waves,
                visual,
            ));
        }
        if weapon == Weapon::Pulse {
            weapon_assets.pulse_beam =
                pulse_beam(&mut assets, &mut class_cache, &mut textures, false).unwrap_or_default();
            weapon_assets.pulse_starter =
                pulse_beam(&mut assets, &mut class_cache, &mut textures, true).unwrap_or_default();
            weapon_assets.pulse_tip = ["phit_a00", "pEnd_a00"].map(|name| {
                find_export(&assets, "botpack", name, "Texture")
                    .and_then(|id| cached_texture(&assets, &mut textures, id))
            });
            let scorch = find_export(&assets, "botpack", "BoltScorch", "Class");
            let scorch = appearance(&mut assets, &mut class_cache, scorch);
            weapon_assets.pulse_scorch = scorch
                .texture
                .and_then(|id| cached_texture(&assets, &mut textures, id))
                .map(|texture| (texture, scorch.scale));
        }
        let load_sound = match weapon {
            Weapon::Hammer => find_export(&assets, "botpack", "ImpactAltFireStart", "Sound"),
            Weapon::Minigun => find_export(&assets, "botpack", "M2WindDown", "Sound"),
            Weapon::Rocket => find_export(&assets, "unrealshare", "Loading", "Sound"),
            Weapon::Bio => find_export(&assets, "botpack", "BioAltRep", "Sound"),
            Weapon::Pulse => find_export(&assets, "botpack", "PulseDown", "Sound"),
            _ => None,
        };
        weapon_assets.load_sound = cached_wave(&assets, &mut waves, load_sound);
        if weapon == Weapon::Hammer {
            weapon_assets.charge_loop = cached_wave(
                &assets,
                &mut waves,
                find_export(&assets, "botpack", "ImpactLoop", "Sound"),
            );
        }
        if weapon == Weapon::Redeemer {
            weapon_assets.guided_hud = ["GuidedX", "Readout", "Static_a00"].map(|name| {
                find_export(&assets, "botpack", name, "Texture")
                    .and_then(|id| cached_texture(&assets, &mut textures, id))
            });
            weapon_assets.charge_loop = cached_wave(
                &assets,
                &mut waves,
                find_export(&assets, "botpack", "WarFly", "Sound"),
            );
        }
        if weapon == Weapon::Rocket {
            weapon_assets.rocket_lock = ["SeekLock", "SeekLost"].map(|name| {
                cached_wave(
                    &assets,
                    &mut waves,
                    find_export(&assets, "unrealshare", name, "Sound"),
                )
            });
            weapon_assets.lock_reticle = find_export(&assets, "botpack", "Crosshair6", "Texture")
                .or_else(|| find_export(&assets, "botpack", "CHair6", "Texture"))
                .and_then(|id| cached_texture(&assets, &mut textures, id));
        }
        if weapon == Weapon::Ripper {
            weapon_assets.fire = cached_wave(
                &assets,
                &mut waves,
                find_export(&assets, "unreali", "StartBlade", "Sound"),
            );
            weapon_assets.alternate = cached_wave(
                &assets,
                &mut waves,
                find_export(&assets, "botpack", "RazorAlt", "Sound"),
            );
        }
        if weapon == Weapon::Sniper {
            weapon_assets.scope = find_export(&assets, "botpack", "RReticle", "Texture")
                .and_then(|id| cached_texture(&assets, &mut textures, id));
        }
        if visual.flash_enabled == Some(true) {
            let flash: Vec<_> = visual
                .flash_variations
                .into_iter()
                .flatten()
                .filter_map(|id| cached_texture(&assets, &mut textures, id))
                .collect();
            let flash = if flash.is_empty() {
                visual
                    .flash_texture
                    .and_then(|id| cached_texture(&assets, &mut textures, id))
                    .into_iter()
                    .collect()
            } else {
                flash
            };
            if !flash.is_empty() {
                weapon_assets.flash = Some(Flash {
                    textures: flash,
                    scale: visual.flash_scale.unwrap_or(1.0),
                    x: -visual.fire_offset.unwrap_or_default().1 * visual.flash_o.unwrap_or(0.0),
                    y: visual.flash_y.unwrap_or(0.0),
                    length: visual.flash_length.unwrap_or(0.02),
                    half_offset: visual.flash_size.unwrap_or(0) as f32 * 0.5,
                });
            }
        }
        if let Some(object) = visual.player_mesh {
            let mesh = meshes.entry(object).or_insert_with(|| {
                let (package, export) = assets.object(object);
                Mesh::read(package, export).ok()
            });
            if let Some(mesh) = mesh {
                let (package, export) = assets.object(object);
                if weapon == Weapon::Enforcer && package.name(export.name).ok() == Some("AutoML") {
                    // Opposite corners of the front barrel opening in the GOTY view mesh.
                    weapon_assets.muzzle_triangle = mesh
                        .triangles
                        .iter()
                        .position(|triangle| triangle.vertices == [76, 84, 85]);
                    weapon_assets.case_triangle = mesh
                        .triangles
                        .iter()
                        .position(|triangle| triangle.vertices == [125, 30, 150]);
                }
                let rest = mesh.sequence(&["Still", "Idle"]).map_or(0, |seq| seq.start);
                let transform =
                    |point| rotate(product(point - mesh.origin, mesh.scale), mesh.rotation);
                if let Some(Vec3(x, y, z)) = visual.player_offset {
                    let offset = Vec3(x, -y, z);
                    let scale = visual.player_scale.unwrap_or(1.0);
                    let mut pose = |index| {
                        let vertices: Vec<_> = mesh
                            .frame(index)
                            .unwrap_or_default()
                            .iter()
                            .copied()
                            .map(|point| (transform(point) * scale + offset) * 64.0)
                            .collect();
                        mesh_faces(&mut assets, &mut textures, object, visual, mesh, &vertices)
                    };
                    let mut motion = |names: &[&str], factor| {
                        mesh.sequence(names)
                            .map_or_else(Motion::default, |seq| Motion {
                                frames: (seq.start..seq.end).map(&mut pose).collect(),
                                rate: seq.rate * factor,
                            })
                    };
                    weapon_assets.primary_motion = motion(
                        &["Shoot", "Fire1", "Fire", "Shoot1", "ShootLoop", "Firing"],
                        match weapon {
                            Weapon::Enforcer => 0.81,
                            Weapon::Shock => shock::firing_rate(false),
                            Weapon::Hammer => 0.65,
                            Weapon::Minigun => 1.6,
                            _ => 1.0,
                        },
                    );
                    weapon_assets.alternate_motion = motion(
                        &["Shot2", "Fire2", "AltFire", "BoltLoop", "AltFiring"],
                        match weapon {
                            Weapon::Shock => shock::firing_rate(true),
                            _ => 1.0,
                        },
                    );
                    weapon_assets.select_motion = motion(&["Select"], 1.0);
                    weapon_assets.down_motion = motion(&["Down"], 1.0);
                    weapon_assets.load = match weapon {
                        Weapon::Hammer => motion(&["Pull"], 0.2),
                        Weapon::Rocket => motion(&["Load1"], 1.0),
                        Weapon::Bio => motion(&["Charging"], 0.24),
                        _ => Motion::default(),
                    };
                    if weapon == Weapon::Hammer {
                        weapon_assets.alternate_motion = motion(&["Shake"], 1.0);
                    }
                    if weapon == Weapon::Minigun {
                        weapon_assets.alternate_motion = motion(&["Shoot1"], 1.3);
                    }
                    if weapon == Weapon::Pulse {
                        weapon_assets.primary_motion = motion(&["ShootLoop"], 1.5);
                        weapon_assets.attack_start = motion(&["BoltStart"], 1.0);
                        weapon_assets.attack_end = motion(&["BoltEnd"], 1.0);
                        weapon_assets.spin_down = motion(&["SpinDown"], 1.0);
                    }
                    if weapon == Weapon::Enforcer {
                        weapon_assets.attack_start = motion(&["T1"], 1.3);
                        weapon_assets.attack_end = motion(&["T2"], 0.9);
                    }
                    weapon_assets.view = pose(rest);
                }
            }
        }
        arsenal.insert(weapon, weapon_assets);
    }
    let hud_texture = |name| {
        let id = find_export(&assets, "botpack", name, "Texture")?;
        let (package, export) = assets.object(id);
        Texture::read(package, export).ok().map(Arc::new)
    };
    let hud = crate::hud_art::Art {
        elements: hud_texture("HudElements1"),
        weapons: hud_texture("HUDWeapons"),
        crosshair: hud_texture("CHair1"),
        doll: hud_texture("Man"),
        belt: hud_texture("ManBelt"),
    };
    let (player, mut player_audio) = read_player(&mut assets, &mut waves);
    player_audio.pain = ["injurL2", "injurL04", "injurM04", "injurH5"].map(|name| {
        cached_wave(
            &assets,
            &mut waves,
            find_export(&assets, "botpack", name, "Sound"),
        )
    });
    player_audio.death = ["deathc1", "deathc51", "deathc3"].map(|name| {
        cached_wave(
            &assets,
            &mut waves,
            find_export(&assets, "botpack", name, "Sound"),
        )
    });
    let (player_deaths, player_idle) = player_models(&mut assets, &mut class_cache, &mut textures);
    let ordnance = Kind::ALL.map(|kind| {
        projectile_art(
            &mut assets,
            &mut class_cache,
            &mut textures,
            &mut waves,
            kind,
        )
    });
    let ballistics = read_ballistics(&mut assets, &mut class_cache, &mut textures, &mut waves);
    let zones = crate::movement::load_zones(map, zone_actors, level, |actor, mut zone| {
        let mut class = assets.resolve(0, actor.class);
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        while let Some(id) = class.filter(|id| visited.insert(*id)) {
            let (_, export) = assets.object(id);
            if assets.packages[id.package].name(export.name).ok() == Some("ZoneInfo") {
                break;
            }
            chain.push(id);
            class = assets.resolve(id.package, export.superclass);
        }
        if chain.is_empty() {
            return crate::movement::stock_liquid_zone(map.object_name(actor.class), zone);
        }
        for id in chain.into_iter().rev() {
            let (package, export) = assets.object(id);
            if let Ok(properties) = class_properties(package, export) {
                let _ = zone.apply(properties);
            }
        }
        zone
    })?;
    Ok(Scene {
        props,
        arsenal,
        hud,
        player,
        player_audio,
        player_deaths,
        player_idle,
        ordnance,
        ballistics,
        zones,
    })
}

fn read_ballistics(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
) -> crate::game::ballistics::Art {
    let [tracer, pistol_case, mini_case] =
        ["MTracer", "UT_ShellCase", "MiniShellCase"].map(|name| {
            let class = find_export(assets, "botpack", name, "Class");
            let visual = appearance(assets, classes, class);
            let Some(object) = visual.mesh else {
                return Motion::default();
            };
            let (package, export) = assets.object(object);
            let Ok(mesh) = Mesh::read(package, export) else {
                return Motion::default();
            };
            effect_motion(assets, textures, object, visual, &mesh, &["All"])
        });
    let mut texture = |name| {
        find_export(assets, "botpack", name, "Texture")
            .and_then(|object| cached_texture(assets, textures, object))
    };
    let smoke = ["us1_a00", "us2_a00", "us3_a00", "us8_a00"].map(&mut texture);
    let spark = texture("Sparky");
    let pock = ["pock0_t", "pock2_t", "pock4_t"].map(&mut texture);
    let pock_class = find_export(assets, "botpack", "Pock", "Class");
    let pock_scale = appearance(assets, classes, pock_class).scale;
    let mut sound = |name| {
        let object = ["botpack", "unrealshare", "unreali"]
            .into_iter()
            .find_map(|package| find_export(assets, package, name, "Sound"));
        cached_wave(assets, waves, object)
    };
    crate::game::ballistics::Art {
        tracer,
        cases: [pistol_case, mini_case],
        smoke,
        spark,
        wall_sounds: ["ricochet", "Impact1", "Impact2"].map(&mut sound),
        flesh: sound("ChunkHit"),
        shell: sound("shell2"),
        pock,
        pock_scale,
    }
}

fn player_models(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
) -> ([Motion; 3], Motion) {
    let class = find_export(assets, "botpack", "TMale2", "Class");
    let mut visual = appearance(assets, classes, class);
    let Some(object) = visual.mesh else {
        return Default::default();
    };
    let (package, export) = assets.object(object);
    let Ok(mesh) = Mesh::read(package, export) else {
        return Default::default();
    };
    for (slot, name) in ["blkt1", "blkt2", "blkt3", "blkt4Malcom"]
        .into_iter()
        .enumerate()
    {
        visual.multiskins[slot] = find_export(assets, "soldierskins", name, "Texture");
    }
    (
        ["Dead1", "Dead3", "Dead8"]
            .map(|name| effect_motion(assets, textures, object, visual, &mesh, &[name])),
        effect_motion(assets, textures, object, visual, &mesh, &["Breath1"]),
    )
}

fn projectile_art(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
    kind: Kind,
) -> ordnance::Art {
    let mut art = ordnance::Art::default();
    let Some(class) = find_export(assets, "botpack", kind.class(), "Class") else {
        return art;
    };
    let visual = appearance(assets, classes, Some(class));
    art.scale = Some(visual.scale);
    let (package, export) = assets.object(class);
    if let Ok(mut properties) = class_properties(package, export) {
        while let Ok(Some((name, mut value))) = properties.next() {
            match name {
                name if name.eq_ignore_ascii_case("speed") => art.speed = value.f32().ok(),
                "Damage" => art.damage = value.f32().ok(),
                "MomentumTransfer" => {
                    art.momentum = value.u32().ok().map(|value| value as i32 as f32)
                }
                _ => {}
            }
        }
    }
    let (texture, package, sound, contact) = match kind {
        Kind::Pulse => ("pblst_a00", "botpack", "PulseExp", None),
        Kind::Rocket | Kind::Grenade => (
            "exp1_a00",
            "unrealshare",
            "Explo1",
            Some(("unrealshare", "GrenadeFloor")),
        ),
        Kind::Chunk => ("Chunk_a00", "botpack", "Hit1", Some(("botpack", "Hit1"))),
        Kind::FlakShell => ("Exp2_a00", "unrealshare", "Expl04", None),
        Kind::Razor => (
            "HEexpl1_a00",
            "unreali",
            "BladeHit",
            Some(("unreali", "BladeHit")),
        ),
        Kind::RazorAlt => ("HEexpl1_a00", "unrealshare", "Expl03", None),
        Kind::Warhead | Kind::GuidedWarhead => ("we_a00", "botpack", "WarExplo", None),
        _ => (
            "g1r_a00",
            "unrealshare",
            "Explg02",
            Some(("botpack", "GelHit")),
        ),
    };
    art.explosion = find_export(assets, "botpack", texture, "Texture")
        .and_then(|id| cached_texture(assets, textures, id));
    art.sound = cached_wave(assets, waves, find_export(assets, package, sound, "Sound"));
    art.contact = contact.and_then(|(package, sound)| {
        cached_wave(assets, waves, find_export(assets, package, sound, "Sound"))
    });
    if kind == Kind::Pulse {
        art.sprite = visual
            .texture
            .and_then(|id| cached_texture(assets, textures, id))
            .or_else(|| art.explosion.clone());
    } else if let Some(object) = visual.mesh {
        let (package, export) = assets.object(object);
        if let Ok(mesh) = Mesh::read(package, export) {
            art.flight = effect_motion(
                assets,
                textures,
                object,
                visual,
                &mesh,
                &["Flying", "Spin", "Wing", "WingIn", "Still", "All"],
            );
            art.stuck = effect_motion(assets, textures, object, visual, &mesh, &["Hit", "Still"]);
        }
    }
    if matches!(kind, Kind::Rocket | Kind::Warhead | Kind::GuidedWarhead) {
        art.trail = find_export(assets, "botpack", "JRFlare", "Texture")
            .and_then(|id| cached_texture(assets, textures, id));
    }
    if matches!(kind, Kind::Warhead | Kind::GuidedWarhead) {
        art.second_explosion = find_export(assets, "botpack", "ne_a00", "Texture")
            .and_then(|id| cached_texture(assets, textures, id));
        let class = find_export(assets, "botpack", "ShockWave", "Class");
        let wave = appearance(assets, classes, class);
        if let Some(object) = wave.mesh {
            let (package, export) = assets.object(object);
            if let Ok(mesh) = Mesh::read(package, export)
                && let Some(frame) = mesh.frame(0)
            {
                let vertices = effect_vertices(&mesh, frame, 1.0);
                art.wavefront = mesh_faces(assets, textures, object, wave, &mesh, &vertices);
            }
        }
    }
    art
}

fn effect_motion(
    assets: &mut Assets<'_>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    object: Object,
    visual: Appearance,
    mesh: &Mesh,
    names: &[&str],
) -> Motion {
    let Some(seq) = mesh.sequence(names) else {
        return Motion::default();
    };
    Motion {
        rate: seq.rate,
        frames: (seq.start..seq.end)
            .map(|frame| {
                let vertices =
                    effect_vertices(mesh, mesh.frame(frame).unwrap_or_default(), visual.scale);
                mesh_faces(assets, textures, object, visual, mesh, &vertices)
            })
            .collect(),
    }
}

fn pulse_beam(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    starter: bool,
) -> Option<Vec<Vec<Face>>> {
    let class = find_export(
        assets,
        "botpack",
        if starter { "StarterBolt" } else { "PBolt" },
        "Class",
    )?;
    let mut visual = appearance(assets, classes, Some(class));
    let object = visual.mesh?;
    let (package, export) = assets.object(object);
    let mesh = Mesh::read(package, export).ok()?;
    let mut vertices = effect_vertices(&mesh, mesh.frame(0)?, visual.scale);
    let min = vertices.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max = vertices
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    if max - min < 0.1 {
        return None;
    }
    for point in &mut vertices {
        point.0 = (point.0 - min) * 81.0 / (max - min);
    }
    let mut frames = Vec::new();
    let textures_names = if starter {
        ["sbolt0", "sbolt1", "sbolt2", "sbolt3", "sbolt4"]
    } else {
        ["pbolt0", "pbolt1", "pbolt2", "pbolt3", "pbolt4"]
    };
    for name in textures_names {
        visual.skin = find_export(assets, "botpack", name, "Texture");
        frames.push(mesh_faces(
            assets, textures, object, visual, &mesh, &vertices,
        ));
    }
    Some(frames)
}

fn read_shock(
    assets: &mut Assets<'_>,
    classes: &mut HashMap<Object, Appearance>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
    weapon: Appearance,
) -> shock::Assets {
    let mut art = shock::Assets::default();
    art.fire_offset = weapon.fire_offset.unwrap_or(art.fire_offset);
    if let Some(bolt) = find_export(assets, "botpack", "ShockProj", "Class") {
        let visual = appearance(assets, classes, Some(bolt));
        art.lifetime = visual.lifetime.unwrap_or(art.lifetime);
        art.bolt_scale = visual.scale;
        art.bolt = visual
            .texture
            .and_then(|id| cached_texture(assets, textures, id));
        let (package, export) = assets.object(bolt);
        let mut impact = 0;
        let mut combo = 0;
        if let Ok(mut properties) = class_properties(package, export) {
            while let Ok(Some((name, mut value))) = properties.next() {
                let property = match name {
                    "MomentumTransfer" => {
                        if let Ok(value) = value.u32() {
                            art.momentum = value as i32 as f32;
                        }
                        continue;
                    }
                    "Speed" => &mut art.speed,
                    "Damage" => &mut art.damage,
                    "CollisionRadius" => &mut art.radius,
                    "CollisionHeight" => &mut art.half_height,
                    "ImpactSound" => {
                        impact = value.compact().unwrap_or(0);
                        continue;
                    }
                    "ExploSound" => {
                        combo = value.compact().unwrap_or(0);
                        continue;
                    }
                    _ => continue,
                };
                if let Ok(value) = value.f32() {
                    *property = value;
                }
            }
        }
        let impact = assets.resolve(bolt.package, impact);
        let combo = assets.resolve(bolt.package, combo);
        art.hit_sound = cached_wave(assets, waves, impact);
        art.combo_sound = cached_wave(assets, waves, combo);
    }
    art.beam = (|| {
        let class = find_export(assets, "botpack", "ShockBeam", "Class")?;
        let visual = appearance(assets, classes, Some(class));
        let texture = cached_texture(assets, textures, visual.texture?)?;
        let (package, export) = assets.object(visual.mesh?);
        let mesh = Mesh::read(package, export).ok()?;
        let points = effect_vertices(&mesh, mesh.frame(0)?, visual.scale);
        Some(shock::BeamArt {
            texture,
            points,
            scale: visual.scale,
            lifetime: visual.lifetime?,
        })
    })();
    art.blast = (|| {
        let class = find_export(assets, "botpack", "ShockExplo", "Class")?;
        let visual = appearance(assets, classes, Some(class));
        let texture = cached_texture(assets, textures, visual.texture?)?;
        let (package, export) = assets.object(class);
        let mut properties = class_properties(package, export).ok()?;
        let mut sound = 0;
        while let Ok(Some((name, mut value))) = properties.next() {
            if name == "EffectSound1" {
                sound = value.compact().unwrap_or(0);
                break;
            }
        }
        let sound = assets.resolve(class.package, sound);
        Some(shock::BlastArt {
            texture,
            scale: visual.scale,
            lifetime: visual.lifetime?,
            sound: cached_wave(assets, waves, sound),
        })
    })();
    art.wavefront = (|| {
        let class = find_export(assets, "botpack", "ShockrifleWave", "Class")?;
        let visual = appearance(assets, classes, Some(class));
        let object = visual.mesh?;
        let (package, export) = assets.object(object);
        let mesh = Mesh::read(package, export).ok()?;
        let vertices = effect_vertices(&mesh, mesh.frame(0)?, visual.scale);
        Some(shock::WavefrontArt {
            faces: mesh_faces(assets, textures, object, visual, &mesh, &vertices),
            lifetime: visual.lifetime?,
        })
    })();
    for (index, (name, rate)) in [
        ("UT_RingExplosion", 0.35),
        ("UT_RingExplosion5", 0.35),
        ("UT_ComboRing", 0.1),
    ]
    .into_iter()
    .enumerate()
    {
        art.rings[index] = (|| {
            let class = find_export(assets, "botpack", name, "Class")?;
            let visual = appearance(assets, classes, Some(class));
            let object = visual.mesh?;
            let (package, export) = assets.object(object);
            let mesh = Mesh::read(package, export).ok()?;
            let seq = mesh.sequence(&["Explo"])?;
            let frames: Vec<_> = (seq.start..seq.end)
                .map(|frame| {
                    let vertices =
                        effect_vertices(&mesh, mesh.frame(frame).unwrap_or_default(), visual.scale);
                    mesh_faces(assets, textures, object, visual, &mesh, &vertices)
                })
                .collect();
            Some(shock::RingArt {
                motion: Motion {
                    frames,
                    rate: seq.rate * rate,
                },
                lifetime: visual.lifetime?,
            })
        })();
    }
    art
}

fn effect_vertices(mesh: &Mesh, frame: &[Vec3], scale: f32) -> Vec<Vec3> {
    frame
        .iter()
        .map(|&point| rotate(product(point - mesh.origin, mesh.scale), mesh.rotation) * scale)
        .collect()
}

fn read_player(
    assets: &mut Assets<'_>,
    waves: &mut HashMap<Object, Option<Arc<Wave>>>,
) -> (crate::movement::Profile, crate::movement::PlayerAudio) {
    let mut player = crate::movement::Profile::default();
    let mut audio = crate::movement::PlayerAudio::default();
    let mut class = find_export(assets, "botpack", "TMale2", "Class")
        .or_else(|| find_export(assets, "botpack", "TournamentPlayer", "Class"));
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    while let Some(object) = class.filter(|id| visited.insert(*id)) {
        chain.push(object);
        let (_, export) = assets.object(object);
        class = assets.resolve(object.package, export.superclass);
    }
    for object in chain.into_iter().rev() {
        let (package, export) = assets.object(object);
        let Ok(mut props) = class_properties(package, export) else {
            continue;
        };
        let mut references = Vec::new();
        while let Ok(Some((name, mut value))) = props.next() {
            let field = match name {
                "GroundSpeed" => &mut player.ground_speed,
                "WaterSpeed" => &mut player.water_speed,
                "AccelRate" => &mut player.acceleration,
                "AirControl" => &mut player.air_control,
                "JumpZ" => &mut player.jump_speed,
                "BaseEyeHeight" => &mut player.eye_height,
                "MaxStepHeight" => &mut player.step_height,
                "Bob" => &mut player.bob,
                "Mass" => &mut player.mass,
                "Buoyancy" => &mut player.buoyancy,
                "UnderWaterTime" => &mut player.underwater_time,
                "Footstep1" | "Footstep2" | "Footstep3" | "JumpSound" | "Land" | "WaterStep" => {
                    if let Ok(reference) = value.compact() {
                        references.push((name.to_owned(), reference));
                    }
                    continue;
                }
                _ => continue,
            };
            if let Ok(value) = value.f32() {
                *field = value.max(0.0);
            }
        }
        for (name, reference) in references {
            let wave = assets.resolve(object.package, reference);
            let sound = cached_wave(assets, waves, wave);
            match name.as_str() {
                "Footstep1" => audio.steps[0] = sound,
                "Footstep2" => audio.steps[1] = sound,
                "Footstep3" => audio.steps[2] = sound,
                "JumpSound" => audio.jump = sound,
                "Land" => audio.land = sound,
                "WaterStep" => audio.water_step = sound,
                _ => {}
            }
        }
    }
    (player, audio)
}

fn find_export(assets: &Assets<'_>, name: &str, object: &str, class: &str) -> Option<Object> {
    let &package = assets.names.get(name)?;
    let data = &assets.packages[package];
    let export = data.exports.iter().position(|item| {
        data.names[item.name].eq_ignore_ascii_case(object)
            && data
                .object_name(item.class)
                .unwrap_or("Class")
                .eq_ignore_ascii_case(class)
    })?;
    Some(Object { package, export })
}

fn cached_wave(
    assets: &Assets<'_>,
    cache: &mut HashMap<Object, Option<Arc<Wave>>>,
    object: Option<Object>,
) -> Option<Arc<Wave>> {
    let object = object?;
    cache
        .entry(object)
        .or_insert_with(|| {
            let (package, export) = assets.object(object);
            Wave::from_export(package, export).ok().map(Arc::new)
        })
        .clone()
}

fn cached_texture(
    assets: &Assets<'_>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    id: Object,
) -> Option<Arc<Texture>> {
    textures
        .entry(id)
        .or_insert_with(|| {
            let (package, export) = assets.object(id);
            Texture::read(package, export).ok().map(Arc::new)
        })
        .clone()
}

fn mesh_faces(
    assets: &mut Assets<'_>,
    textures: &mut HashMap<Object, Option<Arc<Texture>>>,
    object: Object,
    visual: Appearance,
    mesh: &Mesh,
    vertices: &[Vec3],
) -> Vec<Face> {
    let (package, export) = assets.object(object);
    let color = material_color(package.name(export.name).ok());
    let mut faces = Vec::new();
    for (index, tri) in mesh.triangles.iter().enumerate() {
        if tri.flags & 1 != 0 {
            continue;
        }
        let [Some(a), Some(b), Some(c)] = tri.vertices.map(|i| vertices.get(i).copied()) else {
            continue;
        };
        let normal = unit((b - a).cross(c - a));
        let ab = b - a;
        let ac = c - a;
        let cross = ac.cross(normal);
        let determinant = ab.dot(cross);
        if determinant.abs() < 0.00001 {
            continue;
        }
        let reference = mesh.textures.get(tri.texture).copied().unwrap_or(0);
        let texture_object = visual
            .multiskins
            .get(tri.texture)
            .copied()
            .flatten()
            .or(visual.skin.filter(|_| tri.texture == 0 || reference == 0))
            .or_else(|| assets.resolve(object.package, reference))
            .or(visual.texture)
            .or_else(|| visual.multiskins.iter().rev().copied().flatten().next());
        let texture = texture_object.and_then(|id| cached_texture(assets, textures, id));
        let (width, height) = texture.as_ref().map_or((256.0, 256.0), |t| t.size());
        let [uv0, uv1, uv2] = tri
            .uv
            .map(|[u, v]| [f32::from(u) * width / 256.0, f32::from(v) * height / 256.0]);
        let d1 = cross * determinant.recip();
        let d2 = normal.cross(ab) * determinant.recip();
        faces.push(Face {
            vertices: vec![a, b, c],
            mesh_triangle: Some(index),
            normal,
            color,
            flags: tri.flags,
            texture,
            lightmap: None,
            sky: false,
            origin: a,
            pan: uv0,
            u_axis: d1 * (uv1[0] - uv0[0]) + d2 * (uv2[0] - uv0[0]),
            v_axis: d1 * (uv1[1] - uv0[1]) + d2 * (uv2[1] - uv0[1]),
        });
    }
    faces
}

struct Triangle {
    vertices: [usize; 3],
    uv: [[u8; 2]; 3],
    flags: u32,
    texture: usize,
}

struct Sequence {
    name: String,
    start: usize,
    end: usize,
    rate: f32,
}

struct Mesh {
    frame_vertices: usize,
    sequences: Vec<Sequence>,
    vertices: Vec<Vec3>,
    triangles: Vec<Triangle>,
    textures: Vec<i32>,
    scale: Vec3,
    origin: Vec3,
    rotation: Vec3,
}

impl Mesh {
    fn frame(&self, index: usize) -> Option<&[Vec3]> {
        let start = index.checked_mul(self.frame_vertices)?;
        self.vertices
            .get(start..start.checked_add(self.frame_vertices)?)
    }

    fn sequence(&self, names: &[&str]) -> Option<&Sequence> {
        names.iter().find_map(|name| {
            self.sequences
                .iter()
                .find(|seq| seq.name.eq_ignore_ascii_case(name))
        })
    }
    fn read(package: &Package<'_>, export: &Export) -> io::Result<Self> {
        let kind = package.object_name(export.class);
        if !matches!(kind, Some("Mesh" | "LodMesh")) {
            return Err(invalid("Unsupported mesh class"));
        }
        let mut reader = native_data(package, export)?;
        let modern = package.version > 61;
        let bounds_size = if modern { 41 } else { 37 };
        reader.take(bounds_size)?;
        if modern {
            reader.u32()?;
        }
        let mut vertices = Vec::new();
        for _ in 0..reader.count(4)? {
            let packed = reader.u32()? as i32;
            vertices.push(Vec3(
                ((packed << 21) >> 21) as f32,
                ((packed << 10) >> 21) as f32,
                (packed >> 22) as f32,
            ));
        }
        if modern {
            reader.u32()?;
        }
        let mut triangles = Vec::new();
        for _ in 0..reader.count(20)? {
            let vertices = [
                reader.u16()? as usize,
                reader.u16()? as usize,
                reader.u16()? as usize,
            ];
            let uv = [
                [reader.byte()?, reader.byte()?],
                [reader.byte()?, reader.byte()?],
                [reader.byte()?, reader.byte()?],
            ];
            triangles.push(Triangle {
                vertices,
                uv,
                flags: reader.u32()?,
                texture: reader.u32()? as usize,
            });
        }
        let mut sequences = Vec::new();
        for _ in 0..reader.count(15)? {
            let name = package.name(reader.index()?)?.to_owned();
            reader.compact()?; // Group.
            let start = reader.u32()? as usize;
            let count = reader.u32()? as usize;
            for _ in 0..reader.count(5)? {
                reader.u32()?;
                reader.compact()?;
            }
            let rate = reader.f32()?;
            if count != 0 {
                sequences.push(Sequence {
                    name,
                    start,
                    end: start.saturating_add(count),
                    rate: if rate > 0.0 { rate } else { 30.0 },
                });
            }
        }
        if modern {
            reader.u32()?;
        }
        skip_array(&mut reader, 8)?;
        reader.take(bounds_size)?;
        if modern {
            reader.u32()?;
        }
        skip_array(&mut reader, 4)?;
        let textures = (0..reader.count(1)?)
            .map(|_| reader.compact())
            .collect::<io::Result<Vec<_>>>()?;
        skip_array(&mut reader, 25)?;
        skip_array(&mut reader, if modern { 16 } else { 12 })?;
        let frame_vertices = reader.u32()? as usize;
        reader.take(12)?; // Frame count and mesh flags.
        let scale = Vec3::read(&mut reader)?;
        let origin = Vec3::read(&mut reader)?;
        let rotation = read_rotation(&mut reader)?;
        reader.take(8)?; // Editor cursor.
        if package.version == 65 {
            reader.u32()?;
        }
        if package.version >= 66 {
            skip_array(&mut reader, 4)?;
        }
        if kind == Some("LodMesh") {
            triangles = read_lod(&mut reader)?;
        }
        if frame_vertices == 0 || frame_vertices > vertices.len() {
            return Err(invalid("Incomplete mesh frame"));
        }
        sequences.retain(|seq| seq.end <= vertices.len() / frame_vertices);
        Ok(Self {
            frame_vertices,
            sequences,
            vertices,
            triangles,
            textures,
            scale,
            origin,
            rotation,
        })
    }
}

fn read_lod(reader: &mut Reader<'_>) -> io::Result<Vec<Triangle>> {
    skip_array(reader, 2)?; // Collapse points.
    skip_array(reader, 2)?; // Face levels.
    let mut faces = Vec::new();
    for _ in 0..reader.count(8)? {
        let wedges = [
            reader.u16()? as usize,
            reader.u16()? as usize,
            reader.u16()? as usize,
        ];
        faces.push((wedges, reader.u16()? as usize));
    }
    skip_array(reader, 2)?;
    let mut wedges = Vec::new();
    for _ in 0..reader.count(4)? {
        wedges.push((reader.u16()? as usize, [reader.byte()?, reader.byte()?]));
    }
    let mut materials = Vec::new();
    for _ in 0..reader.count(8)? {
        materials.push((reader.u32()?, reader.u32()? as usize));
    }
    skip_array(reader, 8)?; // Special attachment faces.
    reader.u32()?; // Model vertex count.
    let special = reader.u32()? as usize;
    Ok(faces
        .into_iter()
        .filter_map(|(ids, material)| {
            let [Some(a), Some(b), Some(c)] = ids.map(|id| wedges.get(id).copied()) else {
                return None;
            };
            let (flags, texture) = *materials.get(material)?;
            Some(Triangle {
                vertices: [a.0 + special, b.0 + special, c.0 + special],
                uv: [a.1, b.1, c.1],
                flags,
                texture,
            })
        })
        .collect())
}

fn skip_array(reader: &mut Reader<'_>, width: usize) -> io::Result<()> {
    let count = reader.count(width)?;
    reader.take(count * width)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Map;

    #[test]
    #[ignore = "requires the local GOTY maps and System packages"]
    fn stock_meshes_inherit_their_visuals_and_keep_original_textures() {
        for (name, expected) in [
            ("DM-Deck16][.unr", 69),
            ("DM-Liandri.unr", 47),
            ("CTF-Face.unr", 77),
        ] {
            let path = Path::new("local/goty/Maps").join(name);
            let data = fs::read(&path).unwrap();
            let package = Package::read(&data).unwrap();
            let map = Map::load(&package, Some(&path)).unwrap();
            assert_eq!(map.props.len(), expected, "{name}");
            assert!(
                map.props
                    .iter()
                    .all(|prop| prop.radius.is_finite() && prop.radius > 0.0),
                "{name}"
            );
            assert!(
                map.props
                    .iter()
                    .filter(|prop| prop.faces.iter().any(|face| face.texture.is_some()))
                    .count()
                    > expected * 3 / 4,
                "{name}"
            );
        }
    }
}
