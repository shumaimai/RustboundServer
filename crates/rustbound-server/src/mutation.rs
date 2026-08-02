//! Tick-owned world mutation facade.
//!
//! Provides a clean, typed API for mutating the world from outside the tick
//! thread. All operations are sent as [`TickMessage`]s to the tick loop, which
//! owns the [`World`](crate::world::World) and applies mutations serially.
//!
//! This is the prerequisite for a future Mod API: mods hold a [`WorldFacade`]
//! and call its methods without needing direct access to the world or
//! knowledge of the internal message protocol.

use std::sync::mpsc::Sender;

use crate::tick::TickMessage;

/// A facade for tick-owned world mutations.
///
/// Wraps a [`Sender<TickMessage>`] and provides typed methods that send
/// mutation requests to the tick loop. The tick loop processes these
/// serially on its authoritative thread, ensuring thread-safe world access.
///
/// # Example
/// ```no_run
/// use rustbound_server::mutation::WorldFacade;
/// use rustbound_server::tick::TickMessage;
/// use std::sync::mpsc::channel;
///
/// let (tx, _rx) = channel::<TickMessage>();
/// let facade = WorldFacade::new(tx);
/// facade.set_block(10, 64, -5, 1); // set block to state 1 (stone)
/// facade.fill_block(0, 0, 0, 10, 10, 10, 0); // fill region with air
/// ```
#[derive(Debug, Clone)]
pub struct WorldFacade {
    sender: Sender<TickMessage>,
}

impl WorldFacade {
    /// Creates a new `WorldFacade` from a tick loop sender.
    pub fn new(sender: Sender<TickMessage>) -> Self {
        Self { sender }
    }

    /// Sets a single block at the given coordinates to the specified block state.
    ///
    /// The tick loop will broadcast a Block Update to all connected sessions.
    pub fn set_block(&self, x: i32, y: i32, z: i32, block_state: i32) {
        let _ = self.sender.send(TickMessage::SetBlock {
            position: (x, y, z),
            block_state,
        });
    }

    /// Fills a rectangular region with the specified block state.
    ///
    /// Sends one `SetBlock` message per block in the region. The tick loop
    /// processes these serially. For large fills, consider batching.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_block(
        &self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        block_state: i32,
    ) {
        let (x_min, x_max) = (x1.min(x2), x1.max(x2));
        let (y_min, y_max) = (y1.min(y2), y1.max(y2));
        let (z_min, z_max) = (z1.min(z2), z1.max(z2));
        for y in y_min..=y_max {
            for z in z_min..=z_max {
                for x in x_min..=x_max {
                    self.set_block(x, y, z, block_state);
                }
            }
        }
    }

    /// Sets a column of blocks from `y_min` to `y_max` at the given (x, z).
    pub fn set_column(&self, x: i32, y_min: i32, y_max: i32, z: i32, block_state: i32) {
        for y in y_min..=y_max {
            self.set_block(x, y, z, block_state);
        }
    }

    /// Returns the underlying sender, allowing custom message construction.
    pub fn sender(&self) -> &Sender<TickMessage> {
        &self.sender
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn set_block_sends_set_block_message() {
        let (tx, rx) = channel::<TickMessage>();
        let facade = WorldFacade::new(tx);
        facade.set_block(1, 2, 3, 42);
        match rx.recv() {
            Ok(TickMessage::SetBlock {
                position,
                block_state,
            }) => {
                assert_eq!(position, (1, 2, 3));
                assert_eq!(block_state, 42);
            }
            other => panic!("expected SetBlock, got {other:?}"),
        }
    }

    #[test]
    fn fill_block_sends_correct_number_of_messages() {
        let (tx, rx) = channel::<TickMessage>();
        let facade = WorldFacade::new(tx);
        // 3x2x1 = 6 blocks
        facade.fill_block(0, 0, 0, 2, 1, 0, 1);
        let mut count = 0;
        while let Ok(TickMessage::SetBlock { block_state, .. }) = rx.try_recv() {
            assert_eq!(block_state, 1);
            count += 1;
        }
        assert_eq!(count, 6, "should send 6 SetBlock messages for 3x2x1 region");
    }

    #[test]
    fn fill_block_handles_inverted_coords() {
        let (tx, rx) = channel::<TickMessage>();
        let facade = WorldFacade::new(tx);
        // Inverted: (5,5,5) to (3,3,3) = 3x3x3 = 27 blocks
        facade.fill_block(5, 5, 5, 3, 3, 3, 0);
        let mut count = 0;
        while let Ok(TickMessage::SetBlock { .. }) = rx.try_recv() {
            count += 1;
        }
        assert_eq!(count, 27, "should handle inverted coordinates");
    }

    #[test]
    fn set_column_sends_messages_for_y_range() {
        let (tx, rx) = channel::<TickMessage>();
        let facade = WorldFacade::new(tx);
        facade.set_column(10, 0, 4, 20, 1);
        let mut count = 0;
        while let Ok(TickMessage::SetBlock { position, .. }) = rx.try_recv() {
            assert_eq!(position.0, 10);
            assert_eq!(position.2, 20);
            count += 1;
        }
        assert_eq!(count, 5, "should send 5 messages for y=0..=4");
    }

    #[test]
    fn facade_is_cloneable() {
        let (tx, _rx) = channel::<TickMessage>();
        let facade1 = WorldFacade::new(tx);
        let facade2 = facade1.clone();
        // Both should be usable
        facade1.set_block(0, 0, 0, 1);
        facade2.set_block(1, 1, 1, 2);
    }

    #[test]
    fn sender_returns_underlying_sender() {
        let (tx, _rx) = channel::<TickMessage>();
        let facade = WorldFacade::new(tx.clone());
        let sender = facade.sender();
        let _ = sender.send(TickMessage::SetBlock {
            position: (0, 0, 0),
            block_state: 0,
        });
    }
}
