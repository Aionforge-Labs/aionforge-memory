//! Acceptance for the single-family startup warning (07 §3, M6.T01 design Q7):
//! when a consolidating family is declared and every enrolled agent's family
//! compares Same against it, construction surfaces a typed warning the host logs
//! AND writes a `subliminal_guard_warning` audit row (content-addressed, so
//! restarts dedup). No declared family, a mixed fleet, or an empty store skips
//! the check — the per-call guard stays the enforcement either way.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{FakeEmbedder, migrated_store, ts};

use aionforge_domain::blocks::Identity;
use aionforge_domain::ids::Id;
use aionforge_domain::namespace::Namespace;
use aionforge_domain::nodes::agent::{Agent, AgentStatus, TrustScores};
use aionforge_engine::{ConsolidationGuardPolicy, Memory, MemoryConfig, StartupWarning};
use aionforge_store::{BoundQuery, QueryResult, Store, Value};

fn enroll_with_family(store: &Store, family: &str, seed: &[u8]) {
    let agent = Agent {
        identity: Identity {
            id: Id::from_content_hash(seed),
            ingested_at: ts(0),
            namespace: Namespace::Agent("ops".to_string()),
            expired_at: None,
        },
        public_key: "dGVzdC1rZXk=".to_string(),
        model_family: family.to_string(),
        model_version: None,
        trust_scores: TrustScores(BTreeMap::new()),
        status: AgentStatus::Active,
    };
    store.create_agent(&agent).expect("enroll");
}

fn config_declaring(family: Option<&str>) -> MemoryConfig {
    MemoryConfig {
        consolidation_guard: ConsolidationGuardPolicy {
            declared_consolidator_family: family.map(str::to_string),
            ..ConsolidationGuardPolicy::default()
        },
        ..MemoryConfig::default()
    }
}

fn guard_rows(store: &Store) -> u64 {
    let query = BoundQuery::new("MATCH (a:AuditEvent) WHERE a.kind = $k RETURN count(a) AS n")
        .bind_str("k", "subliminal_guard_warning")
        .expect("bind kind");
    match store.execute(&query).expect("count") {
        QueryResult::Rows(rows) => match rows.value(0, 0) {
            Some(Value::Uint(n)) => *n,
            Some(Value::Int(n)) => u64::try_from(*n).unwrap_or(0),
            _ => 0,
        },
        _ => 0,
    }
}

#[test]
fn a_single_family_deployment_warns_and_audits_at_construction() {
    let store = migrated_store();
    // The whole fleet declares the bare family; the consolidator declares the full
    // model id — the boundary-prefix rule must still recognize one family.
    enroll_with_family(&store, "claude", b"agent-1");
    enroll_with_family(&store, "claude-opus-4-8", b"agent-2");

    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        config_declaring(Some("claude-sonnet-4-6")),
        &ts(0),
    )
    .expect("memory");

    assert_eq!(
        memory.startup_warnings(),
        &[StartupWarning::SingleFamilyDeployment {
            family: "claude-sonnet-4-6".to_string()
        }],
        "the host has a typed warning to log"
    );
    assert_eq!(guard_rows(&store), 1, "the finding is audited");

    // A restart of the same deployment dedups to the same content-addressed row.
    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        config_declaring(Some("claude-sonnet-4-6")),
        &ts(5),
    )
    .expect("memory again");
    assert_eq!(memory.startup_warnings().len(), 1);
    assert_eq!(guard_rows(&store), 1, "restarts do not flood the audit log");
}

#[test]
fn a_mixed_fleet_or_no_declaration_warns_nobody() {
    let store = migrated_store();
    enroll_with_family(&store, "claude", b"agent-1");
    enroll_with_family(&store, "gpt-5", b"agent-2");

    // Mixed fleet: cross-family content exists, the per-call guard handles the rest.
    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        config_declaring(Some("claude-sonnet-4-6")),
        &ts(0),
    )
    .expect("memory");
    assert!(memory.startup_warnings().is_empty());
    assert_eq!(guard_rows(&store), 0);

    // No declared family: the check is skipped entirely.
    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        config_declaring(None),
        &ts(0),
    )
    .expect("memory");
    assert!(memory.startup_warnings().is_empty());
}

#[test]
fn an_empty_store_has_no_fleet_to_warn_about() {
    let store = migrated_store();
    let memory = Memory::new(
        Arc::clone(&store),
        FakeEmbedder::new(),
        config_declaring(Some("claude-sonnet-4-6")),
        &ts(0),
    )
    .expect("memory");
    assert!(
        memory.startup_warnings().is_empty(),
        "no enrolled agents means no single-family evidence"
    );
    assert_eq!(guard_rows(&store), 0);
}
