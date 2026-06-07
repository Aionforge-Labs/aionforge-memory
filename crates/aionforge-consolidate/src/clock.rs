//! The injected clock for the consolidator's own bookkeeping timestamps.
//!
//! The consolidator stamps its action time onto the cursor (`last_processed_at`) and
//! its audit events. Those are the substrate's own "when did I run" times — legitimately
//! *now* — but they are injected through this seam rather than read from an ambient
//! clock so tests are deterministic and the stored time is never a guess.

use aionforge_domain::time::Timestamp;

/// A source of the current time for the consolidator's bookkeeping.
pub trait Clock: Send + Sync + 'static {
    /// The current instant.
    fn now(&self) -> Timestamp;
}

/// The production clock: the system zoned time.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}
