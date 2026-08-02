//! Thin Rust Mod API (Issue #101).
//!
//! This module defines the core traits and types for the Mod API.
//! The implementation is phased — see `docs/mod-api-design.md` for the
//! full design document.
//!
//! Phase 1 (current): Core trait definitions and context types.
//! The traits are defined but not yet wired into the tick loop.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use rustbound_protocol::primitives::Uuid;

use crate::mutation::WorldFacade;
use crate::tick::{TickHandle, TickMessage};

/// Unique identifier for an entity (player, mob, etc.).
pub type EntityId = i32;

/// Unique identifier for a registered block.
pub type BlockId = i32;

/// Unique identifier for a registered item.
pub type ItemId = i32;

/// Error type for mod operations.
#[derive(Debug, Clone)]
pub enum ModError {
    /// A mod with this ID is already registered.
    DuplicateMod(String),
    /// A block/item with this identifier is already registered.
    DuplicateRegistration(String),
    /// An invalid identifier was provided.
    InvalidIdentifier(String),
    /// A generic mod error.
    Other(String),
}

impl std::fmt::Display for ModError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateMod(id) => write!(f, "duplicate mod: {id}"),
            Self::DuplicateRegistration(id) => write!(f, "duplicate registration: {id}"),
            Self::InvalidIdentifier(id) => write!(f, "invalid identifier: {id}"),
            Self::Other(msg) => write!(f, "mod error: {msg}"),
        }
    }
}

impl std::error::Error for ModError {}

/// Action to take when a player starts breaking a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakAction {
    /// Use default hardness-based break progress.
    Default,
    /// Instantly break the block (like Creative mode).
    Instant,
    /// Cancel the break (block is unbreakable).
    Cancel,
    /// Custom break time in ticks.
    Custom(u64),
}

/// Action to take when a player places a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceAction {
    /// Allow the placement.
    Default,
    /// Cancel the placement.
    Cancel,
}

/// Action to take when a player uses (right-clicks) a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseAction {
    /// Default vanilla behavior.
    Default,
    /// Consume the use event (no default behavior).
    Consume,
}

/// Action to take when a player uses an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemUseAction {
    /// Default vanilla behavior.
    Default,
    /// Consume the use event.
    Consume,
    /// Transform the item to a different item ID.
    Transform(ItemId),
}

/// Read-only player information snapshot.
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    /// The entity ID.
    pub entity_id: EntityId,
    /// The player's UUID.
    pub uuid: Uuid,
    /// The player's username.
    pub username: String,
    /// The player's gamemode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
    pub gamemode: u8,
    /// The player's position (x, y, z).
    pub position: (f64, f64, f64),
    /// The player's health.
    pub health: f32,
    /// The player's food level.
    pub food: i32,
}

/// Read-only view of all online players.
#[derive(Debug, Clone)]
pub struct PlayerView {
    players: Vec<PlayerInfo>,
}

impl PlayerView {
    /// Creates a new `PlayerView` from a list of player info.
    pub fn new(players: Vec<PlayerInfo>) -> Self {
        Self { players }
    }

    /// Returns an iterator over all online players.
    pub fn iter(&self) -> impl Iterator<Item = &PlayerInfo> {
        self.players.iter()
    }

    /// Returns the player with the given entity ID, if online.
    pub fn get(&self, entity_id: EntityId) -> Option<&PlayerInfo> {
        self.players.iter().find(|p| p.entity_id == entity_id)
    }

    /// Returns the number of online players.
    pub fn count(&self) -> usize {
        self.players.len()
    }
}

/// Context passed to mod callbacks.
///
/// Provides access to world mutations, player snapshots, and the registry.
/// All access is read-only except for `WorldFacade`, which sends messages
/// to the tick thread.
pub struct ModContext {
    /// Facade for world mutations.
    world: WorldFacade,
    /// Read-only player snapshot.
    players: PlayerView,
    /// Current tick count.
    tick_count: u64,
}

impl ModContext {
    /// Creates a new `ModContext`.
    pub fn new(world: WorldFacade, players: PlayerView, tick_count: u64) -> Self {
        Self {
            world,
            players,
            tick_count,
        }
    }

    /// Returns the world mutation facade.
    pub fn world(&self) -> &WorldFacade {
        &self.world
    }

    /// Returns the read-only player view.
    pub fn players(&self) -> &PlayerView {
        &self.players
    }

    /// Returns the current tick count.
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }
}

