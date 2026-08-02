# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status

**Pre-alpha, offline multiplayer mini-server.** Phases A–H done; this PR lands early Phase I: gamemode wiring, item registry, held-item Creative place, block/player persistence, autosave. Remaining: unload, respawn re-sync, food/combat, Mod API prereqs ([#134](https://github.com/shumaimai/RustboundServer/issues/134)).

| Layer | Status |
|-------|--------|
| Protocol + Login/Play + sessions/tick | Implemented |
| Chunks, light, streaming, overrides | Working (unload packet still incomplete — #127) |
| Dig / place (Creative) + held-item place | Working |
| Multiplayer motion + chat + gamemode broadcast | Working |
| Keep Alive timeout / offline UUID | Working |
| Health / void death / respawn | Working (minimal) |
| Persistence (overrides + players + autosave) | **In this PR** (#124–#126) |
| Online mode | [#60](https://github.com/shumaimai/RustboundServer/issues/60) |
| Forge jars / JVM | **Out of scope** |
| Thin Rust Mod API | Long-term ([#101](https://github.com/shumaimai/RustboundServer/issues/101)) |

See [PROGRESS.md](PROGRESS.md). **Tracking:** [#134](https://github.com/shumaimai/RustboundServer/issues/134).

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

```console
cargo run -p rustbound-server
```

Optional: `--config path/to/server.properties`, `--host`, `--port`. Stop with Ctrl+C.

## Architecture notes

- One authoritative tick thread (20 TPS); connection threads talk via `mpsc` — avoid global `Arc<Mutex<World>>` world state.
- Prefer existing `LoginStateMachine` / `PlayStateMachine` over re-implementing flows in the server.
- Conformance probes should drive integration; Forge may be used as a local oracle only (never commit its artifacts).
- `unsafe` only in isolated, documented, audited modules (`forbid` at workspace level today).

## Roadmap

| Phase | Focus | Status |
|-------|--------|--------|
| M1–M4 + A–E | Foundations → mini Play | **Done** |
| **F** | Play hardening (#86–#90) | **Done** |
| **G** | Motion, streaming, overrides, config (#91–#94) | **Done** |
| **H** | Inventory, chat, KA timeout, UUID, vitals (#95–#99) | **Done** |
| **I** | Persistence + polish (#122–#133) | **Next → [#134](https://github.com/shumaimai/RustboundServer/issues/134)** |
| Later | Online mode (#60) | Queued |
| Long-term | Thin Rust Mod API (#101, #131–#132) | After I |

**Not a goal:** drop-in Java Forge mod compatibility or an automatic Forge→Rust compiler.

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
