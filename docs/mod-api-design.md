# Rust Mod API Design (Issue #101)

## Overview

A thin, trait-based Rust Mod API that allows third-party code to extend the
server without direct access to internal data structures. The API is built on
top of the tick-owned world mutation facade (#132) and the existing
`TickMessage` / `SessionEvent` message-passing architecture.

## Design Principles

1. **Tick-thread ownership**: The tick thread owns all mutable state (world,
   inventories, player positions). Mods never get `&mut` to world data.
   All mutations go through message passing via `WorldFacade`.

2. **Trait-based extensibility**: Mods implement traits (`Mod`, `BlockBehavior`,
   `ItemBehavior`) that the server calls at well-defined points.

3. **No global state**: Mods receive explicit context objects (`ModContext`)
   rather than accessing globals. This preserves the project's
   "no `Arc<Mutex<_>>` global state" rule.

4. **Clean-room boundary**: The API exposes only public types. Internal
   modules (`tick`, `session`, `world` internals) are not leaked.

5. **Serial execution**: All mod callbacks run on the tick thread, so mods
   don't need synchronization. This matches the "one authoritative tick
   thread" rule.

## Core Traits

### `Mod` — Entry point

```rust
pub trait Mod: Send + Sync {
    /// Unique mod identifier (used for namespacing, dependencies).
    fn id(&self) -> &str;

    /// Called once on server startup, before the tick loop begins.
    /// The mod can register block/item behaviors, set up config, etc.
    fn init(&self, ctx: &ModContext) -> Result<(), ModError>;

    /// Called every tick (20 TPS). Optional — return `None` for no-op.
    fn tick(&self, _ctx: &ModContext) -> Option<Result<(), ModError>> {
        None
    }

    /// Called on graceful shutdown.
    fn shutdown(&self, _ctx: &ModContext) {}
}
```

### `BlockBehavior` — Custom block logic

```rust
pub trait BlockBehavior: Send + Sync {
    /// Called when a player starts breaking this block.
    fn on_break_start(
        &self,
        ctx: &ModContext,
        player: EntityId,
        position: (i32, i32, i32),
    ) -> BreakAction {
        BreakAction::Default
    }

    /// Called when a block is placed by a player.
    fn on_place(
        &self,
        ctx: &ModContext,
        player: EntityId,
        position: (i32, i32, i32),
    ) -> PlaceAction {
        PlaceAction::Default
    }

    /// Returns the hardness (seconds to break by hand) for this block.
    fn hardness(&self) -> f32 {
        1.5 // default
    }

    /// Called when a player right-clicks the block.
    fn on_use(
        &self,
        ctx: &ModContext,
        player: EntityId,
        position: (i32, i32, i32),
    ) -> UseAction {
        UseAction::Default
    }
}
```

### `ItemBehavior` — Custom item logic

```rust
pub trait ItemBehavior: Send + Sync {
    /// Called when a player uses the item (right-click).
    fn on_use(
        &self,
        ctx: &ModContext,
        player: EntityId,
    ) -> ItemUseAction;

    /// Called when a player uses the item on a block.
    fn on_use_on_block(
        &self,
        ctx: &ModContext,
        player: EntityId,
        position: (i32, i32, i32),
        face: i32,
    ) -> ItemUseAction;
}
```

## Context Objects

### `ModContext` — Passed to all mod callbacks

```rust
pub struct ModContext {
    /// Facade for world mutations (set blocks, fill regions).
    world: WorldFacade,
    /// Read-only access to player list.
    players: PlayerView,
    /// Registry for registering custom blocks/items.
    registry: RegistryHandle,
    /// Server tick count (monotonically increasing).
    tick_count: u64,
}

impl ModContext {
    pub fn world(&self) -> &WorldFacade { &self.world }
    pub fn players(&self) -> &PlayerView { &self.players }
    pub fn registry(&self) -> &RegistryHandle { &self.registry }
    pub fn tick_count(&self) -> u64 { self.tick_count }
}
```

### `PlayerView` — Read-only player access

```rust
pub struct PlayerView {
    // Backed by a snapshot taken at tick start
    players: Vec<PlayerInfo>,
}

pub struct PlayerInfo {
    pub entity_id: i32,
    pub uuid: Uuid,
    pub username: String,
    pub gamemode: u8,
    pub position: (f64, f64, f64),
    pub health: f32,
    pub food: i32,
}

impl PlayerView {
    pub fn iter(&self) -> impl Iterator<Item = &PlayerInfo> { ... }
    pub fn get(&self, entity_id: i32) -> Option<&PlayerInfo> { ... }
    pub fn count(&self) -> usize { ... }
}
```

### `RegistryHandle` — Register custom content

```rust
pub struct RegistryHandle { /* internal sender */ }

impl RegistryHandle {
    /// Registers a custom block with the given behavior.
    pub fn register_block(
        &self,
        identifier: &str,
        behavior: Arc<dyn BlockBehavior>,
    ) -> Result<BlockId, ModError>;

    /// Registers a custom item with the given behavior.
    pub fn register_item(
        &self,
        identifier: &str,
        behavior: Arc<dyn ItemBehavior>,
    ) -> Result<ItemId, ModError>;
}
```

## Action Enums

```rust
pub enum BreakAction {
    /// Use default hardness-based break progress.
    Default,
    /// Instantly break the block (like Creative).
    Instant,
    /// Cancel the break (block is unbreakable).
    Cancel,
    /// Custom break time in ticks.
    Custom(u64),
}

pub enum PlaceAction {
    /// Allow the placement.
    Default,
    /// Cancel the placement.
    Cancel,
}

pub enum UseAction {
    /// Default vanilla behavior.
    Default,
    /// Consume the use event (no default behavior).
    Consume,
    /// Cancel and open a custom UI (future).
    OpenUi(UiId),
}

pub enum ItemUseAction {
    /// Default vanilla behavior.
    Default,
    /// Consume the use event.
    Consume,
    /// Transform the item (e.g., consume one, change to another).
    Transform(ItemId),
}
```

## Registration Flow

1. Server starts → loads mod shared libraries (via `dlopen` or static linking)
2. Each mod's `init()` is called with a `ModContext`
3. Mods register blocks/items via `RegistryHandle`
4. Tick loop begins → mods' `tick()` called every tick
5. On player events (dig, place, use), the server looks up the registered
   behavior and calls the appropriate method

## Integration with Existing Architecture

```
┌─────────────┐     TickMessage      ┌──────────────┐
│   Session    │ ──────────────────→ │  Tick Loop   │
│  (per-conn)  │ ←────────────────── │  (authorit.) │
└─────────────┘    SessionEvent      └──────┬───────┘
                                            │
                                   ┌────────┴────────┐
                                   │  Mod callbacks  │
                                   │  (on tick thread)│
                                   └────────┬────────┘
                                            │
                                   ┌────────┴────────┐
                                   │  WorldFacade    │
                                   │  (sends msgs)   │
                                   └─────────────────┘
```

- Mods run **on the tick thread** during the message-processing phase
- `WorldFacade` sends `TickMessage`s that are processed in the same tick
- Player snapshots (`PlayerView`) are taken at tick start for consistency

## Phased Implementation

### Phase 1: Core traits + context (MVP)
- Define `Mod`, `ModContext`, `WorldFacade` (already exists)
- Implement `ModRegistry` for loading/storing mods
- Wire `Mod::tick()` into the tick loop
- No custom blocks/items yet — just tick callbacks

### Phase 2: Block/Item behaviors
- Implement `BlockBehavior`, `ItemBehavior` traits
- Extend `RegistryHandle` for custom content
- Hook into dig/place/use event handling
- Replace hardcoded hardness stub (#133) with `BlockBehavior::hardness()`

### Phase 3: Player events
- `PlayerJoinEvent`, `PlayerLeaveEvent`, `PlayerDeathEvent`
- `ChatEvent` (cancelable)
- `PlayerMoveEvent` (cancelable for anti-cheat)

### Phase 4: Persistence + config
- Mod-specific data persistence (per-world, per-player)
- Mod configuration files (TOML/JSON)
- Mod dependency resolution

## Security Considerations

- Mods are `Send + Sync` but run on the tick thread — no raw pointer access
- `WorldFacade` only exposes safe operations (no chunk corruption)
- `PlayerView` is a snapshot — mods can't hold references across ticks
- Registry identifiers are namespaced (`modid:block_name`) to prevent conflicts

## Compatibility

- The API is additive — vanilla behavior is the default when no mod handles
  an event
- Mods can be enabled/disabled at runtime (for future hot-reloading)
- The API version is semver-tracked; breaking changes require a major bump

## Open Questions

1. **Dynamic loading**: Should mods be dynamically loaded (`libloading`) or
   statically linked? Static is simpler and safer; dynamic enables
   hot-reload but adds `unsafe` complexity.

2. **Event priority**: Should mods have priority levels for event handling
   (like Bukkit's `EventPriority`)? Or first-registered-first-served?

3. **Cross-mod communication**: Should mods be able to call other mods'
   APIs directly, or only through a shared event bus?

4. **Scheduling**: Should mods be able to schedule delayed tasks
   (`ctx.schedule(20, || { ... })` for "run in 1 second")?
