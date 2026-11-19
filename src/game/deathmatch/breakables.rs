use crate::{
    assault::Effects,
    game::{ballistics, shock::Blast},
    map::{Face, Map, Vec3},
    mover::rotate,
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
    position: Vec3,
    velocity: Vec3,
    rotation: Vec3,
    spin: Vec3,
    first_impact: bool,
    resting: bool,
    in_water: bool,
    expires: f32,
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
        self.break_wall(data, index, time, momentum)
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

    fn break_wall(
        &mut self,
        data: &Effects,
        index: usize,
        time: f32,
        momentum: Vec3,
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
                    direction(&mut self.rng) * (wall.force + roll(&mut self.rng) * 150.0 + 100.0)
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
                    position,
                    velocity,
                    rotation: Vec3::default(),
                    spin,
                    first_impact: true,
                    resting: false,
                    in_water: false,
                    expires: time + 20.0 + 40.0 * roll(&mut self.rng),
                });
            }
        }
        wall.event
    }

    pub fn tick(
        &mut self,
        map: &Map,
        data: &Effects,
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
            if time >= fragment.expires {
                return false;
            }
            if fragment.resting {
                return true;
            }
            let zone = map.collision.regions.at(fragment.position);
            if zone.pain {
                return false;
            }
            if zone.water && !fragment.in_water {
                fragment.velocity = fragment.velocity * 0.2;
                fragment.spin = fragment.spin * 0.2;
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
            if fragment.first_impact && speed_squared < 400.0_f32.powi(2) {
                fragment.first_impact = false;
                fragment.rotation = Vec3(0.0, fragment.rotation.1, 0.0);
                fragment.spin = Vec3(0.0, fragment.spin.1, 0.0);
            }
            if speed_squared < 60.0_f32.powi(2) && normal.2 > 0.7 {
                fragment.resting = true;
            } else if speed_squared > 50.0_f32.powi(2) {
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
                material.scale,
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