/// Core mod trait — the entry point for all mods.
///
/// Mods implement this trait to hook into the server lifecycle.
/// All callbacks run on the tick thread, so no synchronization is needed.
pub trait Mod: Send + Sync {
    /// Returns the mod's unique identifier (e.g., "my_mod").
    fn id(&self) -> &str;

    /// Called once on server startup, before the tick loop begins.
    fn init(&self, _ctx: &ModContext) -> Result<(), ModError> {
        Ok(())
    }

    /// Called every tick (20 TPS). Default implementation is a no-op.
    fn tick(&self, _ctx: &ModContext) -> Result<(), ModError> {
        Ok(())
    }

    /// Called on graceful shutdown.
    fn shutdown(&self, _ctx: &ModContext) {}
}

/// Custom block behavior.
pub trait BlockBehavior: Send + Sync {
    /// Called when a player starts breaking this block.
    fn on_break_start(
        &self,
        _ctx: &ModContext,
        _player: EntityId,
        _position: (i32, i32, i32),
    ) -> BreakAction {
        BreakAction::Default
    }

    /// Called when a block is placed by a player.
    fn on_place(
        &self,
        _ctx: &ModContext,
        _player: EntityId,
        _position: (i32, i32, i32),
    ) -> PlaceAction {
        PlaceAction::Default
    }

    /// Returns the hardness (seconds to break by hand) for this block.
    fn hardness(&self) -> f32 {
        1.5
    }

    /// Called when a player right-clicks the block.
    fn on_use(
        &self,
        _ctx: &ModContext,
        _player: EntityId,
        _position: (i32, i32, i32),
    ) -> UseAction {
        UseAction::Default
    }
}

/// Custom item behavior.
pub trait ItemBehavior: Send + Sync {
    /// Called when a player uses the item (right-click in air).
    fn on_use(&self, _ctx: &ModContext, _player: EntityId) -> ItemUseAction {
        ItemUseAction::Default
    }

    /// Called when a player uses the item on a block.
    fn on_use_on_block(
        &self,
        _ctx: &ModContext,
        _player: EntityId,
        _position: (i32, i32, i32),
        _face: i32,
    ) -> ItemUseAction {
        ItemUseAction::Default
    }
}

/// Registry for mods and custom content.
///
/// Maintained by the server; mods register themselves and their custom
/// blocks/items during `init()`.
pub struct ModRegistry {
    mods: Vec<Arc<dyn Mod>>,
}

impl ModRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self { mods: Vec::new() }
    }

    /// Registers a mod.
    pub fn register(&mut self, mod_impl: Arc<dyn Mod>) -> Result<(), ModError> {
        let id = mod_impl.id();
        if self.mods.iter().any(|m| m.id() == id) {
            return Err(ModError::DuplicateMod(id.to_string()));
        }
        self.mods.push(mod_impl);
        Ok(())
    }

    /// Returns an iterator over all registered mods.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Mod>> {
        self.mods.iter()
    }

    /// Returns the number of registered mods.
    pub fn len(&self) -> usize {
        self.mods.len()
    }

    /// Returns true if no mods are registered.
    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }
}

impl Default for ModRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A mod host that manages the lifecycle of registered mods.
///
/// Created from a `TickHandle` or a `Sender<TickMessage>`, this provides
/// the integration point between the tick loop and the mod system.
pub struct ModHost {
    registry: ModRegistry,
    facade: WorldFacade,
}

impl ModHost {
    /// Creates a new `ModHost` from a tick handle.
    pub fn new(handle: &TickHandle) -> Self {
        Self {
            registry: ModRegistry::new(),
            facade: handle.world_facade(),
        }
    }

    /// Creates a new `ModHost` from a tick message sender.
    ///
    /// This is useful when the `TickHandle` doesn't exist yet (e.g.,
    /// the sender is being passed into `start_tick_loop`).
    pub fn from_sender(sender: Sender<TickMessage>) -> Self {
        Self {
            registry: ModRegistry::new(),
            facade: WorldFacade::new(sender),
        }
    }

    /// Registers a mod.
    pub fn register(&mut self, mod_impl: Arc<dyn Mod>) -> Result<(), ModError> {
        self.registry.register(mod_impl)
    }

    /// Initializes all registered mods.
    pub fn init_all(&self, players: PlayerView, tick_count: u64) -> Result<(), ModError> {
        let ctx = ModContext::new(self.facade.clone(), players, tick_count);
        for m in self.registry.iter() {
            m.init(&ctx)?;
        }
        Ok(())
    }

    /// Calls `tick()` on all registered mods.
    pub fn tick_all(&self, players: PlayerView, tick_count: u64) -> Result<(), ModError> {
        let ctx = ModContext::new(self.facade.clone(), players, tick_count);
        for m in self.registry.iter() {
            m.tick(&ctx)?;
        }
        Ok(())
    }

