//! The chat-completion provider selector.

use std::str::FromStr;

use crate::error::CompleteError;

/// Which provider wire format a [`HttpCompleter`](crate::HttpCompleter) speaks. A deployment
/// declares exactly one — there is no auto-routing between providers (08 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// OpenAI Chat Completions (`/chat/completions`), and every OpenAI-compatible local/open
    /// server (vLLM, Ollama, LM Studio, llama.cpp).
    OpenAiChat,
    /// OpenAI's Responses API (`/responses`), used statelessly.
    OpenAiResponses,
    /// Anthropic's Messages API (`/messages`).
    Anthropic,
}

impl Provider {
    /// The resource path appended to the configured base URL (which carries the version
    /// segment, e.g. `.../v1`). Mirrors how the embedding client appends `/embeddings`.
    #[must_use]
    pub(crate) fn path_suffix(self) -> &'static str {
        match self {
            Provider::OpenAiChat => "/chat/completions",
            Provider::OpenAiResponses => "/responses",
            Provider::Anthropic => "/messages",
        }
    }

    /// The config string form (the inverse of [`FromStr`]).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::OpenAiChat => "openai_chat",
            Provider::OpenAiResponses => "openai_responses",
            Provider::Anthropic => "anthropic",
        }
    }
}

impl FromStr for Provider {
    type Err = CompleteError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openai_chat" => Ok(Provider::OpenAiChat),
            "openai_responses" => Ok(Provider::OpenAiResponses),
            "anthropic" => Ok(Provider::Anthropic),
            other => Err(CompleteError::Config(format!(
                "unknown completer provider `{other}` \
                 (expected `openai_chat`, `openai_responses`, or `anthropic`)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_strings() {
        for provider in [
            Provider::OpenAiChat,
            Provider::OpenAiResponses,
            Provider::Anthropic,
        ] {
            assert_eq!(provider.as_str().parse::<Provider>().unwrap(), provider);
        }
    }

    #[test]
    fn unknown_provider_is_a_config_error() {
        let err = "vertex".parse::<Provider>().unwrap_err();
        assert!(matches!(err, CompleteError::Config(_)));
    }

    #[test]
    fn path_suffixes_are_the_resource_paths() {
        assert_eq!(Provider::OpenAiChat.path_suffix(), "/chat/completions");
        assert_eq!(Provider::OpenAiResponses.path_suffix(), "/responses");
        assert_eq!(Provider::Anthropic.path_suffix(), "/messages");
    }
}
