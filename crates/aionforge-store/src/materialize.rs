//! Atomic materialization of consolidation-derived memory (write-and-consolidation
//! §2–§3, M2.T04).
//!
//! A consolidation pass reads a snapshot and returns a [`ConsolidationArtifacts`]
//! payload — the entities, facts, and edges it derived from one episode. The scheduler
//! hands that payload to [`Store::commit_consolidation_episode`](crate::Store), which
//! writes every artifact in the **same** transaction as the episode's state flip, so
//! derived memory and the flip are inseparable: a crash commits both or neither, never
//! an orphan fact and never a double-apply on re-run.
//!
//! Idempotency is content-addressed, not transaction-scoped. The pass gives each fact a
//! deterministic id (`Id::from_content_hash` over its canonical key), and
//! materialization **skips** a fact whose `(subject_id, predicate)` plus object value
//! already exists, so re-consolidating the same episode (after a crash, or because the
//! pass over-extracts) collapses to a no-op. New entities likewise dedup against the
//! committed graph by `(canonical_name, type, namespace)`; when a derived entity turns
//! out to already exist, its fresh id is remapped to the persisted one so the facts that
//! reference it land on the canonical node. The support, derivation, and mention edges
//! are created only when absent, so they accumulate across episodes without duplicating.

use std::collections::{HashMap, HashSet};

use aionforge_domain::edges::{About, Audit, DerivedFrom, Mentions, Supports};
use aionforge_domain::ids::Id;
use aionforge_domain::nodes::forensic::AuditEvent;
use aionforge_domain::nodes::semantic::{Entity, Fact};
use aionforge_domain::time::{BiTemporal, Timestamp};
use aionforge_domain::value::ObjectValue;
use selene_core::{NodeId, PropertyMap, Value, db_string};
use selene_graph::{Mutator, RowIndex, SeleneGraph};

use crate::convert::{id_value, string_value, timestamp_value};
use crate::error::StoreError;
use crate::{audit, entity, fact};

/// A fact to materialize, with the bi-temporal window for its `ABOUT` edge.
///
/// The window lives on the edge, never on the `Fact` node (currentness is edge
/// presence, 02 §4.2), so the pass carries it alongside the fact rather than inside it.
#[derive(Debug, Clone)]
pub struct MaterializedFact {
    /// The fact node to write. Its `subject_id`/entity-object id may be remapped to a
    /// deduped canonical entity at materialize time.
    pub fact: Fact,
    /// The fact's validity window, written onto the `Fact -ABOUT-> Entity` edge.
    pub about: About,
}

/// Everything one consolidation pass derived for the scheduler to commit atomically
/// with the episode flip (M2.T04).
///
/// Built by a pass via [`ConsolidationArtifacts::default`] plus field pushes (the type
/// stays `#[non_exhaustive]` so later milestones — supersession, summarization — can add
/// payload fields without breaking the seam). The scheduler merges one of these per pass
/// into a single set with [`ConsolidationArtifacts::merge`] before committing.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ConsolidationArtifacts {
    /// Entities the pass decided are new (deduped against the graph at materialize time).
    pub new_entities: Vec<Entity>,
    /// Facts to assert, each with its `ABOUT` validity window.
    pub facts: Vec<MaterializedFact>,
    /// Entity ids (new or already-existing) the episode mentions; wired as `MENTIONS`.
    pub mentioned_entities: Vec<Id>,
    /// Audit events recording the pass's decisions (e.g. `canonicalize`).
    pub audit_events: Vec<AuditEvent>,
}

impl ConsolidationArtifacts {
    /// Whether the pass derived nothing to write (the common no-op case).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.new_entities.is_empty()
            && self.facts.is_empty()
            && self.mentioned_entities.is_empty()
            && self.audit_events.is_empty()
    }

    /// Fold another pass's output into this one (the scheduler's per-episode accumulator).
    pub fn merge(&mut self, other: ConsolidationArtifacts) {
        self.new_entities.extend(other.new_entities);
        self.facts.extend(other.facts);
        self.mentioned_entities.extend(other.mentioned_entities);
        self.audit_events.extend(other.audit_events);
    }
}

