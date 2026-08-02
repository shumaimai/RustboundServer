//! Tick loop for the Rustbound server.
//!
//! Runs at a fixed 20 TPS (50ms per tick) on a single authoritative thread.
//! The tick loop owns the world and processes periodic tasks like keep-alive
//! scheduling.

use std::collections::HashMap;
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
                    event_sender,
                } => {
                    let player =
                        crate::world::PlayerHandle::new(entity_id, uuid, username.clone(), 0);
                    let (px, py, pz) = player.position();
                    world.add_player(player);
                    session_senders.insert(entity_id, event_sender.clone());

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

                    let _ = event_tx.send(TickEvent::PlayerAdded { entity_id });
                    player_count.fetch_add(1, Ordering::AcqRel);
                }
                TickMessage::PlayerLeft { entity_id } => {
                    // Get the player's UUID before removing
                    let uuid = world.get_player(entity_id).map(|p| p.uuid);
                    world.remove_player(entity_id);
                    session_senders.remove(&entity_id);
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
                    on_ground: _,
                } => {
                    if let Some(player) = world.get_player_mut(entity_id) {
                        player.set_position(x, y, z);
                        player.set_rotation(yaw, pitch);
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
    use super::{KEEP_ALIVE_INTERVAL_TICKS, TICK_DURATION, TPS, TickEvent, start_tick_loop};
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
}
