//! Rustbound server: a pure Rust Minecraft Java Edition 1.20.1 server.
//!
//! This crate contains the server core: TCP listener, connection handler,
//! tick loop, world management, and player session orchestration.

pub mod connection;
pub mod listener;
