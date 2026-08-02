//! Tick loop for the Rustbound server.
//!
//! Runs at a fixed 20 TPS (50ms per tick) on a single authoritative thread.
//! The tick loop owns the world and processes periodic tasks like keep-alive
//! scheduling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::SessionEvent;
use crate::world::World;
use rustbound_protocol::primitives::Uuid;

/// The target tick rate (20 ticks per second).
pub const TPS: u64 = 20;

/// The duration of a single tick (50 milliseconds).
pub const TICK_DURATION: Duration = Duration::from_millis(1000 / TPS);

/// The interval between keep-alive packets (15 seconds = 300 ticks).
pub const KEEP_ALIVE_INTERVAL_TICKS: u64 = 300;

/// The initial chunk radius sent at join (matches `send_initial_chunks`).
const INITIAL_CHUNK_RADIUS: i32 = 2;

/// Per-player chunk streaming state, owned by the tick loop.
///
/// Tracks the player's current center chunk, effective view distance, and
/// the set of chunks the client currently has loaded. The tick loop uses
/// this to compute which new chunks to send when the player crosses chunk
/// borders.
#[derive(Debug, Clone)]
struct PlayerChunkState {
    /// The current center chunk X.
    center_x: i32,
    /// The current center chunk Z.
    center_z: i32,
    /// The effective view distance (min of server and client).
    view_distance: i32,
    /// The set of chunk positions the client currently has loaded.
    loaded_chunks: HashSet<crate::world::ChunkPos>,
}

/// The number of slots in a player inventory (36 main + 4 armor + 1 offhand + 5 crafting).
pub const PLAYER_INVENTORY_SIZE: usize = 46;

/// Default health for a new or respawned player.
pub const DEFAULT_HEALTH: f32 = 20.0;
/// Default food for a new or respawned player.
pub const DEFAULT_FOOD: i32 = 20;
/// Default food saturation for a new or respawned player.
pub const DEFAULT_FOOD_SATURATION: f32 = 5.0;
/// The Y coordinate below which the void kills players.
pub const VOID_DEATH_Y: f64 = -64.0;
/// Food drain interval in ticks (every 4 ticks = 5 times per second).
/// This is a stub approximation of vanilla exhaustion; real Minecraft
/// ties this to exhaustion level (0.1/tick passive), but for the minimal
/// implementation we use a simple tick-count-based drain.
pub const FOOD_DRAIN_INTERVAL_TICKS: u64 = 80; // 4 seconds per drain tick
/// Starvation damage applied when food reaches 0.
pub const STARVATION_DAMAGE: f32 = 1.0;
/// Gamemode Creative (exempt from food drain).
pub const GAMEMODE_CREATIVE: u8 = 1;
/// Default block hardness for the minimal stub (seconds to break by hand).
/// Real Minecraft varies per block type; this is a uniform approximation.
pub const DEFAULT_BLOCK_HARDNESS_TICKS: u64 = 30; // 1.5 seconds at 20 TPS
/// Number of destroy stages (0–9).
pub const MAX_DESTROY_STAGE: i8 = 9;

/// Tracks per-player block break progress.
#[derive(Debug, Clone)]
struct BlockBreakProgress {
    /// The block position being broken.
    position: (i32, i32, i32),
    /// Ticks elapsed since break started.
    ticks_elapsed: u64,
    /// Total ticks needed to break (based on hardness).
    total_ticks: u64,
    /// Last destroy stage sent to client.
    last_stage: i8,
}

/// Per-player inventory state, owned by the tick loop.
///
/// Tracks the player's 46-slot inventory, held hotbar slot, and vitals
/// (health, food, saturation). The tick loop is the single owner of
/// inventory and vital mutations; sessions forward client requests
/// (Set Creative Mode Slot, Set Held Item, Client Status respawn) to the
/// tick loop, which applies them and sends confirmation events back.
#[derive(Debug, Clone)]
struct PlayerInventory {
    /// The 46 inventory slots (0-8 hotbar, 9-35 main, 36-39 armor, 40 offhand, 41-45 crafting).
    slots: Vec<rustbound_protocol::play::Slot>,
    /// The currently held hotbar slot (0-8).
    held_slot: u8,
    /// Server-managed state ID for synchronization.
    state_id: i32,
    /// The player's health (0 or less = dead, 20 = full HP).
    health: f32,
    /// The player's food level (0-20).
    food: i32,
    /// The player's food saturation (0.0 to 5.0).
    food_saturation: f32,
    /// Whether the player is currently dead (awaiting respawn).
    is_dead: bool,
}

impl PlayerInventory {
    fn new() -> Self {
        Self {
            slots: vec![rustbound_protocol::play::Slot::empty(); PLAYER_INVENTORY_SIZE],
            held_slot: 0,
            state_id: 0,
            health: DEFAULT_HEALTH,
            food: DEFAULT_FOOD,
            food_saturation: DEFAULT_FOOD_SATURATION,
            is_dead: false,
        }
    }

    /// Sets a slot and returns true if the slot actually changed.
    fn set_slot(&mut self, index: usize, item: rustbound_protocol::play::Slot) -> bool {
        if index >= self.slots.len() {
            return false;
        }
        let changed = self.slots[index] != item;
        if changed {
            self.slots[index] = item;
            self.state_id += 1;
        }
        changed
    }

    /// Sets the held slot, clamping to 0-8. Returns true if changed.
    fn set_held_slot(&mut self, slot: i16) -> bool {
        if !(0..=8).contains(&slot) {
            return false;
        }
        let new_slot = slot as u8;
        if self.held_slot != new_slot {
            self.held_slot = new_slot;
            return true;
        }
        false
    }

    /// Kills the player (sets health to 0, marks as dead).
    fn kill(&mut self) {
        self.health = 0.0;
        self.is_dead = true;
    }

    /// Respawns the player (resets vitals to default, clears death state).
    fn respawn(&mut self) {
        self.health = DEFAULT_HEALTH;
        self.food = DEFAULT_FOOD;
        self.food_saturation = DEFAULT_FOOD_SATURATION;
        self.is_dead = false;
    }

    /// Applies one food drain tick: deplete saturation first, then food.
    /// Returns true if food or saturation changed.
    fn drain_food(&mut self) -> bool {
        if self.food_saturation > 0.0 {
            self.food_saturation = (self.food_saturation - 1.0).max(0.0);
            true
        } else if self.food > 0 {
            self.food -= 1;
            true
        } else {
            false
        }
    }

    /// Applies starvation damage if food is 0 and player is not dead.
    /// Returns true if damage was applied.
    fn starve(&mut self) -> bool {
        if self.food == 0 && !self.is_dead {
            self.health = (self.health - STARVATION_DAMAGE).max(0.0);
            if self.health <= 0.0 {
                self.is_dead = true;
            }
            true
        } else {
            false
        }
    }
}

impl PlayerChunkState {
    /// Creates a new chunk state for a player joining at the spawn point
    /// (center chunk 0, 0) with the given server view distance.
    fn new(server_view_distance: i32) -> Self {
        let view_distance = server_view_distance.clamp(0, INITIAL_CHUNK_RADIUS);
        let loaded_chunks = World::desired_chunks(0, 0, INITIAL_CHUNK_RADIUS)
            .into_iter()
            .collect();
        Self {
            center_x: 0,
            center_z: 0,
            view_distance,
            loaded_chunks,
        }
    }

    /// Computes the desired chunk set for the current center and view distance.
    fn desired_chunk_set(&self) -> HashSet<crate::world::ChunkPos> {
        World::desired_chunks(self.center_x, self.center_z, self.view_distance)
            .into_iter()
            .collect()
    }

    /// Updates the center chunk. Returns (center_changed, new_chunks, unloaded_chunks) where
    /// new_chunks is the set of chunks to load (in desired but not in loaded) and
    /// unloaded_chunks is the set of chunks to unload (in loaded but not in desired).
    fn update_center(
        &mut self,
        new_cx: i32,
        new_cz: i32,
    ) -> (
        bool,
        Vec<crate::world::ChunkPos>,
        Vec<crate::world::ChunkPos>,
    ) {
        if self.center_x == new_cx && self.center_z == new_cz {
            return (false, Vec::new(), Vec::new());
        }
        self.center_x = new_cx;
        self.center_z = new_cz;
        let desired = self.desired_chunk_set();
        let new_chunks: Vec<_> = desired
            .iter()
            .filter(|pos| !self.loaded_chunks.contains(pos))
            .copied()
            .collect();
        let unloaded_chunks: Vec<_> = self
            .loaded_chunks
            .iter()
            .filter(|pos| !desired.contains(pos))
            .copied()
            .collect();
        self.loaded_chunks = desired;
        (true, new_chunks, unloaded_chunks)
    }

    /// Updates the view distance. Returns (new_chunks, unloaded_chunks) where
    /// new_chunks are chunks now in range but weren't before, and
    /// unloaded_chunks are chunks no longer in range.
    fn update_view_distance(
        &mut self,
        new_vd: i32,
    ) -> (Vec<crate::world::ChunkPos>, Vec<crate::world::ChunkPos>) {
        let new_vd = new_vd.max(0);
        if self.view_distance == new_vd {
            return (Vec::new(), Vec::new());
        }
        self.view_distance = new_vd;
        let desired = self.desired_chunk_set();
        let new_chunks: Vec<_> = desired
            .iter()
            .filter(|pos| !self.loaded_chunks.contains(pos))
            .copied()
            .collect();
        let unloaded_chunks: Vec<_> = self
            .loaded_chunks
            .iter()
            .filter(|pos| !desired.contains(pos))
            .copied()
            .collect();
        self.loaded_chunks = desired;
        (new_chunks, unloaded_chunks)
    }
}

/// Converts a world X coordinate to its chunk X coordinate.
fn chunk_x_from_world(x: f64) -> i32 {
    (x.floor() as i32) >> 4
}

