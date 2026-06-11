//! Unit tests for supersession/contradiction detection — split from `detect.rs`
//! under the 700 LOC cap. Pure-function tests; fixtures build `CurrentFact` and
//! `MaterializedFact` values directly and assert the emitted instructions.

use super::*;
use aionforge_domain::blocks::{Identity, Stats};
use aionforge_domain::edges::About;
use aionforge_domain::nodes::forensic::AuditKind;
use aionforge_domain::nodes::semantic::FactStatus;
use aionforge_domain::time::BiTemporal;

use crate::config::PredicateRule;

fn ts(text: &str) -> Timestamp {
    text.parse().expect("valid zoned datetime literal")
}

/// 09:00 — the incumbent's `valid_from` in most cases.
fn t1() -> Timestamp {
    ts("2026-06-06T09:00:00Z[UTC]")
}

/// 11:00 — the new episode's `captured_at` (strictly after `t1`).
fn t2() -> Timestamp {
    ts("2026-06-06T11:00:00Z[UTC]")
}

fn ns() -> Namespace {
    Namespace::Agent("tester".to_string())
}

fn stats(trust: f64) -> Stats {
    Stats {
        importance: 0.5,
        trust,
        last_access: t1(),
        access_count_recent: 0,
        referenced_count: 0,
        surprise: 0.0,
        is_pinned: false,
    }
}

/// A committed current fact opened at `t1`, with the given trust.
fn current(subject: &Id, predicate: &str, object: ObjectValue, trust: f64) -> CurrentFact {
    current_at(subject, predicate, object, t1(), trust)
}

/// A committed current fact opened at an explicit `valid_from`, with the given trust.
fn current_at(
    subject: &Id,
    predicate: &str,
    object: ObjectValue,
    valid_from: Timestamp,
    trust: f64,
) -> CurrentFact {
    CurrentFact {
        id: Id::generate(),
        hint_eligible: false,
        key: FactKey {
            subject_id: *subject,
            predicate: predicate.to_string(),
            object,
        },
        valid_from,
        trust,
    }
}

/// A new materialized fact with an explicit id (so tie-break ordering is decidable).
fn mfact_with_id(id: Id, subject: &Id, predicate: &str, object: ObjectValue) -> MaterializedFact {
    MaterializedFact {
        fact: Fact {
            identity: Identity {
                id,
                ingested_at: t2(),
                namespace: ns(),
                expired_at: None,
            },
            stats: stats(0.9),
            subject_id: *subject,
            predicate: predicate.to_string(),
            object,
            confidence: 0.9,
            status: FactStatus::Active,
            statement: String::new(),
            embedding: None,
            embedder_model: None,
            extraction: None,
            cooled_until: None,
        },
        about: About {
            temporal: BiTemporal {
                valid_from: t2(),
                valid_to: None,
                ingested_at: t2(),
                expired_at: None,
            },
        },
    }
}

fn mfact(subject: &Id, predicate: &str, object: ObjectValue) -> MaterializedFact {
    mfact_with_id(Id::generate(), subject, predicate, object)
}

/// A new materialized fact with an explicit writer trust (drives the contradiction
/// quarantine decision).
fn mfact_trust(subject: &Id, predicate: &str, object: ObjectValue, trust: f64) -> MaterializedFact {
    let mut materialized = mfact(subject, predicate, object);
    materialized.fact.stats.trust = trust;
    materialized
}

fn text(value: &str) -> ObjectValue {
    ObjectValue::Text(value.to_string())
}

/// Run detection with the standard event/transaction times and a throwaway actor.
fn run(
    current: &[CurrentFact],
    new: &[MaterializedFact],
    cfg: &DetectionConfig,
) -> DetectionOutput {
    detect(current, new, cfg, &ns(), &t2(), &t2(), &Id::generate())
}

#[test]
fn functional_predicate_supersedes_on_a_newer_object() {
    let cfg = DetectionConfig::with_default_rules(); // `based_in` is functional
    let subject = Id::generate();
    let cur = vec![current(&subject, "based_in", text("NYC"), 0.9)];
    let new = vec![mfact(&subject, "based_in", text("SF"))];

    let out = run(&cur, &new, &cfg);

    assert_eq!(out.supersessions.len(), 1, "one supersession");
    assert!(
        out.contradictions.is_empty(),
        "supersession, not contradiction"
    );
    let s = &out.supersessions[0];
    assert_eq!(
        s.old_fact.object,
        text("NYC"),
        "the prior object is retired"
    );
    assert_eq!(s.new_fact.object, text("SF"), "the newer object wins");
    assert_eq!(
        s.valid_from,
        t2(),
        "the window closes at the new event time"
    );
}

