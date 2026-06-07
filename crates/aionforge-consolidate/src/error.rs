//! The consolidation error space.

use std::time::Duration;

use aionforge_store::StoreError;

/// An error from the consolidation scheduler.
///
/// A pass's own failures (transient / fatal) are not errors here — they are reported
/// per episode and audited (see [`crate::PassError`]). This type covers what stops a
/// whole tick: a store failure, or a pass that exceeded its wall-clock budget.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConsolidationError {
    /// A store read or write the scheduler itself issued failed.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// A pass exceeded its per-episode timeout. Treated as a transient pass failure at
    /// the episode level; surfaced here only when it must abort the tick.
    #[error("a consolidation pass exceeded its {0:?} timeout")]
    Timeout(Duration),
}
