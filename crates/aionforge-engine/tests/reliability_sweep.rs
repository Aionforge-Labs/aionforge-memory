//! End-to-end tests for the automatic D1 reliability-decay sweep (06 §5, M4.T05 PR-E2):
//! `Memory::sweep_reliability_decays` reads committed contradiction-quarantine audit rows off
//! the L0 all-namespaces spine and records the producer decays the host wrappers would
//! otherwise drive by hand — idempotently, behind the existing reliability off-switch, with a
//! host-round-tripped watermark cursor.
//!
//! Most tests commit emitter-shaped quarantine rows directly (the cheap path); the round-trip
//! drift guard at the end drives the *real* consolidation pipeline — extractor, contradiction
//! detection, scheduler co-commit — so a change to the emitter's payload shape or reason string
//! fails here, not silently in production.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use aionforge_consolidate::{
    CONTRADICTION_QUARANTINE_REASON, ConsolidationConfig, Consolidator, DetectionConfig,
    FactExtractionPass, InductionConfig, ObjectRule, PassConfig, PredicateRule, ResolutionConfig,
    Rule, RuleExtractor, RuleSummarizer, SummarizationConfig,
};
use aionforge_domain::blocks::{Identity, Stats};
use aionforge_domain::contracts::Embedder;
use aionforge_domain::edges::About;
use aionforge_domain::embedding::{EmbedderModel, Embedding};
use aionforge_domain::ids::{ContentHash, Id};
use aionforge_domain::namespace::Namespace;
use aionforge_domain::nodes::agent::{Agent, AgentStatus, TrustCategory, TrustScores};
use aionforge_domain::nodes::episodic::{ConsolidationState, Episode, Role};
use aionforge_domain::nodes::forensic::{AuditEvent, AuditKind};
use aionforge_domain::nodes::semantic::{Entity, Fact, FactStatus};
use aionforge_domain::time::{BiTemporal, Timestamp};
use aionforge_domain::value::ObjectValue;
use aionforge_engine::{D1SweepReport, Memory, MemoryConfig};
use aionforge_store::{BoundQuery, NodeId, Store, StoreConfig};
use aionforge_trust::ReliabilityPolicy;

const PREDICATE: &str = "preferred_by";
const DIM: u32 = 12;
const EPS: f64 = 1e-9;

#[derive(Clone)]
struct FakeEmbedder {
    model: EmbedderModel,
}
impl FakeEmbedder {
    fn new() -> Self {
        Self {
            model: EmbedderModel {
                family: "fake".to_string(),
                version: "1".to_string(),
                dimension: DIM,
            },
        }
    }
}
#[derive(Debug)]
struct NeverError;
impl std::fmt::Display for NeverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unreachable")
    }
}
impl std::error::Error for NeverError {}
impl Embedder for FakeEmbedder {
    type Error = NeverError;
    fn embed(
        &self,
        inputs: &[String],
    ) -> impl Future<Output = Result<Vec<Embedding>, Self::Error>> + Send {
        let out = inputs
            .iter()
            .map(|_| {
                let mut components = vec![0.0f32; DIM as usize];
                components[0] = 1.0;
                Embedding::new(components).expect("valid")
            })
            .collect();
        async move { Ok(out) }
    }
    fn model(&self) -> &EmbedderModel {
        &self.model
    }
}

/// A one-hot embedder for the real-pipeline test: distinct surfaces are orthogonal (never
/// cluster), identical surfaces coreference — mirrors the consolidate detection fixture so the
/// "up"/"down" contradiction survives resolution as two distinct facts.
#[derive(Clone)]
struct AxisEmbedder {
    model: EmbedderModel,
}
impl AxisEmbedder {
    fn new() -> Self {
        Self {
            model: EmbedderModel {
                family: "axis-fake".to_string(),
                version: "1".to_string(),
                dimension: DIM,
            },
        }
    }
}
impl Embedder for AxisEmbedder {
    type Error = NeverError;
    fn embed(
        &self,
        inputs: &[String],
    ) -> impl Future<Output = Result<Vec<Embedding>, Self::Error>> + Send {
        let out: Vec<Embedding> = inputs
            .iter()
            .map(|text| {
                let axis = text.trim().bytes().map(usize::from).sum::<usize>() % (DIM as usize);
                let mut components = vec![0.0f32; DIM as usize];
                components[axis] = 1.0;
                Embedding::new(components).expect("valid")
            })
            .collect();
        async move { Ok(out) }
    }
    fn model(&self) -> &EmbedderModel {
        &self.model
    }
}

