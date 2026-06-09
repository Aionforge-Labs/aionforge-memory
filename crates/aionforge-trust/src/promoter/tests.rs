//! Unit tests for the promoter's audit-id schemes. Split out of `promoter.rs` to keep the
//! implementation file under the line cap; as a child module it still reaches the parent's
//! private `attest_audit` builder and `DemotionReason` tags.

use super::DemotionReason;

/// The structural and reliability demotions must never share an audit content id, and the
/// structural tags must stay byte-for-byte what they were before the shared-helper refactor —
/// the content-addressed audit id is `(tag, subject)`, so a changed tag would either collide
/// the two paths or silently re-key the existing lost-support audit.
#[test]
fn demotion_reason_tags_are_distinct_and_structural_tags_are_pinned() {
    let lost = DemotionReason::LostSupport;
    let decay = DemotionReason::ReliabilityDecay;

    assert_eq!(lost.reason(), "lost_support");
    assert_eq!(lost.demote_tag(), "demote");
    assert_eq!(lost.quarantine_tag(), "quarantine");

    assert_eq!(decay.reason(), "reliability_decay");
    assert_eq!(decay.demote_tag(), "demote_reliability");
    assert_eq!(decay.quarantine_tag(), "quarantine_reliability");

    assert_ne!(lost.reason(), decay.reason());
    assert_ne!(lost.demote_tag(), decay.demote_tag());
    assert_ne!(lost.quarantine_tag(), decay.quarantine_tag());
}

/// The dual of `a_consolidation_audit_id_ignores_the_clock`: an *accepted* attestation is
/// content-addressed and ignores the instant, the opposite of the governance `cycle_id` that
/// folds it. The `ATTESTED_BY` edge is write-once with no de-attest path, so a re-attest by the
/// same attester at a *later* instant is a no-op at the edge — the audit must mirror that and
/// collapse to one row, never minting a second under the discriminator. This guards against a
/// future change that "fixes" `attest_audit` to fold time the way governance audits do.
#[test]
fn the_attest_audit_id_ignores_the_instant() {
    use super::{AttestRequest, Promoter};
    use crate::{
        AttestationGate, Ed25519Verifier, PromotionPolicy, StoreKeyResolver, SystemWallClock,
    };
    use aionforge_domain::ids::Id;
    use aionforge_domain::time::Timestamp;
    use aionforge_store::{Store, StoreConfig};
    use std::sync::Arc;

    // attest_audit never reads the store or the gate, so a default promoter drives the pure
    // request -> AuditEvent builder; the gate pieces only exist because the constructor takes them.
    let store = Arc::new(
        Store::open_with_config(StoreConfig {
            embedding_dimension: 4,
        })
        .expect("open store"),
    );
    let gate = AttestationGate::new(
        Ed25519Verifier,
        Arc::new(StoreKeyResolver::new(Arc::clone(&store))),
        Arc::new(SystemWallClock),
        5_000,
    );
    let promoter = Promoter::new(store, gate, PromotionPolicy::default());

    let fact_id = Id::generate();
    let attester_id = Id::generate();
    let req_at = |s: &str| AttestRequest {
        fact_id,
        attester_id,
        attested_at: s.parse::<Timestamp>().expect("valid zoned datetime"),
        signature_b64: String::new(),
        category: Some("reliability".to_string()),
    };
    let early = promoter.attest_audit(&req_at("2026-06-08T09:00:00-05:00[America/Chicago]"));
    let late = promoter.attest_audit(&req_at("2026-06-08T11:30:00-05:00[America/Chicago]"));

    assert_ne!(
        early.occurred_at, late.occurred_at,
        "the two attestations are at genuinely different instants"
    );
    assert_eq!(
        early.identity.id, late.identity.id,
        "a re-attest at a later instant collapses to the same audit row (write-once edge)"
    );
}
