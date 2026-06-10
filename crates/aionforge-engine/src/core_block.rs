//! The core-block facade (05 §4, M5.T04): namespace-authorized genesis create, the
//! one-call attested edit, and visible-set-scoped reads.
//!
//! Split out of `lib.rs` (which sits against the file-size cap), mirroring the erasure
//! facade. Creation is the **un-attested** half of the identity contract — a genesis
//! block has no prior identity to drift from, so the gate on it is namespace
//! authorization (the same injected [`Authorizer`](aionforge_domain::authz::Authorizer)
//! that bounds capture) plus the `core_edit` genesis audit. Edits route through the
//! always-constructed [`CoreEditor`](aionforge_trust::CoreEditor), which rules on the
//! editor's namespace authority first and the attestation quorum second — attesters
//! vouch for content, never for authority. Reads are scoped by the principal's visible
//! set, like every read surface (06 §1).

use aionforge_domain::authz::Principal;
use aionforge_domain::blocks::{Identity, Stats};
use aionforge_domain::contracts::Embedder;
use aionforge_domain::embedding::{EmbedderModel, Embedding};
use aionforge_domain::ids::{ContentHash, Id};
use aionforge_domain::namespace::Namespace;
use aionforge_domain::nodes::core::{BlockKind, CoreBlock};
use aionforge_domain::nodes::forensic::{AuditEvent, AuditKind};
use aionforge_domain::time::Timestamp;

use crate::{CoreEditOutcome, CoreEditRequest, EngineError, Memory};

/// What a genesis create supplies: where the block lives and what it says. The block's
/// one stable id, its stats block, and the genesis audit are the facade's to mint —
/// the id is generated here and never changes for the block's life (05 §4), and the
/// drift baseline starts empty (computing it is the drift detector's privileged call,
/// never the writer's).
#[derive(Debug, Clone)]
pub struct CoreBlockDraft {
    /// The namespace the block lives in; the principal must hold write authority.
    pub namespace: Namespace,
    /// The block body.
    pub content: String,
    /// The block's category (persona / commitment / redline).
    pub block_kind: BlockKind,
    /// Sensitivity classification, driving the edit-time attestation requirement.
    pub sensitivity: Option<String>,
    /// Write-time importance, validated finite in `[0, 1]`. Identity-tier blocks
    /// conventionally sit high — they anchor who the agent is.
    pub importance: f64,
    /// Writer trust, validated finite in `[0, 1]` — caller-supplied like the capture
    /// path's writer context. Deliberately *louder* than capture, which clamps: a
    /// capture absorbs arbitrary writer input mid-conversation and degrades
    /// gracefully, but a core-block create is a deliberate host act on the identity
    /// tier, where a wild stat is a caller bug worth surfacing, not smoothing.
    pub trust: f64,
    /// The content's embedding with its model identity, or `None` to store the block
    /// unembedded (recall's always-include path does not depend on it).
    pub embedding: Option<(Embedding, EmbedderModel)>,
}

/// The outcome of a genesis create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreBlockCreate {
    /// The block and its `core_edit` genesis audit were committed together.
    Created {
        /// The block's one stable id, minted by this create.
        block_id: Id,
        /// The genesis audit row's id.
        audit_id: Id,
    },
    /// The namespace authority refused the write; the denial is audited under the one
    /// cross-namespace `namespace_denied` kind and nothing was written.
    Unauthorized {
        /// The namespace the principal may not write.
        namespace: Namespace,
    },
}

