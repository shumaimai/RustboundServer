//! Hakoniwa H4: simple decorative and lightly hostile mobs.
//!
//! No pathfinding / full AI — wander toward random garden points, and a few
//! hostile kinds that slowly chase and contact-damage players.

use crate::hakoniwa::{DimensionId, GardenSpec};
use rustbound_protocol::primitives::Uuid;

/// Protocol entity-type IDs for Minecraft Java 1.20.1 (`minecraft:entity_type`).
pub mod entity_type {
    /// `minecraft:pig`
    pub const PIG: i32 = 72;
    /// `minecraft:cow`
    pub const COW: i32 = 18;
    /// `minecraft:zombie`
    pub const ZOMBIE: i32 = 118;
    /// `minecraft:zombified_piglin`
    pub const ZOMBIFIED_PIGLIN: i32 = 121;
    /// `minecraft:enderman`
    pub const ENDERMAN: i32 = 29;
}

/// Kind of hakoniwa mob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    /// Passive overworld pig (wander).
    Pig,
    /// Passive overworld cow (wander).
    Cow,
    /// Hostile overworld zombie (chase + contact damage).
    Zombie,
    /// Mildly hostile nether zombified piglin.
    ZombifiedPiglin,
    /// End enderman (wander; light chase when close).
    Enderman,
}

impl MobKind {
    /// Registry entity-type ID.
    pub fn entity_type_id(self) -> i32 {
        match self {
            Self::Pig => entity_type::PIG,
            Self::Cow => entity_type::COW,
            Self::Zombie => entity_type::ZOMBIE,
            Self::ZombifiedPiglin => entity_type::ZOMBIFIED_PIGLIN,
            Self::Enderman => entity_type::ENDERMAN,
        }
    }

    /// Maximum health points.
    pub fn max_health(self) -> f32 {
        match self {
            Self::Pig | Self::Cow => 10.0,
            Self::Zombie | Self::ZombifiedPiglin => 20.0,
            Self::Enderman => 40.0,
        }
    }

    /// Movement speed in blocks per tick while wandering / chasing.
    pub fn speed(self) -> f64 {
        match self {
            Self::Pig | Self::Cow => 0.08,
            Self::Zombie => 0.12,
            Self::ZombifiedPiglin => 0.11,
            Self::Enderman => 0.14,
        }
    }

    /// True when this kind chases players.
    pub fn is_hostile(self) -> bool {
        matches!(self, Self::Zombie | Self::ZombifiedPiglin | Self::Enderman)
    }

    /// Contact damage dealt to a survival player.
    pub fn contact_damage(self) -> f32 {
        match self {
            Self::Pig | Self::Cow => 0.0,
            Self::Zombie => 2.0,
            Self::ZombifiedPiglin => 3.0,
            Self::Enderman => 4.0,
        }
    }

    /// Preferred home dimension for built-in spawns.
    pub fn home_dimension(self) -> DimensionId {
        match self {
            Self::Pig | Self::Cow | Self::Zombie => DimensionId::Overworld,
            Self::ZombifiedPiglin => DimensionId::Nether,
            Self::Enderman => DimensionId::End,
        }
    }
}

/// A living mob tracked by the tick loop.
#[derive(Debug, Clone)]
pub struct Mob {
    /// Entity ID shared with the protocol.
    pub entity_id: i32,
    /// Stable UUID for Spawn Entity.
    pub uuid: Uuid,
    /// Kind / species.
    pub kind: MobKind,
    /// Dimension the mob currently lives in.
    pub dimension: DimensionId,
    /// Feet X.
    pub x: f64,
    /// Feet Y.
    pub y: f64,
    /// Feet Z.
    pub z: f64,
    /// Body yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Remaining health.
    pub health: f32,
    /// Optional wander destination (x, z); Y stays on the plateau.
    pub wander_target: Option<(f64, f64)>,
    /// Ticks until the next wander retarget.
    pub retarget_ticks: u32,
    /// Ticks until the mob may deal contact damage again.
    pub attack_cooldown: u32,
}

impl Mob {
    /// Creates a mob at the given feet position.
    pub fn new(entity_id: i32, kind: MobKind, x: f64, y: f64, z: f64) -> Self {
        Self {
            entity_id,
            uuid: Uuid::new(0x4D4F_4200, i64::from(entity_id)), // "MOB\0" namespace-ish
            kind,
            dimension: kind.home_dimension(),
            x,
            y,
            z,
            yaw: 0.0,
            pitch: 0.0,
            health: kind.max_health(),
            wander_target: None,
            retarget_ticks: 0,
            attack_cooldown: 0,
        }
    }

    /// Returns true if health has dropped to zero or below.
    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }

    /// Applies melee damage from a player; returns true if the mob died.
    pub fn hurt(&mut self, amount: f32) -> bool {
        self.health -= amount;
        self.is_dead()
    }
}

/// Built-in garden population for H4.
pub fn initial_garden_mobs(next_entity_id: &mut i32) -> Vec<Mob> {
    let mut out = Vec::new();
    let spawns: &[(MobKind, f64, f64, f64)] = &[
        (MobKind::Pig, 4.5, 64.0, 4.5),
        (MobKind::Pig, -3.5, 64.0, 5.5),
        (MobKind::Cow, 7.5, 64.0, -3.5),
        (MobKind::Zombie, 14.5, 64.0, 14.5),
        (MobKind::ZombifiedPiglin, 3.5, 64.0, 3.5),
        (MobKind::Enderman, 8.5, 64.0, 8.5),
    ];
    for &(kind, x, y, z) in spawns {
        let id = *next_entity_id;
        *next_entity_id += 1;
        out.push(Mob::new(id, kind, x, y, z));
    }
    out
}

