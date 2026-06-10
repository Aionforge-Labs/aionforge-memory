//! Core-block edit-strictness configuration (05 §4, M5.T04).
//!
//! Its own module because the identity-tier edit posture is a coherent unit: the
//! baseline second-attester rule, the per-sensitivity overrides, the redline human
//! flag, and the deployment's certified-human allowlist. Unlike its siblings there is
//! **no master switch** — the engine's core-block edit gate is always on, because a
//! "disabled" gate would mean unattested writes to identity, exactly the threat the
//! gate exists for (05 §4). Only the *strictness* is configurable, and the all-default
//! posture (one non-editor attester, no human requirement) is the spec's floor, not an
//! off state.
//!
//! The host maps these knobs field-for-field into the engine's `CoreEditPolicy` (which
//! re-validates its own copy at construction), the same host-side indirection as
//! [`ForgettingConfig`](crate::ForgettingConfig) into the forgetting policy, so the
//! engine takes no config dependency.

use std::collections::{BTreeMap, BTreeSet};

use aionforge_domain::ids::Id;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Core-block edit posture (05 §4): how many distinct non-editor attesters an
/// identity-tier edit needs, and when one of them must be a certified human.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreBlockConfig {
    /// The baseline requirement for every block.
    pub default_rule: CoreEditRuleConfig,
    /// Whether a `redline` block additionally requires a human attester — the spec's
    /// named sensitive class, composed strictest-per-axis with the sensitivity rules.
    pub redline_requires_human: bool,
    /// Per-sensitivity overrides, keyed by the block's `sensitivity` string. A
    /// `BTreeMap` keeps the rendered key order canonical, like the promotion
    /// categories.
    pub rules: BTreeMap<String, CoreEditRuleConfig>,
    /// The agent ids this deployment certifies as human-controlled keys. A host
    /// policy assertion (06 §1) — never a property an agent can self-declare, which is
    /// why it lives here and not on the `Agent` row.
    pub human_attester_ids: BTreeSet<Id>,
}

/// One edit requirement (05 §4): the distinct-attester count and the human flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreEditRuleConfig {
    /// Distinct verified attesters required, the editor never counted. At least 1 —
    /// "an edit requires a second attester" is the floor, not a knob.
    pub k: u64,
    /// Whether at least one verified attester must be on the certified-human list.
    pub require_human: bool,
}

impl Default for CoreEditRuleConfig {
    fn default() -> Self {
        Self {
            k: 1,
            require_human: false,
        }
    }
}

impl CoreBlockConfig {
    /// Validate the posture, fail-closed: a zero `k` anywhere would re-enable
    /// single-writer identity edits, and a human requirement with an empty human list
    /// is an unsatisfiable gate that bricks every sensitive edit.
    ///
    /// # Errors
    /// Returns [`ConfigError`] naming the offending key.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.default_rule.k == 0 {
            return Err(ConfigError::invalid(
                "core_block.default_rule.k",
                "must be at least 1 (a quorum of none re-enables single-writer edits)",
            ));
        }
        for (sensitivity, rule) in &self.rules {
            if rule.k == 0 {
                return Err(ConfigError::invalid(
                    format!("core_block.rules.{sensitivity}.k"),
                    "must be at least 1 (a quorum of none re-enables single-writer edits)",
                ));
            }
        }
        let any_human = self.redline_requires_human
            || self.default_rule.require_human
            || self.rules.values().any(|rule| rule.require_human);
        if any_human && self.human_attester_ids.is_empty() {
            return Err(ConfigError::invalid(
                "core_block.human_attester_ids",
                "must name at least one certified human attester when any rule requires one, \
                 or every sensitive edit is unsatisfiable",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_posture_is_the_spec_floor_and_validates() {
        let config = CoreBlockConfig::default();
        assert_eq!(config.default_rule.k, 1, "a second attester is the floor");
        assert!(!config.default_rule.require_human);
        assert!(!config.redline_requires_human);
        assert!(config.rules.is_empty());
        assert!(config.human_attester_ids.is_empty());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn a_zero_k_is_rejected_wherever_it_appears() {
        let mut config = CoreBlockConfig {
            default_rule: CoreEditRuleConfig {
                k: 0,
                require_human: false,
            },
            ..CoreBlockConfig::default()
        };
        assert!(config.validate().is_err(), "default rule");

        config.default_rule.k = 1;
        config.rules.insert(
            "pii".to_string(),
            CoreEditRuleConfig {
                k: 0,
                require_human: false,
            },
        );
        let err = config.validate().expect_err("sensitivity rule");
        assert!(
            err.to_string().contains("core_block.rules.pii.k"),
            "the error names the offending rule: {err}"
        );
    }

    #[test]
    fn a_human_requirement_needs_a_non_empty_allowlist() {
        for config in [
            CoreBlockConfig {
                redline_requires_human: true,
                ..CoreBlockConfig::default()
            },
            CoreBlockConfig {
                default_rule: CoreEditRuleConfig {
                    k: 1,
                    require_human: true,
                },
                ..CoreBlockConfig::default()
            },
        ] {
            assert!(
                config.validate().is_err(),
                "an unsatisfiable human gate is a configuration error"
            );
        }

        let mut sound = CoreBlockConfig {
            redline_requires_human: true,
            ..CoreBlockConfig::default()
        };
        sound
            .human_attester_ids
            .insert(Id::from_content_hash(b"reviewer"));
        assert!(sound.validate().is_ok());
    }

    #[test]
    fn the_posture_round_trips_with_ids_as_uuid_strings() {
        let reviewer = Id::from_content_hash(b"reviewer");
        let mut config = CoreBlockConfig {
            redline_requires_human: true,
            ..CoreBlockConfig::default()
        };
        config.rules.insert(
            "pii".to_string(),
            CoreEditRuleConfig {
                k: 2,
                require_human: true,
            },
        );
        config.human_attester_ids.insert(reviewer);

        // The file/env layers ride these same serde impls (figment), so the round
        // trip pins the wire shape: ids render as their uuid strings.
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(
            json.contains(&reviewer.to_string()),
            "ids render as their uuid strings: {json}"
        );
        let back: CoreBlockConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, config);
    }
}
