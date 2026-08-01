# Project Rules

- Build a pure Rust server compatible with Minecraft Java Edition 1.20.1. Arbitrary Java Forge mod compatibility is not a goal.
- Use public documentation and black-box observation only; maintain a clean-room implementation.
- Never redistribute or commit Minecraft, Forge, mappings, decompiled output, local reference installations, or other reference artifacts.
- Do not copy implementation details from decompiled Minecraft code or Forge patches.
- Keep one authoritative tick thread until measurements justify carefully designed parallelism.
- Permit `unsafe` only in isolated, documented, audited modules.
- Avoid global `Arc<Mutex<_>>` state; use explicit ownership and narrow synchronization boundaries.
- Use differential tests against the local Forge 47.4.10 installation as an oracle, without incorporating its artifacts into this repository.
- Treat `素体データ/` as read-only local reference data: never modify, delete, stage, or commit it.
