//! The canonical timestamp type and the bi-temporal four-timestamp block.

use serde::{Deserialize, Serialize};

/// The canonical timestamp type.
///
/// Maps to selene-db's `ZONED DATETIME`: nanosecond resolution with a real IANA
/// time zone, carried by [`jiff::Zoned`]. The storage layer translates to and from
/// the engine's value at the boundary.
pub type Timestamp = jiff::Zoned;

/// The four-timestamp validity block carried by every bi-temporal edge (02 §5).
///
/// Event time (`valid_from`/`valid_to`) records when the underlying fact was true
/// in the world; transaction time (`ingested_at`/`expired_at`) records when the
/// substrate believed it. An open (`None`) upper bound means "still in effect".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BiTemporal {
    /// Event-time lower bound: when the fact became true.
    pub valid_from: Timestamp,
    /// Event-time upper bound: when it stopped being true; `None` while current.
    pub valid_to: Option<Timestamp>,
    /// Transaction-time lower bound: when the substrate recorded it (immutable).
    pub ingested_at: Timestamp,
    /// Transaction-time upper bound: when the record was expired; `None` while live.
    pub expired_at: Option<Timestamp>,
}

impl BiTemporal {
    /// True when the record is currently live in transaction time (`expired_at` is open).
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.expired_at.is_none()
    }

    /// True when the fact is currently valid in event time (`valid_to` is open).
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.valid_to.is_none()
    }
}
