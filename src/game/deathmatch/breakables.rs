use crate::{
    assault::Effects,
    collision::Collision,
    game::{ballistics, shock::Blast},
    map::{Face, Map, Vec3},
    mover::rotate,
    render::Camera,
    sound::{CONTACT_SOUND_RADIUS, SoundEvents},
};

struct Wall {
    health: i32,
    broken: bool,
}

struct Fragment {
    wall: usize,
    material: usize,
    model: usize,
    scale: f32,
    position: Vec3,
    velocity: Vec3,
    rotation: Vec3,
    spin: Vec3,
    first_impact: bool,
    resting: bool,
    in_water: bool,
    expires: f32,
    phase: Phase,
}

enum Phase {
    Flying { until: f32 },
    Dying { check_at: f32 },
}

impl Fragment {
    fn begin_dying(&mut self, time: f32) {
        if matches!(self.phase, Phase::Flying { .. }) {
            self.phase = Phase::Dying {
                check_at: time + 1.5,
            };
        }
    }

    fn keep(&mut self, collision: &Collision, view: &Camera, time: f32) -> bool {
        if time >= self.expires {
            return false;
        }
        match &mut self.phase {
            Phase::Flying { until } if time >= *until => {
                let until = *until;
                self.begin_dying(until);
            }
            Phase::Dying { check_at } if time >= *check_at => {
                *check_at += 1.5;
                let offset = self.position - view.position;
                return offset.dot(view.look_direction()) > 0.0
                    && collision.ray(view.position, offset).is_none();
            }
            _ => {}
        }
        true
    }
}

pub(super) struct Debris {
    walls: Vec<Wall>,
    fragments: Vec<Fragment>,
    sounds: Vec<usize>,
    rng: u32,
}

impl Debris {
    pub fn new(data: &Effects, side: usize) -> Self {
        Self {
            walls: data
                .walls
                .iter()
                .map(|wall| Wall {
                    health: wall.health,
                    broken: false,
                })
                .collect(),
            fragments: Vec::new(),
            sounds: Vec::new(),
            rng: 0x3dac_17e5 ^ side as u32,
        }
    }

    pub fn targetable(&self, data: &Effects, index: usize) -> bool {
        let wall = &data.walls[index];
        !self.walls[index].broken && wall.projectile_target && !wall.trigger_only
    }

    pub fn signal(&mut self, data: &Effects, event: usize, time: f32) -> Vec<usize> {
        data.walls
            .iter()
            .enumerate()
            .filter_map(|(index, wall)| {
                if wall.tag != Some(event) {
                    return None;
                }
                self.break_wall(
                    data,
                    index,
                    time,
                    rotate(Vec3(1.0, 0.0, 0.0), wall.rotation),
                    None,
                )
            })
            .collect()
    }

    pub fn strike(
        &mut self,
        data: &Effects,
        index: usize,
        damage_type: &str,
        damage: u16,
        time: f32,
        momentum: Vec3,
    ) -> Option<usize> {
        if damage == 0 || self.walls[index].broken || !data.walls[index].accepts(damage_type) {
            return None;
        }
        let wall = &mut self.walls[index];
        wall.health = wall.health.saturating_sub(i32::from(damage));
        if wall.health > 0 {
            return None;
        }
        self.break_wall(data, index, time, momentum, None)
    }

    pub fn blast(
        &mut self,
        map: &Map,
        data: &Effects,
        blast: &Blast,
        damage_type: &str,
        scale: u16,
        time: f32,
    ) -> Vec<usize> {
        data.walls
            .iter()
            .enumerate()
            .filter_map(|(index, wall)| {
                if self.walls[index].broken || !wall.accepts(damage_type) {
                    return None;
                }
                let (damage, impulse) =
                    blast.hit_actor(&map.collision, wall.volume.position, wall.volume.radius)?;
                self.strike(
                    data,
                    index,
                    damage_type,
                    damage.saturating_mul(scale),
                    time,
                    impulse,
                )
            })
            .collect()
    }

    pub fn break_decoration(
        &mut self,
        data: &Effects,
        index: usize,
        time: f32,
        momentum: Vec3,
        size: f32,
    ) {
        self.break_wall(data, index, time, momentum, Some(size));
    }

