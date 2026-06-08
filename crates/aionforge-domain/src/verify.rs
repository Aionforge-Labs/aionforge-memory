//! The signature-verification seam for Ed25519 provenance (06 §3).
//!
//! Writes carry an Ed25519 signature over a canonical [`signing`](crate::signing)
//! payload. The substrate stores the writer's public key (`Agent.public_key`) and the
//! signature (`ProvenanceRecord.signature`) and *verifies* — a private key never enters
//! the process. This module declares the verification seam and its typed error; the
//! Ed25519 implementation lives in the trust layer (M4), so this crate stays free of a
//! crypto dependency. The seam is generic over the message bytes, so the same primitive
//! verifies provenance now and attestation (M4.T04) later.

use thiserror::Error;

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
