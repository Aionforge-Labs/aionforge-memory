//! The Ed25519 crypto seams: write-provenance verification (06 §3) and substrate audit
//! signing (06 §6).
//!
//! Writes carry an Ed25519 signature over a canonical [`signing`](crate::signing)
//! payload. On the **writer channel** the substrate stores the writer's public key
//! (`Agent.public_key`) and the signature (`ProvenanceRecord.signature`) and *verifies*
//! — a writer's private key never enters the process. The one carve-out is the **audit
//! channel** (06 §6): the substrate is itself the author of the audit events it emits,
//! so it holds its own audit keypair and signs through [`AuditEventSigner`]. Both seams
//! are declared here and implemented in the trust layer (M4), so this crate stays free
//! of a crypto dependency. The verification seam is generic over the message bytes, so
//! the same primitive verifies provenance, attestation (M4.T04), and audit signatures.

use thiserror::Error;

use crate::ids::Id;
use crate::nodes::forensic::AuditEvent;

/// Resolves a writer agent's stored base64 public key by agent id.
///
/// Implemented by the trust layer over the store, so the domain seam stays free of a
/// store dependency. Returns `Ok(None)` for an agent that is not registered — the gate
/// treats an unregistered writer as a failure (fail-closed) when signed writes are on.
pub trait PublicKeyResolver: Send + Sync {
    /// The base64 public key registered for `agent_id`, or `None` if no such agent.
    fn public_key(&self, agent_id: &Id) -> Result<Option<String>, ResolveError>;
}

/// A backend failure while resolving a public key. The underlying store error is carried
/// as text so this domain seam need not name the store's error type.
#[derive(Debug, Error)]
#[error("public-key resolution failed: {0}")]
pub struct ResolveError(pub String);

/// Verifies an Ed25519 signature over arbitrary message bytes against a public key.
///
/// The key and signature are the base64 strings stored on the `Agent` (`public_key`)
/// and the `ProvenanceRecord` (`signature`); the implementation decodes and checks them.
/// Implemented by the trust layer.
pub trait SignatureVerifier: Send + Sync {
    /// Verify `signature_b64` over `message` against `public_key_b64`. Returns `Ok(())`
    /// only on a valid signature; every other outcome is a [`VerifyError`].
    fn verify(
        &self,
        public_key_b64: &str,
        signature_b64: &str,
        message: &[u8],
    ) -> Result<(), VerifyError>;
}

/// Why a signature failed to verify.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// The stored public key was not valid base64 or not a 32-byte Ed25519 key.
    #[error("malformed public key")]
    MalformedPublicKey,
    /// The signature was not valid base64 or not a 64-byte Ed25519 signature.
    #[error("malformed signature")]
    MalformedSignature,
    /// The signature did not verify against the key and message.
    #[error("signature does not verify")]
    Invalid,
}

/// Signs a substrate-authored [`AuditEvent`] over its canonical audit payload (06 §6).
///
/// The signing dual of [`SignatureVerifier`], for the one channel where the substrate is an
/// author rather than a verifier: the audit events it emits about its own operations. The
/// trait is object-safe so the store's commit functions can take an optional
/// `&dyn AuditEventSigner` and stamp signatures where the events cross into the store,
/// without the store naming a crypto crate — the same layering as the verification seam
/// (declared here, implemented in the trust layer).
///
/// The returned string is the base64 Ed25519 signature over
/// [`signing::audit_payload`](crate::signing::audit_payload). That payload excludes the
/// `signature` field itself, so the caller signs once every other field is final and stamps
/// the result onto `AuditEvent::signature` without invalidating the bytes. Implementations
/// must be deterministic (Ed25519 is, per RFC 8032): a crash-replay that rebuilds the same
/// event re-signs to identical bytes, so the store's dedup-by-id write stays a true no-op.
pub trait AuditEventSigner: Send + Sync {
    /// The base64 signature over `event`'s canonical audit payload.
    fn sign(&self, event: &AuditEvent) -> String;
}

#[cfg(test)]
mod tests {
    use super::AuditEventSigner;
    use crate::blocks::Identity;
    use crate::ids::Id;
    use crate::namespace::Namespace;
    use crate::nodes::forensic::{AuditEvent, AuditKind};
    use crate::time::Timestamp;

    /// A stand-in implementation: the seam carries no crypto of its own, so any signer
    /// shape — here a marker the assertions can recognize — satisfies it.
    struct StubSigner;

    impl AuditEventSigner for StubSigner {
        fn sign(&self, event: &AuditEvent) -> String {
            format!("stub:{:?}", event.kind)
        }
    }

    fn event() -> AuditEvent {
        let at: Timestamp = "2026-06-09T09:00:00-05:00[America/Chicago]"
            .parse()
            .expect("valid zoned datetime");
        AuditEvent {
            identity: Identity {
                id: Id::from_content_hash(b"verify-seam-test"),
                ingested_at: at.clone(),
                namespace: Namespace::System,
                expired_at: None,
            },
            kind: AuditKind::KeyRotation,
            subject_id: Id::from_content_hash(b"subject"),
            actor_id: Id::from_content_hash(b"actor"),
            payload: serde_json::json!({}),
            signature: String::new(),
            occurred_at: at,
        }
    }

    /// The seam is object-safe: the store will hold it as a trait object, so a regression
    /// to a non-object-safe shape (a generic method, a `Self` return) must fail here.
    #[test]
    fn the_signer_seam_is_object_safe() {
        let signer: Box<dyn AuditEventSigner> = Box::new(StubSigner);
        assert_eq!(signer.sign(&event()), "stub:KeyRotation");

        fn sign_through(signer: &dyn AuditEventSigner, event: &AuditEvent) -> String {
            signer.sign(event)
        }
        assert_eq!(sign_through(signer.as_ref(), &event()), "stub:KeyRotation");
    }
}