/// A timestamp at the given minute, so rows order deterministically under `(occurred_at, id)`.
fn ts(minute: u32) -> Timestamp {
    format!("2026-06-09T09:{minute:02}:00-05:00[America/Chicago]")
        .parse()
        .expect("valid zoned datetime")
}

fn migrated_store() -> Arc<Store> {
    let store = Store::open_with_config(StoreConfig {
        embedding_dimension: DIM,
    })
    .expect("open store");
    store.migrate(&ts(0)).expect("migrate");
    Arc::new(store)
}

/// A memory with reliability scoring on (the sweep's only switch); promotion stays off.
fn memory(store: &Arc<Store>) -> Memory<FakeEmbedder> {
    let config = MemoryConfig {
        reliability: ReliabilityPolicy {
            enabled: true,
            ..ReliabilityPolicy::default()
        },
        ..MemoryConfig::default()
    };
    Memory::new(Arc::clone(store), FakeEmbedder::new(), config, &ts(0)).expect("memory")
}

fn stats(trust: f64) -> Stats {
    Stats {
        importance: 0.5,
        trust,
        last_access: ts(0),
        access_count_recent: 0,
        referenced_count: 0,
        surprise: 0.1,
        is_pinned: false,
    }
}

fn enroll(store: &Store) -> Id {
    let id = Id::generate();
    let mut scores = BTreeMap::new();
    scores.insert(
        PREDICATE.to_string(),
        TrustCategory {
            alpha: 1.0,
            beta: 1.0,
            score: 0.95,
        },
    );
    let agent = Agent {
        identity: Identity {
            id,
            ingested_at: ts(0),
            namespace: Namespace::Agent("ops".to_string()),
            expired_at: None,
        },
        public_key: "dGVzdC1rZXk=".to_string(),
        model_family: "test".to_string(),
        model_version: None,
        trust_scores: TrustScores(scores),
        status: AgentStatus::Active,
    };
    store.create_agent(&agent).expect("enroll");
    id
}

/// Assert a fact about a fresh subject entity in `namespace`; returns `(node, fact id)`.
fn fact(store: &Store, namespace: &Namespace, statement: &str) -> (NodeId, Id) {
    let subject = Entity {
        identity: Identity {
            id: Id::generate(),
            ingested_at: ts(0),
            namespace: namespace.clone(),
            expired_at: None,
        },
        stats: stats(0.5),
        canonical_name: format!("subject {statement}"),
        entity_type: "Concept".to_string(),
        aliases: vec![],
        description: None,
        embedding: None,
        embedder_model: None,
        attributes: None,
    };
    let subject_node = store.insert_entity(&subject).expect("entity");
    let fact = Fact {
        identity: Identity {
            id: Id::generate(),
            ingested_at: ts(0),
            namespace: namespace.clone(),
            expired_at: None,
        },
        stats: stats(0.8),
        subject_id: subject.identity.id,
        predicate: PREDICATE.to_string(),
        object: ObjectValue::Text("the team".to_string()),
        confidence: 0.9,
        status: FactStatus::Active,
        statement: statement.to_string(),
        embedding: None,
        embedder_model: None,
        extraction: None,
    };
    let node = store
        .assert_fact(
            &fact,
            subject_node,
            &About {
                temporal: BiTemporal {
                    valid_from: ts(0),
                    valid_to: None,
                    ingested_at: ts(0),
                    expired_at: None,
                },
            },
        )
        .expect("assert fact");
    (node, fact.identity.id)
}

