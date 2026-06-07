//! The hybrid retriever: router → signals → fusion → recall bundle (03).
//!
//! [`HybridRetriever`] implements the domain [`Retriever`] contract. It classifies the
//! query, runs the weighted signals the mode profile calls for, fuses them, authorizes
//! and diversity-caps the candidate set, and assembles the [`RecallBundle`]. It is
//! generic over the [`Embedder`] seam; when the embedder is unreachable the dense
//! signal drops out and retrieval degrades to the rest, flagged in the explanation
//! (03 §6, §8.1).
//!
//! In this milestone only the lexical and dense signals exist, so those are the
//! signals that run; the graph, recency, and trust signals land with their tasks.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use aionforge_domain::contracts::{Embedder, Retriever};
use aionforge_domain::ids::SerializationId;
use aionforge_domain::namespace::Namespace;
use aionforge_domain::nodes::episodic::{Episode, Role};
use aionforge_store::{SearchKind, Store};

use crate::bundle::{RecallBundle, RecallExplanation, StageTimings, StructuredEntry, render};
use crate::error::RetrievalError;
use crate::fusion::{DEFAULT_RRF_K, FusedCandidate, WeightedRanking, fuse};
use crate::query::RecallQuery;
use crate::router::{profile_for, route};
use crate::signals::{Signal, dense_ranking, lexical_ranking};

/// The serialization-id kind tag for an episode (02 §10).
const SERIALIZATION_KIND_TAG: &str = "episode";

/// Tuning for the retriever that is not per-query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrieverConfig {
    /// How many candidates each signal pulls before fusion, when a query does not set
    /// its own fan-out. A wider fan-out gives fusion and the diversity cap more to
    /// work with at the cost of more candidate reads.
    pub default_fanout: usize,
}

impl Default for RetrieverConfig {
    fn default() -> Self {
        Self { default_fanout: 50 }
    }
}

/// A hybrid retriever over a shared store and an embedder.
pub struct HybridRetriever<E> {
    store: Arc<Store>,
    embedder: E,
    config: RetrieverConfig,
}

impl<E: Embedder> HybridRetriever<E> {
    /// Build a retriever over a shared store, an embedder, and its config.
    #[must_use]
    pub fn new(store: Arc<Store>, embedder: E, config: RetrieverConfig) -> Self {
        Self {
            store,
            embedder,
            config,
        }
    }

    /// Run one recall.
    async fn run(&self, query: RecallQuery) -> Result<RecallBundle, RetrievalError> {
        let started = Instant::now();
        let deadline = query.options.deadline.map(|budget| started + budget);

        // 1. Classify (or honor an override).
        let profile = query
            .options
            .mode_override
            .map_or_else(|| route(&query.text), profile_for);
        let classify_ms = started.elapsed().as_millis();
        bail_if_past(deadline)?;

        // 2. Run the signals the profile weights call for. Lexical and dense are the
        //    signals implemented this milestone.
        let signals_started = Instant::now();
        let fanout = effective_fanout(&query, &self.config);
        let mut rankings: Vec<WeightedRanking> = Vec::new();
        let mut signals_run: Vec<Signal> = Vec::new();
        let mut embedder_available = true;

        if profile.weights.lexical > 0.0 {
            let ranking = lexical_ranking(&self.store, SearchKind::Episode, &query.text, fanout)?;
            rankings.push(WeightedRanking::new(profile.weights.lexical, ranking));
            signals_run.push(Signal::Lexical);
        }
        bail_if_past(deadline)?;

        if profile.weights.dense > 0.0 {
            let dense = dense_ranking(
                &self.store,
                &self.embedder,
                SearchKind::Episode,
                &query.text,
                fanout,
                profile.exact_rerank,
            )
            .await?;
            embedder_available = dense.embedder_available;
            if dense.embedder_available {
                rankings.push(WeightedRanking::new(profile.weights.dense, dense.ranking));
                signals_run.push(Signal::Dense);
            }
        }
        let signals_ms = signals_started.elapsed().as_millis();
        bail_if_past(deadline)?;

        // 3. Fuse, then resolve, authorize, and diversity-cap the candidates.
        let assemble_started = Instant::now();
        let fused = fuse(&rankings, DEFAULT_RRF_K);
        let selection = self.select(&query, fused)?;

        // 4. Structured view stays in score order; the rendered view re-sorts by
        //    serialization id so the same set renders byte-identically (03 §6).
        let structured = selection.entries;
        let mut rendered_order = structured.clone();
        rendered_order.sort_by(|a, b| a.serialization_id.cmp(&b.serialization_id));
        let rendered = render(&rendered_order);
        let assemble_ms = assemble_started.elapsed().as_millis();

        let explanation = RecallExplanation {
            class: profile.class,
            weights: profile.weights,
            signals_run,
            embedder_available,
            candidates_considered: selection.considered,
            returned: structured.len(),
            timings_ms: StageTimings {
                classify: classify_ms,
                signals: signals_ms,
                assemble: assemble_ms,
            },
        };

        Ok(RecallBundle {
            structured,
            rendered,
            explanation,
        })
    }

