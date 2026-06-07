//! Capture-path tuning knobs (04 §1).

/// Tuning for the capture path.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureConfig {
    /// Whether to embed content on the capture path. When `false`, episodes are
    /// written without a vector and embedded lazily during consolidation (04 §1).
    pub embed_on_capture: bool,
    /// The cosine-*similarity* threshold above which a new episode counts as a
    /// near-duplicate of an existing one (04 §1 step 2). In `[0, 1]`; a value of
    /// `0.95` flags anything within cosine distance `0.05`. Near-duplicate episodes
    /// are still written — episodes are immutable and append-only — but flagged on
    /// the receipt so consolidation can cluster or summarize them.
    pub near_duplicate_threshold: f64,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            embed_on_capture: true,
            near_duplicate_threshold: 0.95,
        }
    }
}

impl CaptureConfig {
    /// Check the tuning knobs are in range.
    ///
    /// # Errors
    /// Returns a message naming the offending knob when `near_duplicate_threshold` is not a
    /// finite value in `[0, 1]` (it is a cosine similarity, so anything outside that range —
    /// or `NaN` — would silently disable or mis-fire near-duplicate flagging).
    pub fn validate(&self) -> Result<(), String> {
        if !self.near_duplicate_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.near_duplicate_threshold)
        {
            return Err(format!(
                "capture.near_duplicate_threshold must be a finite value in [0, 1], got {}",
                self.near_duplicate_threshold
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureConfig;

    #[test]
    fn the_default_config_validates() {
        CaptureConfig::default()
            .validate()
            .expect("default is in range");
    }

    #[test]
    fn an_out_of_range_or_nan_threshold_is_rejected() {
        for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            let config = CaptureConfig {
                near_duplicate_threshold: bad,
                ..CaptureConfig::default()
            };
            assert!(
                config.validate().is_err(),
                "threshold {bad} should be rejected"
            );
        }
    }
}