/// Insert a raw episode captured by `agent` and wire `fact_id -DERIVED_FROM-> episode`, making
/// `agent` a producer of the fact.
fn produce(store: &Store, fact_id: &Id, agent: Id, seed: u128) {
    let episode_id = Id::generate();
    let episode = Episode {
        identity: Identity {
            id: episode_id,
            ingested_at: ts(0),
            namespace: Namespace::Agent("ops".to_string()),
            expired_at: None,
        },
        stats: stats(0.8),
        content: format!("source {seed}"),
        role: Role::User,
        captured_at: ts(0),
        agent_id: agent,
        session_id: None,
        content_hash: ContentHash::of(&seed.to_le_bytes()),
        embedding: None,
        embedder_model: None,
        consolidation_state: ConsolidationState::Raw,
        origin: None,
    };
    store.insert_episode(&episode).expect("insert episode");
    let query = BoundQuery::new(
        "MATCH (f:Fact {id: $fact}), (e:Episode {id: $ep}) \
         INSERT (f)-[:DERIVED_FROM {derived_at: $at}]->(e)",
    )
    .bind_uuid("fact", fact_id)
    .expect("bind fact")
    .bind_uuid("ep", episode_id)
    .expect("bind ep")
    .bind_timestamp("at", &ts(0))
    .expect("bind at");
    store.execute(&query).expect("wire DERIVED_FROM");
}

/// A producer-backed victim fact in `namespace`: asserted, derived from one episode by a fresh
/// enrolled agent. Returns `(fact id, producer agent id)`.
fn victim(store: &Store, namespace: &Namespace, seed: u128) -> (Id, Id) {
    let agent = enroll(store);
    let (_, fact_id) = fact(store, namespace, &format!("victim {seed}"));
    produce(store, &fact_id, agent, seed);
    (fact_id, agent)
}

/// An emitter-shaped contradiction-quarantine audit row (the consolidation pass's shape: the
/// victim is the subject, a pass actor distinct from it, the victim/survivor payload, and the
/// shared reason const). Committed through the store's audit funnel like the real co-commit.
fn commit_contradiction_quarantine(
    store: &Store,
    victim_id: &Id,
    namespace: &Namespace,
    survivor_object: &str,
    minute: u32,
) {
    let at = ts(minute);
    let event = AuditEvent {
        identity: Identity {
            id: Id::from_content_hash(
                format!("test-quarantine|{victim_id}|{survivor_object}").as_bytes(),
            ),
            ingested_at: at.clone(),
            namespace: namespace.clone(),
            expired_at: None,
        },
        kind: AuditKind::Quarantine,
        subject_id: *victim_id,
        actor_id: Id::from_content_hash(b"pass-actor"),
        payload: serde_json::json!({
            "predicate": PREDICATE,
            "victim_object": "down",
            "victim_trust": 0.5,
            "survivor_object": survivor_object,
            "survivor_trust": 0.9,
            "reason": CONTRADICTION_QUARANTINE_REASON,
        }),
        signature: String::new(),
        occurred_at: at,
    };
    store.commit_audit(&event).expect("commit quarantine");
}

/// A governance demotion-quarantine row (the promoter's shape: subject == actor == the global
/// copy, the demote payload) — must be skipped by the D1 sweep.
fn commit_governance_quarantine(store: &Store, minute: u32) {
    let global = Id::from_content_hash(b"global-copy");
    let at = ts(minute);
    let event = AuditEvent {
        identity: Identity {
            id: Id::from_content_hash(format!("test-governance|{minute}").as_bytes()),
            ingested_at: at.clone(),
            namespace: Namespace::System,
            expired_at: None,
        },
        kind: AuditKind::Quarantine,
        subject_id: global,
        actor_id: global,
        payload: serde_json::json!({
            "candidate_fact_id": "candidate",
            "promoted_fact_id": global.to_string(),
            "reason": "lost_support",
            "posterior": 0.4,
            "k": 2,
        }),
        signature: String::new(),
        occurred_at: at,
    };
    store
        .commit_audit(&event)
        .expect("commit governance quarantine");
}

fn agent_score_in(store: &Store, agent: &Id, category: &str) -> Option<f64> {
    store
        .agent_by_id(agent)
        .expect("agent")
        .and_then(|a| a.trust_scores.0.get(category).map(|c| c.score))
}

fn agent_score(store: &Store, agent: &Id) -> Option<f64> {
    agent_score_in(store, agent, PREDICATE)
}

fn reliability_event_count(store: &Store) -> usize {
    store
        .audit_by_kind(AuditKind::ReliabilityUpdate, None, 200)
        .expect("read")
        .events
        .len()
}