/// Deterministic LCG used for wander retargets (no `rand` dependency).
fn next_u32(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

fn next_f64(state: &mut u32) -> f64 {
    f64::from(next_u32(state)) / f64::from(u32::MAX)
}

/// One AI tick for a single mob. Returns whether position/rotation changed.
pub fn tick_mob(
    mob: &mut Mob,
    garden: &GardenSpec,
    rng: &mut u32,
    nearest_player: Option<(f64, f64, f64)>,
) -> bool {
    if mob.attack_cooldown > 0 {
        mob.attack_cooldown -= 1;
    }

    let old = (mob.x, mob.y, mob.z, mob.yaw);

    // Hostile chase when a player is in range (same dimension assumed by caller).
    let mut chasing = false;
    if mob.kind.is_hostile() {
        if let Some((px, _py, pz)) = nearest_player {
            let dx = px - mob.x;
            let dz = pz - mob.z;
            let dist_sq = dx * dx + dz * dz;
            let aggro = match mob.kind {
                MobKind::Enderman => 8.0,
                _ => 16.0,
            };
            if dist_sq < aggro * aggro && dist_sq > 0.01 {
                chasing = true;
                let dist = dist_sq.sqrt();
                let speed = mob.kind.speed();
                mob.x += dx / dist * speed;
                mob.z += dz / dist * speed;
                mob.yaw = (dz.atan2(dx).to_degrees() as f32) - 90.0;
                mob.wander_target = None;
            }
        }
    }

    if !chasing {
        if mob.retarget_ticks == 0 || mob.wander_target.is_none() {
            mob.retarget_ticks = 40 + (next_u32(rng) % 60);
            let min_x = f64::from(garden.min_block_x) + 1.5;
            let max_x = f64::from(garden.max_block_x) - 0.5;
            let min_z = f64::from(garden.min_block_z) + 1.5;
            let max_z = f64::from(garden.max_block_z) - 0.5;
            let tx = min_x + next_f64(rng) * (max_x - min_x);
            let tz = min_z + next_f64(rng) * (max_z - min_z);
            mob.wander_target = Some((tx, tz));
        } else {
            mob.retarget_ticks -= 1;
        }

        if let Some((tx, tz)) = mob.wander_target {
            let dx = tx - mob.x;
            let dz = tz - mob.z;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < 0.25 {
                mob.wander_target = None;
            } else {
                let dist = dist_sq.sqrt();
                let speed = mob.kind.speed();
                mob.x += dx / dist * speed;
                mob.z += dz / dist * speed;
                mob.yaw = (dz.atan2(dx).to_degrees() as f32) - 90.0;
            }
        }
    }

    // Keep feet on the garden surface and inside the border.
    let (cx, _, cz) = garden.clamp_horizontal(mob.x, mob.y, mob.z);
    mob.x = cx;
    mob.z = cz;
    mob.y = garden.surface_y;

    old != (mob.x, mob.y, mob.z, mob.yaw)
}

/// Returns true when the mob is close enough to strike `player`.
pub fn in_melee_range(mob: &Mob, px: f64, py: f64, pz: f64) -> bool {
    let dx = px - mob.x;
    let dy = py - mob.y;
    let dz = pz - mob.z;
    dx * dx + dy * dy + dz * dz < 2.25 // ~1.5 blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hakoniwa::MapSize;

    #[test]
    fn initial_mobs_cover_three_dimensions() {
        let mut next = 100;
        let mobs = initial_garden_mobs(&mut next);
        assert!(mobs.iter().any(|m| m.kind == MobKind::Pig));
        assert!(mobs.iter().any(|m| m.kind == MobKind::Zombie));
        assert!(mobs.iter().any(|m| m.dimension == DimensionId::Nether));
        assert!(mobs.iter().any(|m| m.dimension == DimensionId::End));
        assert_eq!(next, 100 + mobs.len() as i32);
    }

    #[test]
    fn wander_moves_within_garden() {
        let garden = GardenSpec::from_size(MapSize::Tiny);
        let mut mob = Mob::new(1, MobKind::Pig, 0.5, 64.0, 0.5);
        let mut rng = 1u32;
        for _ in 0..200 {
            tick_mob(&mut mob, &garden, &mut rng, None);
        }
        assert!(garden.contains_block(mob.x.floor() as i32, mob.z.floor() as i32));
        assert!((mob.y - 64.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zombie_chases_player() {
        let garden = GardenSpec::from_size(MapSize::Tiny);
        let mut mob = Mob::new(1, MobKind::Zombie, 0.5, 64.0, 0.5);
        let mut rng = 42u32;
        let before = mob.x;
        for _ in 0..30 {
            tick_mob(&mut mob, &garden, &mut rng, Some((10.0, 64.0, 0.5)));
        }
        assert!(mob.x > before, "zombie should move toward the player");
    }

    #[test]
    fn hurt_kills_at_zero() {
        let mut mob = Mob::new(1, MobKind::Pig, 0.0, 64.0, 0.0);
        assert!(!mob.hurt(9.0));
        assert!(mob.hurt(2.0));
        assert!(mob.is_dead());
    }
}
