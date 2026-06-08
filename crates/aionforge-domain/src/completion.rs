//! Chat-completion domain types (08 §1, M3.T07).
//!
//! The optional chat half of the OpenAI-compatible inference client: the request a
//! [`Completer`](crate::contracts::Completer) consumes and the [`Completion`] it returns.
//! Like [`EmbedderModel`](crate::embedding::EmbedderModel), the [`CompleterModel`] identity
//! is recorded so a later cross-family guard (M6) can tell which model family produced a
//! distilled artifact — and so a provider that silently swaps models is detectable, since the
//! [`Completion`] records the model the endpoint *actually* responded with.
//!
//! Sampling is deliberately absent from the request: temperature and seed are pinned by the
//! client for reproducibility, not chosen per call, so a completion stays as close to
//! deterministic as the provider allows.

use serde::{Deserialize, Serialize};

/// The role of a message in a chat completion (OpenAI-compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    /// The system / instruction message.
    System,
    /// A user turn.
    User,
    /// A model / assistant turn.
    Assistant,
}

/// One message in a chat-completion conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Who produced the message.
    pub role: ChatRole,
    /// The message text.
    pub content: String,
}

impl ChatMessage {
    /// A system-role message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    /// A user-role message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    /// An assistant-role message, for constructing a multi-turn conversation (e.g. a distiller
    /// few-shot, or a prior assistant turn).
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// A chat-completion request: the conversation, plus an optional output-length bound.
///
/// Sampling parameters (temperature, seed) are not part of the request — the client pins
/// them for reproducibility, so a caller cannot make a completion less deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The conversation to complete.
    pub messages: Vec<ChatMessage>,
    /// An optional cap on the number of tokens to generate.
    pub max_tokens: Option<u32>,
}

impl CompletionRequest {
    /// A request from a sequence of messages, with no output-length bound.
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            max_tokens: None,
        }
    }
}

/// A completed chat response and the identity of the model that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    /// The assistant message content.
    pub content: String,
    /// The model the endpoint reported as responding. Recorded verbatim so a provider that
    /// silently swaps models (e.g. cost-first auto-routing) is detectable against the
    /// declared [`CompleterModel`] — the consolidating model family stays verifiable (08 §1).
    pub responding_model: String,
    /// Why generation stopped, normalized across providers to a small common vocabulary so a
    /// caller need not know which provider answered: `"stop"` (natural end), `"length"`
    /// (truncated at the token cap), `"filter"` (content filtered), `"refusal"`, or the
    /// provider's own value when it maps to none of these. `None` when the endpoint reports
    /// no reason. `Some("length")` is the truncation sentinel — a distiller's detail-retention
    /// guard rejects such a lossy completion rather than store it.
    pub finish_reason: Option<String>,
}

/// The declared identity of a completion model (08 §1).
///
/// Mirrors [`EmbedderModel`](crate::embedding::EmbedderModel) without a dimension. Recorded so
/// a distilled artifact's provenance names the family that produced it, for the cross-family
/// guard (M6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompleterModel {
    /// The model family.
    pub family: String,
    /// The model version.
    pub version: String,
}