    /// Resolve fused candidates to authorized episodes, applying the session-diversity
    /// cap and filling from the spill only when the bundle is under-filled (03 §6).
    fn select(
        &self,
        query: &RecallQuery,
        fused: Vec<FusedCandidate>,
    ) -> Result<Selection, RetrievalError> {
        let cap = query.options.session_diversity_cap;
        let mut primary: Vec<StructuredEntry> = Vec::new();
        let mut spill: Vec<StructuredEntry> = Vec::new();
        let mut per_session: HashMap<Option<String>, usize> = HashMap::new();
        let mut considered = 0usize;

        for candidate in fused {
            if primary.len() >= query.limit {
                break;
            }
            let Some(episode) = self.store.episode_by_node_id(candidate.node)? else {
                continue;
            };
            if !admit(query, &episode) {
                continue;
            }
            considered += 1;
            let entry = entry_from(&episode, &candidate);
            let session = episode
                .session_id
                .as_ref()
                .map(|id| id.as_str().to_string());
            let seen = per_session.entry(session).or_insert(0);
            if cap == 0 || *seen < cap {
                *seen += 1;
                primary.push(entry);
            } else {
                spill.push(entry);
            }
        }

        // Under-filled: top up from the spilled overflow, in score order.
        if primary.len() < query.limit {
            for entry in spill {
                if primary.len() >= query.limit {
                    break;
                }
                primary.push(entry);
            }
        }

        Ok(Selection {
            entries: primary,
            considered,
        })
    }
}

impl<E: Embedder> Retriever for HybridRetriever<E> {
    type Query = RecallQuery;
    type Bundle = RecallBundle;
    type Error = RetrievalError;

    fn recall(
        &self,
        query: Self::Query,
    ) -> impl Future<Output = Result<Self::Bundle, Self::Error>> + Send {
        self.run(query)
    }
}

/// The chosen entries plus how many candidates were considered.
struct Selection {
    entries: Vec<StructuredEntry>,
    considered: usize,
}

/// True once `deadline` has passed.
fn bail_if_past(deadline: Option<Instant>) -> Result<(), RetrievalError> {
    if deadline.is_some_and(|at| Instant::now() >= at) {
        Err(RetrievalError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

/// Candidates per signal: the query's fan-out, else the configured default, never
/// below the requested bundle size.
fn effective_fanout(query: &RecallQuery, config: &RetrieverConfig) -> usize {
    let base = if query.options.fanout > 0 {
        query.options.fanout
    } else {
        config.default_fanout
    };
    base.max(query.limit).max(1)
}

/// Whether an episode may surface for this query: not a system-role message, active
/// unless history was asked for, and visible to the viewer's namespace (03 §5, §8).
fn admit(query: &RecallQuery, episode: &Episode) -> bool {
    if episode.role == Role::System {
        return false;
    }
    if !query.options.include_expired && episode.identity.expired_at.is_some() {
        return false;
    }
    visible_to(&query.viewer, &episode.identity.namespace)
}

/// Namespace authorization: a viewer sees the global namespace and its own; private
/// content from any other namespace never surfaces (06 §1). Team membership is not
/// modeled yet, so a team namespace is visible only to that exact namespace.
fn visible_to(viewer: &Namespace, candidate: &Namespace) -> bool {
    matches!(candidate, Namespace::Global) || candidate == viewer
}

/// Build a structured entry from an episode and its fused candidate.
fn entry_from(episode: &Episode, candidate: &FusedCandidate) -> StructuredEntry {
    StructuredEntry {
        id: episode.identity.id.clone(),
        serialization_id: SerializationId::derive(
            SERIALIZATION_KIND_TAG,
            episode.content_hash.as_str().as_bytes(),
        ),
        namespace: episode.identity.namespace.clone(),
        role: episode.role,
        ingested_at: episode.identity.ingested_at.clone(),
        expired_at: episode.identity.expired_at.clone(),
        trust: episode.stats.trust,
        score: candidate.score,
        contributions: candidate.contributions.clone(),
        content: episode.content.clone(),
    }
}
