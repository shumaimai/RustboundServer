//! Structured diff between two normalized status snapshots.

use crate::snapshot::NormalizedSnapshot;

/// A single field-level difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiffEntry {
    /// The field name that differs.
    pub field: &'static str,
    /// The expected (oracle) value, rendered as a string.
    pub expected: String,
    /// The actual (candidate) value, rendered as a string.
    pub actual: String,
}

/// The result of comparing two normalized snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusDiffResult {
    /// The snapshots are semantically equivalent.
    Match,
    /// One or more required fields differ.
    Mismatch(Vec<StatusDiffEntry>),
}

impl StatusDiffResult {
    /// Returns `true` if the result is a match.
    pub fn is_match(&self) -> bool {
        matches!(self, Self::Match)
    }

    /// Returns `true` if the result is a mismatch.
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Self::Mismatch(_))
    }
}

/// A structured diff between two normalized snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDiff {
    /// The diff result.
    pub result: StatusDiffResult,
}

/// Compares two normalized snapshots and returns a structured diff.
///
/// Only required semantic fields are compared. Unknown or Forge-specific JSON
/// extensions are not part of the snapshot and are intentionally ignored.
pub fn diff(expected: &NormalizedSnapshot, actual: &NormalizedSnapshot) -> StatusDiff {
    let mut entries = Vec::new();

    macro_rules! compare {
        ($field:expr, $expected:expr, $actual:expr) => {
            if $expected != $actual {
                entries.push(StatusDiffEntry {
                    field: $field,
                    expected: format!("{:?}", $expected),
                    actual: format!("{:?}", $actual),
                });
            }
        };
    }

    compare!("version_name", expected.version_name, actual.version_name);
    compare!(
        "protocol_version",
        expected.protocol_version,
        actual.protocol_version
    );
    compare!("max_players", expected.max_players, actual.max_players);
    compare!(
        "online_players",
        expected.online_players,
        actual.online_players
    );
    compare!(
        "description_text",
        expected.description_text,
        actual.description_text
    );
    compare!("has_favicon", expected.has_favicon, actual.has_favicon);
    compare!("sample_names", expected.sample_names, actual.sample_names);
    compare!("pong_echoed", expected.pong_echoed, actual.pong_echoed);

    let result = if entries.is_empty() {
        StatusDiffResult::Match
    } else {
        StatusDiffResult::Mismatch(entries)
    };

    StatusDiff { result }
}

impl std::fmt::Display for StatusDiff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.result {
            StatusDiffResult::Match => formatter.write_str("snapshots match"),
            StatusDiffResult::Mismatch(entries) => {
                writeln!(formatter, "snapshots differ ({} field(s)):", entries.len())?;
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
    use super::{StatusDiffResult, diff};
    use crate::snapshot::NormalizedSnapshot;

    fn base_snapshot() -> NormalizedSnapshot {
        NormalizedSnapshot {
            version_name: "1.20.1".to_owned(),
            protocol_version: 763,
            max_players: 20,
            online_players: 0,
            description_text: "A Minecraft Server".to_owned(),
            has_favicon: false,
            sample_names: vec![],
            pong_echoed: true,
        }
    }

    #[test]
    fn identical_snapshots_match() {
        let snapshot = base_snapshot();
        let result = diff(&snapshot, &snapshot);
        assert_eq!(result.result, StatusDiffResult::Match);
        assert!(result.result.is_match());
    }

    #[test]
    fn version_name_mismatch_is_reported() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.version_name = "1.20.2".to_owned();
        let result = diff(&expected, &actual);
        assert!(result.result.is_mismatch());
        if let StatusDiffResult::Mismatch(entries) = result.result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].field, "version_name");
        }
    }

    #[test]
    fn multiple_field_mismatches_are_all_reported() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.protocol_version = 764;
        actual.max_players = 100;
        actual.description_text = "Different".to_owned();
        let result = diff(&expected, &actual);
        if let StatusDiffResult::Mismatch(entries) = result.result {
            assert_eq!(entries.len(), 3);
            let fields: Vec<_> = entries.iter().map(|e| e.field).collect();
            assert!(fields.contains(&"protocol_version"));
            assert!(fields.contains(&"max_players"));
            assert!(fields.contains(&"description_text"));
        } else {
            panic!("expected mismatch");
        }
    }

    #[test]
    fn sample_names_order_independence_via_normalization() {
        // NormalizedSnapshot.sample_names is already sorted by
        // StatusSnapshot::from_response, so both snapshots should have
        // the same sorted order regardless of the original server's order.
        let mut actual = base_snapshot();
        actual.sample_names = vec!["Alice".to_owned(), "Bob".to_owned()];
        let mut expected_with_sample = base_snapshot();
        expected_with_sample.sample_names = vec!["Alice".to_owned(), "Bob".to_owned()];
        let result = diff(&expected_with_sample, &actual);
        assert_eq!(result.result, StatusDiffResult::Match);
    }

    #[test]
    fn pong_echo_mismatch_is_reported() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.pong_echoed = false;
        let result = diff(&expected, &actual);
        if let StatusDiffResult::Mismatch(entries) = result.result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].field, "pong_echoed");
        } else {
            panic!("expected mismatch");
        }
    }

    #[test]
    fn display_formats_mismatch_human_readably() {
        let expected = base_snapshot();
        let mut actual = base_snapshot();
        actual.protocol_version = 999;
        let result = diff(&expected, &actual);
        let display = format!("{result}");
        assert!(display.contains("protocol_version"));
        assert!(display.contains("763"));
        assert!(display.contains("999"));
    }

    #[test]
    #[allow(dead_code)]
    fn ensure_statusdiff_entry_unused_warning_is_suppressed() {
        // StatusDiffEntry is part of the public API and is constructed
        // internally by diff(). This test exists to prevent dead-code
        // warnings when the struct is not directly referenced in tests.
        let _entry = super::StatusDiffEntry {
            field: "test",
            expected: "a".to_owned(),
            actual: "b".to_owned(),
        };
    }
}
