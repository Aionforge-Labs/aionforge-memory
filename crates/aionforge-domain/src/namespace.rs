//! Trust and visibility namespaces (02 §11).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A trust/visibility boundary. Serialized as its canonical string form
/// (`agent:<id>`, `team:<id>`, `global`, `system`) so it is a cheap, queryable
/// scalar in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// Private to a single agent (`agent:<id>`). Untrusted writes land here.
    Agent(String),
    /// Shared within a team, attested (`team:<id>`).
    Team(String),
    /// Promoted, quorum-gated (`global`).
    Global,
    /// Substrate-internal (`system`): audit, control nodes, system-role episodes.
    System,
}

impl Namespace {
    /// True for the private per-agent namespace.
    #[must_use]
    pub fn is_private(&self) -> bool {
        matches!(self, Namespace::Agent(_))
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Namespace::Agent(id) => write!(f, "agent:{id}"),
            Namespace::Team(id) => write!(f, "team:{id}"),
            Namespace::Global => f.write_str("global"),
            Namespace::System => f.write_str("system"),
        }
    }
}

impl FromStr for Namespace {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "global" => Ok(Namespace::Global),
            "system" => Ok(Namespace::System),
            _ => {
                if let Some(id) = s.strip_prefix("agent:")
                    && !id.is_empty()
                {
                    return Ok(Namespace::Agent(id.to_string()));
                }
                if let Some(id) = s.strip_prefix("team:")
                    && !id.is_empty()
                {
                    return Ok(Namespace::Team(id.to_string()));
                }
                Err(DomainError::InvalidNamespace(s.to_string()))
            }
        }
    }
}

impl Serialize for Namespace {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
