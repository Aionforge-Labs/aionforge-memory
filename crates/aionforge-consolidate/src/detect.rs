//! Supersession / contradiction detection (write-and-consolidation §2, M2.T05b).
//!
//! A pure function over the committed current facts and the episode's newly extracted
//! facts: it decides, conservatively and deterministically, which new facts supersede a
//! prior one (functional predicate, newer different object) and which contradict one
//! (mutually-exclusive object), and whether a contradiction should quarantine the new
//! fact (a high-trust incumbent). It produces only INSTRUCTIONS — the store materializes
//! them in the flip transaction. Being store-free, every branch is unit-testable.
//!
//! **Convergence (06 §2).** A functional `(subject, predicate)` holds exactly one current
//! object, and which object wins is the **K1 order**: the assertion with the greater
//! event time (`valid_from`) wins, and a simultaneous tie — equal `valid_from`, which for a
//! functional predicate always means two distinct objects — is settled by
//! [`object_order_key`], the canonical object order. Both components are a pure function of
//! the assertion itself (never the substrate's arrival clock, and deliberately **not** the
//! content-hash `Fact.id` or the originating agent, both of which are fixed by whichever
//! episode wins the dedup race and so are arrival-fragile), so the winner is identical under
//! any consolidation order. The comparison is symmetric: the loser is superseded by the
//! winner whichever side it is on, so a stale assertion arriving after a newer incumbent is
//! retired into history rather than lingering as a second current value.

use std::collections::{BTreeMap, BTreeSet};

use aionforge_domain::ids::Id;
use aionforge_domain::namespace::Namespace;
use aionforge_domain::nodes::forensic::AuditEvent;
use aionforge_domain::nodes::semantic::Fact;
use aionforge_domain::time::Timestamp;
use aionforge_domain::value::ObjectValue;
use aionforge_store::{Contradiction, FactKey, MaterializedFact, Supersession};

use crate::config::DetectionConfig;
use crate::merge::{new_is_contradiction_victim, new_wins_functional_slot, object_order_key};

/// A committed current fact (no live supersession/contradiction), projected for detection.
pub(crate) struct CurrentFact {
    /// The fact's node id — needed to name it as the quarantine victim when a contradiction
    /// resolves against it (the victim can be the incumbent, not only the new fact).
    pub id: Id,
    /// The fact's identifying triple.
    pub key: FactKey,
    /// The fact's event-time `valid_from` (its `ABOUT` window open instant).
    pub valid_from: Timestamp,
    /// The fact's derivation/writer trust (drives the quarantine decision).
    pub trust: f64,
    /// Whether the episode's writer-asserted supersedes hint names this fact's supporting
    /// episode (04 §1 step 3). A hinted incumbent is supersession-eligible even when its
    /// predicate is not in the functional registry — but only with at-least-equal trust:
    /// the registry is owner-declared schema knowledge, the hint is just a writer claim,
    /// so a lower-trust hint routes to the contradiction path (and its quarantine
    /// asymmetry) instead of silently retiring a higher-trust fact.
    pub hint_eligible: bool,
}

/// The instructions detection produced for one episode.
#[derive(Default)]
pub(crate) struct DetectionOutput {
    /// A newer fact supersedes a prior current one (functional predicate).
    pub supersessions: Vec<Supersession>,
    /// A new fact contradicts a current one (with optional quarantine of the new fact).
    pub contradictions: Vec<Contradiction>,
    /// Quarantine reconcile-signal audit events.
    pub audits: Vec<AuditEvent>,
}

