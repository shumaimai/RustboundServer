//! Minimal axis-separated player–block collision for hakoniwa H1.
//!
//! Uses public player dimensions (width 0.6, height 1.8) and treats any
//! non-air block state as a full solid cube. Spectator mode skips collision.

use crate::chunk::AIR_BLOCK_STATE;
use crate::fluid;
use crate::world::World;

/// Spectator gamemode wire value (matches `tick::GAMEMODE_SPECTATOR`).
const GAMEMODE_SPECTATOR: u8 = 3;

/// Player collision box half-width (total width 0.6).
pub const PLAYER_HALF_WIDTH: f64 = 0.3;
/// Player standing height.
pub const PLAYER_HEIGHT: f64 = 1.8;
/// Epsilon when seating the player on top of a block.
const SURFACE_EPS: f64 = 1.0e-4;
/// Maximum horizontal blocks to scan for collisions along one move.
/// Generous so chunk-crossing and creative flight packets still apply.
const MAX_HORIZONTAL_STEP: f64 = 32.0;
/// Maximum fall distance resolved in one correction (void handled separately).
const MAX_FALL: f64 = 64.0;
/// Maximum upward step in one correction.
const MAX_RISE: f64 = 2.0;

/// Result of resolving a movement against solid blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionResult {
    /// Corrected X.
    pub x: f64,
    /// Corrected Y.
    pub y: f64,
    /// Corrected Z.
    pub z: f64,
    /// True when standing on a solid after the move.
    pub on_ground: bool,
    /// True when the resolved position differs from the proposal.
    pub corrected: bool,
}

/// Returns true if the block state is a full solid for collision.
///
/// Air and static fluids (water / lava) are non-solid so players can enter them.
pub fn is_solid(block_state: i32) -> bool {
    block_state != AIR_BLOCK_STATE && !fluid::is_fluid(block_state)
}

/// Axis-aligned player box at feet position `(x, y, z)`.
#[derive(Debug, Clone, Copy)]
struct Aabb {
    min_x: f64,
    min_y: f64,
    min_z: f64,
    max_x: f64,
    max_y: f64,
    max_z: f64,
}

impl Aabb {
    fn from_feet(x: f64, y: f64, z: f64) -> Self {
        Self {
            min_x: x - PLAYER_HALF_WIDTH,
            min_y: y,
            min_z: z - PLAYER_HALF_WIDTH,
            max_x: x + PLAYER_HALF_WIDTH,
            max_y: y + PLAYER_HEIGHT,
            max_z: z + PLAYER_HALF_WIDTH,
        }
    }

    fn overlaps_block(self, bx: i32, by: i32, bz: i32) -> bool {
        let bmin_x = f64::from(bx);
        let bmin_y = f64::from(by);
        let bmin_z = f64::from(bz);
        let bmax_x = bmin_x + 1.0;
        let bmax_y = bmin_y + 1.0;
        let bmax_z = bmin_z + 1.0;
        self.min_x < bmax_x
            && self.max_x > bmin_x
            && self.min_y < bmax_y
            && self.max_y > bmin_y
            && self.min_z < bmax_z
            && self.max_z > bmin_z
    }
}

fn floor_i(v: f64) -> i32 {
    v.floor() as i32
}

/// Block index range overlapping `[min, max)` along one axis.
fn block_range(min: f64, max: f64) -> std::ops::RangeInclusive<i32> {
    let start = floor_i(min);
    // Exclusive max: if max is an integer, the face-touching next block is not included.
    let end = if (max - max.round()).abs() < 1e-9 {
        (max.round() as i32) - 1
    } else {
        floor_i(max)
    };
    start..=end.max(start)
}

