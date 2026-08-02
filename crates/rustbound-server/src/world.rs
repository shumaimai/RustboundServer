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

/// A loaded chunk with its data payload.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The chunk position.
    pub pos: ChunkPos,
    /// Whether the chunk has been generated.
    pub generated: bool,
    /// The heightmaps NBT blob.
    pub heightmaps: Vec<u8>,
    /// The chunk sections and biomes data.
    pub data: Vec<u8>,
    /// Block entity NBT blobs.
    pub block_entities: Vec<Vec<u8>>,
}

impl Chunk {
    /// Creates a new empty chunk at the given position.
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            generated: false,
            heightmaps: Vec::new(),
            data: Vec::new(),
            block_entities: Vec::new(),
        }
    }

    /// Creates a new generated chunk at the given position using the flat
    /// world generator.
    pub fn generate(pos: ChunkPos) -> Self {
        let (heightmaps, data, block_entities) = crate::chunk::generate_chunk(pos.x, pos.z);
        Self {
            pos,
            generated: true,
            heightmaps,
            data,
            block_entities,
        }
    }

    /// Returns a `ChunkData` packet from this chunk's data.
    pub fn to_chunk_data(&self) -> rustbound_protocol::play::ChunkData {
        rustbound_protocol::play::ChunkData {
            chunk_x: self.pos.x,
            chunk_z: self.pos.z,
            heightmaps: self.heightmaps.clone(),
            data: self.data.clone(),
            block_entities: self.block_entities.clone(),
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
    /// Block state overrides from dig/place, keyed by absolute position.
    /// Block state 0 = air.
    block_overrides: HashMap<(i32, i32, i32), i32>,
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
            block_overrides: HashMap::new(),
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

    /// Loads a chunk at the given position, generating its data.
    pub fn load_chunk(&mut self, pos: ChunkPos) {
        self.chunks.insert(pos, Chunk::generate(pos));
    }

    /// Unloads a chunk at the given position.
    pub fn unload_chunk(&mut self, pos: ChunkPos) {
        self.chunks.remove(&pos);
    }

    /// Returns whether a chunk is loaded.
    pub fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    /// Returns a reference to a loaded chunk, if present.
    pub fn get_chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    /// Computes the set of chunk positions within the given radius of a
    /// center chunk position.
    ///
    /// The radius is the view distance (in chunks). The result includes
    /// all chunks within a square of side `2*radius + 1` centered on
    /// `(center_x, center_z)`.
    pub fn desired_chunks(center_x: i32, center_z: i32, radius: i32) -> Vec<ChunkPos> {
        let mut result = Vec::new();
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                result.push(ChunkPos::new(center_x + dx, center_z + dz));
            }
        }
        result
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

    /// Sets a block at the given absolute position to the given block state.
    ///
    /// This stores an override in the world's block override map. The chunk
    /// data blob itself is not modified (it remains the generated snapshot).
    /// Block state 0 = air.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block_state: i32) {
        self.block_overrides.insert((x, y, z), block_state);
    }

    /// Gets the block state at the given absolute position.
    ///
    /// Returns the override if present, otherwise 0 (air) for unloaded
    /// chunks. For loaded chunks without an override, returns 0 (air) as
    /// well since the flat generator produces air above the surface.
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> i32 {
        self.block_overrides.get(&(x, y, z)).copied().unwrap_or(0)
    }

    /// Returns the number of block overrides.
    pub fn block_override_count(&self) -> usize {
        self.block_overrides.len()
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
    use super::{Chunk, ChunkPos, PlayerHandle, World};
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

    #[test]
    fn desired_chunks_radius_0() {
        let chunks = World::desired_chunks(0, 0, 0);
        assert_eq!(chunks.len(), 1);
        assert!(chunks.contains(&ChunkPos::new(0, 0)));
    }

    #[test]
    fn desired_chunks_radius_1() {
        let chunks = World::desired_chunks(0, 0, 1);
        assert_eq!(chunks.len(), 9); // 3x3
        assert!(chunks.contains(&ChunkPos::new(-1, -1)));
        assert!(chunks.contains(&ChunkPos::new(0, 0)));
        assert!(chunks.contains(&ChunkPos::new(1, 1)));
    }

    #[test]
    fn desired_chunks_radius_2() {
        let chunks = World::desired_chunks(10, -5, 2);
        assert_eq!(chunks.len(), 25); // 5x5
        assert!(chunks.contains(&ChunkPos::new(8, -7)));
        assert!(chunks.contains(&ChunkPos::new(12, -3)));
    }

    #[test]
    fn desired_chunks_radius_10() {
        let chunks = World::desired_chunks(0, 0, 10);
        assert_eq!(chunks.len(), 441); // 21x21
    }

    #[test]
    fn generated_chunk_has_data() {
        let chunk = Chunk::generate(ChunkPos::new(0, 0));
        assert!(chunk.generated);
        assert!(!chunk.heightmaps.is_empty());
        assert!(!chunk.data.is_empty());
    }

    #[test]
    fn load_chunk_generates_data() {
        let mut world = World::new();
        let pos = ChunkPos::new(5, 10);
        world.load_chunk(pos);
        match world.get_chunk(pos) {
            Some(chunk) => {
                assert!(chunk.generated);
                assert!(!chunk.data.is_empty());
            }
            None => panic!("chunk should be loaded"),
        }
    }

    #[test]
    fn set_and_get_block_override() {
        let mut world = World::new();
        // Default is air (0)
        assert_eq!(world.get_block(10, 64, -20), 0);
        // Set to stone (1)
        world.set_block(10, 64, -20, 1);
        assert_eq!(world.get_block(10, 64, -20), 1);
        assert_eq!(world.block_override_count(), 1);
        // Set to air (0) - still an override
        world.set_block(10, 64, -20, 0);
        assert_eq!(world.get_block(10, 64, -20), 0);
        assert_eq!(world.block_override_count(), 1);
    }
}