/// Detect supersession/contradiction of `new_facts` against the committed `current` set.
///
/// `captured_at` is the new facts' event time (the episode's), `now` the transaction
/// time, `actor_id` the consolidator's audit actor. Pure: no store access.
pub(crate) fn detect(
    current: &[CurrentFact],
    new_facts: &[MaterializedFact],
    cfg: &DetectionConfig,
    namespace: &Namespace,
    captured_at: &Timestamp,
    now: &Timestamp,
    actor_id: &Id,
) -> DetectionOutput {
    let mut out = DetectionOutput::default();
    if !cfg.enabled {
        return out;
    }

    // For each functional (subject, predicate) the episode touches, pick the one new fact
    // that wins — the object that sorts first under `object_order_key` (see
    // `detect_intra_episode_ties`, which uses the same rule to retire the losers). An
    // incumbent is then superseded only by that winner, never by a losing peer. This matters
    // when an episode asserts two new values for one functional predicate (e.g. both SF and
    // Boston for `based_in`) against an NYC incumbent: routing every retirement to the single
    // survivor means NYC and the loser each get exactly one `SUPERSEDED_BY` edge to the
    // winner, so correctness does not rest on the store absorbing redundant,
    // differently-targeted supersessions of one incumbent. Hinted (subject, predicate)
    // pairs join the survivor map for the same single-edge-target reason — a hinted
    // multi-valued pair retires its incumbents to ONE new fact, while the new sibling
    // facts all stay current (multi-valued stays multi-valued; only `detect_intra_episode_ties`
    // retires peers, and it remains functional-only).
    let hinted_pairs: BTreeSet<(String, String)> = current
        .iter()
        .filter(|c| c.hint_eligible)
        .map(|c| (c.key.subject_id.to_string(), c.key.predicate.clone()))
        .collect();
    let survivors = survivors_for(new_facts, cfg, &hinted_pairs);

    // New facts vs the committed current set, scoped to the same (subject, predicate).
    for materialized in new_facts {
        let new_key = fact_key(&materialized.fact);
        let rule = cfg.rule(&new_key.predicate);
        let is_survivor = survivors
            .get(&(new_key.subject_id.to_string(), new_key.predicate.clone()))
            .is_none_or(|winner| *winner == materialized.fact.identity.id.to_string());
        for incumbent in current.iter().filter(|c| {
            c.key.subject_id == new_key.subject_id && c.key.predicate == new_key.predicate
        }) {
            if incumbent.key.object == new_key.object {
                continue; // the same triple — T04a dedup handles it, not a conflict
            }
            // A writer-hinted incumbent supersedes like a functional slot, but only with
            // at-least-equal trust (see `CurrentFact::hint_eligible`): the hint widens
            // WHICH pairs are compared, never who wins a trust fight. A lower-trust hint
            // against this incumbent falls through to the contradiction arm below, whose
            // victim/quarantine rules already protect the higher-trust side.
            let hint_supersedes =
                incumbent.hint_eligible && materialized.fact.stats.trust >= incumbent.trust;
            if (rule.functional || hint_supersedes) && is_survivor {
                // The single functional slot is settled by the K1 order (see the module
                // doc): the winner is a pure function of the two assertions, so the same
                // object ends up current under any consolidation order. The loser is
                // superseded by the winner — retained in history, never dropped.
                if new_wins_functional_slot(
                    &new_key.object,
                    captured_at,
                    &incumbent.key.object,
                    &incumbent.valid_from,
                ) {
                    // The new assertion wins (strictly later, or the tie-winning object):
                    // it retires the incumbent. Its window closes at the new event time,
                    // which is >= the incumbent's here, so the closed window stays ordered.
                    let reason = if rule.functional {
                        "functional predicate superseded by a newer assertion"
                    } else {
                        "writer-hinted supersession of a replaced episode's fact"
                    };
                    out.supersessions.push(Supersession {
                        old_fact: incumbent.key.clone(),
                        new_fact: new_key.clone(),
                        reason: reason.to_string(),
                        valid_from: captured_at.clone(),
                    });
                } else {
                    // The incumbent wins: a stale assertion (older event time) arriving
                    // after a newer incumbent, or the losing side of a simultaneous tie.
                    // The new fact is born superseded — closing it at the incumbent's
                    // `valid_from` keeps its window [new.valid_from, incumbent.valid_from)
                    // ordered (the new event time is <= the incumbent's on this branch).
                    // The old forward-only guard never produced this direction, which is
                    // what let a stale fact linger as a second current value — a divergence.
                    out.supersessions.push(Supersession {
                        old_fact: new_key.clone(),
                        new_fact: incumbent.key.clone(),
                        reason: "stale assertion superseded by a newer incumbent".to_string(),
                        valid_from: incumbent.valid_from.clone(),
                    });
                }
            } else if mutually_exclusive(&rule, &incumbent.key.object, &new_key.object)
                || (incumbent.hint_eligible && !hint_supersedes && !rule.functional)
            {
                // The contradiction's victim — the `CONTRADICTS` source, which the
                // `current_support_facts` provider excludes from recall by edge presence
                // (store providers.rs, `exclude_outgoing(CONTRADICTS)`), regardless of the
                // quarantine status — is the LOWER-TRUST side, ties settled by the smaller
                // object order. A pure function of the unordered pair {(trust, object)}, never
                // of which side is incumbent vs new, so the same contradiction excludes the
                // same value under any consolidation order (06 §2). The survivor stays current;
                // the victim is retained (node, edge, and — when quarantined — an audit signal).
                let new_trust = materialized.fact.stats.trust;
                let new_is_victim = new_is_contradiction_victim(
                    &new_key.object,
                    new_trust,
                    &incumbent.key.object,
                    incumbent.trust,
                );
                // Quarantine — actively flag the victim for review — only when the pair carries
                // real weight: either side at or above the high-trust bar. Symmetric in the
                // pair, not keyed on whichever side happened to be the incumbent.
                let quarantine = new_trust.max(incumbent.trust) >= cfg.high_trust_threshold;
                let (source_fact, target_fact) = if new_is_victim {
                    (new_key.clone(), incumbent.key.clone())
                } else {
                    (incumbent.key.clone(), new_key.clone())
                };
                out.contradictions.push(Contradiction {
                    source_fact,
                    target_fact,
                    detected_by: "detection-v1".to_string(),
                    quarantine_source: quarantine,
                    detected_at: captured_at.clone(),
                });
                if quarantine {
                    let (victim_id, victim_object, victim_trust) = if new_is_victim {
                        (materialized.fact.identity.id, &new_key.object, new_trust)
                    } else {
                        (incumbent.id, &incumbent.key.object, incumbent.trust)
                    };
                    let (survivor_object, survivor_trust) = if new_is_victim {
                        (&incumbent.key.object, incumbent.trust)
                    } else {
                        (&new_key.object, new_trust)
                    };
                    out.audits.push(crate::audit::quarantine_audit(
                        namespace,
                        &new_key.predicate,
                        &victim_id,
                        victim_object,
                        victim_trust,
                        survivor_object,
                        survivor_trust,
                        now,
                        actor_id,
                    ));
                }
            }
            // else independent — additive, no action.
        }
    }

    detect_intra_episode_ties(new_facts, cfg, captured_at, &mut out);
    out
}

