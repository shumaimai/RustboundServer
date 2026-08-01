//! World management for the Rustbound server.
//!
//! Holds chunks, players, and entities. The world is owned by the tick
//! thread and accessed through narrow synchronization boundaries.

use std::collections::HashMap;

use rustbound_protocol::primitives::Uuid;

/// A chunk coordinate pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPos {
    /// The chunk X coordinate.
    pub x: i32,
    /// The chunk Z coordinate.
    pub z: i32,
}

impl ChunkPos {
    /// Creates a new chunk position.
    pub fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }
}

/// A loaded chunk. For the initial implementation, chunk data is opaque.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The chunk position.
    pub pos: ChunkPos,
    /// Whether the chunk has been generated.
    pub generated: bool,
}

impl Chunk {
    /// Creates a new empty chunk at the given position.
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            generated: false,
        }
    }
}

/// The world containing chunks, players, and entities.
///
/// This struct is owned by the tick thread. Access from other threads
/// should go through message channels, not direct references.
#[derive(Debug)]
pub struct World {
    /// Loaded chunks keyed by position.
    chunks: HashMap<ChunkPos, Chunk>,
    /// Players currently in this world, keyed by entity ID.
    players: HashMap<i32, PlayerHandle>,
    /// The next entity ID to assign.
    next_entity_id: i32,
    /// The world's spawn point.
    spawn_x: f64,
    spawn_y: f64,
    spawn_z: f64,
}

impl World {
    /// Creates a new empty world with a default spawn point.
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            players: HashMap::new(),
            next_entity_id: 1,
            spawn_x: 0.0,
            spawn_y: 64.0,
            spawn_z: 0.0,
        }
    }

    /// Allocates the next entity ID.
    pub fn allocate_entity_id(&mut self) -> i32 {
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        id
    }

    /// Returns the spawn point.
    pub fn spawn_point(&self) -> (f64, f64, f64) {
        (self.spawn_x, self.spawn_y, self.spawn_z)
    }

    /// Sets the spawn point.
    pub fn set_spawn_point(&mut self, x: f64, y: f64, z: f64) {
        self.spawn_x = x;
        self.spawn_y = y;
        self.spawn_z = z;
    }

    /// Loads a chunk at the given position.
    pub fn load_chunk(&mut self, pos: ChunkPos) {
        self.chunks.insert(pos, Chunk::new(pos));
    }

    /// Unloads a chunk at the given position.
    pub fn unload_chunk(&mut self, pos: ChunkPos) {
        self.chunks.remove(&pos);
    }

    /// Returns whether a chunk is loaded.
    pub fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    /// Returns the number of loaded chunks.
    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Adds a player to the world.
    pub fn add_player(&mut self, player: PlayerHandle) {
        self.players.insert(player.entity_id, player);
    }

    /// Removes a player from the world by entity ID.
    pub fn remove_player(&mut self, entity_id: i32) -> Option<PlayerHandle> {
        self.players.remove(&entity_id)
    }

    /// Returns the number of players in the world.
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Returns an iterator over all players.
    pub fn players(&self) -> impl Iterator<Item = &PlayerHandle> {
        self.players.values()
    }

    /// Gets a player by entity ID.
    pub fn get_player(&self, entity_id: i32) -> Option<&PlayerHandle> {
        self.players.get(&entity_id)
    }

    /// Gets a mutable player by entity ID.
    pub fn get_player_mut(&mut self, entity_id: i32) -> Option<&mut PlayerHandle> {
        self.players.get_mut(&entity_id)
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// A handle to a player in the world.
#[derive(Debug, Clone)]
pub struct PlayerHandle {
    /// The player's entity ID.
    pub entity_id: i32,
    /// The player's UUID.
    pub uuid: Uuid,
    /// The player's username.
    pub username: String,
    /// The player's X coordinate.
    pub x: f64,
    /// The player's Y coordinate.
    pub y: f64,
    /// The player's Z coordinate.
    pub z: f64,
    /// The player's yaw (degrees).
    pub yaw: f32,
    /// The player's pitch (degrees).
    pub pitch: f32,
    /// The player's gamemode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
    pub gamemode: u8,
}

impl PlayerHandle {
    /// Creates a new player handle at the spawn point.
    pub fn new(entity_id: i32, uuid: Uuid, username: String, gamemode: u8) -> Self {
        Self {
            entity_id,
            uuid,
            username,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            gamemode,
        }
    }

    /// Updates the player's position.
    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.x = x;
        self.y = y;
        self.z = z;
    }

    /// Updates the player's rotation.
    pub fn set_rotation(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Returns the player's position.
    pub fn position(&self) -> (f64, f64, f64) {
        (self.x, self.y, self.z)
    }

    /// Returns the player's rotation.
    pub fn rotation(&self) -> (f32, f32) {
        (self.yaw, self.pitch)
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkPos, PlayerHandle, World};
    use rustbound_protocol::primitives::Uuid;

    #[test]
    fn world_starts_empty() {
        let world = World::new();
        assert_eq!(world.loaded_chunk_count(), 0);
        assert_eq!(world.player_count(), 0);
        assert_eq!(world.spawn_point(), (0.0, 64.0, 0.0));
    }

    #[test]
    fn world_allocates_sequential_entity_ids() {
        let mut world = World::new();
        assert_eq!(world.allocate_entity_id(), 1);
        assert_eq!(world.allocate_entity_id(), 2);
        assert_eq!(world.allocate_entity_id(), 3);
    }

    #[test]
    fn world_loads_and_unloads_chunks() {
        let mut world = World::new();
        let pos = ChunkPos::new(10, -5);
        world.load_chunk(pos);
        assert!(world.is_chunk_loaded(pos));
        assert_eq!(world.loaded_chunk_count(), 1);

        world.unload_chunk(pos);
        assert!(!world.is_chunk_loaded(pos));
        assert_eq!(world.loaded_chunk_count(), 0);
    }

    #[test]
    fn world_adds_and_removes_players() -> Result<(), Box<dyn std::error::Error>> {
        let mut world = World::new();
        let entity_id = world.allocate_entity_id();
        let player = PlayerHandle::new(entity_id, Uuid::new(0, 0), "Steve".to_string(), 0);
        world.add_player(player);

        assert_eq!(world.player_count(), 1);
        assert!(world.get_player(entity_id).is_some());

        let removed = world.remove_player(entity_id);
        assert!(removed.is_some());
        let removed = removed.ok_or("player should have been removed")?;
        assert_eq!(removed.username, "Steve");
        assert_eq!(world.player_count(), 0);
        Ok(())
    }

    #[test]
    fn player_handle_position_and_rotation() {
        let mut player = PlayerHandle::new(1, Uuid::new(0, 0), "Alex".to_string(), 1);
        assert_eq!(player.position(), (0.0, 64.0, 0.0));

        player.set_position(100.0, 70.0, -50.0);
        assert_eq!(player.position(), (100.0, 70.0, -50.0));

        player.set_rotation(90.0, -45.0);
        assert_eq!(player.rotation(), (90.0, -45.0));
    }

    #[test]
    fn world_set_spawn_point() {
        let mut world = World::new();
        world.set_spawn_point(128.0, 70.0, -256.0);
        assert_eq!(world.spawn_point(), (128.0, 70.0, -256.0));
    }
}
