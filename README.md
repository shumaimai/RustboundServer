# Rustbound

Clean-room Minecraft Java Edition **1.20.1** (protocol **763**) server written in Rust.

Built from public documentation and black-box observation only. Minecraft, Forge, mappings, decompiled output, and other reference artifacts are never incorporated or redistributed.

This project is not affiliated with Mojang Studios, Microsoft, or the Forge project. Minecraft and related marks belong to their respective owners.

## Status: complete (Hakoniwa)

The product scope is **Hakoniwa (箱庭)**: a fixed-size garden, not infinite vanilla generation and not Forge compatibility.

| Capability | Status |
|------------|--------|
| Offline join, Play session, 20 TPS tick | Complete |
| Fixed gardens (`tiny` / `small` / `medium`) with world border | Complete |
| Block collision, dig / place | Complete |
| Map packs (overworld / nether / end) | Complete |
| Dimension transfer (portal pads, `/dim`) | Complete |
| Simple mobs | Complete |
| Static water / lava | Complete |
| Chests (minimal containers) | Complete |
| Size-optimized `dist` binary | Complete |
| Online mode | Optional ([#60](https://github.com/shumaimai/RustboundServer/issues/60)) |
| Forge / Bedrock / infinite terrain | Out of scope |

Specification: [docs/hakoniwa.md](docs/hakoniwa.md). Engineering history: [PROGRESS.md](PROGRESS.md).

## Run (offline)

```console
cp server.properties.example server.properties
cargo run -p rustbound-server --release -- --config server.properties
```

Connect a **1.20.1** offline client to `localhost:25565`. Example config defaults to Creative and `hakoniwa-size=tiny`.

Distribution build:

```console
cargo build -p rustbound-server --profile dist
```

Smoke checks:

```console
./scripts/smoke_offline_join.sh
cargo test -p rustbound-server --lib server_offline_playability_smoke
```

## Workspace

```
crates/
  rustbound-protocol/     # Wire codecs, Login / Play state machines
  rustbound-server/       # Listener, session, tick, world, hakoniwa
  rustbound-conformance/  # Black-box probes and differential helpers
```

## Build and test

```console
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## Design constraints

- Single authoritative tick thread (20 TPS); sessions communicate via channels.
- Prefer existing `LoginStateMachine` / `PlayStateMachine`.
- Forge may be used locally as an oracle only; never commit its artifacts.
- `unsafe` is forbidden at the workspace level unless isolated and audited.
- Contributors must follow [AGENTS.md](AGENTS.md).

## License

MIT OR Apache-2.0, at your option.