#[test]
fn the_sweep_is_inert_when_reliability_is_off() {
    let store = migrated_store();
    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        MemoryConfig::default(),
        &ts(0),
    )
    .expect("memory without reliability");
    let (fact_id, _) = victim(&store, &Namespace::Agent("ops".to_string()), 1);
    commit_contradiction_quarantine(
        &store,
        &fact_id,
        &Namespace::Agent("ops".to_string()),
        "up",
        1,
    );

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep");
    assert_eq!(report, D1SweepReport::default(), "off ⇒ inert, log unread");
    assert_eq!(reliability_event_count(&store), 0);
}

#[test]
fn a_contradiction_quarantine_decays_each_producer_once() {
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    // One victim fact, two distinct producers (two source episodes by different agents).
    let (fact_id, ada) = victim(&store, &namespace, 1);
    let bo = enroll(&store);
    produce(&store, &fact_id, bo, 2);
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", 1);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep");
    assert_eq!(report.quarantines_scanned, 1);
    assert_eq!(report.decays_recorded, 2, "one decay per distinct producer");
    assert_eq!(report.victims_unresolved, 0);
    assert!(
        report.next.is_some(),
        "a non-empty page reports a watermark"
    );
    for producer in [&ada, &bo] {
        assert!(
            (agent_score(&store, producer).expect("scored") - 1.0 / 3.0).abs() < EPS,
            "one contradiction folds to 1/3"
        );
    }
}

#[test]
fn the_sweep_skips_governance_demotion_quarantines() {
    let store = migrated_store();
    let memory = memory(&store);
    commit_governance_quarantine(&store, 1);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep");
    assert_eq!(
        report.quarantines_scanned, 0,
        "a D2-channel row is not a D1 trigger"
    );
    assert_eq!(report.decays_recorded, 0);
    assert!(
        report.next.is_some(),
        "a skipped row still advances the watermark past itself"
    );
    assert_eq!(reliability_event_count(&store), 0);
}

#[test]
fn multi_survivor_quarantine_of_one_victim_decays_each_producer_once() {
    // One victim contradicted by two different survivors in one episode mints two distinct
    // quarantine rows — but one wrong fact is one failure: the (victim, producer) decay key
    // collapses both rows to a single decay. (A trigger-keyed scheme would decay twice.)
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    let (fact_id, producer) = victim(&store, &namespace, 1);
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", 1);
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "sideways", 2);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(3))
        .expect("sweep");
    assert_eq!(
        report.quarantines_scanned, 2,
        "both rows are genuine D1 triggers"
    );
    assert_eq!(report.decays_recorded, 1, "but one fact is one failure");
    assert!((agent_score(&store, &producer).expect("scored") - 1.0 / 3.0).abs() < EPS);
}

#[test]
fn a_later_recontradiction_of_the_same_victim_does_not_double_decay() {
    // The ratified Fork-3 semantics: the FACT, not the cycle, is the evidence unit. A later
    // sweep over a fresh quarantine row for an already-decayed victim re-derives the same
    // (victim, producer) event id and records nothing new. A rekey to trigger-id semantics
    // breaks this test.
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    let (fact_id, producer) = victim(&store, &namespace, 1);
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", 1);
    let first = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("first sweep");
    assert_eq!(first.decays_recorded, 1);
    let after_first = agent_score(&store, &producer).expect("scored");

    commit_contradiction_quarantine(&store, &fact_id, &namespace, "another", 3);
    let second = memory
        .sweep_reliability_decays(first.next.as_ref(), 50, &ts(4))
        .expect("second sweep");
    assert_eq!(second.quarantines_scanned, 1, "the new row is scanned");
    assert_eq!(
        second.decays_recorded, 0,
        "but the producer already paid for this fact"
    );
    assert!(
        (agent_score(&store, &producer).expect("scored") - after_first).abs() < EPS,
        "the folded score is unchanged"
    );
}

#[test]
fn host_wrapper_and_auto_sweep_converge_to_one_decay() {
    // The decisive no-double-count proof: the host drives the wrapper first, then the sweep
    // re-derives the SAME content-addressed event and dedups — both paths share one key.
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    let (fact_id, producer) = victim(&store, &namespace, 1);
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", 1);

    assert_eq!(
        memory
            .record_reliability_decay(&fact_id, &ts(2))
            .expect("host wrapper"),
        1
    );
    let report = memory
        .sweep_reliability_decays(None, 50, &ts(3))
        .expect("sweep");
    assert_eq!(report.quarantines_scanned, 1);
    assert_eq!(
        report.decays_recorded, 0,
        "the wrapper already recorded this decay"
    );
    assert!((agent_score(&store, &producer).expect("scored") - 1.0 / 3.0).abs() < EPS);
    assert_eq!(
        reliability_event_count(&store),
        1,
        "one event row, two paths"
    );
}