/// Write every artifact into the open flip transaction, in dependency order.
///
/// Entities first (so facts can wire `ABOUT` to them), then facts (with content dedup),
/// then the support/derivation/mention edges, then the audit trail. Reads go through
/// `mutator.read()` against the committed graph, so cross-episode dedup sees prior
/// consolidations; same-transaction artifacts are deduped in memory via the seen-set and
/// the new-entity node map.
pub(crate) fn materialize_into(
    mutator: &mut Mutator<'_, '_>,
    episode_node_id: NodeId,
    artifacts: &ConsolidationArtifacts,
    now: &Timestamp,
) -> Result<(), StoreError> {
    if artifacts.is_empty() {
        return Ok(());
    }

    // 1. Entities. Dedup against the committed graph; build the fresh-id -> canonical-id
    //    remap and the canonical-id -> NodeId map for entities created in this txn.
    let mut canonical_id: HashMap<Id, Id> = HashMap::new();
    let mut node_of: HashMap<Id, NodeId> = HashMap::new();
    for entity in &artifacts.new_entities {
        if let Some((existing_id, existing_node)) = find_existing_entity(mutator.read(), entity)? {
            canonical_id.insert(entity.identity.id.clone(), existing_id.clone());
            node_of.insert(existing_id, existing_node);
        } else {
            let (labels, props) = entity::to_node(entity)?;
            let node = mutator.create_node(labels, props)?;
            canonical_id.insert(entity.identity.id.clone(), entity.identity.id.clone());
            node_of.insert(entity.identity.id.clone(), node);
        }
    }
    // Map a (possibly fresh) entity id to the persisted canonical id it resolves to.
    let canon = |id: &Id| canonical_id.get(id).cloned().unwrap_or_else(|| id.clone());

    // 2. Facts (+ ABOUT), content-deduped within the batch and against the committed graph.
    let mut seen_facts: HashSet<String> = HashSet::new();
    for materialized in &artifacts.facts {
        let subject_id = canon(&materialized.fact.subject_id);
        let object = remap_object(&materialized.fact.object, canon);

        let key = fact_dedup_key(&subject_id, &materialized.fact.predicate, &object)?;
        if !seen_facts.insert(key) {
            continue; // an exact duplicate already handled earlier in this same batch
        }

        let subject_node =
            resolve_entity_node(mutator.read(), &node_of, &subject_id)?.ok_or_else(|| {
                StoreError::invariant(format!(
                    "fact subject entity {subject_id} has no node to wire ABOUT to"
                ))
            })?;

        let fact_node = match find_existing_fact(
            mutator.read(),
            &subject_id,
            &materialized.fact.predicate,
            &object,
        )? {
            Some(node) => node, // already asserted by a prior episode; reuse it
            None => create_fact(mutator, materialized, subject_id, object, subject_node)?,
        };

        // Episode supports the fact (weight = confidence); fact derives from the episode.
        ensure_edge(
            mutator,
            Supports::LABEL,
            episode_node_id,
            fact_node,
            supports_props(materialized.fact.confidence)?,
        )?;
        ensure_edge(
            mutator,
            DerivedFrom::LABEL,
            fact_node,
            episode_node_id,
            derived_from_props(now)?,
        )?;
    }

    // 3. MENTIONS: the episode mentions each resolved entity (created only when absent).
    for mention in &artifacts.mentioned_entities {
        let entity_id = canon(mention);
        match resolve_entity_node(mutator.read(), &node_of, &entity_id)? {
            Some(entity_node) => ensure_edge(
                mutator,
                Mentions::LABEL,
                episode_node_id,
                entity_node,
                mentions_props(now)?,
            )?,
            None => tracing::warn!(
                entity = entity_id.as_str(),
                "consolidation: mentioned entity has no node; skipping MENTIONS"
            ),
        }
    }

    // 4. Audit the pass's decisions (forensic, append-only): node + AUDIT edge to episode.
    for event in &artifacts.audit_events {
        let (labels, props) = audit::to_node(event)?;
        let audit_node = mutator.create_node(labels, props)?;
        mutator.create_edge(
            db_string(Audit::LABEL)?,
            audit_node,
            episode_node_id,
            PropertyMap::from_pairs(Vec::new())?,
        )?;
    }

    Ok(())
}

/// Create a fact node with the remapped subject/object and wire its `ABOUT` edge.
fn create_fact(
    mutator: &mut Mutator<'_, '_>,
    materialized: &MaterializedFact,
    subject_id: Id,
    object: ObjectValue,
    subject_node: NodeId,
) -> Result<NodeId, StoreError> {
    // Fail closed: never persist a window whose bounds are out of order (02 §5).
    if !materialized.about.temporal.windows_ordered() {
        return Err(StoreError::invariant(
            "consolidation fact ABOUT window bounds are out of order".to_string(),
        ));
    }
    let mut fact = materialized.fact.clone();
    fact.subject_id = subject_id;
    fact.object = object;
    let (labels, props) = fact::to_node(&fact)?;
    let fact_node = mutator.create_node(labels, props)?;
    mutator.create_edge(
        db_string(About::LABEL)?,
        fact_node,
        subject_node,
        fact::about_props(&materialized.about)?,
    )?;
    Ok(fact_node)
}

/// Create an `source -label-> target` edge only if no such edge already exists.
///
/// Keeps the support/derivation/mention edges idempotent: re-running an episode, or a
/// second episode that supports the same fact, never piles up duplicate edges.
fn ensure_edge(
    mutator: &mut Mutator<'_, '_>,
    label: &str,
    source: NodeId,
    target: NodeId,
    props: PropertyMap,
) -> Result<(), StoreError> {
    let label = db_string(label)?;
    let exists = mutator
        .read()
        .outgoing_edges(source)
        .is_some_and(|adjacency| {
            adjacency
                .iter_label(&label)
                .any(|edge| edge.neighbor == target)
        });
    if !exists {
        mutator.create_edge(label, source, target, props)?;
    }
    Ok(())
}

