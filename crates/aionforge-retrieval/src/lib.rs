//! Hybrid retrieval: lexical/dense/graph/recency/trust signals, RRF fusion, query-class router, and the recall bundle.
//!
//! This milestone implements the lexical and dense signals ([`lexical_ranking`] and
//! [`dense_ranking`], M1.T03–T04) — each turns a query into a best-first ranked
//! candidate list — the deterministic Reciprocal Rank Fusion that merges them
//! ([`fuse`], M1.T05), and the mandatory query-class router ([`route`], M1.T06) that
//! picks the mode weights (03 §1–§3). The graph, recency, and trust signals and the
//! recall bundle land with their tasks.

mod error;
mod fusion;
mod router;
mod signals;

pub use error::RetrievalError;
pub use fusion::{Contribution, DEFAULT_RRF_K, FusedCandidate, WeightedRanking, fuse};
pub use router::{QueryClass, RetrievalProfile, SignalWeights, classify, profile_for, route};
pub use signals::{
    DenseRanking, RankedCandidate, Signal, SignalRanking, dense_ranking, lexical_ranking,
};