#[test]
fn a_full_rescan_is_a_no_op_after_a_first_sweep() {
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    let (fact_a, ada) = victim(&store, &namespace, 1);
    let (fact_b, bo) = victim(&store, &namespace, 2);
    commit_contradiction_quarantine(&store, &fact_a, &namespace, "up", 1);
    commit_contradiction_quarantine(&store, &fact_b, &namespace, "up", 2);

    let first = memory
        .sweep_reliability_decays(None, 50, &ts(3))
        .expect("first");
    assert_eq!(first.decays_recorded, 2);
    let scores = (
        agent_score(&store, &ada).expect("ada"),
        agent_score(&store, &bo).expect("bo"),
    );

    // A host that lost its watermark rescans from the top: every event dedups, every refold
    // converges to the same fold — crash-replay safety as a visible no-op.
    let second = memory
        .sweep_reliability_decays(None, 50, &ts(4))
        .expect("rescan");
    assert_eq!(second.quarantines_scanned, 2);
    assert_eq!(second.decays_recorded, 0);
    assert_eq!(
        (
            agent_score(&store, &ada).expect("ada"),
            agent_score(&store, &bo).expect("bo"),
        ),
        scores,
        "the refolded caches are identical after the rescan"
    );
}

#[test]
fn a_quarantine_whose_victim_was_purged_is_counted_not_an_error() {
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    // A quarantine row whose subject resolves to no live fact (e.g. hard-purged since).
    let gone = Id::generate();
    commit_contradiction_quarantine(&store, &gone, &namespace, "up", 1);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep must not abort");
    assert_eq!(report.quarantines_scanned, 1);
    assert_eq!(report.victims_unresolved, 1);
    assert_eq!(report.decays_recorded, 0);
}

#[test]
fn a_victim_with_no_producers_records_nothing() {
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    // A live fact with no DERIVED_FROM source — nobody to decay, cleanly.
    let (_, fact_id) = fact(&store, &namespace, "unproduced");
    commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", 1);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep");
    assert_eq!(report.quarantines_scanned, 1);
    assert_eq!(report.victims_unresolved, 0);
    assert_eq!(report.decays_recorded, 0);
}

#[test]
fn the_sweep_reads_team_namespace_quarantines_without_a_principal() {
    // The namespace posture: quarantine audits live in the VICTIM's namespace, and the sweep
    // reads the all-namespaces L0 spine with no principal — a scoped read would have silently
    // skipped this team row and under-penalized.
    let store = migrated_store();
    let memory = memory(&store);
    let team = Namespace::Team("acme".to_string());
    let (fact_id, producer) = victim(&store, &team, 1);
    commit_contradiction_quarantine(&store, &fact_id, &team, "up", 1);

    let report = memory
        .sweep_reliability_decays(None, 50, &ts(2))
        .expect("sweep");
    assert_eq!(report.decays_recorded, 1);
    assert!((agent_score(&store, &producer).expect("scored") - 1.0 / 3.0).abs() < EPS);
}

#[test]
fn the_cursor_resumes_across_pages_without_reprocessing() {
    let store = migrated_store();
    let memory = memory(&store);
    let namespace = Namespace::Agent("ops".to_string());
    let mut producers = Vec::new();
    for seed in 1..=5u128 {
        let (fact_id, producer) = victim(&store, &namespace, seed);
        let minute = u32::try_from(seed).expect("small");
        commit_contradiction_quarantine(&store, &fact_id, &namespace, "up", minute);
        producers.push(producer);
    }

    // Page through with limit 2: 2 + 2 + 1 rows, each page reporting a watermark.
    let mut after: Option<aionforge_engine::AuditCursor> = None;
    let mut total_scanned = 0;
    let mut total_decays = 0;
    let mut pages = 0;
    loop {
        let report = memory
            .sweep_reliability_decays(after.as_ref(), 2, &ts(10))
            .expect("page");
        if report.next.is_none() {
            assert_eq!(
                report.quarantines_scanned, 0,
                "the empty page ends the loop"
            );
            break;
        }
        pages += 1;
        total_scanned += report.quarantines_scanned;
        total_decays += report.decays_recorded;
        after = report.next;
    }
    assert_eq!(pages, 3, "5 rows at limit 2 ⇒ pages of 2, 2, 1");
    assert_eq!(total_scanned, 5);
    assert_eq!(
        total_decays, 5,
        "every victim decayed exactly once across pages"
    );
    for producer in &producers {
        assert!((agent_score(&store, producer).expect("scored") - 1.0 / 3.0).abs() < EPS);
    }
}