#[test]
fn multi_valued_predicate_is_additive() {
    // `knows` is unregistered, so it is multi-valued and its objects are independent.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let cur = vec![current(&subject, "knows", text("Rust"), 0.9)];
    let new = vec![mfact(&subject, "knows", text("Go"))];

    let out = run(&cur, &new, &cfg);

    assert!(out.supersessions.is_empty(), "additive: nothing is retired");
    assert!(
        out.contradictions.is_empty(),
        "independent objects do not conflict"
    );
}

#[test]
fn opposite_booleans_contradict_and_a_high_trust_incumbent_quarantines() {
    let cfg = DetectionConfig::with_default_rules(); // boolean inversion is always on
    let subject = Id::generate();
    let cur = vec![current(&subject, "is_up", ObjectValue::Bool(true), 0.9)];
    let new = vec![mfact(&subject, "is_up", ObjectValue::Bool(false))];

    let out = run(&cur, &new, &cfg);

    assert!(
        out.supersessions.is_empty(),
        "is_up is multi-valued, not functional"
    );
    assert_eq!(out.contradictions.len(), 1, "one contradiction");
    assert!(
        out.contradictions[0].quarantine_source,
        "a high-trust pair quarantines the victim"
    );
    // The victim (the quarantined CONTRADICTS source) is the smaller object order on a
    // trust tie: object_order_key(false) < object_order_key(true), so `false` is the
    // victim — here that is the new fact, but by the symmetric rule, not by being new.
    assert_eq!(
        out.contradictions[0].source_fact.object,
        ObjectValue::Bool(false),
        "the smaller-object-order side is the victim"
    );
    assert_eq!(
        out.audits.len(),
        1,
        "the quarantine raises one reconcile signal"
    );
    assert_eq!(out.audits[0].kind, AuditKind::Quarantine);
}

#[test]
fn a_symmetric_low_trust_contradiction_records_without_quarantine() {
    // Both sides below the high-trust bar: the contradiction is still recorded, the victim
    // is still chosen symmetrically (smaller object order), but neither side is quarantined
    // — max trust does not clear the threshold.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let cur = vec![current(&subject, "is_up", ObjectValue::Bool(true), 0.5)];
    let new = vec![mfact_trust(
        &subject,
        "is_up",
        ObjectValue::Bool(false),
        0.5,
    )];

    let out = run(&cur, &new, &cfg);

    assert_eq!(out.contradictions.len(), 1, "still recorded");
    assert!(
        !out.contradictions[0].quarantine_source,
        "below the trust threshold neither side is quarantined"
    );
    assert_eq!(
        out.contradictions[0].source_fact.object,
        ObjectValue::Bool(false),
        "the victim is still the smaller object order, deterministically"
    );
    assert!(out.audits.is_empty(), "no quarantine, no reconcile signal");
}

#[test]
fn the_lower_trust_side_is_the_victim_in_either_arrival_order() {
    // up@0.5 vs down@0.9, mutually exclusive. The lower-trust side is the victim (the
    // quarantined CONTRADICTS source) regardless of which side is the incumbent — including
    // the direction the old incumbent-keyed rule could never produce: a higher-trust
    // newcomer quarantining the lower-trust incumbent, with the audit naming the incumbent.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();

    // The low-trust `true` is the incumbent; the high-trust `false` arrives and wins.
    let incumbent = current(&subject, "is_up", ObjectValue::Bool(true), 0.5);
    let incumbent_id = incumbent.id;
    let a = detect(
        &[incumbent],
        &[mfact_trust(
            &subject,
            "is_up",
            ObjectValue::Bool(false),
            0.9,
        )],
        &cfg,
        &ns(),
        &t2(),
        &t2(),
        &Id::generate(),
    );
    assert_eq!(a.contradictions.len(), 1);
    assert_eq!(
        a.contradictions[0].source_fact.object,
        ObjectValue::Bool(true),
        "the lower-trust incumbent is the victim/source"
    );
    assert_eq!(
        a.contradictions[0].target_fact.object,
        ObjectValue::Bool(false),
        "the higher-trust newcomer survives as the target"
    );
    assert!(
        a.contradictions[0].quarantine_source,
        "max trust 0.9 clears the bar"
    );
    assert_eq!(
        a.audits[0].subject_id, incumbent_id,
        "the audit names the quarantined incumbent, not the new fact"
    );

    // Mirror arrival order: the high-trust `false` is the incumbent, the low-trust `true`
    // arrives. The same low-trust `true` is the victim, so the contradiction converges.
    let b = detect(
        &[current(&subject, "is_up", ObjectValue::Bool(false), 0.9)],
        &[mfact_trust(&subject, "is_up", ObjectValue::Bool(true), 0.5)],
        &cfg,
        &ns(),
        &t2(),
        &t2(),
        &Id::generate(),
    );
    assert_eq!(
        b.contradictions[0].source_fact.object,
        ObjectValue::Bool(true),
        "the same low-trust side is the victim in the reverse order"
    );
    assert_eq!(
        a.contradictions[0].source_fact.object, b.contradictions[0].source_fact.object,
        "the victim is identical in both arrival orders — the contradiction converges"
    );
}

