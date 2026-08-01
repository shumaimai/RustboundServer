//! Protocol support for the targeted Minecraft Java Edition release.

pub mod framing;
pub mod handshake;
pub mod primitives;
pub mod state;

/// The Minecraft Java Edition protocol version targeted by this workspace.
pub const PROTOCOL_VERSION: u32 = 763;

/// The Minecraft Java Edition game version targeted by this workspace.
pub const GAME_VERSION: &str = "1.20.1";

#[cfg(test)]
mod tests {
    use super::{GAME_VERSION, PROTOCOL_VERSION};

    #[test]
    fn protocol_version_matches_target() {
        assert_eq!(PROTOCOL_VERSION, 763);
    }

    #[test]
    fn game_version_matches_target() {
        assert_eq!(GAME_VERSION, "1.20.1");
    }
}
