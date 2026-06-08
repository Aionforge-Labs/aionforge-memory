//! The OpenAI Chat Completions wire format (`POST {base}/chat/completions`).
//!
//! Also the open de-facto standard for self-hosted OpenAI-compatible servers. Auth is a bearer
//! token. Sampling is pinned (`temperature`, `seed`). The output-token cap is sent as
//! `max_tokens` — deliberately the legacy field rather than the newer `max_completion_tokens`:
//! `max_tokens` is what every OpenAI-compatible local/open server understands, and it is still
//! accepted on OpenAI's standard chat models. (OpenAI's reasoning models reject both `max_tokens`
//! and `temperature: 0`/`seed`; route those through the Responses provider instead.)

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use aionforge_domain::completion::{ChatRole, Completion, CompletionRequest};

use crate::error::CompleteError;
use crate::{PINNED_SEED, PINNED_TEMPERATURE};

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    temperature: f64,
    seed: i64,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

fn role_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

/// Build the bearer-authenticated `POST {url}` request for a chat completion.
pub(crate) fn build(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    api_key: Option<&SecretString>,
    request: &CompletionRequest,
    max_tokens: u32,
) -> reqwest::RequestBuilder {
    let messages = request
        .messages
        .iter()
        .map(|m| WireMessage {
            role: role_str(m.role),
            content: &m.content,
        })
        .collect();
    let body = Request {
        model,
        messages,
        temperature: PINNED_TEMPERATURE,
        seed: PINNED_SEED,
        max_tokens,
        stream: false,
    };
    let mut builder = client.post(url).json(&body);
    if let Some(key) = api_key {
        builder = builder.bearer_auth(key.expose_secret());
    }
    builder
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    model: String,
    // `default` so a body that omits `choices` decodes and falls through to a clear `Decode`
    // error rather than a raw serde failure (forward-compat; matches the other providers).
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

/// Parse a Chat Completions response into a [`Completion`].
pub(crate) fn parse(bytes: &[u8]) -> Result<Completion, CompleteError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|e| CompleteError::Decode(e.to_string()))?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| CompleteError::Decode("response had no choices".to_owned()))?;
    // Reject both an absent and an empty content string, so an empty completion is a clear
    // Decode error rather than a silently-empty success (matches the other providers).
    let content = choice
        .message
        .content
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            CompleteError::Decode("response choice carried no message content".to_owned())
        })?;
    Ok(Completion {
        content,
        responding_model: response.model,
        finish_reason: choice.finish_reason.map(normalize_finish_reason),
    })
}

/// Map the OpenAI `finish_reason` to the common vocabulary (08 §1, [`Completion::finish_reason`]).
fn normalize_finish_reason(reason: String) -> String {
    match reason.as_str() {
        "stop" => "stop".to_owned(),
        "length" => "length".to_owned(),
        "content_filter" => "filter".to_owned(),
        _ => reason,
    }
}