#[test]
fn a_configured_antonym_pair_contradicts_order_insensitively() {
    let mut cfg = DetectionConfig::with_default_rules();
    cfg.predicates.insert(
        "status".to_string(),
        PredicateRule {
            functional: false,
            contradicts: vec![(text("up"), text("down"))],
        },
    );
    let subject = Id::generate();

    let forward = run(
        &[current(&subject, "status", text("up"), 0.9)],
        &[mfact(&subject, "status", text("down"))],
        &cfg,
    );
    assert_eq!(forward.contradictions.len(), 1, "up vs down contradicts");

    let reverse = run(
        &[current(&subject, "status", text("down"), 0.9)],
        &[mfact(&subject, "status", text("up"))],
        &cfg,
    );
    assert_eq!(
        reverse.contradictions.len(),
        1,
        "down vs up contradicts too"
    );

    // The victim is identical regardless of which side arrived first: equal trust, so the
    // smaller object order ('down' < 'up') is the victim in BOTH orders. This is the
    // arrival-order-symmetry the old incumbent-keyed rule lacked.
    assert_eq!(
        forward.contradictions[0].source_fact.object,
        text("down"),
        "'down' is the victim when 'up' is the incumbent"
    );
    assert_eq!(
        reverse.contradictions[0].source_fact.object,
        text("down"),
        "'down' is still the victim when 'down' is the incumbent"
    );
}

#[test]
fn the_same_triple_is_dedup_not_a_conflict() {
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let cur = vec![current(&subject, "based_in", text("NYC"), 0.9)];
    let new = vec![mfact(&subject, "based_in", text("NYC"))];

    let out = run(&cur, &new, &cfg);

    assert!(
        out.supersessions.is_empty(),
        "an identical object is not superseded"
    );
    assert!(
        out.contradictions.is_empty(),
        "an identical object is not a conflict"
    );
}

#[test]
fn an_intra_episode_functional_tie_keeps_the_lexicographically_smallest_object() {
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    // Two new facts for the same functional (subject, predicate) with different objects.
    // They share the episode's captured_at, so the K1 order reduces to the object order:
    // the survivor is the one whose object sorts first, decoupled from the (arrival-
    // fragile) content-hash fact id the rule used to key on.
    let nyc = mfact(&subject, "based_in", text("NYC"));
    let sf = mfact(&subject, "based_in", text("SF"));

    let out = run(&[], &[nyc.clone(), sf.clone()], &cfg);

    assert_eq!(
        out.supersessions.len(),
        1,
        "the tie yields one supersession"
    );
    let s = &out.supersessions[0];
    assert_eq!(
        s.new_fact.object,
        text("NYC"),
        "the smallest object order ('NYC' < 'SF') survives"
    );
    assert_eq!(s.old_fact.object, text("SF"), "the rest are retired by it");

    // Swapping the input order keeps the same survivor — the tiebreak is on the object,
    // not on input/arrival order.
    let swapped = run(&[], &[sf, nyc], &cfg);
    assert_eq!(swapped.supersessions.len(), 1);
    assert_eq!(
        swapped.supersessions[0].new_fact.object,
        text("NYC"),
        "survivor is independent of input order"
    );
}

#[test]
fn detection_disabled_is_a_no_op() {
    let mut cfg = DetectionConfig::with_default_rules();
    cfg.enabled = false;
    let subject = Id::generate();
    let cur = vec![current(&subject, "based_in", text("NYC"), 0.9)];
    let new = vec![mfact(&subject, "based_in", text("SF"))];

    let out = run(&cur, &new, &cfg);

    assert!(out.supersessions.is_empty());
    assert!(out.contradictions.is_empty());
    assert!(out.audits.is_empty());
}

