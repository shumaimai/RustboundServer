//! Hakoniwa H5: static fluids (no flow simulation).
//!
//! Water and lava are placed as fixed source blocks in map packs. Collision
//! treats them as non-solid so players can enter them. Movement gets light
//! water damping / buoyancy, and lava deals periodic contact damage.

use crate::hakoniwa::DimensionId;
use crate::world::World;

/// Still-water source block state (1.20.1 global palette default).
pub const WATER_BLOCK_STATE: i32 = 80;
/// Still-lava source block state (1.20.1 global palette default).
pub const LAVA_BLOCK_STATE: i32 = 96;

/// Inclusive max state ID for water levels (source … flowing).
const WATER_STATE_MAX: i32 = 95;
/// Inclusive max state ID for lava levels.
const LAVA_STATE_MAX: i32 = 111;

/// How strongly water slows downward motion (0 = none, 1 = stop).
pub const WATER_SINK_DAMPING: f64 = 0.72;
/// Upward nudge (blocks/tick) when fully submerged in water.
pub const WATER_BUOYANCY: f64 = 0.04;
/// Lava damage interval in ticks (10 = 0.5 s at 20 TPS).
pub const LAVA_DAMAGE_INTERVAL_TICKS: u64 = 10;
/// Hearts-worth of damage per lava tick (vanilla is ~4 / half-second roughly).
pub const LAVA_DAMAGE: f32 = 4.0;

/// True when `block_state` is any water level.
pub fn is_water(block_state: i32) -> bool {
    (WATER_BLOCK_STATE..=WATER_STATE_MAX).contains(&block_state)
}

/// True when `block_state` is any lava level.
pub fn is_lava(block_state: i32) -> bool {
    (LAVA_BLOCK_STATE..=LAVA_STATE_MAX).contains(&block_state)
}

/// True when the block is a fluid (non-solid for collision).
pub fn is_fluid(block_state: i32) -> bool {
    is_water(block_state) || is_lava(block_state)
}

/// Fluid occupying the block column at the player's feet (y).
pub fn fluid_at_feet(world: &World, dimension: DimensionId, x: f64, y: f64, z: f64) -> i32 {
    let bx = x.floor() as i32;
    let by = y.floor() as i32;
    let bz = z.floor() as i32;
    world.get_block(dimension, bx, by, bz)
}

/// Fluid at eye height (~1.62 above feet) for "submerged" checks.
pub fn fluid_at_eyes(world: &World, dimension: DimensionId, x: f64, y: f64, z: f64) -> i32 {
    let bx = x.floor() as i32;
    let by = (y + 1.62).floor() as i32;
    let bz = z.floor() as i32;
    world.get_block(dimension, bx, by, bz)
}

/// Applies static-water swim damping / buoyancy to a resolved position.
///
/// - Downward motion is slowed (you sink slowly instead of freefalling).
/// - When eyes are underwater, a small upward nudge helps surface swimming.
///
/// Returns `(x, y, z, corrected)`.
#[allow(clippy::too_many_arguments)]
pub fn apply_water_motion(
    world: &World,
    dimension: DimensionId,
    _old_x: f64,
    old_y: f64,
    _old_z: f64,
    x: f64,
    mut y: f64,
    z: f64,
) -> (f64, f64, f64, bool) {
    let feet = fluid_at_feet(world, dimension, x, y, z);
    if !is_water(feet) {
        // Also treat standing in water one block below eye when feet are in air
        // (waading): check body mid-point.
        let mid = world.get_block(
            dimension,
            x.floor() as i32,
            (y + 0.9).floor() as i32,
            z.floor() as i32,
        );
        if !is_water(mid) {
            return (x, y, z, false);
        }
    }

    let mut corrected = false;
    if y < old_y {
        let dy = old_y - y;
        let damped = dy * (1.0 - WATER_SINK_DAMPING);
        y = old_y - damped;
        corrected = true;
    }

    if is_water(fluid_at_eyes(world, dimension, x, y, z)) {
        y += WATER_BUOYANCY;
        corrected = true;
    }

    (x, y, z, corrected)
}

/// True when any part of the player's AABB overlaps lava.
pub fn touching_lava(world: &World, dimension: DimensionId, x: f64, y: f64, z: f64) -> bool {
    let half = 0.3;
    let min_x = (x - half).floor() as i32;
    let max_x = (x + half).floor() as i32;
    let min_y = y.floor() as i32;
    let max_y = (y + 1.8).floor() as i32;
    let min_z = (z - half).floor() as i32;
    let max_z = (z + half).floor() as i32;
    for by in min_y..=max_y {
        for bx in min_x..=max_x {
            for bz in min_z..=max_z {
                if is_lava(world.get_block(dimension, bx, by, bz)) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn water_and_lava_ranges() {
        assert!(is_water(WATER_BLOCK_STATE));
        assert!(is_water(95));
        assert!(!is_water(79));
        assert!(is_lava(LAVA_BLOCK_STATE));
        assert!(is_lava(111));
        assert!(!is_lava(112));
        assert!(is_fluid(WATER_BLOCK_STATE));
        assert!(is_fluid(LAVA_BLOCK_STATE));
    }

    #[test]
    fn water_damps_fall() {
        let mut world = World::new();
        let dim = DimensionId::Overworld;
        // Column of water at (0,64,0)
        world.set_block(dim, 0, 64, 0, WATER_BLOCK_STATE);
        world.set_block(dim, 0, 65, 0, WATER_BLOCK_STATE);
        let (x, y, z, corrected) = apply_water_motion(&world, dim, 0.5, 65.5, 0.5, 0.5, 64.0, 0.5);
        assert!(corrected);
        assert!((x - 0.5).abs() < f64::EPSILON);
        assert!((z - 0.5).abs() < f64::EPSILON);
        // Should not freefall all the way to 64.0 in one step.
        assert!(y > 64.0);
        assert!(y < 65.5);
    }

    #[test]
    fn lava_touch_detects_feet() {
        let mut world = World::new();
        let dim = DimensionId::Nether;
        world.set_block(dim, 0, 64, 0, LAVA_BLOCK_STATE);
        assert!(touching_lava(&world, dim, 0.5, 64.0, 0.5));
        assert!(!touching_lava(&world, dim, 5.5, 64.0, 5.5));
    }
}
