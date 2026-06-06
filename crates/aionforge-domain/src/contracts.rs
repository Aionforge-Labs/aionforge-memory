//! Subsystem contract traits — the type-only seams between Aionforge's subsystems.
//!
//! The domain crate is the one crate every other crate depends on, so the
//! cross-cutting subsystem contracts live here: declaring them centrally lets any
//! layer name a seam without inducing a dependency cycle. These are forward
//! declarations. Each names a subsystem's primary operation and its fallible,
//! mostly-async shape; where a request/response is not yet expressible in domain
//! terms it is an associated type the implementing milestone defines, so nothing
//! here invents a persisted surface ahead of the milestone that owns it.
//!
//! Async methods are written `-> impl Future<Output = …> + Send` rather than
//! `async fn` so the returned future's `Send` bound is explicit (required by the
//! multi-threaded Tokio runtime) and the public-`async-fn`-in-trait lint stays
//! quiet under `-D warnings`.

use std::future::Future;

use crate::embedding::{EmbedderModel, Embedding};
use crate::ids::Id;
use crate::nodes::episodic::Redaction;
use crate::nodes::procedural::Skill;

/// The fast, ADD-oriented capture path (04 §1). Implemented in M1.
pub trait Capture: Send + Sync {
    /// The raw-event capture request (content plus writer/session context).
    type Request: Send;
    /// The capture receipt (assigned ids, dedup verdict, audit reference).
    type Receipt: Send;
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Capture one event on the fast path; never blocks on consolidation (04 §1).
    fn capture(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Receipt, Self::Error>> + Send;
}

/// The composed, query-class-conditional retrieval operation (03). Implemented in M1.
pub trait Retriever: Send + Sync {
    /// The retrieval query (text, mode weights, bi-temporal selector, deadline).
    type Query: Send;
    /// The recall bundle: coordinated structured and rendered views (03 §6).
    type Bundle: Send;
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Run a retrieval, returning a deterministic recall bundle (03 §6).
    fn recall(
        &self,
        query: Self::Query,
    ) -> impl Future<Output = Result<Self::Bundle, Self::Error>> + Send;
}

/// The slow, asynchronous, durable consolidation path (04 §2). Implemented in M2.
pub trait Consolidator: Send + Sync {
    /// A summary of one pass: rules applied, cursor advance, observed lag.
    type Report: Send;
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Advance the durable cursor by one bounded, idempotent pass (04 §2–§3).
    fn advance(&self) -> impl Future<Output = Result<Self::Report, Self::Error>> + Send;
}

/// Procedural memory: skills stored as data and their reliability (05). Implemented in M3.
pub trait ProceduralMemory: Send + Sync {
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Save a new skill version (deprecate-never-delete), returning its id (05).
    fn save_skill(&self, skill: Skill) -> impl Future<Output = Result<Id, Self::Error>> + Send;

    /// Record a success/failure outcome against a skill, updating its counters (05).
    fn record_outcome(
        &self,
        skill_id: Id,
        success: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Multi-agent CRDT merge across namespaces (06). Implemented in M4.
pub trait Merge: Send + Sync {
    /// The merge request: the two namespaced states to reconcile.
    type Request: Send;
    /// The merge resolution: the reconciled state plus conflict records.
    type Resolution: Send;
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Merge two namespaced states deterministically (06).
    fn merge(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Self::Resolution, Self::Error>> + Send;
}

/// Decay, active forgetting, and the hard-erasure cascade (05). Implemented in M5.
pub trait Forgetting: Send + Sync {
    /// A summary of a hard-erasure cascade (e.g. the count of cascaded nodes/edges).
    type EraseReport: Send;
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Soft-forget a memory: set `expired_at`; reversible; audited `forget` (05).
    fn forget(&self, id: Id) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Hard-erase a memory and its derivation cascade: irreversible; audited `purge` (05).
    fn erase(&self, id: Id) -> impl Future<Output = Result<Self::EraseReport, Self::Error>> + Send;
}

/// The OpenAI-compatible embedding client (08 §1). Implemented in M0.T08.
///
/// The one contract expressible entirely in domain terms today: it consumes text
/// and produces validated [`Embedding`]s, recording the [`EmbedderModel`] identity
/// for the startup dimension-consistency check and the cross-family guard.
pub trait Embedder: Send + Sync {
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Embed a batch in input order; a wrong returned vector count is an error (08 §1).
    fn embed(
        &self,
        inputs: &[String],
    ) -> impl Future<Output = Result<Vec<Embedding>, Self::Error>> + Send;

    /// The identity of the model this embedder produces vectors with.
    fn model(&self) -> &EmbedderModel;
}

/// The outcome of the capture-path privacy/injection filter (04 §1, 02 §6.1, 07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterOutcome {
    /// The content after sensitive spans were redacted.
    pub cleaned: String,
    /// The redactions applied, recorded in `Episode.origin` (02 §6.1).
    pub redactions: Vec<Redaction>,
    /// Ids of detected prompt-injection markers, recorded in `Episode.origin`.
    pub injection_flags: Vec<String>,
}

/// The privacy and prompt-injection filter on the capture hot path (04 §1, 07). Implemented in M6.
///
/// Synchronous because v1.0.0 filtering is local (configured redaction patterns
/// plus known-marker detection), so it adds no network round-trip to capture.
pub trait PrivacyFilter: Send + Sync {
    /// The typed error this seam surfaces.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Redact sensitive spans and flag injection markers in raw capture content (04 §1).
    fn filter(&self, content: &str) -> Result<FilterOutcome, Self::Error>;
}
