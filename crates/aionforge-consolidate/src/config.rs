//! Scheduler tuning (write-and-consolidation §3).

use std::time::Duration;

/// How the background consolidator paces and bounds itself.
///
/// Every field is a bound: how often to look for work, how much to take at once, how
/// long a single pass may run, how many times to retry a transient failure before
/// giving up on an episode, and the lag above which the scheduler warns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationConfig {
    /// How often the spawned loop wakes to drain work.
    pub tick_interval: Duration,
    /// The most episodes a single tick will take (the per-tick concurrency bound).
    pub batch_size: usize,
    /// The wall-clock budget for one pass over one episode.
    pub apply_timeout: Duration,
    /// How many transient failures an episode may accrue before it is marked failed.
    pub max_retries: u32,
    /// The steady-state lag ceiling; the scheduler warns when the oldest pending
    /// episode is older than this.
    pub lag_ceiling: Duration,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(5),
            batch_size: 32,
            apply_timeout: Duration::from_secs(30),
            max_retries: 5,
            lag_ceiling: Duration::from_secs(5),
        }
    }
}

/// How the fact-extraction pass resolves surface forms to canonical entities
/// (write-and-consolidation §2). Pass-level tuning, kept separate from the scheduler's
/// [`ConsolidationConfig`]: it carries a float threshold (so it derives `PartialEq`, not
/// `Eq`), and only the extraction pass reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolutionConfig {
    /// How many candidate entities each lexical/vector probe pulls before filtering.
    pub candidate_k: usize,
    /// The cosine-distance ceiling under which an embedding neighbor is judged the same
    /// entity (lower is nearer). Above it, the surface forms a new entity (conservative).
    pub merge_threshold: f64,
}

impl Default for ResolutionConfig {
    fn default() -> Self {
        Self {
            candidate_k: 8,
            merge_threshold: 0.12,
        }
    }
}