/// Converts a world Z coordinate to its chunk Z coordinate.
fn chunk_z_from_world(z: f64) -> i32 {
    (z.floor() as i32) >> 4
}

/// Formats a player chat message as a JSON chat component string.
///
/// In offline mode, we use the System Chat Message packet (0x5D) for all
/// chat. The content is a JSON chat component. We produce a simple
/// `<username> <message>` format with the username in gray and the
/// message in white, matching the vanilla chat appearance.
fn format_chat_message(username: &str, message: &str) -> String {
    // Build a JSON chat component: {"text":"<username> message"}
    // Simple format: {"text":"<username> <message>"}
    // We escape the username and message for JSON safety.
    let escaped = format!("<{username}> {message}")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("{{\"text\":\"{escaped}\"}}")
}

/// A message sent to the tick loop.
#[derive(Debug)]
pub enum TickMessage {
    /// Shut down the tick loop.
    Shutdown,
    /// A player joined (entity ID assigned externally).
    PlayerJoined {
        /// The entity ID.
        entity_id: i32,
        /// The player's UUID.
        uuid: Uuid,
        /// The player's username.
        username: String,
        /// The player's gamemode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
        gamemode: u8,
        /// The server view distance (in chunks).
        view_distance: i32,
        /// Channel for sending events back to this player's session.
        event_sender: Sender<SessionEvent>,
    },
    /// A player left.
    PlayerLeft {
        /// The entity ID.
        entity_id: i32,
    },
    /// A player's position was updated.
    PlayerPositionUpdate {
        /// The entity ID.
        entity_id: i32,
        /// The new X coordinate.
        x: f64,
        /// The new Y coordinate.
        y: f64,
        /// The new Z coordinate.
        z: f64,
        /// The new yaw (degrees).
        yaw: f32,
        /// The new pitch (degrees).
        pitch: f32,
        /// Whether the player is on the ground.
        on_ground: bool,
    },
    /// A block was changed by a player (dig/place).
    SetBlock {
        /// The block position.
        position: (i32, i32, i32),
        /// The new block state ID (0 = air).
        block_state: i32,
    },
    /// A player attempted to place a block (UseItemOn).
    /// The tick loop looks up the held hotbar item and places the
    /// corresponding block from the registry.
    PlaceBlock {
        /// The entity ID of the player.
        entity_id: i32,
        /// The position of the block being placed against.
        position: (i32, i32, i32),
        /// The face being placed on (0=bottom, 1=top, 2=north, 3=south, 4=west, 5=east).
        face: i32,
    },
    /// A client sent its view distance via Client Information.
    SetClientViewDistance {
        /// The entity ID of the player.
        entity_id: i32,
        /// The client's requested view distance (in chunks).
        view_distance: i32,
    },
    /// A Survival player started/aborted/stopped destroying a block.
    DigBlock {
        /// The entity ID of the player.
        entity_id: i32,
        /// The dig action (0=start, 1=abort, 2=stop).
        action: i32,
        /// The block position.
        position: (i32, i32, i32),
    },
    /// A creative-mode player set a slot (Set Creative Mode Slot packet).
    SetCreativeSlot {
        /// The entity ID of the player.
        entity_id: i32,
        /// The slot index (-1 for drop).
        slot: i16,
        /// The item to place in the slot.
        item: rustbound_protocol::play::Slot,
    },
    /// A player changed their hotbar selection (Set Held Item serverbound).
    SetHeldItem {
        /// The entity ID of the player.
        entity_id: i32,
        /// The hotbar slot (0-8).
        slot: i16,
    },
    /// A player sent a chat message.
    ChatMessage {
        /// The entity ID of the sender.
        entity_id: i32,
        /// The sender's UUID.
        uuid: Uuid,
        /// The sender's username.
        username: String,
        /// The chat message text.
        message: String,
    },
    /// A player sent Client Status (Perform Respawn).
    ClientStatus {
        /// The entity ID of the player.
        entity_id: i32,
        /// The action ID (0 = Perform respawn, 1 = Request stats).
        action: i32,
    },
}

/// A message sent from the tick loop to the server.
#[derive(Debug, Clone)]
pub enum TickEvent {
    /// The tick loop has shut down.
    Shutdown,
    /// A player was added to the world.
    PlayerAdded {
        /// The entity ID.
        entity_id: i32,
    },
    /// A player was removed from the world.
    PlayerRemoved {
        /// The entity ID.
        entity_id: i32,
    },
}

/// A handle to the running tick loop.
pub struct TickHandle {
    shutdown: Arc<AtomicBool>,
    sender: Sender<TickMessage>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TickHandle {
    /// Sends a message to the tick loop.
    pub fn send(
        &self,
        message: TickMessage,
    ) -> Result<(), std::sync::mpsc::SendError<TickMessage>> {
        self.sender.send(message)
    }

    /// Returns a clone of the sender for sending messages to the tick loop.
    pub fn sender(&self) -> Sender<TickMessage> {
        self.sender.clone()
    }

    /// Signals the tick loop to shut down and waits for it to exit.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.sender.send(TickMessage::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TickHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// An error encountered while starting the tick loop.
#[derive(Debug)]
pub enum TickStartError {
    /// Failed to spawn the tick thread.
    ThreadSpawn(std::io::Error),
}

impl std::fmt::Display for TickStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThreadSpawn(error) => write!(formatter, "failed to spawn tick thread: {error}"),
        }
    }
}

impl std::error::Error for TickStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ThreadSpawn(error) => Some(error),
        }
    }
}

/// Starts the tick loop on a new thread.
///
/// Returns a handle for sending messages and a receiver for events.
/// The `player_count` atomic is incremented on join and decremented on leave.
/// The `level_name` is used to load/save block overrides and player data to disk.
/// The `autosave_interval_secs` controls periodic saves (0 = disabled, save only on shutdown).
pub fn start_tick_loop(
    player_count: Arc<AtomicUsize>,
    level_name: String,
    autosave_interval_secs: u64,
) -> Result<(TickHandle, Receiver<TickEvent>), TickStartError> {
    let (msg_tx, msg_rx) = channel::<TickMessage>();
    let (event_tx, event_rx) = channel::<TickEvent>();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    let thread = thread::Builder::new()
        .name("rustbound-tick".to_string())
        .spawn(move || {
            run_tick_loop(
                msg_rx,
                event_tx,
                shutdown_clone,
                player_count,
                level_name,
                autosave_interval_secs,
            );
        })
        .map_err(TickStartError::ThreadSpawn)?;

    Ok((
        TickHandle {
            shutdown,
            sender: msg_tx,
            thread: Some(thread),
        },
        event_rx,
    ))
}

