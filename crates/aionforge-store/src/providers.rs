//! The maintained candidate-state providers (data-model §9).
//!
//! Providers are not GQL/WAL objects — they are `Arc<dyn IndexProvider>` attached to
//! the graph at construction (and re-attached on recovery), so their specs are a
//! code-level constant the store builder wires in every boot, not a migration
//! statement. Each spec is a membership rule over labels and edge presence/absence
//! only; the engine cannot express a scalar predicate, so `current_support_facts`'
//! `status = 'active'` filter (§9) is applied at query time over the provider's
//! superset, not encoded here.

use std::sync::Arc;

use selene_core::db_string;
use selene_graph::{
    CandidateStateSpec, GraphError, IndexProvider, MaintainedCandidateStateProvider,
};

use crate::error::StoreError;
use crate::store::Store;

/// The five candidate-state specs (data-model §9), built fresh each call.
fn candidate_state_specs() -> Result<Vec<CandidateStateSpec>, StoreError> {
    let fact = db_string("Fact")?;
    let superseded_by = db_string("SUPERSEDED_BY")?;
    let contradicts = db_string("CONTRADICTS")?;

    Ok(vec![
        // current_support_facts: a Fact with no live SUPERSEDED_BY and no live
        // CONTRADICTS edge. Both edges remove the *source* (the superseded fact, and the
        // quarantined contradicting fact) per the domain edge docs, so both are
        // excluded outgoing. The `status = 'active'` half of §9 is a query-time filter
        // over this superset.
        CandidateStateSpec::new(db_string("current_support_facts")?)
            .require_label(fact.clone())
            .exclude_outgoing(superseded_by.clone())
            .exclude_outgoing(contradicts.clone()),
        // provenance_current_support_facts: the above, plus an incoming SUPPORTS and an
        // outgoing HAS_PROVENANCE grounding.
        CandidateStateSpec::new(db_string("provenance_current_support_facts")?)
            .require_label(fact.clone())
            .exclude_outgoing(superseded_by)
            .exclude_outgoing(contradicts.clone())
            .require_incoming(db_string("SUPPORTS")?)
            .require_outgoing(db_string("HAS_PROVENANCE")?),
        // scope_membership: anything with a live IN_SCOPE edge. This is the coarse
        // "in some scope" set; per-scope selection is query-time candidate-set algebra.
        CandidateStateSpec::new(db_string("scope_membership")?)
            .require_outgoing(db_string("IN_SCOPE")?),
        // recency_active: anything with a live RECENT_IN edge (coarse, like scope).
        CandidateStateSpec::new(db_string("recency_active")?)
            .require_outgoing(db_string("RECENT_IN")?),
        // unresolved_current: a Fact that nothing currently contradicts — no live
        // *incoming* CONTRADICTS. This is the deliberate dual of current_support_facts,
        // which drops the contradiction *source* (outgoing). Keeping the directions
        // opposite is what makes the §9 set algebra pay off: current_support_facts minus
        // unresolved_current is exactly the facts something contradicts but that are
        // otherwise still current — the contested incumbents the §9 "quarantine
        // reasoning" use names — while the intersection is the clean active set. Excluding
        // outgoing here instead would re-derive current_support_facts and lose that.
        CandidateStateSpec::new(db_string("unresolved_current")?)
            .require_label(fact)
            .exclude_incoming(contradicts),
    ])
}

/// Build the candidate-state provider the store attaches at construction.
pub(crate) fn candidate_state_provider() -> Result<Arc<dyn IndexProvider>, StoreError> {
    let provider = MaintainedCandidateStateProvider::new(candidate_state_specs()?)
        .map_err(GraphError::Provider)?;
    Ok(Arc::new(provider))
}

/// A maintained candidate-state set and its current size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateStateInfo {
    /// The provider's stable set name.
    pub name: String,
    /// How many nodes are currently in the set.
    pub candidate_count: usize,
}

impl Store {
    /// The current candidate-state sets and their sizes (data-model §9 introspection).
    ///
    /// # Errors
    /// Returns [`StoreError`] if the provider cannot prove it is current with the graph.
    pub fn candidate_state_infos(&self) -> Result<Vec<CandidateStateInfo>, StoreError> {
        let infos = self
            .graph()
            .vector_candidate_state_infos()
            .map_err(GraphError::Provider)?;
        Ok(infos
            .into_iter()
            .map(|info| CandidateStateInfo {
                name: info.name.as_str().to_owned(),
                candidate_count: info.candidate_count,
            })
            .collect())
    }
}
