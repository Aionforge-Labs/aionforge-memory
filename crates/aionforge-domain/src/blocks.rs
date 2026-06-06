//! The reusable identity and stats property blocks (02 §3).
//!
//! Every kind composes the [`Identity`] block. Retrievable memory kinds
//! additionally compose the [`Stats`] block; forensic and control kinds omit it.

use serde::{Deserialize, Serialize};

use crate::ids::Id;
use crate::namespace::Namespace;
use crate::time::Timestamp;

/// The identity block carried by every kind (02 §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// Stable, unique, immutable external id.
    pub id: Id,
    /// Transaction-time creation instant (immutable).
    pub ingested_at: Timestamp,
    /// Trust/visibility namespace.
    pub namespace: Namespace,
    /// Soft-expiry instant; `None` while active/trusted. Set by active forgetting.
    pub expired_at: Option<Timestamp>,
}

/// The stats block carried by retrievable memory kinds (02 §3).
///
/// Drives the importance/recency/relevance ranking shape and decay. Trust and the
/// `[0, 1]` scores are validated by the constructing layer, not by the type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Importance score; decays with elapsed time since `last_access`.
    pub importance: f64,
    /// Writer/derivation trust in `[0, 1]`; sinks low-trust memories in retrieval.
    pub trust: f64,
    /// Last access instant.
    pub last_access: Timestamp,
    /// Recent access count.
    pub access_count_recent: u64,
    /// Number of times referenced by derived memories.
    pub referenced_count: u64,
    /// Surprise/novelty score.
    pub surprise: f64,
    /// Pinned memories never decay out of retrieval eligibility.
    pub is_pinned: bool,
}
