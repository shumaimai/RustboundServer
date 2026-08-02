# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status

**Pre-alpha, joinable offline mini-server.** Phases A–E are merged: offline Login → Play, join sequence, flat chunks, keep-alive, tab list, remote spawn/despawn, dig/place codecs, conformance helpers. It is **not** a full survival/creative game yet.

| Layer | Status |
|-------|--------|
| Protocol codecs (Handshake / Status / Login / Play core) | Implemented |
| Login / Play state machines | Wired into the live server |
| Conformance probes (Status / Login / Play) | Implemented |
| TCP listener + connection handler | Status + offline Login + Play |
| Tick loop (20 TPS) + World / sessions | Wired |
| Flat chunk join + initial columns | Working (limited radius; light incomplete) |
| Multiplayer tab list / spawn | Partial (no continuous motion broadcast) |
| Dig / place | Codecs + handlers present; **decode path / compression gaps remain** |
| Inventory / health / chat / persistence | Not started |
| Online mode | Not started ([#60](https://github.com/shumaimai/RustboundServer/issues/60)) |
| Forge jar mods / JVM | **Out of scope** |
| Thin Rust Mod API | **Long-term goal** (after a solid vanilla loop) |

See [PROGRESS.md](PROGRESS.md) for milestone history. Next work: **[#102](https://github.com/shumaimai/RustboundServer/issues/102)** (Phase F+ tracking).

## Workspace

```
crates/
  rustbound-protocol/     # Wire codecs + Login/Play state machines
  rustbound-server/       # Listener, connection, session, tick, world, chunks
  rustbound-conformance/  # Black-box probes + Status/Play diff helpers
```

## Build and test

```console
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Run the server (defaults: `0.0.0.0:25565`, offline mode):

```console
cargo run -p rustbound-server
```

Optional: `--config path/to/server.properties`, `--host`, `--port`. Stop with Ctrl+C.

## Architecture notes

- One authoritative tick thread (20 TPS); connection threads talk via `mpsc` — avoid global `Arc<Mutex<_>>` world state.
- Prefer existing `LoginStateMachine` / `PlayStateMachine` over re-implementing flows in the server.
- Conformance probes should drive integration; Forge may be used as a local oracle only (never commit its artifacts).
- `unsafe` only in isolated, documented, audited modules (`forbid` at workspace level today).

## Roadmap

| Phase | Focus | Tracking |
|-------|--------|----------|
| M1–M4 + A–E | Foundations through mini Play | Done — see PROGRESS.md / [#74](https://github.com/shumaimai/RustboundServer/issues/74) |
| **F** | Play hardening (regression, dig/place wire-up, compression, light, gamemode) | [#102](https://github.com/shumaimai/RustboundServer/issues/102) · start [#86](https://github.com/shumaimai/RustboundServer/issues/86)–[#90](https://github.com/shumaimai/RustboundServer/issues/90) |
| **G** | Multiplayer motion + chunk streaming + config wiring | [#91](https://github.com/shumaimai/RustboundServer/issues/91)–[#94](https://github.com/shumaimai/RustboundServer/issues/94) |
| **H** | Inventory, vitals, chat, keep-alive timeout | [#95](https://github.com/shumaimai/RustboundServer/issues/95)–[#99](https://github.com/shumaimai/RustboundServer/issues/99) |
| **I** | Persistence | [#100](https://github.com/shumaimai/RustboundServer/issues/100) |
| Later | Online mode [#60](https://github.com/shumaimai/RustboundServer/issues/60) | |
| **Long-term** | Thin **Rust Mod API** (not Forge jar compatibility); port license-clean famous mods to Rust | [#101](https://github.com/shumaimai/RustboundServer/issues/101) |

**Not a goal:** drop-in Java Forge mod compatibility or an automatic Forge→Rust compiler.

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md) (clean-room rules, tick-thread ownership, no reference artifacts in-repo).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
