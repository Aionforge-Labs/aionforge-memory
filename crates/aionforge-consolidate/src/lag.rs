//! The observable consolidation lag (write-and-consolidation §3).

use std::time::Duration;

use aionforge_domain::time::Timestamp;
use aionforge_store::LagSnapshot;

/// A point-in-time view of the consolidation backlog.
///
/// `oldest_pending_lag` is the wall-clock from the oldest unconsolidated episode's
/// `captured_at` to *now* — the "capture to derived" latency the SLA tracks. It is zero
/// when nothing is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationLag {
    /// Wall-clock lag of the oldest pending episode (zero when the backlog is empty).
    pub oldest_pending_lag: Duration,
    /// Episodes still `raw` or `in_progress`.
    pub episodes_pending: u64,
    /// Episodes marked `failed`.
    pub episodes_failed: u64,
    /// The current graph generation (the commit-stream watermark).
    pub generation: u64,
}

impl ConsolidationLag {
    /// Derive the lag from a store [`LagSnapshot`] and the current instant.
    #[must_use]
    pub fn from_snapshot(snapshot: &LagSnapshot, now: &Timestamp) -> Self {
        let oldest_pending_lag = snapshot
            .oldest_pending_captured_at
            .as_ref()
            .map(|captured| lag_between(now, captured))
            .unwrap_or_default();
        Self {
            oldest_pending_lag,
            episodes_pending: snapshot.episodes_pending,
            episodes_failed: snapshot.episodes_failed,
            generation: snapshot.generation,
        }
    }
}

/// The non-negative wall-clock duration from `captured` to `now`.
///
/// Computed over the underlying instant's whole seconds, which is robust across
/// time-zone representations and ample for a lag metric; a negative delta (a clock
/// stepping backward, or a future-stamped capture) clamps to zero.
fn lag_between(now: &Timestamp, captured: &Timestamp) -> Duration {
    let seconds = now.timestamp().as_second() - captured.timestamp().as_second();
    Duration::from_secs(seconds.max(0).unsigned_abs())
}
