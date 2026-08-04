//! Hakoniwa (箱庭): fixed-size playable gardens instead of infinite worldgen.
//!
//! See `docs/hakoniwa.md` for the product definition. This module owns size
//! presets, dimension flags, and world-border helpers.

/// Preset garden sizes (chunk radius from spawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapSize {
    /// 9×9 chunks (~144 blocks across).
    #[default]
    Tiny,
    /// 17×17 chunks (~272 blocks across).
    Small,
    /// 33×33 chunks (~528 blocks across).
    Medium,
}

impl MapSize {
    /// Parses `tiny` / `small` / `medium` (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "tiny" => Some(Self::Tiny),
            "small" => Some(Self::Small),
            "medium" => Some(Self::Medium),
            _ => None,
        }
    }

    /// Config / display name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::Medium => "medium",
        }
    }

    /// Half-width in chunks from the origin chunk (inclusive radius).
    pub fn chunk_radius(self) -> i32 {
        match self {
            Self::Tiny => 4,
            Self::Small => 8,
            Self::Medium => 16,
        }
    }

    /// Chunks along one edge (`2 * radius + 1`).
    pub fn chunk_span(self) -> i32 {
        self.chunk_radius() * 2 + 1
    }
}

/// Which dimensions a garden pack may expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionSet {
    /// Overworld (always required for join today).
    pub overworld: bool,
    /// Nether (H3+).
    pub nether: bool,
    /// The End including end-city content when packed (H3+).
    pub end: bool,
}

impl Default for DimensionSet {
    fn default() -> Self {
        Self {
            overworld: true,
            nether: false,
            end: false,
        }
    }
}

/// Authoritative garden bounds and metadata for the tick loop.
#[derive(Debug, Clone, PartialEq)]
pub struct GardenSpec {
    /// Size preset.
    pub size: MapSize,
    /// Enabled dimensions.
    pub dimensions: DimensionSet,
    /// Inclusive min block X (feet / entity X may be fractional).
    pub min_block_x: i32,
    /// Inclusive max block X.
    pub max_block_x: i32,
    /// Inclusive min block Z.
    pub min_block_z: i32,
    /// Inclusive max block Z.
    pub max_block_z: i32,
    /// Surface Y for the built-in flat plateau (top of stone = 63, stand at 64).
    pub surface_y: f64,
}

impl Default for GardenSpec {
    fn default() -> Self {
        Self::from_size(MapSize::Tiny)
    }
}

impl GardenSpec {
    /// Builds a centered garden for the given size (overworld-only for H0).
    pub fn from_size(size: MapSize) -> Self {
        let radius = size.chunk_radius();
        // Chunks [-radius, +radius] → blocks [radius*-16, radius*16+15]
        let min_block = -radius * 16;
        let max_block = radius * 16 + 15;
        Self {
            size,
            dimensions: DimensionSet::default(),
            min_block_x: min_block,
            max_block_x: max_block,
            min_block_z: min_block,
            max_block_z: max_block,
            surface_y: 64.0,
        }
    }

    /// Recommended view-distance cap so clients do not stream past the garden.
    pub fn recommended_view_distance(&self) -> i32 {
        self.size.chunk_radius()
    }

    /// Returns true if chunk coordinates lie inside the garden.
    pub fn contains_chunk(&self, chunk_x: i32, chunk_z: i32) -> bool {
        let r = self.size.chunk_radius();
        chunk_x >= -r && chunk_x <= r && chunk_z >= -r && chunk_z <= r
    }

    /// Returns true if the block column (x, z) lies inside the garden border.
    pub fn contains_block(&self, x: i32, z: i32) -> bool {
        x >= self.min_block_x
            && x <= self.max_block_x
            && z >= self.min_block_z
            && z <= self.max_block_z
    }

    /// Clamps X/Z into the garden. Y is unchanged (void / fall handled elsewhere).
    pub fn clamp_horizontal(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let min_x = f64::from(self.min_block_x) + 0.5;
        let max_x = f64::from(self.max_block_x) + 0.5;
        let min_z = f64::from(self.min_block_z) + 0.5;
        let max_z = f64::from(self.max_block_z) + 0.5;
        (x.clamp(min_x, max_x), y, z.clamp(min_z, max_z))
    }

    /// Clamps X/Z and, if Y is below the void line, snaps to the plateau surface.
    /// Used when restoring saved positions so players do not reload into the void.
    pub fn clamp_spawn_position(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (x, y, z) = self.clamp_horizontal(x, y, z);
        let y = if y < -64.0 { self.surface_y } else { y };
        (x, y, z)
    }

    /// True when horizontal clamping would change the position.
    pub fn is_outside_horizontally(&self, x: f64, z: f64) -> bool {
        let (cx, _, cz) = self.clamp_horizontal(x, 0.0, z);
        (cx - x).abs() > f64::EPSILON || (cz - z).abs() > f64::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_expected_radius() {
        assert_eq!(MapSize::Tiny.chunk_radius(), 4);
        assert_eq!(MapSize::Small.chunk_span(), 17);
        assert_eq!(MapSize::Medium.chunk_span(), 33);
    }

    #[test]
    fn tiny_garden_bounds_match_chunk_radius() {
        let g = GardenSpec::from_size(MapSize::Tiny);
        assert_eq!(g.min_block_x, -64);
        assert_eq!(g.max_block_x, 79);
        assert!(g.contains_chunk(0, 0));
        assert!(g.contains_chunk(4, -4));
        assert!(!g.contains_chunk(5, 0));
    }

    #[test]
    fn clamp_pulls_outside_position_in() {
        let g = GardenSpec::from_size(MapSize::Tiny);
        let (x, y, z) = g.clamp_horizontal(10_000.0, 70.0, -10_000.0);
        assert!(x <= f64::from(g.max_block_x) + 0.5);
        assert!(z >= f64::from(g.min_block_z) + 0.5);
        assert_eq!(y, 70.0);
    }

    #[test]
    fn clamp_spawn_rescues_void_y() {
        let g = GardenSpec::default();
        let (_, y, _) = g.clamp_spawn_position(0.5, -100.0, 0.5);
        assert_eq!(y, 64.0);
    }

    #[test]
    fn parse_map_size() {
        assert_eq!(MapSize::parse("TINY"), Some(MapSize::Tiny));
        assert_eq!(MapSize::parse("nope"), None);
    }
}