    fn break_wall(
        &mut self,
        data: &Effects,
        index: usize,
        time: f32,
        momentum: Vec3,
        decoration_size: Option<f32>,
    ) -> Option<usize> {
        let state = &mut self.walls[index];
        if state.broken {
            return None;
        }
        state.broken = true;
        let wall = &data.walls[index];
        self.sounds.push(index);
        let location = wall.volume.position;
        for (kind, material) in wall.materials.iter().enumerate() {
            if material.shapes.is_empty() || material.scale == 0.0 {
                continue;
            }
            for _ in 0..material.count {
                let position = location + direction(&mut self.rng) * wall.spread;
                let velocity = if kind == 2 {
                    let random = direction(&mut self.rng);
                    Vec3(
                        random.0 * momentum.0,
                        random.1 * momentum.1,
                        random.2 * momentum.2,
                    ) * ((1.0 + roll(&mut self.rng)) * 0.0001)
                } else {
                    let extra = if decoration_size.is_some() {
                        momentum.dot(momentum).sqrt() / 80.0
                    } else {
                        wall.force
                    };
                    direction(&mut self.rng) * (extra + roll(&mut self.rng) * 150.0 + 100.0)
                        + Vec3(0.0, 0.0, if kind == 1 { wall.force * 0.5 } else { 0.0 })
                };
                let model = ((roll(&mut self.rng) * material.shapes.len() as f32) as usize)
                    .min(material.shapes.len() - 1);
                let spin =
                    direction(&mut self.rng) * (125_000.0 * std::f32::consts::TAU / 65_536.0);
                let minimum_spin = 10_000.0 * std::f32::consts::TAU / 65_536.0;
                let spin = Vec3(
                    if spin.0.abs() < minimum_spin {
                        minimum_spin
                    } else {
                        spin.0
                    },
                    spin.1,
                    if spin.2.abs() < minimum_spin {
                        minimum_spin
                    } else {
                        spin.2
                    },
                );
                self.fragments.push(Fragment {
                    wall: index,
                    material: kind,
                    model,
                    scale: decoration_size.map_or(material.scale, |size| {
                        size * (0.5 + 0.7 * roll(&mut self.rng))
                    }),
                    position,
                    velocity,
                    rotation: Vec3::default(),
                    spin,
                    first_impact: true,
                    resting: false,
                    in_water: false,
                    expires: time + 20.0 + 40.0 * roll(&mut self.rng),
                    phase: Phase::Flying { until: time + 5.0 },
                });
            }
        }
        wall.event
    }

    pub fn tick(
        &mut self,
        map: &Map,
        data: &Effects,
        view: &Camera,
        time: f32,
        dt: f32,
        sounds: &mut SoundEvents,
    ) {
        for index in self.sounds.drain(..) {
            let wall = &data.walls[index];
            sounds.extend_at(
                wall.sound.iter().cloned(),
                wall.volume.position,
                CONTACT_SOUND_RADIUS,
                1.0,
            );
        }
        let rng = &mut self.rng;
        self.fragments.retain_mut(|fragment| {
            if !fragment.keep(&map.collision, view, time) {
                return false;
            }
            if fragment.resting {
                return true;
            }
            let zone = map.collision.regions.at(fragment.position);
            if zone.pain && zone.damage_per_second > 0 {
                return false;
            }
            if zone.water && !fragment.in_water && matches!(fragment.phase, Phase::Flying { .. }) {
                fragment.velocity = fragment.velocity * 0.2;
                fragment.spin = fragment.spin * 0.2;
                if let Some([entry, _]) = zone.liquid_audio.and_then(|id| map.liquid_audio.get(id))
                {
                    let gain = (0.0005 * (250.0 - 0.5 * fragment.velocity.2)).max(0.0);
                    sounds.extend_at(
                        entry.iter().cloned(),
                        fragment.position,
                        CONTACT_SOUND_RADIUS,
                        gain,
                    );
                }
                fragment.begin_dying(time);
            }
            fragment.in_water = zone.water;
            let material = &data.walls[fragment.wall].materials[fragment.material];
            let gravity = zone.gravity;
            let travel = (fragment.velocity + gravity * (dt * 0.5)) * dt;
            fragment.velocity = fragment.velocity + gravity * dt;
            fragment.rotation = fragment.rotation + fragment.spin * dt;
            let Some((impact, normal)) = map.collision.projectile_surface(
                fragment.position,
                travel,
                material.collision[0],
                material.collision[1],
            ) else {
                fragment.position = fragment.position + travel;
                return true;
            };
            fragment.position = impact + normal * 0.2;
            fragment.velocity =
                (fragment.velocity - normal * (2.0 * fragment.velocity.dot(normal))) * 0.5;
            fragment.spin = fragment.spin * 0.75;
            let speed_squared = fragment.velocity.dot(fragment.velocity);
            let flying = matches!(fragment.phase, Phase::Flying { .. });
            if fragment.first_impact && speed_squared < 400.0_f32.powi(2) {
                fragment.first_impact = false;
                fragment.rotation = Vec3(0.0, fragment.rotation.1, 0.0);
                fragment.spin = Vec3(0.0, fragment.spin.1, 0.0);
            }
            let settled = if flying {
                speed_squared < 60.0_f32.powi(2)
            } else {
                fragment.velocity.2 < 50.0
            };
            let impact_speed: f32 = if flying { 50.0 } else { 80.0 };
            if settled && normal.2 > 0.7 {
                fragment.resting = true;
                fragment.begin_dying(time);
            } else if speed_squared > impact_speed.powi(2) {
                let sound = usize::from(roll(rng) >= 0.5);
                sounds.extend_at(
                    material.impacts[sound].iter().cloned(),
                    impact,
                    300.0,
                    0.5 + roll(rng) * 0.5,
                );
            }
            true
        });
    }

