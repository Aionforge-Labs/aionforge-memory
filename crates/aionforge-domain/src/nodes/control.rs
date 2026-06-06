//! Control singletons: the consolidation cursor and schema-version markers (02 §4.13).
//!
//! These are control-plane nodes, not retrievable memories, so per spec §3 they carry
//! only the reduced identity block ([`Identity`]) and omit [`crate::blocks::Stats`].
//! Each kind is a singleton: the substrate maintains exactly one live instance.

use serde::{Deserialize, Serialize};

use crate::blocks::Identity;
use crate::ids::Id;
use crate::time::Timestamp;

/// The consolidation cursor: a single node tracking how far the consolidator has
/// processed the episodic stream (02 §4.13).
///
/// One live instance exists. The consolidator advances `last_position` as it drains
/// committed episodes, so a crash resumes from the last durably recorded point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationCursor {
    /// Reduced identity block (no stats — this is a control node, not a memory).
    pub identity: Identity,
    /// Opaque commit/WAL position the consolidator has processed up to.
    ///
    /// Treated as a non-empty cursor token; its internal structure belongs to the
    /// storage layer and is never interpreted by the domain.
    pub last_position: String,
    /// The id of the last episode consolidated, if any has been processed.
    pub last_episode_id: Option<Id>,
    /// When the cursor last advanced, if it ever has.
    pub last_processed_at: Option<Timestamp>,
    /// Versions of the consolidation rules in force at `last_position` (02 §4.13).
    ///
    /// Intentionally open-shaped JSON: the rule-version map evolves with the
    /// consolidation pipeline and is not pinned to a fixed schema here.
    pub rule_versions: serde_json::Value,
}

impl ConsolidationCursor {
    /// The selene-db node label for this kind.
    pub const LABEL: &str = "ConsolidationCursor";
}

/// The schema-version singleton tracking the applied migration level (02 §4.13).
///
/// One live instance exists. The forward-only, idempotent migration runner reads
/// `current_version` to decide which pending migrations to apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaVersion {
    /// Reduced identity block (no stats — this is a control node, not a memory).
    pub identity: Identity,
    /// The currently applied schema version number.
    pub current_version: i64,
    /// When the current version was applied.
    pub applied_at: Timestamp,
}

impl SchemaVersion {
    /// The selene-db node label for this kind.
    pub const LABEL: &str = "SchemaVersion";
}
