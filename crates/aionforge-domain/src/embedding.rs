//! Dense embedding values and embedder model identity.

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// A dense embedding: a finite, non-empty `f32` vector (selene-db `VECTOR`).
///
/// The storage layer translates this to the engine's native vector value; this
/// type guarantees the engine's invariants (non-empty, all-finite) up front.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Embedding(Vec<f32>);

impl Embedding {
    /// Construct an embedding, validating that it is non-empty and all-finite.
    ///
    /// # Errors
    /// Returns [`DomainError::InvalidEmbedding`] if empty or if any component is
    /// NaN or infinite.
    pub fn new(components: Vec<f32>) -> Result<Self, DomainError> {
        if components.is_empty() {
            return Err(DomainError::InvalidEmbedding("empty".to_string()));
        }
        if components.iter().any(|c| !c.is_finite()) {
            return Err(DomainError::InvalidEmbedding(
                "non-finite component".to_string(),
            ));
        }
        Ok(Self(components))
    }

    /// The number of dimensions.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.0.len()
    }

    /// The components.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Return a unit-L2-normalized copy.
    ///
    /// Cosine is the default metric and the engine has no normalization primitive,
    /// so normalization is a write-path obligation of the embedding client. A
    /// zero-norm vector is returned unchanged (the engine rejects it at scoring).
    #[must_use]
    pub fn normalized(&self) -> Self {
        let norm = self
            .0
            .iter()
            .map(|c| f64::from(*c) * f64::from(*c))
            .sum::<f64>()
            .sqrt();
        if norm > 0.0 {
            Self(
                self.0
                    .iter()
                    .map(|c| (f64::from(*c) / norm) as f32)
                    .collect(),
            )
        } else {
            self.clone()
        }
    }
}

/// The identity of the model that produced an embedding.
///
/// Recorded on every embedding for provenance, the startup dimension-consistency
/// check, and the cross-family consolidation guard.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmbedderModel {
    /// Model family (the cross-family guard compares this against the writer's).
    pub family: String,
    /// Model version.
    pub version: String,
    /// Output dimension; checked against each vector index's dimension at startup.
    pub dimension: u32,
}
