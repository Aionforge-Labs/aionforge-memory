//! The typed object value of a semantic fact (02 §4.2).

use serde::{Deserialize, Serialize};

use crate::ids::Id;
use crate::time::Timestamp;

/// The kind tag of a fact's object, recorded alongside the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// The object is another canonical entity (referenced by id).
    Entity,
    /// A string literal.
    #[serde(rename = "string")]
    Text,
    /// A numeric literal.
    Number,
    /// A boolean literal.
    Bool,
    /// A timestamp literal.
    #[serde(rename = "datetime")]
    DateTime,
    /// A structured JSON value.
    Json,
}

/// The typed object value of a fact.
///
/// Serialized adjacently as `{ "kind": ..., "value": ... }` so the kind tag is
/// explicit and tuple/newtype variants serialize cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ObjectValue {
    /// A reference to a canonical entity.
    Entity(Id),
    /// A string value.
    #[serde(rename = "string")]
    Text(String),
    /// A numeric value.
    Number(f64),
    /// A boolean value.
    Bool(bool),
    /// A timestamp value.
    #[serde(rename = "datetime")]
    DateTime(Timestamp),
    /// A structured JSON value (value-equality only; not order-comparable).
    Json(serde_json::Value),
}

impl ObjectValue {
    /// The kind tag for this value.
    #[must_use]
    pub fn kind(&self) -> ObjectKind {
        match self {
            ObjectValue::Entity(_) => ObjectKind::Entity,
            ObjectValue::Text(_) => ObjectKind::Text,
            ObjectValue::Number(_) => ObjectKind::Number,
            ObjectValue::Bool(_) => ObjectKind::Bool,
            ObjectValue::DateTime(_) => ObjectKind::DateTime,
            ObjectValue::Json(_) => ObjectKind::Json,
        }
    }
}
