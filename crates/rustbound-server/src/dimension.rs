//! Hakoniwa dimension switching helpers (H3).

use crate::hakoniwa::DimensionId;
use crate::world::World;

/// Block states used as portal pads (1.20.1 global palette).
pub const PORTAL_GLOWSTONE: i32 = crate::chunk::GLOWSTONE_BLOCK_STATE;
pub const PORTAL_END_STONE: i32 = crate::chunk::END_STONE_BLOCK_STATE;

/// Destination when standing on a portal pad in `current`.
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
            if block_under_feet == PORTAL_END_STONE {
                Some(DimensionId::Overworld)
            } else {
                None
            }
        }
    }
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
    fn parse_dim_command_ok() {
        assert_eq!(parse_dim_command("/dim nether"), Some(DimensionId::Nether));
        assert_eq!(parse_dim_command(" /dim end "), Some(DimensionId::End));
        assert_eq!(parse_dim_command("hello"), None);
    }
}
