//! The identity tier: stable identity, commitments, and redlines (02 §4.7).

use serde::{Deserialize, Serialize};

use crate::blocks::{Identity, Stats};
use crate::embedding::{EmbedderModel, Embedding};

/// The category of a [`CoreBlock`] (`CoreBlock.block_kind`, 02 §4.7).
///
/// Serialized as the spec's lowercase string label (`persona` / `commitment` /
/// `redline`); the storage layer indexes this field for fast block-kind lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// Stable self-description / identity narrative.
    Persona,
    /// A standing commitment the agent has made.
    Commitment,
    /// An inviolable constraint; crossing it is never permitted.
    Redline,
}

/// An identity-tier core block: persona, commitment, or redline (02 §4.7).
///
/// Core blocks anchor the agent's identity and are the most strongly protected
/// memories: edits are audited (`core_edit`) and high-`sensitivity` blocks require
/// attestation (`ATTESTED_BY`). `drift_baseline` records the embedding/summary
/// baseline that drift is measured against, so divergence from the canonical
/// identity can be detected over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreBlock {
    /// Shared identity block.
    pub identity: Identity,
    /// Shared stats block.
    pub stats: Stats,
    /// The block body.
    pub content: String,
    /// The category of this block (indexed).
    pub block_kind: BlockKind,
    /// Sensitivity classification; drives the attestation requirement. `None`
    /// leaves the requirement to policy default.
    pub sensitivity: Option<String>,
    /// The embedding/summary baseline that drift is measured against (02 §4.7).
    ///
    /// Intentionally open JSON: the baseline shape (embedding snapshot, summary
    /// text, computed thresholds) is owned and evolved by the drift-detection
    /// layer, not pinned by the domain type. `None` until the drift-detection layer
    /// has computed a baseline (02 §4.7 lists this `JSON` field without `NOT NULL`).
    pub drift_baseline: Option<serde_json::Value>,
    /// Content embedding, if computed (`embedding_v1`).
    pub embedding: Option<Embedding>,
    /// Identity of the model that produced the embedding.
    ///
    /// Present per the 02 §13.5 dimension-consistency invariant even though the
    /// §4.7 prose lists only the embedding itself.
    pub embedder_model: Option<EmbedderModel>,
}

impl CoreBlock {
    /// The selene-db node label for this kind.
    pub const LABEL: &str = "CoreBlock";
}
