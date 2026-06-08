//! The Anthropic Messages wire format (`POST {base}/messages`).
//!
//! Auth is the `x-api-key` header plus the required `anthropic-version` header (not a bearer
//! token). The system prompt is a top-level `system` field, not a message role — Anthropic's
//! `messages` carry only `user`/`assistant` — so any system-role messages are concatenated into
//! `system`. `max_tokens` is required. There is no `seed`; temperature `0.0` is the only
//! determinism lever. The assistant text is gathered from the `content` text blocks, and
//! `stop_reason` maps to the common stop vocabulary.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use aionforge_domain::completion::{ChatRole, Completion, CompletionRequest};

use crate::PINNED_TEMPERATURE;
use crate::error::CompleteError;

/// The Anthropic API version pin. A frozen contract string (not a "latest" date); current.
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<WireMessage<'a>>,
    temperature: f64,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// Build the `x-api-key`-authenticated `POST {url}` request for a Messages completion.
pub(crate) fn build(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    api_key: Option<&SecretString>,
    request: &CompletionRequest,
    max_tokens: u32,
) -> reqwest::RequestBuilder {
    // System prompts are a top-level field, not a message role: lift them out of the turn list.
    let system: Vec<&str> = request
        .messages
        .iter()
        .filter(|m| m.role == ChatRole::System)
        .map(|m| m.content.as_str())
        .collect();
    let system = if system.is_empty() {
        None
    } else {
        Some(system.join("\n\n"))
    };
    let messages = request
        .messages
        .iter()
        .filter(|m| m.role != ChatRole::System)
        .map(|m| WireMessage {
            role: match m.role {
                ChatRole::Assistant => "assistant",
                // System is filtered above; everything else is a user turn.
                _ => "user",
            },
            content: &m.content,
        })
        .collect();
    let body = Request {
        model,
        max_tokens,
        system,
        messages,
        temperature: PINNED_TEMPERATURE,
    };
    let mut builder = client
        .post(url)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body);
    if let Some(key) = api_key {
        builder = builder.header("x-api-key", key.expose_secret());
    }
    builder
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    model: String,
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

/// Parse a Messages response into a [`Completion`], gathering the text content blocks.
pub(crate) fn parse(bytes: &[u8]) -> Result<Completion, CompleteError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|e| CompleteError::Decode(e.to_string()))?;
    let mut content = String::new();
    for block in &response.content {
        if block.kind == "text"
            && let Some(text) = &block.text
        {
            content.push_str(text);
        }
    }
    if content.is_empty() {
        return Err(CompleteError::Decode(
            "response carried no text content block".to_owned(),
        ));
    }
    Ok(Completion {
        content,
        responding_model: response.model,
        finish_reason: response.stop_reason.map(normalize_stop_reason),
    })
}

/// Map an Anthropic `stop_reason` to the common vocabulary ([`Completion::finish_reason`]).
fn normalize_stop_reason(reason: String) -> String {
    match reason.as_str() {
        // Intentional stops — the model finished, hit a stop sequence, wants a tool, or paused a
        // long turn. None is a truncation, filter, or refusal, so all normalize to "stop".
        "end_turn" | "stop_sequence" | "tool_use" | "pause_turn" => "stop".to_owned(),
        "max_tokens" => "length".to_owned(),
        "refusal" => "refusal".to_owned(),
        _ => reason,
    }
}