impl<E: Embedder> Memory<E> {
    /// Create a core block (05 §4): the un-attested genesis, gated by namespace
    /// authorization. One commit writes the node plus its `core_edit` genesis audit;
    /// `CoreBlock.id` is `UNIQUE`, so the minted id can never silently rewrite an
    /// existing block. `now` is the caller's clock — the facade keeps no ambient one.
    ///
    /// # Errors
    /// Returns [`EngineError::Config`] for an out-of-range draft stat, or
    /// [`EngineError::Store`] if the commit fails. A namespace refusal is the typed
    /// [`CoreBlockCreate::Unauthorized`], audited, never an error.
    pub fn create_core_block(
        &self,
        principal: &Principal,
        draft: CoreBlockDraft,
        now: &Timestamp,
    ) -> Result<CoreBlockCreate, EngineError> {
        for (name, value) in [("importance", draft.importance), ("trust", draft.trust)] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(EngineError::Config(format!(
                    "core-block draft {name} must be a finite value in [0, 1]"
                )));
            }
        }
        if let Err(denied) = self.authorizer.authorize_write(principal, &draft.namespace) {
            // The capture path's namespace_denied shape (06 §1, 07 §T9): the system
            // namespace, the one queryable kind for every cross-namespace attempt. A
            // refused create has no memory subject, so the subject is the agent.
            self.store.commit_audit(&AuditEvent {
                identity: Identity {
                    id: Id::generate(),
                    ingested_at: now.clone(),
                    namespace: Namespace::System,
                    expired_at: None,
                },
                kind: AuditKind::NamespaceDenied,
                subject_id: principal.agent_id,
                actor_id: principal.agent_id,
                payload: serde_json::json!({
                    "requested_namespace": denied.target,
                    "reason": denied.reason.as_str(),
                    "agent": denied.agent,
                    "surface": "core_block_create",
                }),
                signature: String::new(),
                occurred_at: now.clone(),
            })?;
            return Ok(CoreBlockCreate::Unauthorized {
                namespace: draft.namespace,
            });
        }

        let block_id = Id::generate();
        let (embedding, embedder_model) = match draft.embedding {
            Some((embedding, model)) => (Some(embedding), Some(model)),
            None => (None, None),
        };
        let block = CoreBlock {
            identity: Identity {
                id: block_id,
                ingested_at: now.clone(),
                namespace: draft.namespace.clone(),
                expired_at: None,
            },
            stats: Stats {
                importance: draft.importance,
                trust: draft.trust,
                last_access: now.clone(),
                access_count_recent: 0,
                referenced_count: 0,
                surprise: 0.0,
                is_pinned: false,
            },
            content: draft.content,
            block_kind: draft.block_kind,
            sensitivity: draft.sensitivity,
            drift_baseline: None,
            embedding,
            embedder_model,
        };
        let audit = AuditEvent {
            // Generated, like the applied-edit row's per-verdict id: create
            // idempotency lives in the UNIQUE block id, which fails the whole commit
            // (audit included) on a duplicate.
            identity: Identity {
                id: Id::generate(),
                ingested_at: now.clone(),
                namespace: draft.namespace,
                expired_at: None,
            },
            kind: AuditKind::CoreEdit,
            subject_id: block_id,
            actor_id: principal.agent_id,
            payload: serde_json::json!({
                "outcome": "created",
                "editor_id": principal.agent_id.to_string(),
                "block_kind": block.block_kind,
                "sensitivity": block.sensitivity,
                "new_content_hash": ContentHash::of(block.content.as_bytes()).as_str(),
            }),
            signature: String::new(),
            occurred_at: now.clone(),
        };
        let audit_id = audit.identity.id;
        self.store.create_core_block(&block, &audit)?;
        Ok(CoreBlockCreate::Created { block_id, audit_id })
    }

    /// Apply one attested whole-value edit through the always-on gate (05 §4): the
    /// one-call host-coordinated surface — the caller collected the editor's and the
    /// attesters' transition-bound signatures out-of-band and presents them together.
    /// The injected namespace authority rules first; every refusal is typed and every
    /// gate rejection audited.
    ///
    /// # Errors
    /// Returns [`EngineError::CoreEdit`] if a store read/write fails or key resolution
    /// hits a backend fault. Security refusals are never errors — they are the typed
    /// [`CoreEditOutcome`] variants.
    pub fn edit_core_block(
        &self,
        principal: &Principal,
        request: &CoreEditRequest,
    ) -> Result<CoreEditOutcome, EngineError> {
        Ok(self
            .core_editor
            .edit(principal, self.authorizer.as_ref(), request)?)
    }

    /// Read one core block by id, scoped to the principal's visible set (06 §1).
    /// Returns the block live or retired — the stable id names it for its whole life —
    /// or `None` when it does not exist or sits outside the principal's view (the two
    /// are deliberately indistinguishable; the read is no namespace oracle).
    ///
    /// # Errors
    /// Returns [`EngineError::Store`] if the read fails.
    pub fn core_block(
        &self,
        principal: &Principal,
        id: &Id,
    ) -> Result<Option<CoreBlock>, EngineError> {
        let Some(block) = self.store.core_block_by_id(id)? else {
            return Ok(None);
        };
        let visible = self.authorizer.visible_namespaces(principal);
        Ok(visible.contains(&block.identity.namespace).then_some(block))
    }

    /// The live core blocks the principal can see, in stable id order — the read the
    /// recall pre-pass and a session-start briefing share (05 §4).
    ///
    /// # Errors
    /// Returns [`EngineError::Store`] if the scan fails.
    pub fn live_core_blocks(&self, principal: &Principal) -> Result<Vec<CoreBlock>, EngineError> {
        let visible = self.authorizer.visible_namespaces(principal);
        Ok(self
            .store
            .live_core_blocks()?
            .into_iter()
            .filter(|block| visible.contains(&block.identity.namespace))
            .collect())
    }
}
