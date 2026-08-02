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

    /// Updates the center chunk. Returns (center_changed, new_chunks) where
    /// new_chunks is the set of chunks to load (in desired but not in loaded).
    fn update_center(&mut self, new_cx: i32, new_cz: i32) -> (bool, Vec<crate::world::ChunkPos>) {
        if self.center_x == new_cx && self.center_z == new_cz {
            return (false, Vec::new());
        }
        self.center_x = new_cx;
        self.center_z = new_cz;
        let desired = self.desired_chunk_set();
        let new_chunks: Vec<_> = desired
            .iter()
            .filter(|pos| !self.loaded_chunks.contains(pos))
            .copied()
            .collect();
        self.loaded_chunks = desired;
        (true, new_chunks)
    }

    /// Updates the view distance. Returns the list of new chunks to load
    /// (chunks that are now in range but weren't before).
    fn update_view_distance(&mut self, new_vd: i32) -> Vec<crate::world::ChunkPos> {
        let new_vd = new_vd.max(0);
        if self.view_distance == new_vd {
            return Vec::new();
        }
        self.view_distance = new_vd;
        let desired = self.desired_chunk_set();
        let new_chunks: Vec<_> = desired
            .iter()
            .filter(|pos| !self.loaded_chunks.contains(pos))
            .copied()
            .collect();
        self.loaded_chunks = desired;
        new_chunks
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
    /// A client sent its view distance via Client Information.
    SetClientViewDistance {
        /// The entity ID of the player.
        entity_id: i32,
        /// The client's requested view distance (in chunks).
        view_distance: i32,
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
pub fn start_tick_loop(
    player_count: Arc<AtomicUsize>,
) -> Result<(TickHandle, Receiver<TickEvent>), TickStartError> {
    let (msg_tx, msg_rx) = channel::<TickMessage>();
    let (event_tx, event_rx) = channel::<TickEvent>();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    let thread = thread::Builder::new()
        .name("rustbound-tick".to_string())
        .spawn(move || {
            run_tick_loop(msg_rx, event_tx, shutdown_clone, player_count);
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
) {
    let mut world = World::new();
    let mut tick_count: u64 = 0;
    let mut last_keep_alive_tick: u64 = 0;
    let mut session_senders: HashMap<i32, Sender<SessionEvent>> = HashMap::new();
    let mut chunk_states: HashMap<i32, PlayerChunkState> = HashMap::new();
    let mut inventories: HashMap<i32, PlayerInventory> = HashMap::new();

    while !shutdown.load(Ordering::Acquire) {
        let tick_start = Instant::now();

        // Process incoming messages (non-blocking)
        while let Ok(msg) = msg_rx.try_recv() {
            match msg {
                TickMessage::Shutdown => {
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
                    let player = crate::world::PlayerHandle::new(
                        entity_id,
                        uuid,
                        username.clone(),
                        gamemode,
                    );
                    let (px, py, pz) = player.position();
                    world.add_player(player);
                    session_senders.insert(entity_id, event_sender.clone());
                    chunk_states.insert(entity_id, PlayerChunkState::new(view_distance));
                    inventories.insert(entity_id, PlayerInventory::new());

                    // Send initial inventory (empty) to the new player
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
                                let (center_changed, new_chunks) =
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
                TickMessage::SetClientViewDistance {
                    entity_id,
                    view_distance,
                } => {
                    if let Some(chunk_state) = chunk_states.get_mut(&entity_id) {
                        let new_chunks = chunk_state.update_view_distance(view_distance);
                        if let Some(sender) = session_senders.get(&entity_id) {
                            for chunk in new_chunks {
                                let _ = sender.send(SessionEvent::LoadChunk {
                                    chunk_x: chunk.x,
                                    chunk_z: chunk.z,
                                });
                            }
                        }
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
                TickMessage::ClientStatus { entity_id, action } => {
                    // Action 0 = Perform respawn
                    if action == 0 {
                        if let Some(inv) = inventories.get_mut(&entity_id) {
                            if inv.is_dead {
                                inv.respawn();
                                if let Some(sender) = session_senders.get(&entity_id) {
                                    // Send Respawn packet
                                    let _ = sender.send(SessionEvent::RespawnPlayer {
                                        dimension_type: "minecraft:overworld".to_string(),
                                        dimension_name: "minecraft:overworld".to_string(),
                                        hashed_seed: 0,
                                        gamemode: 1, // Creative
                                        previous_gamemode: -1,
                                        is_debug: false,
                                        is_flat: false,
                                        has_death_location: false,
                                        death_dimension_name: String::new(),
                                        death_location: (0, 0, 0),
                                        portal_cooldown: 0,
                                        data_kept: 0,
                                        x: 0.0,
                                        y: 64.0,
                                        z: 0.0,
                                    });
                                    // Send updated health after respawn
                                    let _ = sender.send(SessionEvent::SetHealth {
                                        health: inv.health,
                                        food: inv.food,
                                        food_saturation: inv.food_saturation,
                                    });
                                }
                                // Reset player position in world to spawn
                                if let Some(player) = world.get_player_mut(entity_id) {
                                    player.set_position(0.0, 64.0, 0.0);
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
                    if let Some(sender) = session_senders.get(&entity_id) {
                        let _ = sender.send(SessionEvent::SetHealth {
                            health: inv.health,
                            food: inv.food,
                            food_saturation: inv.food_saturation,
                        });
                    }
                }
            }
        }

        tick_count += 1;

        // Sleep for the remaining tick time
        let elapsed = tick_start.elapsed();
        if elapsed < TICK_DURATION {
            thread::sleep(TICK_DURATION - elapsed);
        }
    }

    let _ = event_tx.send(TickEvent::Shutdown);
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_FOOD, DEFAULT_FOOD_SATURATION, DEFAULT_HEALTH, KEEP_ALIVE_INTERVAL_TICKS,
        PlayerChunkState, TICK_DURATION, TPS, TickEvent, VOID_DEATH_Y, chunk_x_from_world,
        chunk_z_from_world, start_tick_loop,
    };
    use crate::session::SessionEvent;
    use rustbound_protocol::primitives::Uuid;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;
        std::thread::sleep(Duration::from_millis(100));
        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_processes_player_join_and_leave() -> Result<(), Box<dyn std::error::Error>> {
        let (mut handle, event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;
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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (changed, new_chunks) = state.update_center(0, 0);
        assert!(!changed);
        assert!(new_chunks.is_empty());
    }

    #[test]
    fn chunk_state_update_center_new_chunks() {
        let mut state = PlayerChunkState::new(2);
        // Move from (0,0) to (1,0) - center shifts by 1 chunk
        let (changed, new_chunks) = state.update_center(1, 0);
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
    }

    #[test]
    fn chunk_state_update_view_distance_grows() {
        let mut state = PlayerChunkState::new(10);
        // Initial view distance is min(10, 2) = 2
        assert_eq!(state.view_distance, 2);
        // Client requests view distance 5
        let new_chunks = state.update_view_distance(5);
        assert_eq!(state.view_distance, 5);
        // New chunks: those in radius 5 but not in radius 2
        // Radius 5: 11x11 = 121, radius 2: 5x5 = 25, new = 96
        assert_eq!(new_chunks.len(), 96);
    }

    #[test]
    fn chunk_state_update_view_distance_shrinks() {
        let mut state = PlayerChunkState::new(10);
        // Grow to 5 first
        state.update_view_distance(5);
        assert_eq!(state.view_distance, 5);
        // Shrink to 3
        let new_chunks = state.update_view_distance(3);
        assert_eq!(state.view_distance, 3);
        // No new chunks when shrinking
        assert!(new_chunks.is_empty());
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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let inv = super::PlayerInventory::new();
        assert_eq!(inv.slots.len(), super::PLAYER_INVENTORY_SIZE);
        for slot in &inv.slots {
            assert!(!slot.present, "all slots should be empty by default");
        }
        assert_eq!(inv.held_slot, 0);
        assert_eq!(inv.state_id, 0);
    }

    #[test]
    fn player_inventory_set_slot_changes_state() {
        let mut inv = super::PlayerInventory::new();
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
        let mut inv = super::PlayerInventory::new();
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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
    fn player_inventory_default_vitals() {
        let inv = super::PlayerInventory::new();
        assert_eq!(inv.health, DEFAULT_HEALTH);
        assert_eq!(inv.food, DEFAULT_FOOD);
        assert_eq!(inv.food_saturation, DEFAULT_FOOD_SATURATION);
        assert!(!inv.is_dead);
    }

    #[test]
    fn player_inventory_kill_and_respawn() {
        let mut inv = super::PlayerInventory::new();
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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
    fn tick_loop_void_death_kills_player() -> Result<(), Box<dyn std::error::Error>> {
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        while start.elapsed() < Duration::from_millis(500) {
            match event_rx.try_recv() {
                Ok(SessionEvent::SetHealth { health, .. }) => {
                    if health <= 0.0 {
                        got_death = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(
            got_death,
            "should receive SetHealth with 0 HP on void death"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_respawn_after_death() -> Result<(), Box<dyn std::error::Error>> {
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert!(got_respawn, "should receive RespawnPlayer event");
        assert!(
            got_health_restore,
            "should receive SetHealth with restored HP after respawn"
        );

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn tick_loop_respawn_ignored_when_not_dead() -> Result<(), Box<dyn std::error::Error>> {
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
        let (mut handle, _event_rx) =
            start_tick_loop(std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)))?;

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
}
