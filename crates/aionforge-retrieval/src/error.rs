//! The retrieval error space.
//!
//! An unreachable embedder is *not* an error here — the dense signal degrades to an
//! empty ranking and reports the embedder unavailable, so retrieval falls back to
//! the lexical and graph signals (03 §6, §8.1). A failed search or a malformed query
//! is a hard error.

/// An error raised while producing a retrieval signal or assembling a bundle.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum RetrievalError {
    /// A store search or read failed.
    #[error("the retrieval store operation failed")]
    Store(#[from] aionforge_store::StoreError),

    /// The retrieval ran past its deadline and was abandoned (03 §8).
    #[error("retrieval exceeded its deadline")]
    DeadlineExceeded,
}