    pub fn visuals<'a>(
        &'a self,
        data: &'a Effects,
    ) -> impl Iterator<Item = (Vec3, Vec3, f32, &'a [Face])> + 'a {
        self.fragments.iter().map(|fragment| {
            let material = &data.walls[fragment.wall].materials[fragment.material];
            (
                fragment.position,
                fragment.rotation,
                fragment.scale,
                material.shapes[fragment.model].as_slice(),
            )
        })
    }
}

fn roll(seed: &mut u32) -> f32 {
    (ballistics::jitter(seed) + 1.0) * 0.5
}

fn direction(seed: &mut u32) -> Vec3 {
    let z = ballistics::jitter(seed);
    let yaw = roll(seed) * std::f32::consts::TAU;
    let radius = (1.0 - z * z).max(0.0).sqrt();
    Vec3(radius * yaw.cos(), radius * yaw.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assault::Target,
        game::deathmatch::{Command, Match, Mode, Rules},
        package::Package,
    };
    use std::path::Path;

    #[test]
    fn settled_fragments_check_visibility_and_still_expire() {
        let mut fragment = Fragment {
            wall: 0,
            material: 0,
            model: 0,
            scale: 1.0,
            position: Vec3(100.0, 0.0, 0.0),
            velocity: Vec3::default(),
            rotation: Vec3::default(),
            spin: Vec3::default(),
            first_impact: false,
            resting: true,
            in_water: false,
            expires: 20.0,
            phase: Phase::Flying { until: 5.0 },
        };
        let mut collision = Collision::test_world();
        let mut view = Camera {
            position: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
        };
        fragment.begin_dying(1.0);
        assert!(fragment.keep(&collision, &view, 2.5));
        view.yaw = std::f32::consts::PI;
        assert!(fragment.keep(&collision, &view, 3.99));
        assert!(!fragment.keep(&collision, &view, 4.0));
        view.yaw = 0.0;
        collision.add_box(Vec3(30.0, -10.0, -10.0), Vec3(40.0, 10.0, 10.0));
        assert!(!fragment.keep(&collision, &view, 5.5));
        assert!(!fragment.keep(&collision, &view, 20.0));
    }

    #[test]
    #[ignore = "requires the local reference-client map installation"]
    fn mazon_crystal_uses_its_stock_fragment_count_and_resets_between_rounds() {
        let path = Path::new("local/reference-client/Maps/AS-Mazon.unr");
        let bytes = std::fs::read(path).unwrap();
        let package = Package::read(&bytes).unwrap();
        let mut map = Map::load(&package, Some(path)).unwrap();
        let data = map.assault.as_ref().unwrap();
        let (index, wall) = data
            .effects
            .walls
            .iter()
            .enumerate()
            .find(|(_, wall)| {
                wall.tag
                    .is_some_and(|tag| package.names[tag].eq_ignore_ascii_case("Crystal"))
            })
            .unwrap();
        let tag = wall.tag.unwrap();
        let fort = data
            .forts
            .iter()
            .position(|target| target.event == Some(tag))
            .unwrap();
        let total: usize = wall.materials.iter().map(|material| material.count).sum();
        assert_eq!(total, 45);
        assert!(wall.materials[0].shapes.len() > 1);
        assert!(
            wall.materials[0]
                .shapes
                .iter()
                .flatten()
                .all(|face| face.texture.is_some())
        );
        let mut game = Match::new(
            &map,
            0,
            Rules {
                mode: Mode::Assault,
                bots: 0,
                ..Rules::default()
            },
        );
        game.damage_objective(&map, 0, Target::Wall(index), 1000, "all");
        assert!(
            game.assault_debris(&map).next().is_none(),
            "the crystal fragments are released by the objective's event"
        );
        game.damage_objective(&map, 0, Target::Fort(fort), 1000, "exploded");
        game.step(&mut map, Command::default(), 1.0 / 120.0);
        let round = game.assault.as_ref().unwrap();
        assert!(round.debris.walls[index].broken);
        assert_eq!(round.debris.fragments.len(), total);
        assert_eq!(game.assault_debris(&map).count(), total);
        let start = round.debris.fragments[0].position;
        game.assault.as_mut().unwrap().emit(game.time, tag, true);
        game.step(&mut map, Command::default(), 1.0 / 120.0);
        let round = game.assault.as_ref().unwrap();
        assert_eq!(round.debris.fragments.len(), total);
        let movement = round.debris.fragments[0].position - start;
        assert!(movement.dot(movement) > 0.0);
        game.time = round.limit;
        game.step(&mut map, Command::default(), 1.0 / 120.0);
        let round = game.assault.as_ref().unwrap();
        assert_eq!(round.attack_team, 1);
        assert!(!round.debris.walls[index].broken && round.debris.fragments.is_empty());
    }
}