#[test]
fn a_stale_assertion_is_superseded_by_a_newer_incumbent() {
    // The incumbent opens at 11:00; the "new" fact's event time is 09:00 — older. A
    // functional slot holds exactly one current object, so the stale assertion does not
    // become a second current value (the old forward-only guard's divergence): it is born
    // superseded by the newer incumbent, retained in history with a closed window.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let cur = vec![CurrentFact {
        id: Id::generate(),
        hint_eligible: false,
        key: FactKey {
            subject_id: subject,
            predicate: "based_in".to_string(),
            object: text("SF"),
        },
        valid_from: t2(),
        trust: 0.9,
    }];
    let new = vec![mfact(&subject, "based_in", text("NYC"))];

    let out = detect(&cur, &new, &cfg, &ns(), &t1(), &t1(), &Id::generate());

    assert_eq!(
        out.supersessions.len(),
        1,
        "the stale assertion is retired by the incumbent, not left additive"
    );
    let s = &out.supersessions[0];
    assert_eq!(
        s.old_fact.object,
        text("NYC"),
        "the stale new fact is the side that is superseded"
    );
    assert_eq!(
        s.new_fact.object,
        text("SF"),
        "the newer incumbent is the survivor"
    );
    assert_eq!(
        s.valid_from,
        t2(),
        "the stale fact's window closes at the incumbent's valid_from"
    );
    assert!(
        out.contradictions.is_empty(),
        "a functional supersession, not a contradiction"
    );
}

#[test]
fn equal_valid_from_functional_assertions_converge_regardless_of_incumbent() {
    // Two functional assertions with the SAME event time but different objects. Whichever
    // one is the committed incumbent, the winner of the single slot is the same — the
    // smaller object order — so the outcome cannot depend on which arrived first. The
    // simultaneous-tie convergence guard at the detect level.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();

    // SF incumbent, NYC new, both at t2. 'NYC' < 'SF', so NYC wins the slot.
    let sf_incumbent = vec![current_at(&subject, "based_in", text("SF"), t2(), 0.9)];
    let nyc_new = vec![mfact(&subject, "based_in", text("NYC"))];
    let a = detect(
        &sf_incumbent,
        &nyc_new,
        &cfg,
        &ns(),
        &t2(),
        &t2(),
        &Id::generate(),
    );

    // The mirror arrival order: NYC incumbent, SF new, both at t2.
    let nyc_incumbent = vec![current_at(&subject, "based_in", text("NYC"), t2(), 0.9)];
    let sf_new = vec![mfact(&subject, "based_in", text("SF"))];
    let b = detect(
        &nyc_incumbent,
        &sf_new,
        &cfg,
        &ns(),
        &t2(),
        &t2(),
        &Id::generate(),
    );

    // In both orders the survivor is NYC and SF is the retired side — identical current
    // state regardless of which assertion happened to be committed first.
    assert_eq!(a.supersessions.len(), 1, "one supersession either way");
    assert_eq!(b.supersessions.len(), 1, "one supersession either way");
    assert_eq!(
        a.supersessions[0].new_fact.object,
        text("NYC"),
        "NYC wins when SF is the incumbent"
    );
    assert_eq!(
        b.supersessions[0].new_fact.object,
        text("NYC"),
        "NYC still wins when NYC is the incumbent (it retires the new SF)"
    );
    assert_eq!(a.supersessions[0].old_fact.object, text("SF"));
    assert_eq!(b.supersessions[0].old_fact.object, text("SF"));
}

