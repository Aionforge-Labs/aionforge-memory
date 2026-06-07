//! Translation between a domain [`ProvenanceRecord`] and a selene-db node (02 §4.10).
//!
//! A forensic kind: it carries only the [`Identity`] block (no `Stats`), so the
//! translation is the identity columns plus the proof fields. `source_episode_ids`
//! is a nullable `LIST<STRING>` — an empty list is the canonical "absent" value, so
//! it is omitted when empty (it reads back as empty).

use aionforge_domain::blocks::Identity;
use aionforge_domain::nodes::forensic::ProvenanceRecord;
use selene_core::{DbString, LabelSet, PropertyMap, Value, db_string};

use crate::convert::{
    as_f64, as_id, as_id_list, as_namespace, as_str, as_timestamp, id_list_value, id_value, key,
    namespace_value, string_value, timestamp_value,
};
use crate::error::StoreError;

const ID: &str = "id";
const INGESTED_AT: &str = "ingested_at";
const NAMESPACE: &str = "namespace";
const EXPIRED_AT: &str = "expired_at";
const SUBJECT_ID: &str = "subject_id";
const WRITER_AGENT_ID: &str = "writer_agent_id";
const SIGNATURE: &str = "signature";
const SOURCE_EPISODE_IDS: &str = "source_episode_ids";
const MODEL_FAMILY: &str = "model_family";
const MODEL_VERSION: &str = "model_version";
const TRUST_AT_WRITE: &str = "trust_at_write";

/// The selene-db node label for a provenance record.
pub(crate) fn label() -> Result<LabelSet, StoreError> {
    Ok(LabelSet::single(db_string(ProvenanceRecord::LABEL)?))
}

/// Translate a [`ProvenanceRecord`] into `(labels, properties)` for `create_node`.
pub(crate) fn to_node(record: &ProvenanceRecord) -> Result<(LabelSet, PropertyMap), StoreError> {
    let mut pairs: Vec<(DbString, Value)> = Vec::with_capacity(11);

    pairs.push((key(ID)?, id_value(&record.identity.id)?));
    pairs.push((
        key(INGESTED_AT)?,
        timestamp_value(&record.identity.ingested_at),
    ));
    pairs.push((
        key(NAMESPACE)?,
        namespace_value(&record.identity.namespace)?,
    ));
    if let Some(expired_at) = &record.identity.expired_at {
        pairs.push((key(EXPIRED_AT)?, timestamp_value(expired_at)));
    }
    pairs.push((key(SUBJECT_ID)?, id_value(&record.subject_id)?));
    pairs.push((key(WRITER_AGENT_ID)?, id_value(&record.writer_agent_id)?));
    pairs.push((key(SIGNATURE)?, string_value(&record.signature)?));
    if !record.source_episode_ids.is_empty() {
        pairs.push((
            key(SOURCE_EPISODE_IDS)?,
            id_list_value(&record.source_episode_ids)?,
        ));
    }
    pairs.push((key(MODEL_FAMILY)?, string_value(&record.model_family)?));
    if let Some(version) = &record.model_version {
        pairs.push((key(MODEL_VERSION)?, string_value(version)?));
    }
    pairs.push((key(TRUST_AT_WRITE)?, Value::Float(record.trust_at_write)));

    Ok((label()?, PropertyMap::from_pairs(pairs)?))
}

/// Reconstruct a [`ProvenanceRecord`] from a node's stored property map.
pub(crate) fn from_properties(props: &PropertyMap) -> Result<ProvenanceRecord, StoreError> {
    let get =
        |name: &str| -> Result<Option<&Value>, StoreError> { Ok(props.get(&db_string(name)?)) };
    let require = |name: &str| -> Result<&Value, StoreError> {
        get(name)?.ok_or_else(|| StoreError::decode(format!("missing required property `{name}`")))
    };

    Ok(ProvenanceRecord {
        identity: Identity {
            id: as_id(require(ID)?)?,
            ingested_at: as_timestamp(require(INGESTED_AT)?)?,
            namespace: as_namespace(require(NAMESPACE)?)?,
            expired_at: get(EXPIRED_AT)?.map(as_timestamp).transpose()?,
        },
        subject_id: as_id(require(SUBJECT_ID)?)?,
        writer_agent_id: as_id(require(WRITER_AGENT_ID)?)?,
        signature: as_str(require(SIGNATURE)?)?.to_string(),
        source_episode_ids: get(SOURCE_EPISODE_IDS)?
            .map(as_id_list)
            .transpose()?
            .unwrap_or_default(),
        model_family: as_str(require(MODEL_FAMILY)?)?.to_string(),
        model_version: get(MODEL_VERSION)?
            .map(as_str)
            .transpose()?
            .map(ToString::to_string),
        trust_at_write: as_f64(require(TRUST_AT_WRITE)?)?,
    })
}
