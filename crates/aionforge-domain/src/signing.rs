//! Canonical byte encodings for provenance and attestation signatures (02 §10).
//!
//! Provenance and attestation signatures are computed over a fixed, versioned
//! canonical byte encoding so verification is reproducible across writers and
//! releases. This module produces only the *payload bytes*; the Ed25519 signing
//! and verification live in the trust layer (M4/M6), keeping this crate free of
//! I/O and crypto. The encoding is domain-separated (a per-purpose tag) and
//! length-prefixed (a `u32` before each field), so neither a cross-protocol reuse
//! nor a field-boundary ambiguity can produce a colliding payload.

use crate::ids::Id;
use crate::time::Timestamp;

/// The version byte prefixing every canonical signing payload.
///
/// Bump this — and the domain-separation tags — whenever the layout changes, so a
/// signature made under one layout can never validate under another.
pub const SIGNING_ENCODING_VERSION: u8 = 1;

const PROVENANCE_TAG: &str = "aionforge.provenance.v1";
const ATTESTATION_TAG: &str = "aionforge.attestation.v1";

/// The canonical provenance signing payload over `(subject_id, writer_agent_id,
/// ingested_at)` (02 §10).
///
/// The writer signs these bytes; verification recomputes them from the stored
/// `ProvenanceRecord` fields and checks them against the writer's public key.
#[must_use]
pub fn provenance_payload(
    subject_id: &Id,
    writer_agent_id: &Id,
    ingested_at: &Timestamp,
) -> Vec<u8> {
    encode(
        PROVENANCE_TAG,
        &[subject_id.as_str(), writer_agent_id.as_str()],
        ingested_at,
    )
}

/// The canonical attestation signing payload over `(fact_id, attester_id,
/// attested_at)` (02 §10).
///
/// The attester signs these bytes; verification recomputes them from the stored
/// `ATTESTED_BY` edge fields and checks them against the attester's public key.
#[must_use]
pub fn attestation_payload(fact_id: &Id, attester_id: &Id, attested_at: &Timestamp) -> Vec<u8> {
    encode(
        ATTESTATION_TAG,
        &[fact_id.as_str(), attester_id.as_str()],
        attested_at,
    )
}

/// Encode a versioned, domain-separated, length-prefixed payload: the version
/// byte, then the tag, then each string field, then the instant as big-endian
/// epoch milliseconds.
fn encode(tag: &str, fields: &[&str], instant: &Timestamp) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(SIGNING_ENCODING_VERSION);
    push_field(&mut buf, tag.as_bytes());
    for field in fields {
        push_field(&mut buf, field.as_bytes());
    }
    let millis = instant.timestamp().as_millisecond();
    buf.extend_from_slice(&millis.to_be_bytes());
    buf
}

/// Append a `u32` big-endian length prefix followed by the bytes, so two adjacent
/// fields can never be reinterpreted as a single field of a different split.
fn push_field(buf: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("signing field length fits in u32");
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp as Instant;
    use jiff::tz::TimeZone;

    fn ts(ms: i64) -> Timestamp {
        Instant::from_millisecond(ms)
            .unwrap()
            .to_zoned(TimeZone::UTC)
    }

    fn id(seed: u128) -> Id {
        Id::parse(ulid::Ulid::from_parts(0, seed).to_string()).unwrap()
    }

    #[test]
    fn payload_is_deterministic() {
        let a = provenance_payload(&id(1), &id(2), &ts(1_700_000_000_000));
        let b = provenance_payload(&id(1), &id(2), &ts(1_700_000_000_000));
        assert_eq!(a, b);
    }

    #[test]
    fn payload_starts_with_the_version_byte() {
        let payload = provenance_payload(&id(1), &id(2), &ts(0));
        assert_eq!(payload[0], SIGNING_ENCODING_VERSION);
    }

    #[test]
    fn distinct_inputs_yield_distinct_payloads() {
        let base = provenance_payload(&id(1), &id(2), &ts(10));
        assert_ne!(base, provenance_payload(&id(9), &id(2), &ts(10)));
        assert_ne!(base, provenance_payload(&id(1), &id(9), &ts(10)));
        assert_ne!(base, provenance_payload(&id(1), &id(2), &ts(11)));
    }

    #[test]
    fn domain_separation_prevents_cross_protocol_reuse() {
        let prov = provenance_payload(&id(1), &id(2), &ts(5));
        let att = attestation_payload(&id(1), &id(2), &ts(5));
        assert_ne!(prov, att);
    }

    #[test]
    fn length_prefix_prevents_field_boundary_collisions() {
        let split_a = encode("t", &["ab", "c"], &ts(0));
        let split_b = encode("t", &["a", "bc"], &ts(0));
        assert_ne!(split_a, split_b);
    }
}