fn run_tick_loop(
    msg_rx: Receiver<TickMessage>,
    event_tx: Sender<TickEvent>,
    shutdown: Arc<AtomicBool>,
    player_count: Arc<AtomicUsize>,
    level_name: String,
    autosave_interval_secs: u64,
) {
    // Load block overrides and player data from disk on startup
    let loaded = crate::persist::load_overrides(&level_name);
    let mut saved_players = crate::persist::load_players(&level_name);
    let mut world = World::new();
    world.load_block_overrides(loaded);
    // Autosave interval in ticks (20 TPS). 0 means disabled.
    let autosave_interval_ticks: u64 = autosave_interval_secs.saturating_mul(20);
    let mut last_autosave_tick: u64 = 0;
    let mut tick_count: u64 = 0;
    let mut last_keep_alive_tick: u64 = 0;
    let mut session_senders: HashMap<i32, Sender<SessionEvent>> = HashMap::new();
    let mut chunk_states: HashMap<i32, PlayerChunkState> = HashMap::new();
    let mut inventories: HashMap<i32, PlayerInventory> = HashMap::new();
    let mut break_progress: HashMap<i32, BlockBreakProgress> = HashMap::new();

    while !shutdown.load(Ordering::Acquire) {
        let tick_start = Instant::now();

        // Process incoming messages (non-blocking)
        while let Ok(msg) = msg_rx.try_recv() {
            match msg {
                TickMessage::Shutdown => {
                    // Save block overrides to disk before shutting down
                    if let Err(e) =
                        crate::persist::save_overrides(&level_name, world.block_overrides())
                    {
                        eprintln!("error: failed to save block overrides: {}", e);
                    }
                    // Merge online players into the cache, then write everyone
                    // (including players who already left this session).
                    merge_online_into_saved(&world, &inventories, &mut saved_players);
                    if let Err(e) = crate::persist::save_players(&level_name, &saved_players) {
                        eprintln!("error: failed to save player data: {}", e);
                    }
                    let _ = event_tx.send(TickEvent::Shutdown);
                    return;
                }
                TickMessage::PlayerJoined {
                    entity_id,
                    uuid,
                    username,
                    gamemode,
                    view_distance,
                    event_sender,
                } => {
                    // Check for saved player data
                    let saved = saved_players.get(&uuid).cloned();
                    // Prefer persisted gamemode; otherwise keep the join-message value (#122).
                    let gamemode = saved.as_ref().map(|d| d.gamemode).unwrap_or(gamemode);
                    let mut player = crate::world::PlayerHandle::new(
                        entity_id,
                        uuid,
                        username.clone(),
                        gamemode,
                    );
                    // Restore position if saved
                    if let Some(ref d) = saved {
                        player.set_position(d.x, d.y, d.z);
                        player.set_rotation(d.yaw, d.pitch);
                    }
                    let (px, py, pz) = player.position();
                    world.add_player(player);
                    session_senders.insert(entity_id, event_sender.clone());
                    chunk_states.insert(entity_id, PlayerChunkState::new(view_distance));
                    // Restore inventory or create new
                    let mut inv = PlayerInventory::new();
                    if let Some(ref d) = saved {
                        inv.held_slot = d.held_slot;
                        inv.health = d.health;
                        inv.food = d.food;
                        inv.food_saturation = d.food_saturation;
                        // Restore slots
                        for (i, (present, item_id, count, nbt)) in d.slots.iter().enumerate() {
                            if i < inv.slots.len() {
                                inv.slots[i] = rustbound_protocol::play::Slot {
                                    present: *present,
                                    item_id: *item_id,
                                    count: *count,
                                    nbt: nbt.clone(),
                                };
                            }
                        }
                    }
                    inventories.insert(entity_id, inv);

                    // Send initial inventory to the new player
                    if let Some(inv) = inventories.get(&entity_id) {
                        let _ = event_sender.send(SessionEvent::SetContainerContent {
                            window_id: 0,
                            state_id: inv.state_id,
                            slots: inv.slots.clone(),
                            carried_item: rustbound_protocol::play::Slot::empty(),
                        });
                        // Send initial health
                        let _ = event_sender.send(SessionEvent::SetHealth {
                            health: inv.health,
                            food: inv.food,
                            food_saturation: inv.food_saturation,
                        });
                    }

                    // Broadcast join to all OTHER existing sessions
                    for (&eid, sender) in &session_senders {
                        if eid != entity_id {
                            let _ = sender.send(SessionEvent::PlayerJoined {
                                entity_id,
                                uuid,
                                username: username.clone(),
                                gamemode,
                                x: px,
                                y: py,
                                z: pz,
                            });
                        }
                    }

                    // Send info about all existing players to the new session
                    for p in world.players() {
                        if p.entity_id != entity_id {
                            let _ = event_sender.send(SessionEvent::PlayerJoined {
                                entity_id: p.entity_id,
                                uuid: p.uuid,
                                username: p.username.clone(),
                                gamemode: p.gamemode,
                                x: p.x,
                                y: p.y,
                                z: p.z,
                            });
                        }
                    }

                    // Send block overrides for the initial chunk area so the
                    // new player sees dug/placed blocks in their initial chunks.
                    // The initial chunk radius is 2 (see session::send_initial_chunks).
                    let initial_radius = 2;
                    let mut new_player_overrides: Vec<((i32, i32, i32), i32)> = Vec::new();
                    for cx in -initial_radius..=initial_radius {
                        for cz in -initial_radius..=initial_radius {
                            new_player_overrides
                                .extend(world.get_block_overrides_for_chunk(cx, cz));
                        }
                    }
                    if !new_player_overrides.is_empty() {
                        let _ = event_sender.send(SessionEvent::ChunkBlockOverrides {
                            overrides: new_player_overrides,
                        });
                    }

                    let _ = event_tx.send(TickEvent::PlayerAdded { entity_id });
                    player_count.fetch_add(1, Ordering::AcqRel);
                }
                TickMessage::PlayerLeft { entity_id } => {
                    // Persist this player before dropping them so reconnect
                    // (even in the same process) and autosave cannot wipe them.
                    if let Some(data) = collect_one_player_data(&world, &inventories, entity_id) {
                        if let Some(player) = world.get_player(entity_id) {
                            saved_players.insert(player.uuid, data);
                            if let Err(e) =
                                crate::persist::save_players(&level_name, &saved_players)
                            {
                                eprintln!("error: failed to save player on leave: {}", e);
                            }
                        }
                    }
                    // Get the player's UUID before removing
                    let uuid = world.get_player(entity_id).map(|p| p.uuid);
                    world.remove_player(entity_id);
                    session_senders.remove(&entity_id);
                    chunk_states.remove(&entity_id);
                    inventories.remove(&entity_id);
                    player_count.fetch_sub(1, Ordering::AcqRel);

                    // Broadcast leave to all remaining sessions
                    if let Some(uuid) = uuid {
                        for sender in session_senders.values() {
                            let _ = sender.send(SessionEvent::PlayerLeft { entity_id, uuid });
                        }
                    }

                    let _ = event_tx.send(TickEvent::PlayerRemoved { entity_id });
                }
                TickMessage::PlayerPositionUpdate {
                    entity_id,
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                    on_ground,
                } => {
                    if let Some(player) = world.get_player_mut(entity_id) {
                        let (old_x, old_y, old_z) = player.position();
                        let (old_yaw, old_pitch) = player.rotation();
                        player.set_position(x, y, z);
                        player.set_rotation(yaw, pitch);
                        let pos_changed = (x - old_x).abs() > f64::EPSILON
                            || (y - old_y).abs() > f64::EPSILON
                            || (z - old_z).abs() > f64::EPSILON;
                        let rot_changed = (yaw - old_yaw).abs() > f32::EPSILON
                            || (pitch - old_pitch).abs() > f32::EPSILON;
                        if pos_changed || rot_changed {
                            // Broadcast movement to all OTHER sessions
                            for (&eid, sender) in &session_senders {
                                if eid != entity_id {
                                    let _ = sender.send(SessionEvent::EntityMovement {
                                        entity_id,
                                        old_x,
                                        old_y,
                                        old_z,
                                        new_x: x,
                                        new_y: y,
                                        new_z: z,
                                        new_yaw: yaw,
                                        new_pitch: pitch,
                                        on_ground,
                                    });
                                }
                            }
                        }

                        // Check for chunk border crossing and stream new chunks
                        if pos_changed {
                            let new_cx = chunk_x_from_world(x);
                            let new_cz = chunk_z_from_world(z);
                            if let Some(chunk_state) = chunk_states.get_mut(&entity_id) {
                                let (center_changed, new_chunks, unloaded_chunks) =
                                    chunk_state.update_center(new_cx, new_cz);
                                if let Some(sender) = session_senders.get(&entity_id) {
                                    if center_changed {
                                        let _ = sender.send(SessionEvent::SetCenterChunkEvent {
                                            chunk_x: new_cx,
                                            chunk_z: new_cz,
                                        });
                                    }
                                    for chunk in new_chunks {
                                        let _ = sender.send(SessionEvent::LoadChunk {
                                            chunk_x: chunk.x,
                                            chunk_z: chunk.z,
                                        });
                                    }
                                    for chunk in unloaded_chunks {
                                        let _ = sender.send(SessionEvent::UnloadChunk {
                                            chunk_x: chunk.x,
                                            chunk_z: chunk.z,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                TickMessage::SetBlock {
                    position,
                    block_state,
                } => {
                    world.set_block(position.0, position.1, position.2, block_state);
                    // Broadcast Block Update to all sessions
                    for sender in session_senders.values() {
                        let _ = sender.send(SessionEvent::BlockUpdate {
                            position,
                            block_state,
                        });
                    }
                }
                TickMessage::PlaceBlock {
                    entity_id,
                    position,
                    face,
                } => {
                    // Look up the held hotbar item and place the corresponding block.
                    if let Some(inv) = inventories.get(&entity_id) {
                        let held_idx = inv.held_slot as usize;
                        if let Some(slot) = inv.slots.get(held_idx) {
                            if slot.present {
                                // Use the registry to map item ID to block state
                                if let Some(block_state) =
                                    crate::registry::item_to_block_state(slot.item_id)
                                {
                                    if block_state != 0 {
                                        // Not air — place the block
                                        let target = match face {
                                            0 => (position.0, position.1 - 1, position.2),
                                            1 => (position.0, position.1 + 1, position.2),
                                            2 => (position.0, position.1, position.2 - 1),
                                            3 => (position.0, position.1, position.2 + 1),
                                            4 => (position.0 - 1, position.1, position.2),
                                            5 => (position.0 + 1, position.1, position.2),
                                            _ => position,
                                        };
                                        world.set_block(target.0, target.1, target.2, block_state);
                                        // Broadcast Block Update to all sessions
                                        for sender in session_senders.values() {
                                            let _ = sender.send(SessionEvent::BlockUpdate {
                                                position: target,
                                                block_state,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                TickMessage::SetClientViewDistance {
                    entity_id,
                    view_distance,
                } => {
                    if let Some(chunk_state) = chunk_states.get_mut(&entity_id) {
                        let (new_chunks, unloaded_chunks) =
                            chunk_state.update_view_distance(view_distance);
                        if let Some(sender) = session_senders.get(&entity_id) {
                            for chunk in new_chunks {
                                let _ = sender.send(SessionEvent::LoadChunk {
                                    chunk_x: chunk.x,
                                    chunk_z: chunk.z,
                                });
                            }
                            for chunk in unloaded_chunks {
                                let _ = sender.send(SessionEvent::UnloadChunk {
                                    chunk_x: chunk.x,
                                    chunk_z: chunk.z,
                                });
                            }
                        }
                    }
                }
                TickMessage::DigBlock {
                    entity_id,
                    action,
                    position,
                } => {
                    match action {
                        0 => {
                            // StartDestroy: begin tracking break progress
                            let progress = BlockBreakProgress {
                                position,
                                ticks_elapsed: 0,
                                total_ticks: DEFAULT_BLOCK_HARDNESS_TICKS,
                                last_stage: -1,
                            };
                            break_progress.insert(entity_id, progress);
                        }
                        1 | 2 => {
                            // AbortDestroy or StopDestroy: cancel break
                            if let Some(progress) = break_progress.remove(&entity_id) {
                                // Remove the break animation
                                if let Some(sender) = session_senders.get(&entity_id) {
                                    let _ = sender.send(SessionEvent::SetBlockDestroyStage {
                                        entity_id,
                                        position: progress.position,
                                        destroy_stage: -1, // remove animation
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                TickMessage::SetCreativeSlot {
                    entity_id,
                    slot,
                    item,
                } => {
                    // Creative mode: the client tells us what to put in a slot.
                    // Slot -1 means dropping the item (we just ignore it for now).
                    if slot >= 0 {
                        if let Some(inv) = inventories.get_mut(&entity_id) {
                            let idx = slot as usize;
                            if idx < inv.slots.len() {
                                let state_id = inv.state_id + 1;
                                inv.set_slot(idx, item.clone());
                                if let Some(sender) = session_senders.get(&entity_id) {
                                    let _ = sender.send(SessionEvent::SetContainerSlot {
                                        window_id: 0,
                                        state_id,
                                        slot,
                                        item,
                                    });
                                }
                            }
                        }
                    }
                }
                TickMessage::SetHeldItem { entity_id, slot } => {
                    if let Some(inv) = inventories.get_mut(&entity_id) {
                        if inv.set_held_slot(slot) {
                            if let Some(sender) = session_senders.get(&entity_id) {
                                let _ = sender.send(SessionEvent::SetHeldItemClientbound {
                                    slot: inv.held_slot,
                                });
                            }
                        }
                    }
                }
                TickMessage::ChatMessage {
                    entity_id,
                    uuid: _,
                    username,
                    message,
                } => {
                    // Broadcast chat to all sessions as a System Chat Message.
                    // In offline mode, we use the unsigned/system chat path
                    // (no signed Player Chat Message).
                    let content = format_chat_message(&username, &message);
                    for (eid, sender) in &session_senders {
                        // Don't echo back to the sender
                        if *eid != entity_id {
                            let _ = sender.send(SessionEvent::SystemChat {
                                content: content.clone(),
                            });
                        }
                    }
                }
                TickMessage::ClientStatus { entity_id, action } => {
                    // Action 0 = Perform respawn
                    if action == 0 {
                        if let Some(inv) = inventories.get_mut(&entity_id) {
                            if inv.is_dead {
                                inv.respawn();
                                let (spawn_x, spawn_y, spawn_z) = world.spawn_point();
                                let gamemode = world
                                    .get_player(entity_id)
                                    .map(|player| player.gamemode)
                                    .unwrap_or(0);
                                if let Some(sender) = session_senders.get(&entity_id) {
                                    // Use the player's real gamemode and world spawn
                                    // (not hardcoded Creative / fixed coordinates).
                                    let _ = sender.send(SessionEvent::RespawnPlayer {
                                        dimension_type: "minecraft:overworld".to_string(),
                                        dimension_name: "minecraft:overworld".to_string(),
                                        hashed_seed: 0,
                                        gamemode,
                                        previous_gamemode: -1,
                                        is_debug: false,
                                        is_flat: true,
                                        has_death_location: false,
                                        death_dimension_name: String::new(),
                                        death_location: (0, 0, 0),
                                        portal_cooldown: 0,
                                        data_kept: 0,
                                        x: spawn_x,
                                        y: spawn_y,
                                        z: spawn_z,
                                    });
                                    // Send updated health after respawn
                                    let _ = sender.send(SessionEvent::SetHealth {
                                        health: inv.health,
                                        food: inv.food,
                                        food_saturation: inv.food_saturation,
                                    });
                                    // Re-sync inventory container content
                                    let _ = sender.send(SessionEvent::SetContainerContent {
                                        window_id: 0,
                                        state_id: inv.state_id,
                                        slots: inv.slots.clone(),
                                        carried_item: rustbound_protocol::play::Slot::empty(),
                                    });
                                    // Re-sync held item
                                    let _ = sender.send(SessionEvent::SetHeldItemClientbound {
                                        slot: inv.held_slot,
                                    });
                                }
                                // Reset player position in world to spawn
                                if let Some(player) = world.get_player_mut(entity_id) {
                                    player.set_position(spawn_x, spawn_y, spawn_z);
                                }
                                // Re-sync chunks: reset center to spawn, unload old, load new
                                let spawn_cx = chunk_x_from_world(spawn_x);
                                let spawn_cz = chunk_z_from_world(spawn_z);
                                if let Some(chunk_state) = chunk_states.get_mut(&entity_id) {
                                    // Unload all currently loaded chunks
                                    let old_loaded: Vec<_> =
                                        chunk_state.loaded_chunks.iter().copied().collect();
                                    // Reset center and loaded set to spawn
                                    chunk_state.center_x = spawn_cx;
                                    chunk_state.center_z = spawn_cz;
                                    let new_desired = chunk_state.desired_chunk_set();
                                    // Chunks to load: in new desired but not in old loaded
                                    let new_chunks: Vec<_> = new_desired
                                        .iter()
                                        .filter(|pos| !chunk_state.loaded_chunks.contains(pos))
                                        .copied()
                                        .collect();
                                    // Chunks to unload: in old loaded but not in new desired
                                    let unloaded: Vec<_> = old_loaded
                                        .iter()
                                        .filter(|pos| !new_desired.contains(pos))
                                        .copied()
                                        .collect();
                                    chunk_state.loaded_chunks = new_desired;
                                    if let Some(sender) = session_senders.get(&entity_id) {
                                        // Send Set Center Chunk
                                        let _ = sender.send(SessionEvent::SetCenterChunkEvent {
                                            chunk_x: spawn_cx,
                                            chunk_z: spawn_cz,
                                        });
                                        // Unload old chunks
                                        for chunk in unloaded {
                                            let _ = sender.send(SessionEvent::UnloadChunk {
                                                chunk_x: chunk.x,
                                                chunk_z: chunk.z,
                                            });
                                        }
                                        // Load new chunks
                                        for chunk in new_chunks {
                                            let _ = sender.send(SessionEvent::LoadChunk {
                                                chunk_x: chunk.x,
                                                chunk_z: chunk.z,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Periodic tasks: send KeepAlive to all active sessions
        if tick_count - last_keep_alive_tick >= KEEP_ALIVE_INTERVAL_TICKS {
            for sender in session_senders.values() {
                let _ = sender.send(SessionEvent::KeepAlive {
                    payload: tick_count as i64,
                });
            }
            last_keep_alive_tick = tick_count;
        }

        // Periodic task: check for void death (player Y < VOID_DEATH_Y)
        for (&entity_id, inv) in inventories.iter_mut() {
            if inv.is_dead {
                continue;
            }
            if let Some(player) = world.get_player(entity_id) {
                if player.y < VOID_DEATH_Y {
                    inv.kill();
                    let username = player.username.clone();
                    if let Some(sender) = session_senders.get(&entity_id) {
                        let _ = sender.send(SessionEvent::SetHealth {
                            health: inv.health,
                            food: inv.food,
                            food_saturation: inv.food_saturation,
                        });
                        let _ = sender.send(SessionEvent::CombatDeath {
                            player_id: entity_id,
                            message: format!("{{\"text\":\"{} fell out of the world\"}}", username),
                        });
                    }
                }
            }
        }

        // Periodic task: Survival food drain and starvation damage
        // Creative players are exempt. Food drains every FOOD_DRAIN_INTERVAL_TICKS.
        if tick_count % FOOD_DRAIN_INTERVAL_TICKS == 0 && tick_count > 0 {
            for (&entity_id, inv) in inventories.iter_mut() {
                if inv.is_dead {
                    continue;
                }
                // Check gamemode — Creative is exempt
                let gamemode = world.get_player(entity_id).map(|p| p.gamemode).unwrap_or(0);
                if gamemode == GAMEMODE_CREATIVE {
                    continue;
                }
                // Drain food (saturation first, then food)
                let food_changed = inv.drain_food();
                // Apply starvation damage if food is 0
                let damaged = inv.starve();
                if food_changed || damaged {
                    if let Some(sender) = session_senders.get(&entity_id) {
                        let _ = sender.send(SessionEvent::SetHealth {
                            health: inv.health,
                            food: inv.food,
                            food_saturation: inv.food_saturation,
                        });
                        // If starvation killed the player, send Combat Death
                        if inv.is_dead {
                            let username = world
                                .get_player(entity_id)
                                .map(|p| p.username.clone())
                                .unwrap_or_else(|| "Player".to_string());
                            let _ = sender.send(SessionEvent::CombatDeath {
                                player_id: entity_id,
                                message: format!("{{\"text\":\"{} starved to death\"}}", username),
                            });
                        }
                    }
                }
            }
        }

        // Periodic task: update block break progress for Survival players
        let mut completed_breaks: Vec<(i32, (i32, i32, i32))> = Vec::new();
        for (&entity_id, progress) in break_progress.iter_mut() {
            progress.ticks_elapsed += 1;
            // Calculate destroy stage: 0 to MAX_DESTROY_STAGE
            let stage = u64::checked_div(
                progress.ticks_elapsed * (MAX_DESTROY_STAGE as u64 + 1),
                progress.total_ticks,
            )
            .map(|v| (v as i8).min(MAX_DESTROY_STAGE))
            .unwrap_or(MAX_DESTROY_STAGE);
            if stage != progress.last_stage {
                progress.last_stage = stage;
                if let Some(sender) = session_senders.get(&entity_id) {
                    let _ = sender.send(SessionEvent::SetBlockDestroyStage {
                        entity_id,
                        position: progress.position,
                        destroy_stage: stage,
                    });
                }
            }
            // Check if break is complete
            if progress.ticks_elapsed >= progress.total_ticks {
                completed_breaks.push((entity_id, progress.position));
            }
        }
        // Complete breaks: set block to air and remove progress
        for (entity_id, position) in completed_breaks {
            break_progress.remove(&entity_id);
            world.set_block(position.0, position.1, position.2, 0);
            // Broadcast Block Update to all sessions
            for sender in session_senders.values() {
                let _ = sender.send(SessionEvent::BlockUpdate {
                    position,
                    block_state: 0,
                });
            }
        }

        // Periodic task: autosave block overrides and player data
        if autosave_interval_ticks > 0 && tick_count - last_autosave_tick >= autosave_interval_ticks
        {
            if let Err(e) = crate::persist::save_overrides(&level_name, world.block_overrides()) {
                eprintln!("error: autosave failed for block overrides: {}", e);
            }
            merge_online_into_saved(&world, &inventories, &mut saved_players);
            if let Err(e) = crate::persist::save_players(&level_name, &saved_players) {
                eprintln!("error: autosave failed for player data: {}", e);
            }
            last_autosave_tick = tick_count;
        }

        tick_count += 1;

        // Sleep for the remaining tick time
        let elapsed = tick_start.elapsed();
        if elapsed < TICK_DURATION {
            thread::sleep(TICK_DURATION - elapsed);
        }
    }

    // Flush remaining data to disk on shutdown (covers the case where the
    // shutdown flag was set before the Shutdown message was processed).
    // The Shutdown message handler also saves, but this is a safety net.
    if let Err(e) = crate::persist::save_overrides(&level_name, world.block_overrides()) {
        eprintln!("error: failed to save block overrides on shutdown: {}", e);
    }
    merge_online_into_saved(&world, &inventories, &mut saved_players);
    if let Err(e) = crate::persist::save_players(&level_name, &saved_players) {
        eprintln!("error: failed to save player data on shutdown: {}", e);
    }

    let _ = event_tx.send(TickEvent::Shutdown);
}

/// Merges currently online players into the in-memory saved-player cache.
fn merge_online_into_saved(
    world: &World,
    inventories: &HashMap<i32, PlayerInventory>,
    saved_players: &mut HashMap<Uuid, crate::persist::PlayerData>,
) {
    for (uuid, data) in collect_player_data(world, inventories) {
        saved_players.insert(uuid, data);
    }
}

/// Collects persistence data for a single online player, if present.
fn collect_one_player_data(
    world: &World,
    inventories: &HashMap<i32, PlayerInventory>,
    entity_id: i32,
) -> Option<crate::persist::PlayerData> {
    let player = world.get_player(entity_id)?;
    let inv = inventories.get(&entity_id)?;
    let slots: Vec<(bool, i32, i8, Vec<u8>)> = inv
        .slots
        .iter()
        .map(|s| (s.present, s.item_id, s.count, s.nbt.clone()))
        .collect();
    Some(crate::persist::PlayerData {
        x: player.x,
        y: player.y,
        z: player.z,
        yaw: player.yaw,
        pitch: player.pitch,
        gamemode: player.gamemode,
        held_slot: inv.held_slot,
        health: inv.health,
        food: inv.food,
        food_saturation: inv.food_saturation,
        slots,
    })
}

/// Collects all player data for persistence.
fn collect_player_data(
    world: &World,
    inventories: &HashMap<i32, PlayerInventory>,
) -> HashMap<Uuid, crate::persist::PlayerData> {
    let mut result = HashMap::new();
    for player in world.players() {
        if let Some(data) = collect_one_player_data(world, inventories, player.entity_id) {
            result.insert(player.uuid, data);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_FOOD, DEFAULT_FOOD_SATURATION, DEFAULT_HEALTH, FOOD_DRAIN_INTERVAL_TICKS,
        KEEP_ALIVE_INTERVAL_TICKS, PlayerChunkState, PlayerInventory, TICK_DURATION, TPS,
        TickEvent, VOID_DEATH_Y, chunk_x_from_world, chunk_z_from_world, start_tick_loop,
    };
    use crate::session::SessionEvent;
    use rustbound_protocol::primitives::Uuid;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    /// Ephemeral level directory for tick-loop tests.
    ///
    /// Using a unique name per test avoids cross-test leakage now that
    /// shutdown/autosave persist player and block data under `level_name/`.
    struct TestLevel(String);

    impl TestLevel {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            Self(format!(
                "test-world-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
        }

        fn name(&self) -> String {
            self.0.clone()
        }
    }

    impl Drop for TestLevel {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tick_duration_is_50ms() {
        assert_eq!(TICK_DURATION, Duration::from_millis(50));
        assert_eq!(TPS, 20);
    }

    #[test]
    fn keep_alive_interval_is_15_seconds() {
        assert_eq!(KEEP_ALIVE_INTERVAL_TICKS, 300);
    }

    #[test]
    fn tick_loop_starts_and_stops() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;
        std::thread::sleep(Duration::from_millis(100));
        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_processes_player_join_and_leave() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx_session) = channel::<SessionEvent>();

        // Send player joined
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for the event
        let start = Instant::now();
        let mut got_added = false;
        while start.elapsed() < Duration::from_millis(500) {
            if let Ok(TickEvent::PlayerAdded { entity_id }) = event_rx.try_recv() {
                assert_eq!(entity_id, 1);
                got_added = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(got_added, "did not receive PlayerAdded event");

        // Send player left
        handle.send(super::TickMessage::PlayerLeft { entity_id: 1 })?;

        let start = Instant::now();
        let mut got_removed = false;
        while start.elapsed() < Duration::from_millis(500) {
            if let Ok(TickEvent::PlayerRemoved { entity_id }) = event_rx.try_recv() {
                assert_eq!(entity_id, 1);
                got_removed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(got_removed, "did not receive PlayerRemoved event");

        // The session event receiver should be disconnected after removal
        // (the tick loop drops its sender)
        drop(event_rx_session);

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_shuts_down_on_message() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;
        handle.send(super::TickMessage::Shutdown)?;

        let start = Instant::now();
        let mut got_shutdown = false;
        while start.elapsed() < Duration::from_millis(500) {
            if let Ok(TickEvent::Shutdown) = event_rx.try_recv() {
                got_shutdown = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(got_shutdown, "did not receive Shutdown event");

        // The thread should have exited
        if let Some(thread) = handle.thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }

    #[test]
    fn tick_loop_sends_block_overrides_to_new_player() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player 1 joins and digs some blocks (creates overrides)
        let (event_tx1, event_rx1) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Wait for player 1 to be added
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            if event_rx1.try_recv().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        // Player 1 digs blocks within the initial chunk radius (radius 2)
        // Block at (5, 64, 5) is in chunk (0, 0), within radius 2 of center (0, 0)
        handle.send(super::TickMessage::SetBlock {
            position: (5, 64, 5),
            block_state: 0, // air (dug)
        })?;
        // Block at (20, 64, 20) is in chunk (1, 1), within radius 2
        handle.send(super::TickMessage::SetBlock {
            position: (20, 64, 20),
            block_state: 1, // stone (placed)
        })?;
        // Block at (100, 64, 100) is in chunk (6, 6), OUTSIDE radius 2
        handle.send(super::TickMessage::SetBlock {
            position: (100, 64, 100),
            block_state: 2,
        })?;

        // Wait for the SetBlock messages to be processed
        std::thread::sleep(Duration::from_millis(100));

        // Player 2 joins - should receive block overrides for the initial area
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx2,
        })?;

        // Collect events for player 2
        let start = Instant::now();
        let mut got_overrides = false;
        let mut override_count = 0;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx2.try_recv() {
                Ok(SessionEvent::ChunkBlockOverrides { overrides }) => {
                    got_overrides = true;
                    override_count = overrides.len();
                    // Should contain (5, 64, 5) -> 0 and (20, 64, 20) -> 1
                    // but NOT (100, 64, 100) -> 2 (outside radius 2)
                    let has_5_5 = overrides
                        .iter()
                        .any(|((x, y, z), s)| *x == 5 && *y == 64 && *z == 5 && *s == 0);
                    let has_20_20 = overrides
                        .iter()
                        .any(|((x, y, z), s)| *x == 20 && *y == 64 && *z == 20 && *s == 1);
                    let has_100_100 = overrides
                        .iter()
                        .any(|((x, y, z), _)| *x == 100 && *y == 64 && *z == 100);
                    assert!(has_5_5, "should include override at (5, 64, 5)");
                    assert!(has_20_20, "should include override at (20, 64, 20)");
                    assert!(
                        !has_100_100,
                        "should NOT include override at (100, 64, 100) - outside radius"
                    );
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_overrides,
            "player 2 should have received ChunkBlockOverrides event"
        );
        assert_eq!(
            override_count, 2,
            "should have exactly 2 overrides in range"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_no_overrides_event_when_world_clean() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player joins a clean world (no overrides)
        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Collect events - should NOT receive ChunkBlockOverrides
        let start = Instant::now();
        let mut got_chunk_overrides = false;
        while start.elapsed() < Duration::from_millis(200) {
            match event_rx.try_recv() {
                Ok(SessionEvent::ChunkBlockOverrides { .. }) => {
                    got_chunk_overrides = true;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            !got_chunk_overrides,
            "should not receive ChunkBlockOverrides when world has no overrides"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_broadcasts_position_update_to_peers() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player 1 joins
        let (event_tx1, event_rx1) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Player 2 joins
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx2,
        })?;

        // Wait for join events to be processed
        std::thread::sleep(Duration::from_millis(100));
        // Drain join events
        while event_rx1.try_recv().is_ok() {}
        while event_rx2.try_recv().is_ok() {}

        // Player 1 moves from (0, 64, 0) to (1, 64, 0)
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 1.0,
            y: 64.0,
            z: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            on_ground: true,
        })?;

        // Player 2 should receive EntityMovement for player 1
        let start = Instant::now();
        let mut got_movement = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx2.try_recv() {
                Ok(SessionEvent::EntityMovement {
                    entity_id,
                    old_x,
                    new_x,
                    new_yaw,
                    ..
                }) => {
                    assert_eq!(entity_id, 1, "should be player 1's movement");
                    assert_eq!(old_x, 0.0);
                    assert_eq!(new_x, 1.0);
                    assert_eq!(new_yaw, 90.0);
                    got_movement = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_movement,
            "player 2 should have received EntityMovement for player 1"
        );

        // Player 1 should NOT receive its own movement
        let mut self_received = false;
        while let Ok(event) = event_rx1.try_recv() {
            if matches!(event, SessionEvent::EntityMovement { entity_id: 1, .. }) {
                self_received = true;
            }
        }
        assert!(
            !self_received,
            "player 1 should not receive its own movement"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_no_movement_event_when_position_unchanged()
    -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player 1 joins
        let (event_tx1, event_rx1) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Player 2 joins
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx2,
        })?;

        // Wait for join events
        std::thread::sleep(Duration::from_millis(100));
        while event_rx1.try_recv().is_ok() {}
        while event_rx2.try_recv().is_ok() {}

        // Player 1 sends the SAME position (spawn point 0, 64, 0)
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        })?;

        // Wait and check that player 2 does NOT receive EntityMovement
        std::thread::sleep(Duration::from_millis(100));
        let mut got_movement = false;
        while let Ok(event) = event_rx2.try_recv() {
            if matches!(event, SessionEvent::EntityMovement { .. }) {
                got_movement = true;
            }
        }
        assert!(
            !got_movement,
            "should not receive EntityMovement when position is unchanged"
        );

        handle.shutdown();
        Ok(())
    }

    // --- PlayerChunkState unit tests ---

    #[test]
    fn chunk_state_initial_has_radius_2() {
        let state = PlayerChunkState::new(10);
        // Initial radius is 2 (5x5 = 25 chunks)
        assert_eq!(state.loaded_chunks.len(), 25);
        assert_eq!(state.center_x, 0);
        assert_eq!(state.center_z, 0);
    }

    #[test]
    fn chunk_state_update_center_same_no_change() {
        let mut state = PlayerChunkState::new(10);
        let (changed, new_chunks, unloaded) = state.update_center(0, 0);
        assert!(!changed);
        assert!(new_chunks.is_empty());
        assert!(unloaded.is_empty());
    }

    #[test]
    fn chunk_state_update_center_new_chunks() {
        let mut state = PlayerChunkState::new(2);
        // Move from (0,0) to (1,0) - center shifts by 1 chunk
        let (changed, new_chunks, unloaded) = state.update_center(1, 0);
        assert!(changed);
        assert_eq!(state.center_x, 1);
        assert_eq!(state.center_z, 0);
        // New chunks are those in the new set but not the old set
        // Old: cx in [-2,2], cz in [-2,2] -> 25 chunks
        // New: cx in [-1,3], cz in [-2,2] -> 25 chunks
        // New chunks: cx=3, cz in [-2,2] -> 5 chunks
        assert_eq!(new_chunks.len(), 5);
        for chunk in &new_chunks {
            assert_eq!(chunk.x, 3);
            assert!(chunk.z >= -2 && chunk.z <= 2);
        }
        // Unloaded chunks: cx=-2, cz in [-2,2] -> 5 chunks
        assert_eq!(unloaded.len(), 5);
        for chunk in &unloaded {
            assert_eq!(chunk.x, -2);
            assert!(chunk.z >= -2 && chunk.z <= 2);
        }
    }

    #[test]
    fn chunk_state_update_view_distance_grows() {
        let mut state = PlayerChunkState::new(10);
        // Initial view distance is min(10, 2) = 2
        assert_eq!(state.view_distance, 2);
        // Client requests view distance 5
        let (new_chunks, unloaded) = state.update_view_distance(5);
        assert_eq!(state.view_distance, 5);
        // New chunks: those in radius 5 but not in radius 2
        // Radius 5: 11x11 = 121, radius 2: 5x5 = 25, new = 96
        assert_eq!(new_chunks.len(), 96);
        // Growing should not unload any chunks
        assert!(unloaded.is_empty());
    }

    #[test]
    fn chunk_state_update_view_distance_shrinks() {
        let mut state = PlayerChunkState::new(10);
        // Grow to 5 first
        let _ = state.update_view_distance(5);
        assert_eq!(state.view_distance, 5);
        // Shrink to 3
        let (new_chunks, unloaded) = state.update_view_distance(3);
        assert_eq!(state.view_distance, 3);
        // No new chunks when shrinking
        assert!(new_chunks.is_empty());
        // Unloaded chunks: those in radius 5 but not in radius 3
        // Radius 5: 11x11 = 121, radius 3: 7x7 = 49, unloaded = 72
        assert_eq!(unloaded.len(), 72);
    }

    #[test]
    fn chunk_x_from_world_basic() {
        assert_eq!(chunk_x_from_world(0.0), 0);
        assert_eq!(chunk_x_from_world(15.0), 0);
        assert_eq!(chunk_x_from_world(16.0), 1);
        assert_eq!(chunk_x_from_world(-1.0), -1);
        assert_eq!(chunk_x_from_world(-16.0), -1);
        assert_eq!(chunk_x_from_world(-17.0), -2);
    }

    #[test]
    fn chunk_z_from_world_basic() {
        assert_eq!(chunk_z_from_world(0.0), 0);
        assert_eq!(chunk_z_from_world(15.0), 0);
        assert_eq!(chunk_z_from_world(16.0), 1);
        assert_eq!(chunk_z_from_world(-1.0), -1);
    }

    #[test]
    fn tick_loop_streams_chunks_on_chunk_border_crossing() -> Result<(), Box<dyn std::error::Error>>
    {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player joins at spawn (0, 64, 0) -> chunk (0, 0)
        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 2,
            event_sender: event_tx,
        })?;

        // Wait for join processing
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Player moves to (16, 64, 0) -> crosses into chunk (1, 0)
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 16.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        })?;

        // Should receive SetCenterChunkEvent and LoadChunk events
        let start = Instant::now();
        let mut got_center_change = false;
        let mut load_chunk_count = 0;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetCenterChunkEvent { chunk_x, chunk_z }) => {
                    assert_eq!(chunk_x, 1);
                    assert_eq!(chunk_z, 0);
                    got_center_change = true;
                }
                Ok(SessionEvent::LoadChunk { chunk_x, .. }) => {
                    // New chunks should be at cx=3 (edge of new view)
                    assert_eq!(chunk_x, 3);
                    load_chunk_count += 1;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_center_change, "should receive SetCenterChunkEvent");
        assert_eq!(load_chunk_count, 5, "should load 5 new chunks");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_no_chunk_stream_when_same_chunk() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 2,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Move within the same chunk (0, 64, 0) -> (5, 64, 5) still chunk (0, 0)
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 5.0,
            y: 64.0,
            z: 5.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        })?;

        // Should NOT receive SetCenterChunkEvent or LoadChunk
        std::thread::sleep(Duration::from_millis(100));
        let mut got_center = false;
        let mut got_load = false;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                SessionEvent::SetCenterChunkEvent { .. } => got_center = true,
                SessionEvent::LoadChunk { .. } => got_load = true,
                _ => {}
            }
        }
        assert!(
            !got_center,
            "should not receive SetCenterChunkEvent within same chunk"
        );
        assert!(!got_load, "should not receive LoadChunk within same chunk");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_client_view_distance_loads_more_chunks() -> Result<(), Box<dyn std::error::Error>>
    {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join (initial radius is 2)
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Client sends view distance 5
        handle.send(super::TickMessage::SetClientViewDistance {
            entity_id: 1,
            view_distance: 5,
        })?;

        // Should receive LoadChunk events for new chunks (radius 3,4,5 rings)
        let start = Instant::now();
        let mut load_count = 0;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::LoadChunk { .. }) => load_count += 1,
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        // Radius 5 = 11x11 = 121, initial radius 2 = 5x5 = 25, new = 96
        assert_eq!(
            load_count, 96,
            "should load 96 new chunks when view distance grows to 5"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn player_inventory_default_is_empty() {
        let inv = PlayerInventory::new();
        assert_eq!(inv.slots.len(), super::PLAYER_INVENTORY_SIZE);
        for slot in &inv.slots {
            assert!(!slot.present, "all slots should be empty by default");
        }
        assert_eq!(inv.held_slot, 0);
        assert_eq!(inv.state_id, 0);
    }

    #[test]
    fn player_inventory_set_slot_changes_state() {
        let mut inv = PlayerInventory::new();
        let item = rustbound_protocol::play::Slot::item(10, 64);
        assert!(inv.set_slot(0, item.clone()));
        assert!(inv.slots[0].present);
        assert_eq!(inv.slots[0].item_id, 10);
        assert_eq!(inv.state_id, 1);

        // Setting the same slot to the same value should not change state
        assert!(!inv.set_slot(0, item.clone()));
        assert_eq!(inv.state_id, 1);

        // Out of bounds is a no-op
        assert!(!inv.set_slot(999, item));
        assert_eq!(inv.state_id, 1);
    }

    #[test]
    fn player_inventory_set_held_slot_clamps() {
        let mut inv = PlayerInventory::new();
        assert!(inv.set_held_slot(3));
        assert_eq!(inv.held_slot, 3);

        // Same slot is no-op
        assert!(!inv.set_held_slot(3));

        // Out of range is rejected
        assert!(!inv.set_held_slot(-1));
        assert!(!inv.set_held_slot(9));
        assert_eq!(inv.held_slot, 3);

        // Valid edge values
        assert!(inv.set_held_slot(0));
        assert!(inv.set_held_slot(8));
    }

    #[test]
    fn tick_loop_sends_initial_inventory_on_join() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        let start = Instant::now();
        let mut got_content = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetContainerContent {
                    window_id,
                    slots,
                    carried_item,
                    ..
                }) => {
                    got_content = true;
                    assert_eq!(window_id, 0);
                    assert_eq!(slots.len(), super::PLAYER_INVENTORY_SIZE);
                    assert!(!carried_item.present);
                    for slot in &slots {
                        assert!(!slot.present);
                    }
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_content, "should receive SetContainerContent on join");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_slot_updates_inventory() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join to be processed
        std::thread::sleep(Duration::from_millis(100));
        // Drain initial events
        while event_rx.try_recv().is_ok() {}

        // Send a SetCreativeSlot to put an item in slot 0
        let item = rustbound_protocol::play::Slot::item(5, 32);
        handle.send(super::TickMessage::SetCreativeSlot {
            entity_id: 1,
            slot: 0,
            item: item.clone(),
        })?;

        let start = Instant::now();
        let mut got_slot_update = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetContainerSlot {
                    window_id,
                    slot,
                    item: recv_item,
                    ..
                }) => {
                    got_slot_update = true;
                    assert_eq!(window_id, 0);
                    assert_eq!(slot, 0);
                    assert!(recv_item.present);
                    assert_eq!(recv_item.item_id, 5);
                    assert_eq!(recv_item.count, 32);
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_slot_update, "should receive SetContainerSlot event");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_held_item_updates_send_event() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join to be processed
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Change held slot to 5
        handle.send(super::TickMessage::SetHeldItem {
            entity_id: 1,
            slot: 5,
        })?;

        let start = Instant::now();
        let mut got_held = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetHeldItemClientbound { slot }) => {
                    got_held = true;
                    assert_eq!(slot, 5);
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_held, "should receive SetHeldItemClientbound event");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_slot_drop_ignored() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Slot -1 means dropping an item - server should not update inventory
        let item = rustbound_protocol::play::Slot::item(1, 1);
        handle.send(super::TickMessage::SetCreativeSlot {
            entity_id: 1,
            slot: -1,
            item,
        })?;

        // Should NOT receive a SetContainerSlot event for slot -1
        let start = Instant::now();
        let mut got_slot_update = false;
        while start.elapsed() < Duration::from_millis(200) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetContainerSlot { slot, .. }) => {
                    if slot == -1 {
                        got_slot_update = true;
                    }
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            !got_slot_update,
            "should not receive SetContainerSlot for slot -1 (drop)"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_broadcasts_chat_to_other_players() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player 1 joins
        let (event_tx1, event_rx1) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 2,
            event_sender: event_tx1,
        })?;

        // Player 2 joins
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
            gamemode: 0,
            view_distance: 2,
            event_sender: event_tx2,
        })?;

        // Wait for join events
        std::thread::sleep(Duration::from_millis(100));
        while event_rx1.try_recv().is_ok() {}
        while event_rx2.try_recv().is_ok() {}

        // Player 1 sends a chat message
        handle.send(super::TickMessage::ChatMessage {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            message: "Hello world!".to_string(),
        })?;

        // Player 2 should receive the chat
        let start = Instant::now();
        let mut got_chat = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx2.try_recv() {
                Ok(SessionEvent::SystemChat { content }) => {
                    assert!(
                        content.contains("Hello world!"),
                        "content should contain message: {content}"
                    );
                    assert!(
                        content.contains("Steve"),
                        "content should contain username: {content}"
                    );
                    got_chat = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_chat, "player 2 should have received the chat message");

        // Player 1 should NOT receive its own chat
        let mut self_received = false;
        while let Ok(event) = event_rx1.try_recv() {
            if matches!(event, SessionEvent::SystemChat { .. }) {
                self_received = true;
            }
        }
        assert!(!self_received, "player 1 should not receive its own chat");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn player_inventory_default_vitals() {
        let inv = PlayerInventory::new();
        assert_eq!(inv.health, DEFAULT_HEALTH);
        assert_eq!(inv.food, DEFAULT_FOOD);
        assert_eq!(inv.food_saturation, DEFAULT_FOOD_SATURATION);
        assert!(!inv.is_dead);
    }

    #[test]
    fn player_inventory_kill_and_respawn() {
        let mut inv = PlayerInventory::new();
        assert!(!inv.is_dead);
        inv.kill();
        assert_eq!(inv.health, 0.0);
        assert!(inv.is_dead);
        inv.respawn();
        assert_eq!(inv.health, DEFAULT_HEALTH);
        assert_eq!(inv.food, DEFAULT_FOOD);
        assert!(!inv.is_dead);
    }

    #[test]
    fn tick_loop_sends_initial_health_on_join() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        let start = Instant::now();
        let mut got_health = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetHealth {
                    health,
                    food,
                    food_saturation,
                }) => {
                    got_health = true;
                    assert_eq!(health, DEFAULT_HEALTH);
                    assert_eq!(food, DEFAULT_FOOD);
                    assert_eq!(food_saturation, DEFAULT_FOOD_SATURATION);
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_health, "should receive SetHealth on join");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn format_chat_message_basic() {
        let msg = super::format_chat_message("Alex", "Hi!");
        assert_eq!(msg, r#"{"text":"<Alex> Hi!"}"#);
    }

    #[test]
    fn format_chat_message_escapes_quotes() {
        let msg = super::format_chat_message("Steve", r#"say "hi""#);
        assert!(msg.contains(r#"\""#), "quotes should be escaped: {msg}");
        assert!(msg.starts_with(r#"{"text":"#), "should be JSON: {msg}");
    }

    #[test]
    fn format_chat_message_escapes_backslash() {
        let msg = super::format_chat_message("Steve", r"back\slash");
        assert!(msg.contains(r"\\"), "backslash should be escaped: {msg}");
    }

    #[test]
    fn tick_loop_void_death_kills_player() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Move player below void threshold
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 0.0,
            y: VOID_DEATH_Y - 10.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        })?;

        // Wait for the tick loop to detect void death and send SetHealth(0)
        let start = Instant::now();
        let mut got_death = false;
        let mut got_combat_death = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetHealth { health, .. }) => {
                    if health <= 0.0 {
                        got_death = true;
                    }
                }
                Ok(SessionEvent::CombatDeath { player_id, message }) => {
                    assert_eq!(player_id, 1);
                    assert!(
                        message.contains("Steve"),
                        "death message should contain username"
                    );
                    got_combat_death = true;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_death,
            "should receive SetHealth with 0 HP on void death"
        );
        assert!(
            got_combat_death,
            "should receive CombatDeath event on void death"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_respawn_after_death() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Move player into void to trigger death
        handle.send(super::TickMessage::PlayerPositionUpdate {
            entity_id: 1,
            x: 0.0,
            y: VOID_DEATH_Y - 10.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        })?;

        // Wait for death
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetHealth { health, .. }) if health <= 0.0 => break,
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }

        // Send Client Status (Perform Respawn)
        handle.send(super::TickMessage::ClientStatus {
            entity_id: 1,
            action: 0,
        })?;

        // Wait for RespawnPlayer event and restored health
        let start = Instant::now();
        let mut got_respawn = false;
        let mut got_health_restore = false;
        let mut got_container_content = false;
        let mut got_held_item = false;
        let mut got_center_chunk = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::RespawnPlayer { x, y, z, .. }) => {
                    got_respawn = true;
                    assert_eq!(x, 0.0);
                    assert_eq!(y, 64.0);
                    assert_eq!(z, 0.0);
                }
                Ok(SessionEvent::SetHealth { health, .. }) => {
                    if health > 0.0 {
                        got_health_restore = true;
                    }
                }
                Ok(SessionEvent::SetContainerContent { .. }) => {
                    got_container_content = true;
                }
                Ok(SessionEvent::SetHeldItemClientbound { .. }) => {
                    got_held_item = true;
                }
                Ok(SessionEvent::SetCenterChunkEvent { .. }) => {
                    got_center_chunk = true;
                }
                Ok(SessionEvent::LoadChunk { .. }) => {}
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_respawn, "should receive RespawnPlayer event");
        assert!(
            got_health_restore,
            "should receive SetHealth with restored HP after respawn"
        );
        assert!(
            got_container_content,
            "should receive SetContainerContent (inventory re-sync) after respawn"
        );
        assert!(
            got_held_item,
            "should receive SetHeldItemClientbound after respawn"
        );
        assert!(
            got_center_chunk,
            "should receive SetCenterChunkEvent (chunk center re-sync) after respawn"
        );
        // LoadChunk may or may not fire if spawn == death position (all chunks
        // already loaded). The important thing is the center is re-sent.

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_respawn_ignored_when_not_dead() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Send respawn while alive - should be ignored
        handle.send(super::TickMessage::ClientStatus {
            entity_id: 1,
            action: 0,
        })?;

        // Should NOT receive RespawnPlayer event
        let start = Instant::now();
        let mut got_respawn = false;
        while start.elapsed() < Duration::from_millis(200) {
            match event_rx.try_recv() {
                Ok(SessionEvent::RespawnPlayer { .. }) => {
                    got_respawn = true;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            !got_respawn,
            "should not receive RespawnPlayer when player is alive"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_gamemode_passed_to_handle_and_peers()
    -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        // Player 1 joins in Creative (gamemode=1)
        let (event_tx1, event_rx1) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 1,
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Wait for player 1 to be added
        std::thread::sleep(Duration::from_millis(100));
        while event_rx1.try_recv().is_ok() {}

        // Player 2 joins - should see player 1's Creative gamemode
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
            gamemode: 0,
            view_distance: 10,
            event_sender: event_tx2,
        })?;

        // Player 2 should receive PlayerJoined for player 1 with gamemode=1
        let start = Instant::now();
        let mut got_creative_peer = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx2.try_recv() {
                Ok(SessionEvent::PlayerJoined {
                    entity_id,
                    gamemode,
                    ..
                }) => {
                    if entity_id == 1 && gamemode == 1 {
                        got_creative_peer = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_creative_peer,
            "player 2 should see player 1 with Creative gamemode (1)"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_place_uses_held_item() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 1, // Creative
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Set hotbar slot 0 to dirt (item_id=10, block_state=10)
        handle.send(super::TickMessage::SetCreativeSlot {
            entity_id: 1,
            slot: 0,
            item: rustbound_protocol::play::Slot {
                present: true,
                item_id: 10, // dirt
                count: 1,
                nbt: Vec::new(),
            },
        })?;

        // Wait for slot update
        std::thread::sleep(Duration::from_millis(50));
        while event_rx.try_recv().is_ok() {}

        // Place on top face (face=1) of block at (0,64,0)
        handle.send(super::TickMessage::PlaceBlock {
            entity_id: 1,
            position: (0, 64, 0),
            face: 1,
        })?;

        // Should receive BlockUpdate at (0,65,0) with block_state=10 (dirt)
        let start = Instant::now();
        let mut got_block = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::BlockUpdate {
                    position,
                    block_state,
                }) => {
                    assert_eq!(position, (0, 65, 0));
                    assert_eq!(block_state, 10, "should place dirt (block_state=10)");
                    got_block = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_block, "should receive BlockUpdate for dirt placement");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_place_empty_hand_no_block() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 1, // Creative
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Held slot is empty (default) — place should do nothing
        handle.send(super::TickMessage::PlaceBlock {
            entity_id: 1,
            position: (0, 64, 0),
            face: 1,
        })?;

        // Should NOT receive BlockUpdate
        let start = Instant::now();
        let mut got_block = false;
        while start.elapsed() < Duration::from_millis(200) {
            match event_rx.try_recv() {
                Ok(SessionEvent::BlockUpdate { .. }) => {
                    got_block = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            !got_block,
            "should not receive BlockUpdate when held slot is empty"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_creative_place_stone_uses_stone_block_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0, // disable autosave in tests
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 1, // Creative
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Set hotbar slot 0 to stone (item_id=1, block_state=1)
        handle.send(super::TickMessage::SetCreativeSlot {
            entity_id: 1,
            slot: 0,
            item: rustbound_protocol::play::Slot {
                present: true,
                item_id: 1, // stone
                count: 1,
                nbt: Vec::new(),
            },
        })?;

        // Wait for slot update
        std::thread::sleep(Duration::from_millis(50));
        while event_rx.try_recv().is_ok() {}

        // Place on top face (face=1) of block at (0,64,0)
        handle.send(super::TickMessage::PlaceBlock {
            entity_id: 1,
            position: (0, 64, 0),
            face: 1,
        })?;

        // Should receive BlockUpdate at (0,65,0) with block_state=1 (stone)
        let start = Instant::now();
        let mut got_block = false;
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::BlockUpdate {
                    position,
                    block_state,
                }) => {
                    assert_eq!(position, (0, 65, 0));
                    assert_eq!(block_state, 1, "should place stone (block_state=1)");
                    got_block = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_block, "should receive BlockUpdate for stone placement");

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_autosave_persists_blocks() -> Result<(), Box<dyn std::error::Error>> {
        // Use a unique level name to avoid conflicts
        let level_name = format!(
            "test-autosave-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        // Start tick loop with 1-second autosave
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level_name.clone(),
            1, // 1 second autosave
        )?;

        // Place a block
        handle.send(super::TickMessage::SetBlock {
            position: (10, 64, 20),
            block_state: 1,
        })?;

        // Wait for autosave to trigger (at least 1 second + buffer)
        std::thread::sleep(Duration::from_millis(1500));

        // Shut down
        handle.shutdown();

        // Verify the block was saved
        let loaded = crate::persist::load_overrides(&level_name);
        assert_eq!(
            loaded.get(&(10, 64, 20)),
            Some(&1),
            "block should be persisted by autosave"
        );

        // Cleanup
        std::fs::remove_dir_all(&level_name).ok();
        Ok(())
    }

    #[test]
    fn tick_loop_shutdown_flushes_data() -> Result<(), Box<dyn std::error::Error>> {
        let level_name = format!(
            "test-shutdown-flush-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        // Start tick loop with autosave disabled (0)
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level_name.clone(),
            0, // autosave disabled
        )?;

        // Place a block
        handle.send(super::TickMessage::SetBlock {
            position: (5, 70, -10),
            block_state: 10,
        })?;

        // Wait a bit for the block to be processed
        std::thread::sleep(Duration::from_millis(100));

        // Shut down - should flush data even with autosave disabled
        handle.shutdown();

        // Verify the block was saved on shutdown
        let loaded = crate::persist::load_overrides(&level_name);
        assert_eq!(
            loaded.get(&(5, 70, -10)),
            Some(&10),
            "block should be persisted on shutdown even with autosave disabled"
        );

        // Cleanup
        std::fs::remove_dir_all(&level_name).ok();
        Ok(())
    }

    // --- Food drain and starvation tests ---

    #[test]
    fn drain_food_depletes_saturation_first() {
        let mut inv = PlayerInventory::new();
        assert_eq!(inv.food_saturation, DEFAULT_FOOD_SATURATION); // 5.0
        assert_eq!(inv.food, DEFAULT_FOOD); // 20

        // Drain 5 times: saturation goes 5->4->3->2->1->0
        for _ in 0..5 {
            assert!(inv.drain_food());
        }
        assert_eq!(inv.food_saturation, 0.0);
        assert_eq!(inv.food, 20, "food should not deplete while saturation > 0");
    }

    #[test]
    fn drain_food_then_depletes_food() {
        let mut inv = PlayerInventory::new();
        inv.food_saturation = 0.0;
        inv.food = 10;

        // Drain 3 times: food goes 10->9->8->7
        for _ in 0..3 {
            assert!(inv.drain_food());
        }
        assert_eq!(inv.food, 7);
        assert_eq!(inv.food_saturation, 0.0);
    }

    #[test]
    fn drain_food_at_zero_returns_false() {
        let mut inv = PlayerInventory::new();
        inv.food_saturation = 0.0;
        inv.food = 0;
        assert!(!inv.drain_food(), "drain at 0/0 should return false");
    }

    #[test]
    fn starve_applies_damage_at_zero_food() {
        let mut inv = PlayerInventory::new();
        inv.food = 0;
        inv.food_saturation = 0.0;
        inv.health = 20.0;

        assert!(inv.starve());
        assert_eq!(inv.health, 19.0);
        assert!(!inv.is_dead, "should not be dead at 19 HP");
    }

    #[test]
    fn starve_can_kill() {
        let mut inv = PlayerInventory::new();
        inv.food = 0;
        inv.food_saturation = 0.0;
        inv.health = 1.0;

        assert!(inv.starve());
        assert_eq!(inv.health, 0.0);
        assert!(inv.is_dead, "should be dead at 0 HP from starvation");
    }

    #[test]
    fn starve_no_damage_when_food_nonzero() {
        let mut inv = PlayerInventory::new();
        inv.food = 1;
        inv.health = 20.0;
        assert!(!inv.starve(), "should not starve when food > 0");
        assert_eq!(inv.health, 20.0);
    }

    #[test]
    fn starve_no_damage_when_already_dead() {
        let mut inv = PlayerInventory::new();
        inv.food = 0;
        inv.is_dead = true;
        inv.health = 0.0;
        assert!(!inv.starve(), "should not starve when already dead");
    }

    #[test]
    fn food_drain_interval_is_4_seconds() {
        // 80 ticks at 20 TPS = 4 seconds
        assert_eq!(FOOD_DRAIN_INTERVAL_TICKS, 80);
        assert_eq!(FOOD_DRAIN_INTERVAL_TICKS / TPS, 4);
    }

    // --- Block break progress tests ---

    #[test]
    fn tick_loop_survival_dig_completes_break() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0,
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0, // Survival
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Place a block at (0, 100, 0)
        handle.send(super::TickMessage::SetBlock {
            position: (0, 100, 0),
            block_state: 1,
        })?;

        // Wait for block set
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Start digging
        handle.send(super::TickMessage::DigBlock {
            entity_id: 1,
            action: 0, // StartDestroy
            position: (0, 100, 0),
        })?;

        // Wait for break to complete (DEFAULT_BLOCK_HARDNESS_TICKS = 30 ticks = 1.5s)
        // plus some margin for tick loop scheduling
        let start = Instant::now();
        let mut got_block_update = false;
        while start.elapsed() < Duration::from_secs(5) {
            match event_rx.try_recv() {
                Ok(SessionEvent::BlockUpdate {
                    position,
                    block_state,
                }) => {
                    if position == (0, 100, 0) && block_state == 0 {
                        got_block_update = true;
                        break;
                    }
                }
                Ok(SessionEvent::SetBlockDestroyStage { .. }) => {
                    // Expected: progress updates
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        assert!(
            got_block_update,
            "should receive BlockUpdate with air after break completes"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_survival_dig_abort_cancels_break() -> Result<(), Box<dyn std::error::Error>> {
        let level = TestLevel::new();
        let (mut handle, _event_rx) = start_tick_loop(
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            level.name(),
            0,
        )?;

        let (event_tx, event_rx) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 1,
            uuid: Uuid::new(0, 0),
            username: "Steve".to_string(),
            gamemode: 0, // Survival
            view_distance: 10,
            event_sender: event_tx,
        })?;

        // Wait for join
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Place a block
        handle.send(super::TickMessage::SetBlock {
            position: (0, 100, 0),
            block_state: 1,
        })?;
        std::thread::sleep(Duration::from_millis(100));
        while event_rx.try_recv().is_ok() {}

        // Start digging
        handle.send(super::TickMessage::DigBlock {
            entity_id: 1,
            action: 0,
            position: (0, 100, 0),
        })?;

        // Wait a bit for some progress
        std::thread::sleep(Duration::from_millis(200));
        while event_rx.try_recv().is_ok() {}

        // Abort digging
        handle.send(super::TickMessage::DigBlock {
            entity_id: 1,
            action: 1, // AbortDestroy
            position: (0, 100, 0),
        })?;

        // Wait and check that no BlockUpdate (air) is sent — block should remain
        let start = Instant::now();
        let mut got_unexpected_air = false;
        while start.elapsed() < Duration::from_secs(2) {
            match event_rx.try_recv() {
                Ok(SessionEvent::BlockUpdate { block_state: 0, .. }) => {
                    got_unexpected_air = true;
                }
                Ok(SessionEvent::SetBlockDestroyStage { destroy_stage, .. }) => {
                    // Should receive -1 to remove animation
                    assert_eq!(
                        destroy_stage, -1,
                        "abort should send destroy_stage -1 to remove animation"
                    );
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        assert!(
            !got_unexpected_air,
            "should not receive BlockUpdate(air) after abort"
        );

        handle.shutdown();
        Ok(())
    }
}
