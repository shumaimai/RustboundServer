//! Black-box conformance client and normalizer for Minecraft Java Edition 1.20.1.
//!
//! This crate provides tools to query a Minecraft Java Edition 1.20.1 server's
//! Status and Login endpoints, extract semantic fields, normalize
//! nondeterministic values, and compare two snapshots for behavioral
//! conformance. It is designed for differential testing against a local Forge
//! oracle without incorporating or redistributing any Forge or Minecraft
//! artifacts.

pub mod client;
pub mod diff;
pub mod login_client;
pub mod play_client;
pub mod snapshot;

pub use client::{StatusClient, StatusClientError};
pub use diff::{StatusDiff, StatusDiffEntry, StatusDiffResult};
pub use login_client::{LoginClientError, LoginOutcome, LoginProbeConfig, run_login_probe};
pub use play_client::{PlayClientError, PlayProbeConfig, PlaySnapshot, run_play_probe};
pub use snapshot::{NormalizedSnapshot, StatusSnapshot, normalize};
