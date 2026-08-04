# Project Rules

- Build a pure Rust server compatible with Minecraft Java Edition 1.20.1. Arbitrary Java Forge mod compatibility is not a goal.
- Product landing: **hakoniwa** — fixed-size gardens (see `docs/hakoniwa.md`), not infinite vanilla parity.
- Use public documentation and black-box observation only; maintain a clean-room implementation.
- Never redistribute or commit Minecraft, Forge, mappings, decompiled output, local reference installations, or other reference artifacts.
- Do not copy implementation details from decompiled Minecraft code or Forge patches.
- Keep one authoritative tick thread until measurements justify carefully designed parallelism.
- Permit `unsafe` only in isolated, documented, audited modules.
- Avoid global `Arc<Mutex<_>>` state; use explicit ownership and narrow synchronization boundaries.
- Use differential tests against the local Forge 47.4.10 installation as an oracle, without incorporating its artifacts into this repository.
- Treat `素体データ/` as read-only local reference data: never modify, delete, stage, or commit it.

## Cursor Cloud specific instructions

- This is a single Cargo workspace (`rustbound-server`, `rustbound-protocol`, `rustbound-conformance`); toolchain is pinned to stable via `rust-toolchain.toml` (edition 2024, needs Rust >= 1.85). Standard commands live in `README.md`: `cargo build/test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- The only runnable service is the `rustbound-server` binary. It is self-contained (std threads + `TcpListener` + one 20 TPS tick thread) with no database, broker, or async runtime; `tokio` in `Cargo.lock` comes only from the `rustbound-conformance` test client. No external services need to be started to build, test, or run.
- Run it with `cargo run -p rustbound-server` (defaults to `0.0.0.0:25565`). It runs entirely on built-in defaults; no config file is required. `main.rs` calls `std::thread::park()`, so the process runs until killed (no graceful Ctrl+C handler yet) — start it in tmux/background rather than expecting it to return.
- The server is headless (raw Minecraft protocol 763), so there is no web UI or GUI to test; verify it by speaking the wire protocol. Only Status (handshake -> status -> ping/pong) and offline-mode Login (handshake -> login start -> login success) are implemented; the connection closes right after login success because Play is not implemented yet (see `connection.rs`).
- Differential testing against a local Forge 47.4.10 oracle and the `素体データ/` reference data are local-only and are NOT present in the cloud VM; those oracle-based conformance paths cannot be exercised here.