/// The round-trip drift guard: drive the REAL pipeline — rule extraction, high-trust
/// contradiction detection, the scheduler's co-committed quarantine audit — then sweep. A
/// change to the emitter's reason string or payload shape fails here, where a unit table of
/// hand-built rows could silently drift.
#[tokio::test]
async fn a_real_contradiction_quarantine_is_swept_end_to_end() {
    let store = migrated_store();
    let namespace = Namespace::Agent("ops".to_string());

    // E1 (trust 0.9, agent ada) says the server is up; E2 (trust 0.5, agent bo) says it is
    // down. The contradiction quarantines the LOWER-trust side — bo's "down" fact — so bo is
    // the producer the sweep must decay.
    let ada = enroll(&store);
    let bo = enroll(&store);
    for (minute, content, trust, agent) in [
        (1u32, "Server status up.", 0.9, ada),
        (5, "Server status down.", 0.5, bo),
    ] {
        let at = ts(minute);
        let episode = Episode {
            identity: Identity {
                id: Id::generate(),
                ingested_at: at.clone(),
                namespace: namespace.clone(),
                expired_at: None,
            },
            stats: Stats {
                trust,
                ..stats(trust)
            },
            content: content.to_string(),
            role: Role::User,
            captured_at: at,
            agent_id: agent,
            session_id: None,
            content_hash: ContentHash::of(content.as_bytes()),
            embedding: None,
            embedder_model: None,
            consolidation_state: ConsolidationState::Raw,
            origin: None,
        };
        store.insert_episode(&episode).expect("insert episode");
    }

    // The detection fixture's status rule: "X status Y." extracts (X, status, Y), with
    // up/down declared mutually contradictory.
    let extractor = RuleExtractor::new(
        "rule-status",
        vec![Rule {
            marker: "status".to_string(),
            predicate: "status".to_string(),
            subject_type: "Service".to_string(),
            object: ObjectRule::Text,
            confidence: 0.9,
        }],
    );
    let mut detection = DetectionConfig::with_default_rules();
    detection.predicates.insert(
        "status".to_string(),
        PredicateRule {
            functional: false,
            contradicts: vec![(
                ObjectValue::Text("up".to_string()),
                ObjectValue::Text("down".to_string()),
            )],
        },
    );
    let mut consolidator = Consolidator::new(Arc::clone(&store), ConsolidationConfig::default());
    consolidator.register(Box::new(FactExtractionPass::new(
        Arc::new(extractor),
        Arc::new(AxisEmbedder::new()),
        Arc::new(RuleSummarizer::with_default_rules()),
        PassConfig {
            resolution: ResolutionConfig::default(),
            detection,
            summarization: SummarizationConfig::default(),
            induction: InductionConfig::default(),
        },
    )));
    loop {
        let report = consolidator.tick_once().await.expect("tick");
        if report.pending_after == 0 {
            break;
        }
    }

    let memory = memory(&store);
    let report = memory
        .sweep_reliability_decays(None, 50, &ts(30))
        .expect("sweep");
    assert_eq!(
        report.quarantines_scanned, 1,
        "the pipeline-emitted quarantine row classifies as a D1 trigger: {report:?}"
    );
    assert_eq!(report.decays_recorded, 1, "the victim's producer pays");
    assert_eq!(report.victims_unresolved, 0);
    assert!(
        (agent_score_in(&store, &bo, "status").expect("bo scored") - 1.0 / 3.0).abs() < EPS,
        "the lower-trust producer decays in the fact's category"
    );
    assert!(
        agent_score_in(&store, &ada, "status").is_none(),
        "the survivor's producer is untouched"
    );
}