#[test]
fn one_incumbent_is_superseded_only_by_the_surviving_new_fact() {
    // An episode asserts two new values (SF, Boston) for one functional predicate while
    // an NYC incumbent stands. Every retirement must route to the single survivor — the
    // object that sorts first, 'Boston' < 'SF' — so the incumbent is retired once (not
    // once per new fact), and the losing peer is retired by that same survivor: no retired
    // fact points at anything but the winner.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let sf = mfact(&subject, "based_in", text("SF"));
    let boston = mfact(&subject, "based_in", text("Boston"));
    // 'Boston' sorts before 'SF', so Boston is the survivor regardless of fact ids.
    let cur = vec![current(&subject, "based_in", text("NYC"), 0.9)];

    let out = run(&cur, &[sf.clone(), boston.clone()], &cfg);

    assert_eq!(
        out.supersessions.len(),
        2,
        "the incumbent and the losing peer are each retired exactly once"
    );
    assert!(
        out.supersessions
            .iter()
            .all(|s| s.new_fact.object == text("Boston")),
        "every retirement points at the single survivor (Boston), not a losing peer: {:?}",
        out.supersessions
    );
    let retired: Vec<ObjectValue> = out
        .supersessions
        .iter()
        .map(|s| s.old_fact.object.clone())
        .collect();
    assert!(retired.contains(&text("NYC")), "the incumbent is retired");
    assert!(retired.contains(&text("SF")), "the losing peer is retired");
    assert!(
        out.contradictions.is_empty(),
        "supersession, not contradiction"
    );
}

/// A hinted current fact, off the functional registry.
fn hinted(mut fact: CurrentFact) -> CurrentFact {
    fact.hint_eligible = true;
    fact
}

#[test]
fn a_hinted_incumbent_is_superseded_off_the_functional_registry() {
    // `works_with` is not in the registry, so without the hint the pair is additive.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let incumbent = current_at(&subject, "works_with", text("alice"), t1(), 0.9);

    let additive = run(
        &[incumbent],
        &[mfact(&subject, "works_with", text("bob"))],
        &cfg,
    );
    assert!(additive.supersessions.is_empty(), "no hint, no action");
    assert!(additive.contradictions.is_empty());

    // The same pair with the writer-asserted hint retires the incumbent via K1.
    let incumbent = hinted(current_at(&subject, "works_with", text("alice"), t1(), 0.9));
    let out = run(
        &[incumbent],
        &[mfact(&subject, "works_with", text("bob"))],
        &cfg,
    );
    assert_eq!(out.supersessions.len(), 1, "the hinted incumbent retires");
    let s = &out.supersessions[0];
    assert_eq!(s.old_fact.object, text("alice"));
    assert_eq!(s.new_fact.object, text("bob"));
    assert_eq!(
        s.reason, "writer-hinted supersession of a replaced episode's fact",
        "the instruction names the hint, not the registry"
    );
    assert!(out.contradictions.is_empty());
}

#[test]
fn a_lower_trust_hint_routes_to_the_contradiction_path() {
    // The hint widens which pairs are compared, never who wins a trust fight: a
    // 0.4-trust correction cannot silently retire a 0.9-trust incumbent. It lands in
    // the contradiction arm, the lower-trust new fact is the victim, and the
    // high-trust incumbent (>= 0.7 threshold) makes it a quarantine.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let incumbent = hinted(current_at(&subject, "works_with", text("alice"), t1(), 0.9));

    let out = run(
        &[incumbent],
        &[mfact_trust(&subject, "works_with", text("bob"), 0.4)],
        &cfg,
    );
    assert!(
        out.supersessions.is_empty(),
        "a lower-trust hint never silently supersedes"
    );
    assert_eq!(out.contradictions.len(), 1);
    let c = &out.contradictions[0];
    assert_eq!(
        c.source_fact.object,
        text("bob"),
        "the new fact is the victim"
    );
    assert!(c.quarantine_source, "a high-trust incumbent quarantines");
}

#[test]
fn a_hinted_multi_valued_pair_retires_the_incumbent_to_one_survivor() {
    // Two new values for one hinted non-functional pair: the incumbent gets exactly
    // one SUPERSEDED_BY target (the object-order survivor), and the new siblings are
    // NOT retired against each other — multi-valued stays multi-valued.
    let cfg = DetectionConfig::with_default_rules();
    let subject = Id::generate();
    let incumbent = hinted(current_at(&subject, "works_with", text("zed"), t1(), 0.9));

    let out = run(
        &[incumbent],
        &[
            mfact(&subject, "works_with", text("alpha")),
            mfact(&subject, "works_with", text("beta")),
        ],
        &cfg,
    );
    assert_eq!(
        out.supersessions.len(),
        1,
        "one retirement, routed to the single survivor: {:?}",
        out.supersessions
            .iter()
            .map(|s| (&s.old_fact.object, &s.new_fact.object))
            .collect::<Vec<_>>()
    );
    let s = &out.supersessions[0];
    assert_eq!(s.old_fact.object, text("zed"));
    assert_eq!(
        s.new_fact.object,
        text("alpha"),
        "the object-order winner carries the edge"
    );
    assert!(out.contradictions.is_empty());
}
