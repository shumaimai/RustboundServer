//! Semantic snapshot of a Status response for conformance comparison.

use rustbound_protocol::status::StatusResponse;

/// A semantic snapshot extracted from a Status response, capturing only the
/// fields relevant for conformance comparison without retaining raw
/// copyrighted artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    /// Server version name (e.g. `"1.20.1"`).
    pub version_name: String,
    /// Server protocol version number (e.g. `763`).
    pub protocol_version: i32,
    /// Maximum player count.
    pub max_players: i32,
    /// Current online player count.
    pub online_players: i32,
    /// Plain-text description.
    pub description_text: String,
    /// Whether a favicon was present.
    pub has_favicon: bool,
    /// Sorted list of player sample names, for order-independent comparison.
    pub sample_names: Vec<String>,
    /// Whether the pong payload was echoed correctly.
    pub pong_echoed: bool,
}

impl StatusSnapshot {
    /// Extracts a snapshot from a decoded `StatusResponse` and a pong echo flag.
    pub fn from_response(response: &StatusResponse, pong_echoed: bool) -> Self {
        let sample_names = response
            .players
            .sample
            .as_ref()
            .map(|sample| {
                let mut names: Vec<String> =
                    sample.iter().map(|entry| entry.name.clone()).collect();
                names.sort();
                names
            })
            .unwrap_or_default();

        Self {
            version_name: response.version.name.clone(),
            protocol_version: response.version.protocol,
            max_players: response.players.max,
            online_players: response.players.online,
            description_text: response.description.text.clone(),
            has_favicon: response.favicon.is_some(),
            sample_names,
            pong_echoed,
        }
    }
}

/// A normalized snapshot where nondeterministic values have been stabilized.
///
/// Currently, normalization sorts the player sample names (already done in
/// [`StatusSnapshot`]) and ignores the favicon data content (only presence is
/// tracked). Future normalizations may include latency compensation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSnapshot {
    /// The normalized semantic fields.
    pub version_name: String,
    /// Protocol version.
    pub protocol_version: i32,
    /// Max players.
    pub max_players: i32,
    /// Online players.
    pub online_players: i32,
    /// Description text.
    pub description_text: String,
    /// Favicon presence (content is not compared).
    pub has_favicon: bool,
    /// Sorted player sample names.
    pub sample_names: Vec<String>,
    /// Whether pong was echoed.
    pub pong_echoed: bool,
}

/// Normalizes a [`StatusSnapshot`] for comparison.
///
/// Player sample order is sorted, favicon content is reduced to a boolean, and
/// the pong echo result is preserved. Unknown JSON fields are not captured
/// here; they are preserved for observation by the caller if needed.
pub fn normalize(snapshot: &StatusSnapshot) -> NormalizedSnapshot {
    NormalizedSnapshot {
        version_name: snapshot.version_name.clone(),
        protocol_version: snapshot.protocol_version,
        max_players: snapshot.max_players,
        online_players: snapshot.online_players,
        description_text: snapshot.description_text.clone(),
        has_favicon: snapshot.has_favicon,
        sample_names: snapshot.sample_names.clone(),
        pong_echoed: snapshot.pong_echoed,
    }
}

#[cfg(test)]
mod tests {
    use super::{StatusSnapshot, normalize};
    use rustbound_protocol::status::{
        PlayerSampleEntry, StatusDescription, StatusPlayers, StatusResponse, StatusVersion,
    };

    fn sample_response() -> StatusResponse {
        StatusResponse {
            version: StatusVersion {
                name: "1.20.1".to_owned(),
                protocol: 763,
            },
            players: StatusPlayers {
                max: 20,
                online: 2,
                sample: Some(vec![
                    PlayerSampleEntry {
                        name: "Zebra".to_owned(),
                        id: "aaa".to_owned(),
                    },
                    PlayerSampleEntry {
                        name: "Alpha".to_owned(),
                        id: "bbb".to_owned(),
                    },
                ]),
            },
            description: StatusDescription {
                text: "Test Server".to_owned(),
            },
            favicon: Some("data:image/png;base64,abc".to_owned()),
        }
    }

    #[test]
    fn snapshot_extracts_semantic_fields() {
        let response = sample_response();
        let snapshot = StatusSnapshot::from_response(&response, true);
        assert_eq!(snapshot.version_name, "1.20.1");
        assert_eq!(snapshot.protocol_version, 763);
        assert_eq!(snapshot.max_players, 20);
        assert_eq!(snapshot.online_players, 2);
        assert_eq!(snapshot.description_text, "Test Server");
        assert!(snapshot.has_favicon);
        assert_eq!(snapshot.sample_names, vec!["Alpha", "Zebra"]);
        assert!(snapshot.pong_echoed);
    }

    #[test]
    fn sample_names_are_sorted_independently_of_input_order() {
        let mut response = sample_response();
        // Reverse the sample order.
        if let Some(sample) = &mut response.players.sample {
            sample.reverse();
        }
        let snapshot = StatusSnapshot::from_response(&response, true);
        assert_eq!(snapshot.sample_names, vec!["Alpha", "Zebra"]);
    }

    #[test]
    fn normalization_preserves_semantic_values() {
        let response = sample_response();
        let snapshot = StatusSnapshot::from_response(&response, true);
        let normalized = normalize(&snapshot);
        assert_eq!(normalized.version_name, "1.20.1");
        assert_eq!(normalized.protocol_version, 763);
        assert_eq!(normalized.sample_names, vec!["Alpha", "Zebra"]);
        assert!(normalized.has_favicon);
        assert!(normalized.pong_echoed);
    }

    #[test]
    fn missing_favicon_and_sample_are_handled() {
        let response = StatusResponse {
            version: StatusVersion {
                name: "1.20.1".to_owned(),
                protocol: 763,
            },
            players: StatusPlayers {
                max: 10,
                online: 0,
                sample: None,
            },
            description: StatusDescription {
                text: "Empty".to_owned(),
            },
            favicon: None,
        };
        let snapshot = StatusSnapshot::from_response(&response, false);
        assert!(!snapshot.has_favicon);
        assert!(snapshot.sample_names.is_empty());
        assert!(!snapshot.pong_echoed);

        let normalized = normalize(&snapshot);
        assert!(!normalized.has_favicon);
        assert!(normalized.sample_names.is_empty());
    }
}
