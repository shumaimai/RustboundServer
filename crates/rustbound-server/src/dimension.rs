//! Hakoniwa dimension switching helpers (H3).

use crate::chunk::{END_STONE_BLOCK_STATE, GLOWSTONE_BLOCK_STATE};
use crate::hakoniwa::DimensionId;
use crate::world::World;

/// Overworld → Nether and Nether → Overworld pad (glowstone).
pub const PORTAL_GLOWSTONE: i32 = GLOWSTONE_BLOCK_STATE;
/// Overworld → End pad (end stone). Not used as a return trigger in The End,
/// because the End plateau itself is end stone.
pub const PORTAL_END_STONE: i32 = END_STONE_BLOCK_STATE;

/// Ticks to ignore portal pads after a dimension change (2 seconds).
pub const PORTAL_COOLDOWN_TICKS: u64 = 40;

/// Safe arrival feet Y (above the y=63 plateau top).
pub const DIMENSION_ARRIVAL_Y: f64 = 65.0;

/// Destination when standing on a portal pad in `current`.
///
/// The End returns only via glowstone so the end-stone plateau does not
/// immediately bounce players back to the overworld.
pub fn portal_destination(current: DimensionId, block_under_feet: i32) -> Option<DimensionId> {
    match current {
        DimensionId::Overworld => {
            if block_under_feet == PORTAL_GLOWSTONE {
                Some(DimensionId::Nether)
            } else if block_under_feet == PORTAL_END_STONE {
                Some(DimensionId::End)
            } else {
                None
            }
        }
        DimensionId::Nether => {
            if block_under_feet == PORTAL_GLOWSTONE {
                Some(DimensionId::Overworld)
            } else {
                None
            }
        }
        DimensionId::End => {
            if block_under_feet == PORTAL_GLOWSTONE {
                Some(DimensionId::Overworld)
            } else {
                None
            }
        }
    }
}

/// Arrival feet position for a dimension transfer (garden spawn XZ, safe Y).
pub fn arrival_position(world: &World) -> (f64, f64, f64) {
    let (x, _, z) = world.spawn_point();
    (x, DIMENSION_ARRIVAL_Y, z)
}

/// Block immediately under feet (y-1 rounded).
pub fn block_under_feet(world: &World, dimension: DimensionId, x: f64, y: f64, z: f64) -> i32 {
    let bx = x.floor() as i32;
    let by = (y - 0.05).floor() as i32;
    let bz = z.floor() as i32;
    world.get_block(dimension, bx, by, bz)
}

/// Parses `/dim <name>` chat commands.
pub fn parse_dim_command(message: &str) -> Option<DimensionId> {
    let trimmed = message.trim();
    let rest = trimmed.strip_prefix("/dim ")?.trim();
    DimensionId::parse(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_overworld_glowstone_to_nether() {
        assert_eq!(
            portal_destination(DimensionId::Overworld, PORTAL_GLOWSTONE),
            Some(DimensionId::Nether)
        );
    }

    #[test]
    fn portal_overworld_end_stone_to_end() {
        assert_eq!(
            portal_destination(DimensionId::Overworld, PORTAL_END_STONE),
            Some(DimensionId::End)
        );
    }

    #[test]
    fn end_plateau_end_stone_does_not_bounce() {
        assert_eq!(
            portal_destination(DimensionId::End, PORTAL_END_STONE),
            None,
            "End plateau is end stone; only glowstone returns"
        );
    }

    #[test]
    fn end_glowstone_returns_to_overworld() {
        assert_eq!(
            portal_destination(DimensionId::End, PORTAL_GLOWSTONE),
            Some(DimensionId::Overworld)
        );
    }

    #[test]
    fn parse_dim_command_ok() {
        assert_eq!(parse_dim_command("/dim nether"), Some(DimensionId::Nether));
        assert_eq!(parse_dim_command(" /dim end "), Some(DimensionId::End));
        assert_eq!(parse_dim_command("hello"), None);
    }
}
