//! Structured diff between two play snapshots.
//!
//! Nondeterministic fields (entity IDs, teleport IDs, keepalive payloads,
//! timestamps) are intentionally excluded from the comparison. Only semantic
//! fields that should be consistent between a Rustbound server and a Forge
//! oracle are compared.

use crate::play_client::PlaySnapshot;

/// A single field-level difference between two play snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayDiffEntry {
    /// The field name that differs.
    pub field: &'static str,
    /// The expected (oracle) value, rendered as a string.
    pub expected: String,
    /// The actual (candidate) value, rendered as a string.
    pub actual: String,
}

/// The result of comparing two play snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayDiffResult {
    /// The snapshots are semantically equivalent.
    Match,
    /// One or more required fields differ.
    Mismatch(Vec<PlayDiffEntry>),
}

impl PlayDiffResult {
    /// Returns `true` if the result is a match.
    pub fn is_match(&self) -> bool {
        matches!(self, Self::Match)
    }

    /// Returns `true` if the result is a mismatch.
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Self::Mismatch(_))
    }
}

/// A structured diff between two play snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayDiff {
    /// The diff result.
    pub result: PlayDiffResult,
}

/// Compares two play snapshots and returns a structured diff.
///
/// The following fields are compared (nondeterministic fields are excluded):
/// - `gamemode` (should match between servers)
/// - `dimension_name` (should be `minecraft:overworld`)
/// - `is_hardcore` (should match config)
/// - `is_flat` (should match config)
/// - `phase_reached` (both should reach at least TeleportConfirmed)
/// - `teleport_confirmed` (both should confirm)
/// - `keep_alive_seen` (both should see keepalive)
/// - `chunk_data_seen` (both should see chunk data)
///
/// **Excluded** (nondeterministic):
/// - `entity_id` (server-specific allocation policy)
/// - `hashed_seed` (world-seed dependent)
/// - `uuid` / `username` (probe-specific)
pub fn diff_play(expected: &PlaySnapshot, actual: &PlaySnapshot) -> PlayDiff {
    let mut entries = Vec::new();

    macro_rules! compare {
        ($field:expr, $expected:expr, $actual:expr) => {
            if $expected != $actual {
                entries.push(PlayDiffEntry {
                    field: $field,
                    expected: format!("{:?}", $expected),
                    actual: format!("{:?}", $actual),
                });
            }
        };
    }

    compare!("gamemode", expected.gamemode, actual.gamemode);
    compare!(
        "dimension_name",
        expected.dimension_name,
        actual.dimension_name
    );
    compare!("is_hardcore", expected.is_hardcore, actual.is_hardcore);
    compare!("is_flat", expected.is_flat, actual.is_flat);
    compare!(
        "phase_reached",
        expected.phase_reached,
        actual.phase_reached
    );
    compare!(
        "teleport_confirmed",
        expected.teleport_confirmed,
        actual.teleport_confirmed
    );
    compare!(
        "keep_alive_seen",
        expected.keep_alive_seen,
        actual.keep_alive_seen
    );
    compare!(
        "chunk_data_seen",
        expected.chunk_data_seen,
        actual.chunk_data_seen
    );

    let result = if entries.is_empty() {
        PlayDiffResult::Match
    } else {
        PlayDiffResult::Mismatch(entries)
    };

    PlayDiff { result }
}

impl std::fmt::Display for PlayDiff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.result {
            PlayDiffResult::Match => formatter.write_str("play snapshots match"),
            PlayDiffResult::Mismatch(entries) => {
                writeln!(
                    formatter,
                    "play snapshots differ ({} field(s)):",
                    entries.len()
                )?;
                for entry in entries {
                    writeln!(
                        formatter,
                        "  {}: expected {} but got {}",
                        entry.field, entry.expected, entry.actual
                    )?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlayDiffResult, diff_play};
    use crate::play_client::{PlayPhase, PlaySnapshot};
    use rustbound_protocol::primitives::Uuid;

    fn base_snapshot() -> PlaySnapshot {
        PlaySnapshot {
            entity_id: 42,
            gamemode: 0,
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            is_hardcore: false,
            is_flat: false,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            phase_reached: PlayPhase::PostTeleport,
            teleport_confirmed: true,
            keep_alive_seen: true,
            chunk_data_seen: true,
        }
    }

    #[test]
    fn identical_snapshots_match() {
        let snapshot = base_snapshot();
        let result = diff_play(&snapshot, &snapshot);
        assert_eq!(result.result, PlayDiffResult::Match);
        assert!(result.result.is_match());
    }

    #[test]
    fn entity_id_difference_is_ignored() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.entity_id = 999;
        let result = diff_play(&expected, &actual);
        assert_eq!(result.result, PlayDiffResult::Match);
    }

    #[test]
    fn gamemode_mismatch_is_reported() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.gamemode = 1;
        let result = diff_play(&expected, &actual);
        if let PlayDiffResult::Mismatch(entries) = result.result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].field, "gamemode");
        } else {
            panic!("expected mismatch");
        }
    }

    #[test]
    fn phase_reached_mismatch_is_reported() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.phase_reached = PlayPhase::JoinGame;
        actual.teleport_confirmed = false;
        let result = diff_play(&expected, &actual);
        if let PlayDiffResult::Mismatch(entries) = result.result {
            let fields: Vec<_> = entries.iter().map(|e| e.field).collect();
            assert!(fields.contains(&"phase_reached"));
            assert!(fields.contains(&"teleport_confirmed"));
        } else {
            panic!("expected mismatch");
        }
    }

    #[test]
    fn display_formats_mismatch_human_readably() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.is_flat = true;
        let result = diff_play(&expected, &actual);
        let display = format!("{result}");
        assert!(display.contains("is_flat"));
        assert!(display.contains("false"));
        assert!(display.contains("true"));
    }
}
