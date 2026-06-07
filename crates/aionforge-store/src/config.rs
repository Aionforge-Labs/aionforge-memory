//! Store configuration — the binding knobs L0 needs before the full config model lands.
//!
//! For now this carries only the embedding dimension, which is binding (data-model
//! §13.5): the vector indexes are created at this dimension and a later change is a
//! migration, not an in-place edit. A broader configuration model (paths under
//! `~/.aionforge/`, providers, tuning) will absorb this.

/// The default embedding dimension.
///
/// 1536 is interoperable across the embedders in play: codestral-embed's native size
/// and gemini-embedding's Matryoshka-truncated 1536. Changing it is a migration.
pub const DEFAULT_EMBEDDING_DIMENSION: u32 = 1536;

/// The store's binding configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreConfig {
    /// The embedding dimension every vector index is created at and checked against.
    pub embedding_dimension: u32,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            embedding_dimension: DEFAULT_EMBEDDING_DIMENSION,
        }
    }
}