    /// Calls `shutdown()` on all registered mods.
    pub fn shutdown_all(&self, players: PlayerView, tick_count: u64) {
        let ctx = ModContext::new(self.facade.clone(), players, tick_count);
        for m in self.registry.iter() {
            m.shutdown(&ctx);
        }
    }

    /// Returns the number of registered mods.
    pub fn mod_count(&self) -> usize {
        self.registry.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, dead_code)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A test mod that counts tick calls.
    struct TestMod {
        id: String,
        tick_count: Mutex<u64>,
    }

    impl TestMod {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                tick_count: Mutex::new(0),
            }
        }

        fn ticks(&self) -> u64 {
            *self.tick_count.lock().unwrap()
        }
    }

    impl Mod for TestMod {
        fn id(&self) -> &str {
            &self.id
        }

        fn tick(&self, _ctx: &ModContext) -> Result<(), ModError> {
            *self.tick_count.lock().unwrap() += 1;
            Ok(())
        }
    }

    #[test]
    fn mod_registry_register_and_iterate() {
        let mut registry = ModRegistry::new();
        let mod1 = Arc::new(TestMod::new("mod1"));
        let mod2 = Arc::new(TestMod::new("mod2"));
        registry.register(mod1.clone()).unwrap();
        registry.register(mod2.clone()).unwrap();
        assert_eq!(registry.len(), 2);
        let ids: Vec<_> = registry.iter().map(|m| m.id().to_string()).collect();
        assert!(ids.contains(&"mod1".to_string()));
        assert!(ids.contains(&"mod2".to_string()));
    }

    #[test]
    fn mod_registry_rejects_duplicate() {
        let mut registry = ModRegistry::new();
        let mod1 = Arc::new(TestMod::new("dup"));
        registry.register(mod1.clone()).unwrap();
        let result = registry.register(Arc::new(TestMod::new("dup")));
        assert!(matches!(result, Err(ModError::DuplicateMod(_))));
    }

    #[test]
    fn mod_registry_is_empty() {
        let registry = ModRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn player_view_get_and_count() {
        let players = vec![
            PlayerInfo {
                entity_id: 1,
                uuid: Uuid::new(0, 0),
                username: "Alice".to_string(),
                gamemode: 0,
                position: (0.0, 64.0, 0.0),
                health: 20.0,
                food: 20,
            },
            PlayerInfo {
                entity_id: 2,
                uuid: Uuid::new(0, 1),
                username: "Bob".to_string(),
                gamemode: 1,
                position: (10.0, 64.0, 10.0),
                health: 20.0,
                food: 20,
            },
        ];
        let view = PlayerView::new(players);
        assert_eq!(view.count(), 2);
        assert!(view.get(1).is_some());
        assert!(view.get(999).is_none());
        assert_eq!(view.get(2).unwrap().username, "Bob");
    }

    #[test]
    fn break_action_variants() {
        assert_eq!(BreakAction::Default, BreakAction::Default);
        assert_eq!(BreakAction::Instant, BreakAction::Instant);
        assert_eq!(BreakAction::Cancel, BreakAction::Cancel);
        assert_eq!(BreakAction::Custom(100), BreakAction::Custom(100));
        assert_ne!(BreakAction::Custom(50), BreakAction::Custom(100));
    }

    #[test]
    fn mod_error_display() {
        assert_eq!(
            ModError::DuplicateMod("test".to_string()).to_string(),
            "duplicate mod: test"
        );
        assert_eq!(
            ModError::DuplicateRegistration("block".to_string()).to_string(),
            "duplicate registration: block"
        );
        assert_eq!(
            ModError::InvalidIdentifier("bad!".to_string()).to_string(),
            "invalid identifier: bad!"
        );
        assert_eq!(
            ModError::Other("custom".to_string()).to_string(),
            "mod error: custom"
        );
    }

    #[test]
    fn block_behavior_defaults() {
        struct DefaultBlock;
        impl BlockBehavior for DefaultBlock {}

        let b = DefaultBlock;
        assert_eq!(b.hardness(), 1.5);
    }

    #[test]
    fn item_behavior_defaults() {
        struct DefaultItem;
        impl ItemBehavior for DefaultItem {}

        let i = DefaultItem;
        // Just verify it compiles and has defaults
        let _ = i.on_use(
            &ModContext::new(
                WorldFacade::new(std::sync::mpsc::channel().0),
                PlayerView::new(vec![]),
                0,
            ),
            1,
        );
    }
}
