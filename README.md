# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status

**Pre-alpha offline playability.** Phases A–I are on `main`. Current focus is verifying that a **vanilla 1.20.1 offline client** can join, see the flat world, dig/place (Creative), and chat.

| Layer | Status |
|-------|--------|
| Protocol + Login/Play + sessions/tick | Working |
| Chunks, light, streaming, unload, overrides | Working |
| Dig / place (Creative + Survival stub) | Working |
| Multiplayer motion + chat + gamemode | Working |
| Keep Alive timeout / offline UUID | Working |
| Health / void death / respawn / food stub | Working (minimal) |
| Persistence + autosave | Working |
| Join sequence stubs (abilities, tags, recipes, tab self) | Working |
| Online mode | [#60](https://github.com/shumaimai/RustboundServer/issues/60) |
| Forge jars / JVM | **Out of scope** |
| Thin Rust Mod API | Façade + ModHost wired ([#101](https://github.com/shumaimai/RustboundServer/issues/101)) |

See [PROGRESS.md](PROGRESS.md). Tracking leftovers: [#134](https://github.com/shumaimai/RustboundServer/issues/134).

## Try it (offline)

Requirements: Rust toolchain; optional **Minecraft Java Edition 1.20.1** client set to offline / cracked-offline launcher (no Microsoft auth against this server).

```console
cp server.properties.example server.properties
cargo run -p rustbound-server --release -- --config server.properties
```

Then connect the 1.20.1 client to `localhost:25565` (or the host/port in your properties). Default example uses **Creative** (`gamemode=1`) with a small starter hotbar.

Automated smoke (no Minecraft client required):

```console
./scripts/smoke_offline_join.sh
cargo test -p rustbound-server --lib server_offline_playability_smoke
```

What works today: join flat Overworld, move, chat, Creative dig/place, Survival dig progress stub, multiplayer visibility, persist across restart for the same offline UUID.

What is still rough: full vanilla parity (recipes/tags content, physics, mobs), online mode, and richer Mod API wiring.

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
| **I** | Persistence + polish (#122–#133) | **Done** |
| **J** | Offline join / playability verification | **In progress** |
| Later | Online mode (#60) | Queued |
| Long-term | Thin Rust Mod API (#101, #132) | Façade + host wired |

**Not a goal:** drop-in Java Forge mod compatibility or an automatic Forge→Rust compiler.

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
