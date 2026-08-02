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
                    view_distance,
                    event_sender,
                } => {
                    let player =
                        crate::world::PlayerHandle::new(entity_id, uuid, username.clone(), 0);
                    let (px, py, pz) = player.position();
                    world.add_player(player);
                    session_senders.insert(entity_id, event_sender.clone());
                    chunk_states.insert(entity_id, PlayerChunkState::new(view_distance));

                    // Broadcast join to all OTHER existing sessions
                    for (&eid, sender) in &session_senders {
                        if eid != entity_id {
                            let _ = sender.send(SessionEvent::PlayerJoined {
                                entity_id,
                                uuid,
                                username: username.clone(),
                                gamemode: 0,
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
        KEEP_ALIVE_INTERVAL_TICKS, PlayerChunkState, TICK_DURATION, TPS, TickEvent,
        chunk_x_from_world, chunk_z_from_world, start_tick_loop,
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
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Player 2 joins
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
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
            view_distance: 10,
            event_sender: event_tx1,
        })?;

        // Player 2 joins
        let (event_tx2, event_rx2) = channel::<SessionEvent>();
        handle.send(super::TickMessage::PlayerJoined {
            entity_id: 2,
            uuid: Uuid::new(1, 0),
            username: "Alex".to_string(),
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
}
