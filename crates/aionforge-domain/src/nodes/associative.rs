//! The associative memory tier: the link-evolution `Note` unit (02 §4.6).

use serde::{Deserialize, Serialize};

use crate::blocks::{Identity, Stats};
use crate::embedding::{EmbedderModel, Embedding};
use crate::ids::Id;

/// A free-form associative note: the link-evolution unit (02 §4.6).
///
/// Notes are the substrate's optional associative tier. Unlike facts (which assert
/// canonical, subject-anchored claims), a note is loosely structured text that
/// accretes links to other notes via `RELATES_TO`, letting related ideas evolve a
/// scoped graph of associations independent of the canonical-state machinery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Shared identity block.
    pub identity: Identity,
    /// Shared stats block.
    pub stats: Stats,
    /// The note body.
    pub content: String,
    /// Optional surrounding context that situates the note.
    pub context: Option<String>,
    /// Free-text keywords for lexical recall and clustering.
    pub keywords: Vec<String>,
    /// Content embedding, if computed.
    pub embedding: Option<Embedding>,
    /// Identity of the model that produced the embedding.
    pub embedder_model: Option<EmbedderModel>,
    /// The episode this note was derived from, if any.
    pub derived_from_episode: Option<Id>,
}

impl Note {
    /// The selene-db node label for this kind.
    pub const LABEL: &str = "Note";
}
