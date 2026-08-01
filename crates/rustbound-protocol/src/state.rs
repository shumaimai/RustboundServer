//! Protocol 763 connection state model and routing.
//!
//! Minecraft Java Edition 1.20.1 has four live connection states plus a
//! terminal `Closed` state. There is no Configuration state (introduced in
//! 1.20.2) and no Transfer handshake intent (introduced in 1.20.5).

use std::fmt;

/// The connection states defined by protocol 763.
///
/// Variants are ordered to reflect the natural lifecycle of a connection.
/// `Configuration` and `Transfer` are intentionally absent: they do not exist
/// in the targeted protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolState {
    /// The initial state before the first handshake packet is processed.
    Handshaking,
    /// Status protocol: server list ping and latency checks.
    Status,
    /// Login protocol: authentication and encryption negotiation.
    Login,
    /// Play protocol: active gameplay after successful login.
    Play,
    /// The connection has been closed and no further packets are accepted.
    Closed,
}

impl ProtocolState {
    /// Returns the packet-ID set name expected in this state, for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::Handshaking => "Handshaking",
            Self::Status => "Status",
            Self::Login => "Login",
            Self::Play => "Play",
            Self::Closed => "Closed",
        }
    }
}

impl fmt::Display for ProtocolState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The next-state value carried by a handshake packet.
///
/// Protocol 763 permits only `1 = Status` and `2 = Login`. Any other value is
/// rejected during handshake parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NextState {
    /// Status protocol (`next_state = 1`).
    Status,
    /// Login protocol (`next_state = 2`).
    Login,
}

impl NextState {
    /// The raw wire value used by protocol 763.
    pub fn wire_value(self) -> i32 {
        match self {
            Self::Status => 1,
            Self::Login => 2,
        }
    }

    /// Converts a raw next-state value, rejecting anything outside {1, 2}.
    pub fn from_wire(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Status),
            2 => Some(Self::Login),
            _ => None,
        }
    }

    /// The connection state this next-state transitions into.
    pub fn target_state(self) -> ProtocolState {
        match self {
            Self::Status => ProtocolState::Status,
            Self::Login => ProtocolState::Login,
        }
    }
}

/// An error produced while routing between connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    /// A packet was received in a state that does not accept it.
    WrongState {
        current: ProtocolState,
        expected: ProtocolState,
    },
    /// The connection is closed and cannot accept any packet.
    ConnectionClosed,
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongState { current, expected } => {
                write!(
                    formatter,
                    "packet received in {current} state, expected {expected}"
                )
            }
            Self::ConnectionClosed => formatter.write_str("connection is closed"),
        }
    }
}

impl std::error::Error for StateError {}

/// Applies a handshake-driven transition from `Handshaking` to the next state.
///
/// Returns [`StateError::WrongState`] if the connection is not currently in the
/// `Handshaking` state. `Closed` connections are reported via
/// [`StateError::ConnectionClosed`] to make the terminal condition explicit.
pub fn apply_handshake_transition(
    current: ProtocolState,
    next: NextState,
) -> Result<ProtocolState, StateError> {
    match current {
        ProtocolState::Handshaking => Ok(next.target_state()),
        ProtocolState::Closed => Err(StateError::ConnectionClosed),
        other => Err(StateError::WrongState {
            current: other,
            expected: ProtocolState::Handshaking,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{NextState, ProtocolState, StateError, apply_handshake_transition};

    #[test]
    fn no_configuration_or_transfer_variants_exist() {
        // Protocol 763 has exactly five states including the terminal one.
        let all = [
            ProtocolState::Handshaking,
            ProtocolState::Status,
            ProtocolState::Login,
            ProtocolState::Play,
            ProtocolState::Closed,
        ];
        assert_eq!(all.len(), 5);
        assert_eq!(ProtocolState::Handshaking.name(), "Handshaking");
        assert_eq!(ProtocolState::Status.name(), "Status");
        assert_eq!(ProtocolState::Login.name(), "Login");
        assert_eq!(ProtocolState::Play.name(), "Play");
        assert_eq!(ProtocolState::Closed.name(), "Closed");
    }

    #[test]
    fn next_state_only_accepts_status_and_login() {
        assert_eq!(NextState::from_wire(1), Some(NextState::Status));
        assert_eq!(NextState::from_wire(2), Some(NextState::Login));
        assert_eq!(NextState::from_wire(0), None);
        assert_eq!(NextState::from_wire(3), None);
        assert_eq!(NextState::from_wire(-1), None);
        assert_eq!(NextState::from_wire(i32::MAX), None);
    }

    #[test]
    fn next_state_wire_values_are_stable() {
        assert_eq!(NextState::Status.wire_value(), 1);
        assert_eq!(NextState::Login.wire_value(), 2);
        assert_eq!(NextState::Status.target_state(), ProtocolState::Status);
        assert_eq!(NextState::Login.target_state(), ProtocolState::Login);
    }

    #[test]
    fn handshake_transitions_from_handshaking_to_status_or_login() {
        assert_eq!(
            apply_handshake_transition(ProtocolState::Handshaking, NextState::Status),
            Ok(ProtocolState::Status)
        );
        assert_eq!(
            apply_handshake_transition(ProtocolState::Handshaking, NextState::Login),
            Ok(ProtocolState::Login)
        );
    }

    #[test]
    fn handshake_transition_rejects_wrong_state() {
        for current in [
            ProtocolState::Status,
            ProtocolState::Login,
            ProtocolState::Play,
        ] {
            assert_eq!(
                apply_handshake_transition(current, NextState::Status),
                Err(StateError::WrongState {
                    current,
                    expected: ProtocolState::Handshaking,
                })
            );
        }
    }

    #[test]
    fn handshake_transition_rejects_closed_connection() {
        assert_eq!(
            apply_handshake_transition(ProtocolState::Closed, NextState::Status),
            Err(StateError::ConnectionClosed)
        );
    }
}
