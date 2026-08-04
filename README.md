# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Product goal: Hakoniwa (箱庭)

**Complete a tiny, fixed-map Minecraft experience — then keep the binary extreme-small.**

We do **not** chase full vanilla worldgen or Forge mods. Play happens inside size presets (`tiny` / `small` / `medium`) with Overworld → Nether → End datasets added as packs. Details: [docs/hakoniwa.md](docs/hakoniwa.md).

## Status

**Hakoniwa Phase H0** on the path to a finished garden. Offline join + flat plateau work; garden border + size config ship in H0.

| Layer | Status |
|-------|--------|
| Protocol + Login/Play + sessions/tick | Working |
| Flat plateau chunks + dig/place | Working |
| Hakoniwa size + world border clamp | **H0** |
| Block collision | H1 (next) |
| Map packs / 3 sizes of real content | H2 |
| Nether / End | H3 |
| Simple mobs | H4 |
| Static liquids | H5 |
| Online mode | [#60](https://github.com/shumaimai/RustboundServer/issues/60) (optional) |
| Forge jars / JVM | **Out of scope** |
| Thin Rust Mod API | Façade + ModHost wired |

See [PROGRESS.md](PROGRESS.md) and [docs/hakoniwa.md](docs/hakoniwa.md).

## Try it (offline)

```console
cp server.properties.example server.properties
cargo run -p rustbound-server --release -- --config server.properties
```

Connect a **1.20.1** offline client to `localhost:25565`. Default example is **Creative** with `hakoniwa-size=tiny`.

Smallest distribution binary:

```console
cargo build -p rustbound-server --profile dist
```

Automated smoke:

```console
./scripts/smoke_offline_join.sh
cargo test -p rustbound-server --lib server_offline_playability_smoke
```

## Workspace

```
crates/
  rustbound-protocol/     # Wire codecs + Login/Play state machines
  rustbound-server/       # Listener, connection, session, tick, world, hakoniwa
  rustbound-conformance/  # Black-box probes + Status/Play diff helpers
```

## Build and test

```console
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## Architecture notes

- One authoritative tick thread (20 TPS); connection threads talk via `mpsc`.
- Prefer existing `LoginStateMachine` / `PlayStateMachine`.
- Forge may be a local oracle only (never commit its artifacts).
- `unsafe` only in isolated, documented, audited modules (`forbid` today).

## Roadmap

| Phase | Focus | Status |
|-------|--------|--------|
| M1–M4 + A–I | Foundations → offline play stubs | **Done** |
| **J** | Offline join verification | Done / ongoing polish |
| **H0–H6** | Hakoniwa garden completion + miniaturization | **H0 in progress** |
| Later | Online mode (#60), richer Mod API | Optional |

**Not a goal:** drop-in Java Forge mod compatibility, Bedrock/統合版 protocol, or infinite vanilla terrain.

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
