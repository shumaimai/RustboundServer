# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status

**Pre-alpha, offline multiplayer mini-server.** Phases A–H are on `main` (or this PR): join Play, chunks/streaming, dig/place, inventory, chat, Keep Alive timeout, offline UUID, **health / void death / respawn**. Next: persistence (Phase I) and online mode.

| Layer | Status |
|-------|--------|
| Protocol + Login/Play + sessions/tick | Implemented |
| Chunks, light, streaming, overrides | Working |
| Dig / place (Creative) + inventory/hotbar | Working |
| Multiplayer motion + chat | Working |
| Keep Alive timeout / offline UUID | Working |
| Health / void death / respawn | Working (minimal; no combat yet) |
| Persistence | **Next** ([#100](https://github.com/shumaimai/RustboundServer/issues/100)) |
| Online mode | [#60](https://github.com/shumaimai/RustboundServer/issues/60) |
| Forge jars / JVM | **Out of scope** |
| Thin Rust Mod API | Long-term ([#101](https://github.com/shumaimai/RustboundServer/issues/101)) |

See [PROGRESS.md](PROGRESS.md). **Next:** [#100](https://github.com/shumaimai/RustboundServer/issues/100) (persistence) · tracking [#102](https://github.com/shumaimai/RustboundServer/issues/102).

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
| **G** | Motion, chunk streaming, overrides, config (#91–#94) | **Done** |
| **H** | Inventory, vitals, chat, KA timeout, offline UUID (#95–#99) | **In progress** — #99/#98/#97/#95 in this merge; **#96 remaining** · [#113](https://github.com/shumaimai/RustboundServer/issues/113) |
| **I** | Persistence (#100) | Queued |
| Later | Online mode (#60) | Queued |
| Long-term | Thin Rust Mod API (#101); hand-port license-clean mods | After H–I |

**Not a goal:** drop-in Java Forge mod compatibility or an automatic Forge→Rust compiler.

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