/// Resolve an entity's canonical id to a `NodeId`: first an entity created in this txn,
/// then the committed `Entity.id` index.
fn resolve_entity_node(
    snapshot: &SeleneGraph,
    node_of: &HashMap<Id, NodeId>,
    id: &Id,
) -> Result<Option<NodeId>, StoreError> {
    if let Some(node) = node_of.get(id) {
        return Ok(Some(*node));
    }
    let label = db_string(Entity::LABEL)?;
    let prop = db_string("id")?;
    let value = id_value(id)?;
    let Some(rows) = snapshot.nodes_with_property_eq(&label, &prop, &value) else {
        return Ok(None);
    };
    Ok(rows
        .iter()
        .find_map(|row| snapshot.node_id_for_row(RowIndex::new(row))))
}

/// Find an entity already in the committed graph with the same canonical name, type, and
/// namespace, returning its id and node. `canonical_name` is indexed, so this is a probe.
fn find_existing_entity(
    snapshot: &SeleneGraph,
    entity: &Entity,
) -> Result<Option<(Id, NodeId)>, StoreError> {
    let label = db_string(Entity::LABEL)?;
    let name_prop = db_string("canonical_name")?;
    let value = string_value(&entity.canonical_name)?;
    let Some(rows) = snapshot.nodes_with_property_eq(&label, &name_prop, &value) else {
        return Ok(None);
    };
    for row in rows.iter() {
        let Some(node) = snapshot.node_id_for_row(RowIndex::new(row)) else {
            continue;
        };
        let Some(props) = snapshot.node_properties(node) else {
            continue;
        };
        let candidate = entity::from_properties(props)?;
        if candidate.entity_type == entity.entity_type
            && candidate.identity.namespace == entity.identity.namespace
        {
            return Ok(Some((candidate.identity.id, node)));
        }
    }
    Ok(None)
}

/// Find a fact already asserted with this `(subject_id, predicate)` and object value.
/// `subject_id` is indexed, so this probes the bounded subject set and compares in Rust
/// (`Fact.id` is unique but not indexed, so dedup is by value, not by an id scan).
fn find_existing_fact(
    snapshot: &SeleneGraph,
    subject_id: &Id,
    predicate: &str,
    object: &ObjectValue,
) -> Result<Option<NodeId>, StoreError> {
    let label = db_string(Fact::LABEL)?;
    let subject_prop = db_string("subject_id")?;
    let value = id_value(subject_id)?;
    let Some(rows) = snapshot.nodes_with_property_eq(&label, &subject_prop, &value) else {
        return Ok(None);
    };
    for row in rows.iter() {
        let Some(node) = snapshot.node_id_for_row(RowIndex::new(row)) else {
            continue;
        };
        let Some(props) = snapshot.node_properties(node) else {
            continue;
        };
        let candidate = fact::from_properties(props)?;
        if candidate.predicate == predicate && candidate.object == *object {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// Remap an entity-typed object to its canonical entity id; literals pass through.
fn remap_object(object: &ObjectValue, canon: impl Fn(&Id) -> Id) -> ObjectValue {
    match object {
        ObjectValue::Entity(id) => ObjectValue::Entity(canon(id)),
        other => other.clone(),
    }
}

/// A stable string key over `(subject_id, predicate, object)` for in-batch dedup.
fn fact_dedup_key(
    subject_id: &Id,
    predicate: &str,
    object: &ObjectValue,
) -> Result<String, StoreError> {
    let object_json = serde_json::to_string(object)?;
    Ok(format!(
        "{}\u{1f}{}\u{1f}{}",
        subject_id.as_str(),
        predicate,
        object_json
    ))
}

/// The `SUPPORTS` edge property map (the fact's support weight).
fn supports_props(weight: f64) -> Result<PropertyMap, StoreError> {
    Ok(PropertyMap::from_pairs(vec![(
        db_string("weight")?,
        Value::Float(weight),
    )])?)
}

/// The `DERIVED_FROM` edge property map (the derivation instant).
fn derived_from_props(now: &Timestamp) -> Result<PropertyMap, StoreError> {
    Ok(PropertyMap::from_pairs(vec![(
        db_string("derived_at")?,
        timestamp_value(now),
    )])?)
}

/// The `MENTIONS` edge property map (an open validity window opened at `now`).
fn mentions_props(now: &Timestamp) -> Result<PropertyMap, StoreError> {
    let temporal = BiTemporal {
        valid_from: now.clone(),
        valid_to: None,
        ingested_at: now.clone(),
        expired_at: None,
    };
    Ok(PropertyMap::from_pairs(fact::bitemporal_pairs(&temporal))?)
}