fn aabb_hits_solid(world: &World, dimension: crate::hakoniwa::DimensionId, box_: Aabb) -> bool {
    for by in block_range(box_.min_y, box_.max_y) {
        for bx in block_range(box_.min_x, box_.max_x) {
            for bz in block_range(box_.min_z, box_.max_z) {
                if is_solid(world.get_block(dimension, bx, by, bz))
                    && box_.overlaps_block(bx, by, bz)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn standing_on_ground(
    world: &World,
    dimension: crate::hakoniwa::DimensionId,
    x: f64,
    y: f64,
    z: f64,
) -> bool {
    // Probe a few centimeters below the feet so seating on a block top
    // (y ≈ N + eps) still sees block N-1 / the solid beneath.
    let probe_y = y - 0.05;
    let by = floor_i(probe_y);
    let half = PLAYER_HALF_WIDTH;
    for bx in block_range(x - half, x + half) {
        for bz in block_range(z - half, z + half) {
            if is_solid(world.get_block(dimension, bx, by, bz)) {
                return true;
            }
        }
    }
    false
}

/// Resolves movement from `old` toward `new` against solid blocks.
///
/// Order: X, then Z, then Y (simple; good enough for flat gardens and short falls).
#[allow(clippy::too_many_arguments)]
pub fn resolve_movement(
    world: &World,
    dimension: crate::hakoniwa::DimensionId,
    old_x: f64,
    old_y: f64,
    old_z: f64,
    new_x: f64,
    new_y: f64,
    new_z: f64,
    gamemode: u8,
) -> CollisionResult {
    if gamemode == GAMEMODE_SPECTATOR {
        return CollisionResult {
            x: new_x,
            y: new_y,
            z: new_z,
            on_ground: false,
            corrected: false,
        };
    }

    // Guard absurd teleports (keep within a sane step of old).
    let dx = (new_x - old_x).clamp(-MAX_HORIZONTAL_STEP, MAX_HORIZONTAL_STEP);
    let dy = (new_y - old_y).clamp(-MAX_FALL, MAX_RISE);
    let dz = (new_z - old_z).clamp(-MAX_HORIZONTAL_STEP, MAX_HORIZONTAL_STEP);
    let mut x = old_x + dx;
    let mut y = old_y + dy;
    let mut z = old_z + dz;

    // X axis
    if (x - old_x).abs() > f64::EPSILON {
        let try_box = Aabb::from_feet(x, old_y, old_z);
        if aabb_hits_solid(world, dimension, try_box) {
            x = old_x;
        }
    }

    // Z axis
    if (z - old_z).abs() > f64::EPSILON {
        let try_box = Aabb::from_feet(x, old_y, z);
        if aabb_hits_solid(world, dimension, try_box) {
            z = old_z;
        }
    }

    // Y axis
    if (y - old_y).abs() > f64::EPSILON {
        let try_box = Aabb::from_feet(x, y, z);
        if aabb_hits_solid(world, dimension, try_box) {
            if y < old_y {
                // Falling: sit on the highest solid top below the old feet.
                y = snap_to_ground(world, dimension, x, old_y, z);
            } else {
                // Rising into ceiling
                y = old_y;
            }
        }
    }

    // If still intersecting (started inside a block), push up onto surface.
    if aabb_hits_solid(world, dimension, Aabb::from_feet(x, y, z)) {
        y = snap_to_ground(world, dimension, x, y + PLAYER_HEIGHT, z);
    }

    let on_ground = standing_on_ground(world, dimension, x, y, z);
    let corrected = (x - new_x).abs() > f64::EPSILON
        || (y - new_y).abs() > f64::EPSILON
        || (z - new_z).abs() > f64::EPSILON;

    CollisionResult {
        x,
        y,
        z,
        on_ground,
        corrected,
    }
}

fn snap_to_ground(
    world: &World,
    dimension: crate::hakoniwa::DimensionId,
    x: f64,
    from_y: f64,
    z: f64,
) -> f64 {
    // Search downward from from_y for a solid top within MAX_FALL.
    let start = floor_i(from_y);
    let min_y = start - (MAX_FALL as i32);
    for by in (min_y..=start).rev() {
        for bx in block_range(x - PLAYER_HALF_WIDTH, x + PLAYER_HALF_WIDTH) {
            for bz in block_range(z - PLAYER_HALF_WIDTH, z + PLAYER_HALF_WIDTH) {
                if is_solid(world.get_block(dimension, bx, by, bz)) {
                    let top = f64::from(by) + 1.0;
                    let candidate = top + SURFACE_EPS;
                    if !aabb_hits_solid(world, dimension, Aabb::from_feet(x, candidate, z)) {
                        return candidate;
                    }
                }
            }
        }
    }
    from_y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{FLAT_STONE_MAX_Y, STONE_BLOCK_STATE};

    #[test]
    fn air_is_not_solid_stone_is() {
        assert!(!is_solid(AIR_BLOCK_STATE));
        assert!(is_solid(STONE_BLOCK_STATE));
        assert!(!is_solid(fluid::WATER_BLOCK_STATE));
        assert!(!is_solid(fluid::LAVA_BLOCK_STATE));
    }

    #[test]
    fn player_can_enter_water_column() {
        let mut world = World::new();
        let dim = crate::hakoniwa::DimensionId::Overworld;
        // Clear stone under a water column and fill with water.
        for y in 61..=65 {
            world.set_block(dim, 2, y, 0, fluid::WATER_BLOCK_STATE);
        }
        let r = resolve_movement(&world, dim, 0.5, 64.0, 0.5, 2.5, 64.0, 0.5, 0);
        // Should be able to walk into the water x without being blocked by fluid.
        assert!(
            (r.x - 2.5).abs() < 0.01,
            "water must not act as a solid wall, got x={}",
            r.x
        );
    }

    #[test]
    fn falling_into_water_does_not_force_server_correction() {
        let mut world = World::new();
        let dim = crate::hakoniwa::DimensionId::Overworld;
        for y in 61..=63 {
            world.set_block(dim, 0, y, 0, fluid::WATER_BLOCK_STATE);
        }
        // Client proposes sinking into the water column — server must accept it
        // without rewriting Y (which would spam teleports / lock look).
        let r = resolve_movement(&world, dim, 0.5, 64.0, 0.5, 0.5, 62.2, 0.5, 0);
        assert!(
            !r.corrected,
            "water entry must not mark corrected, y={}",
            r.y
        );
        assert!((r.y - 62.2).abs() < f64::EPSILON);
    }

    #[test]
    fn netherrack_and_end_stone_are_solid() {
        assert!(is_solid(crate::chunk::NETHERRACK_BLOCK_STATE));
        assert!(is_solid(crate::chunk::END_STONE_BLOCK_STATE));
        assert!(is_solid(crate::chunk::GLOWSTONE_BLOCK_STATE));
        assert!(!is_solid(fluid::WATER_BLOCK_STATE));
    }

    #[test]
    fn standing_on_plateau_is_on_ground() {
        let world = World::new();
        let r = resolve_movement(
            &world,
            crate::hakoniwa::DimensionId::Overworld,
            0.5,
            64.0,
            0.5,
            0.5,
            64.0,
            0.5,
            0,
        );
        assert!(r.on_ground);
        assert!(!r.corrected);
        assert!((r.y - 64.0).abs() < 0.01);
    }

    #[test]
    fn falling_onto_plateau_snaps_to_surface() {
        let world = World::new();
        let r = resolve_movement(
            &world,
            crate::hakoniwa::DimensionId::Overworld,
            0.5,
            70.0,
            0.5,
            0.5,
            50.0,
            0.5,
            0,
        );
        assert!(r.corrected);
        assert!(r.on_ground);
        assert!(r.y > f64::from(FLAT_STONE_MAX_Y));
        assert!(r.y < f64::from(FLAT_STONE_MAX_Y) + 1.1);
    }

    #[test]
    fn cannot_walk_into_solid_wall() {
        let mut world = World::new();
        // Pillar at x=2, z=0 from y=64..66
        world.set_block(
            crate::hakoniwa::DimensionId::Overworld,
            2,
            64,
            0,
            STONE_BLOCK_STATE,
        );
        world.set_block(
            crate::hakoniwa::DimensionId::Overworld,
            2,
            65,
            0,
            STONE_BLOCK_STATE,
        );
        let r = resolve_movement(
            &world,
            crate::hakoniwa::DimensionId::Overworld,
            0.5,
            64.0,
            0.5,
            2.0,
            64.0,
            0.5,
            0,
        );
        assert!(r.corrected);
        assert!(r.x < 1.7, "should not enter the pillar, got x={}", r.x);
    }

    #[test]
    fn spectator_ignores_collision() {
        let world = World::new();
        let r = resolve_movement(
            &world,
            crate::hakoniwa::DimensionId::Overworld,
            0.5,
            64.0,
            0.5,
            0.5,
            40.0,
            0.5,
            GAMEMODE_SPECTATOR,
        );
        assert!(!r.corrected);
        assert!((r.y - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn dug_hole_allows_falling_through() {
        let mut world = World::new();
        // Dig a 2x2 shaft under the player through the surface
        for x in 0..2 {
            for z in 0..2 {
                for y in 60..=63 {
                    world.set_block(
                        crate::hakoniwa::DimensionId::Overworld,
                        x,
                        y,
                        z,
                        AIR_BLOCK_STATE,
                    );
                }
            }
        }
        let r = resolve_movement(
            &world,
            crate::hakoniwa::DimensionId::Overworld,
            0.5,
            64.0,
            0.5,
            0.5,
            61.0,
            0.5,
            0,
        );
        // Should fall into the hole (below 64)
        assert!(r.y < 63.5, "should enter dug hole, y={}", r.y);
    }
}