/// The winning new fact id for each functional — or writer-hinted — `(subject, predicate)`
/// the episode asserts: the one whose object sorts first under [`object_order_key`]. Every
/// fact in an episode shares the episode's `captured_at`, so the K1 order reduces here to
/// the object order — the same rule the cross-episode comparison uses, so intra- and
/// cross-episode survivors agree by construction. This is the single survivor every
/// retirement — of an incumbent or of a losing functional peer — points at, so the rule
/// lives in exactly one place and `detect` and `detect_intra_episode_ties` cannot disagree
/// about who won. Hinted pairs need a survivor for the same one-edge-per-incumbent reason,
/// but their losing peers are NOT retired (a multi-valued predicate stays multi-valued).
fn survivors_for(
    new_facts: &[MaterializedFact],
    cfg: &DetectionConfig,
    hinted_pairs: &BTreeSet<(String, String)>,
) -> BTreeMap<(String, String), String> {
    // group -> (winning object order key, winning fact id)
    let mut survivors: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    for materialized in new_facts {
        let key = fact_key(&materialized.fact);
        let group_key = (key.subject_id.to_string(), key.predicate.clone());
        if !cfg.rule(&key.predicate).functional && !hinted_pairs.contains(&group_key) {
            continue;
        }
        let group = (key.subject_id.to_string(), key.predicate);
        let object_key = object_order_key(&key.object);
        let id = materialized.fact.identity.id.to_string();
        survivors
            .entry(group)
            .and_modify(|(winning_object, winning_id)| {
                if object_key < *winning_object {
                    *winning_object = object_key.clone();
                    *winning_id = id.clone();
                }
            })
            .or_insert_with(|| (object_key.clone(), id.clone()));
    }
    survivors
        .into_iter()
        .map(|(group, (_object, id))| (group, id))
        .collect()
}

/// Among new facts that share a functional `(subject, predicate)`, keep the one whose
/// object sorts first under [`object_order_key`] (the `functional_survivors` winner) and
/// supersede the rest by it — a deterministic, clock-free tiebreak for the within-episode
/// case (every fact shares `captured_at`, so the K1 order reduces to the object order).
fn detect_intra_episode_ties(
    new_facts: &[MaterializedFact],
    cfg: &DetectionConfig,
    captured_at: &Timestamp,
    out: &mut DetectionOutput,
) {
    let mut groups: BTreeMap<(String, String), Vec<&MaterializedFact>> = BTreeMap::new();
    for materialized in new_facts {
        let key = fact_key(&materialized.fact);
        if cfg.rule(&key.predicate).functional {
            groups
                .entry((key.subject_id.to_string(), key.predicate))
                .or_default()
                .push(materialized);
        }
    }
    for (_, mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by_key(|a| object_order_key(&a.fact.object));
        let survivor = fact_key(&group[0].fact);
        for loser in &group[1..] {
            let loser_key = fact_key(&loser.fact);
            if loser_key.object == survivor.object {
                continue; // identical object — dedup, not a tie
            }
            out.supersessions.push(Supersession {
                old_fact: loser_key,
                new_fact: survivor.clone(),
                reason: "intra_episode_functional_tie".to_string(),
                valid_from: captured_at.clone(),
            });
        }
    }
}

/// Whether two objects are mutually exclusive for a predicate: the always-on boolean
/// inversion rule, plus any configured antonym pairs (order-insensitive).
fn mutually_exclusive(
    rule: &crate::config::PredicateRule,
    a: &ObjectValue,
    b: &ObjectValue,
) -> bool {
    if let (ObjectValue::Bool(x), ObjectValue::Bool(y)) = (a, b)
        && x != y
    {
        return true;
    }
    rule.contradicts
        .iter()
        .any(|(p, q)| (p == a && q == b) || (p == b && q == a))
}

/// The identifying triple of a fact.
fn fact_key(fact: &Fact) -> FactKey {
    FactKey {
        subject_id: fact.subject_id,
        predicate: fact.predicate.clone(),
        object: fact.object.clone(),
    }
}

#[cfg(test)]
mod tests;
