//! Active-forgetting configuration (05 §3, M5.T02).
//!
//! Its own module because the soft-forget posture is a coherent unit: the master
//! off-switch, the floors a sweep candidate is measured against, and the conservative
//! guards that keep a misconfiguration from sweeping the record. The tier half-lives that
//! feed the decayed-importance axis are deliberately **not** re-declared here — they come
//! from the existing `decay` section, so rank-time decay and sweep-time decay can never
//! disagree about how fast a memory ages. The host passes both into the engine's
//! forgetting policy regardless of whether rank-time decay is enabled: the half-lives are
//! always defined, `decay.enabled` only gates their application at rank time.

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Active-forgetting posture (05 §3, M5.T02): whether the soft-forget sweep runs at all,
/// and the floors a candidate must sit below on *every* axis before it is forgettable.
///
/// Off by default. When disabled the engine builds no forgetter and every forget surface
/// is inert — the same all-defaults-inert posture as the promotion, reliability, and decay
/// sections. The host maps these knobs into the engine's forgetting policy (which
/// re-validates its own copy), the same host-side indirection as
/// [`ReliabilityConfig`](crate::ReliabilityConfig) into the reliability policy, so the
/// engine takes no config dependency.
///
/// The defaults are deliberately conservative: a memory written with the default capture
/// importance (0.5) and trust (0.5) sits far above both floors and is never a candidate
/// until it has genuinely faded — forgetting is for the long tail, not the working set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgettingConfig {
    /// Master off-switch. When false the engine constructs no forgetter, the sweep is a
    /// no-op that reads nothing, and the point forget/unforget surfaces are inert.
    pub enabled: bool,
    /// A candidate is low-importance only when its *decayed* importance sits below this.
    /// Validated finite and in `[0.0, 1.0]` when enabled.
    pub importance_floor: f64,
    /// A candidate is low-trust only when its per-memory trust scalar sits below this.
    /// Validated finite and in `[0.0, 1.0]` when enabled.
    pub trust_floor: f64,
    /// A candidate must be at least this old (seconds since ingestion) before it is
    /// forgettable, so a fresh low-value write is spared while it proves itself. Zero is
    /// permitted; the unsigned type keeps a negative age floor unrepresentable.
    pub min_age_secs: u64,
    /// Per-page candidate cap for the sweep, clamped downstream like every audit page.
    /// Validated non-zero when enabled.
    pub batch_cap: usize,
    /// Whether `BadPattern` records are sweep-eligible. Off by default: negative knowledge
    /// ("what not to do") is protected, because forgetting a failure invites repeating it.
    pub forget_bad_patterns: bool,
}

impl Default for ForgettingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            importance_floor: 0.05,
            trust_floor: 0.30,
            min_age_secs: 2_592_000,
            batch_cap: 200,
            forget_bad_patterns: false,
        }
    }
}

impl ForgettingConfig {
    /// Check the section's binding invariants. A disabled section validates vacuously —
    /// the values are inert, so nothing rejects a config that never runs.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        for (key, floor) in [
            ("forgetting.importance_floor", self.importance_floor),
            ("forgetting.trust_floor", self.trust_floor),
        ] {
            // The `!(…)` form also rejects a NaN floor, which fails every ordered
            // comparison.
            if !(floor.is_finite() && (0.0..=1.0).contains(&floor)) {
                return Err(ConfigError::invalid(
                    key,
                    "must be a finite value in the range [0.0, 1.0]",
                ));
            }
        }
        if self.batch_cap == 0 {
            return Err(ConfigError::invalid(
                "forgetting.batch_cap",
                "must be greater than zero when forgetting is enabled",
            ));
        }
        // The cross-field sanity ceiling: with both floors at the top of their range,
        // nearly every unpinned, unreferenced memory past the minimum age becomes a sweep
        // candidate. That is mass deletion misspelled as configuration, so it is rejected
        // here where the misconfiguration is visible.
        if self.importance_floor >= 1.0 && self.trust_floor >= 1.0 {
            return Err(ConfigError::invalid(
                "forgetting.trust_floor",
                "must not sit at 1.0 together with forgetting.importance_floor at 1.0 \
                 (both floors at the ceiling would make nearly every unpinned memory a \
                 sweep candidate)",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_off_and_valid() {
        let config = ForgettingConfig::default();
        assert!(!config.enabled, "forgetting must be opt-in");
        assert!(!config.forget_bad_patterns, "negative knowledge protected");
        config.validate().expect("defaults validate");
        // The default floors are also valid if a deployment flips the switch and
        // nothing else.
        let enabled = ForgettingConfig {
            enabled: true,
            ..ForgettingConfig::default()
        };
        enabled.validate().expect("enabled defaults validate");
    }

    #[test]
    fn a_disabled_section_validates_vacuously() {
        let config = ForgettingConfig {
            enabled: false,
            importance_floor: f64::NAN,
            trust_floor: -7.0,
            batch_cap: 0,
            ..ForgettingConfig::default()
        };
        config.validate().expect("inert values are never rejected");
    }

    #[test]
    fn out_of_range_floors_are_rejected_when_enabled() {
        for bad in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            for field in ["importance", "trust"] {
                let config = ForgettingConfig {
                    enabled: true,
                    importance_floor: if field == "importance" { bad } else { 0.05 },
                    trust_floor: if field == "trust" { bad } else { 0.30 },
                    ..ForgettingConfig::default()
                };
                let error = config
                    .validate()
                    .expect_err("an out-of-range floor must be rejected");
                assert!(
                    error.to_string().contains(field),
                    "{field} floor {bad}: error must name the key, got: {error}"
                );
            }
        }
    }

    #[test]
    fn a_zero_batch_cap_is_rejected_when_enabled() {
        let config = ForgettingConfig {
            enabled: true,
            batch_cap: 0,
            ..ForgettingConfig::default()
        };
        let error = config.validate().expect_err("a zero page is rejected");
        assert!(error.to_string().contains("batch_cap"), "{error}");
    }

    #[test]
    fn both_floors_at_the_ceiling_are_rejected_together() {
        let config = ForgettingConfig {
            enabled: true,
            importance_floor: 1.0,
            trust_floor: 1.0,
            ..ForgettingConfig::default()
        };
        config
            .validate()
            .expect_err("the sweep-everything misconfig must be rejected");
        // Either floor alone at the ceiling stays legal: the other axis still guards.
        for (importance_floor, trust_floor) in [(1.0, 0.30), (0.05, 1.0)] {
            let config = ForgettingConfig {
                enabled: true,
                importance_floor,
                trust_floor,
                ..ForgettingConfig::default()
            };
            config.validate().expect("one ceiling floor is legal");
        }
    }
}
