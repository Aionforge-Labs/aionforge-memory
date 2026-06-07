//! The recall query and its options (03 §6, §8).

use std::time::Duration;

use aionforge_domain::namespace::Namespace;

use crate::router::QueryClass;

/// A retrieval request: the query text, the namespace asking, and the bundle shape.
///
/// `viewer` is the namespace authorization is applied against — private content from
/// another agent never surfaces to it (03 §8, 06 §1). The text is always bound as a
/// GQL parameter downstream, never interpolated, so hostile query text cannot alter a
/// statement.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallQuery {
    /// The natural-language query.
    pub text: String,
    /// The namespace the recall is performed for; gates what may surface.
    pub viewer: Namespace,
    /// The target number of memories in the bundle.
    pub limit: usize,
    /// Tuning knobs; [`RecallOptions::default`] is the usual choice.
    pub options: RecallOptions,
}

impl RecallQuery {
    /// A query for `text` on behalf of `viewer`, returning up to `limit` memories with
    /// default options.
    #[must_use]
    pub fn new(text: impl Into<String>, viewer: Namespace, limit: usize) -> Self {
        Self {
            text: text.into(),
            viewer,
            limit,
            options: RecallOptions::default(),
        }
    }
}

/// Optional retrieval tuning (03 §3, §5, §6, §8).
#[derive(Debug, Clone, PartialEq)]
pub struct RecallOptions {
    /// Force a query class instead of letting the router classify (mostly for tests
    /// and callers that already know the intent).
    pub mode_override: Option<QueryClass>,
    /// The most memories from a single session allowed to fill the bundle before the
    /// rest spill; spilled memories are appended only if the bundle is under-filled
    /// (03 §6). Zero means no cap.
    pub session_diversity_cap: usize,
    /// A wall-clock budget for the whole recall; exceeding it surfaces as
    /// [`RetrievalError::DeadlineExceeded`](crate::RetrievalError::DeadlineExceeded)
    /// (03 §8). `None` means no deadline.
    pub deadline: Option<Duration>,
    /// Include soft-forgotten (expired) memories — a history query. The default
    /// current retrieval excludes them (03 §5).
    pub include_expired: bool,
    /// How many candidates to pull from each signal before fusion. Zero falls back to
    /// the retriever's configured default.
    pub fanout: usize,
}

impl Default for RecallOptions {
    fn default() -> Self {
        Self {
            mode_override: None,
            session_diversity_cap: 3,
            deadline: None,
            include_expired: false,
            fanout: 0,
        }
    }
}
