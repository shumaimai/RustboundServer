# Rustbound

Rustbound is a clean-room, pure Rust server project targeting Minecraft Java Edition 1.20.1 (protocol 763). It is built only from public documentation and black-box observation; no Minecraft, Forge, mapping, decompiled, or other reference artifacts are incorporated.

This project is not affiliated with, endorsed by, or sponsored by Mojang Studios, Microsoft, or the Forge project. Minecraft and related names and marks belong to their respective owners.

## Status and scope

Rustbound is currently pre-alpha. The repository contains only the initial workspace skeleton; networking and gameplay are not implemented. Compatibility with arbitrary Java Forge mods is explicitly out of scope.

## Build and test

```console
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Contributors must read and follow [AGENTS.md](AGENTS.md).

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option.
