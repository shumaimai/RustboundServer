# Rustbound

Rustbound is a clean-room, pure Rust server targeting Minecraft Java Edition **1.20.1** (protocol **763**). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status

Pre-alpha. Protocol codecs and a runnable server skeleton exist; **a client cannot yet enter Play** (Login Success currently ends the connection). Open issues describe the remaining work.

| Layer | Status |
|-------|--------|
| Protocol codecs (Handshake / Status / Login / Play core) | Implemented |
| Login / Play state machines (in-memory) | Implemented |
| Conformance probes (Status / Login / Play) | Implemented |
| TCP listener + connection handler | Status + offline Login work |
| Tick loop (20 TPS) + World / PlayerHandle | Exist but **not wired** to connections |
| Play session (Join Game → world) | **Not integrated** |
| Chunk delivery / gameplay | Not started |

Closed foundations: Issues [#1](https://github.com/shumaimai/RustboundServer/issues/1)–[#44](https://github.com/shumaimai/RustboundServer/issues/44).

## Workspace

```
crates/
  rustbound-protocol/     # Wire codecs + Login/Play state machines
  rustbound-server/       # Listener, connection router, tick, world, config
  rustbound-conformance/  # Black-box probes + Status diff normalizer
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

Optional: `--config path/to/server.properties`, `--host`, `--port`.

## Architecture notes

- One authoritative tick thread (20 TPS); connection threads talk via `mpsc` messages — avoid global `Arc<Mutex<_>>` world state.
- Prefer existing `LoginStateMachine` / `PlayStateMachine` over re-implementing flows in the server.
- Conformance probes should drive integration; Forge may be used as a local oracle only (never commit its artifacts).
- `unsafe` only in isolated, documented, audited modules (`forbid` at workspace level today).

## Roadmap (next)

Tracked in **[#74](https://github.com/shumaimai/RustboundServer/issues/74)** (Phases A–E checklist). Start here:

| Phase | Focus | Issues |
|-------|--------|--------|
| **A (P0)** | Play integration — Join Game on the live server | [#51](https://github.com/shumaimai/RustboundServer/issues/51)–[#55](https://github.com/shumaimai/RustboundServer/issues/55) |
| **B (P1)** | Compression, login SM, registry codec, join sequence, online mode | [#56](https://github.com/shumaimai/RustboundServer/issues/56)–[#62](https://github.com/shumaimai/RustboundServer/issues/62) |
| **C (P2)** | Chunk generation and delivery | [#63](https://github.com/shumaimai/RustboundServer/issues/63)–[#65](https://github.com/shumaimai/RustboundServer/issues/65) |
| **D (P3)** | Movement, tab list, remote players, dig/place, cleanup | [#66](https://github.com/shumaimai/RustboundServer/issues/66)–[#70](https://github.com/shumaimai/RustboundServer/issues/70) |
| **E (P3)** | Play differentials, live status counts, signal shutdown | [#71](https://github.com/shumaimai/RustboundServer/issues/71)–[#73](https://github.com/shumaimai/RustboundServer/issues/73) |

Recommended first slice: **#52 → #53 → #51 → #54 → #55** (Confirm Teleportation, session, wire Play, tick KeepAlive, conformance).

## Contributing

Contributors must read and follow [AGENTS.md](AGENTS.md) (clean-room rules, tick-thread ownership, no reference artifacts in-repo).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
