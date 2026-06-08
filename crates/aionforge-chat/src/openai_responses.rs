//! The OpenAI Responses wire format (`POST {base}/responses`), used statelessly.
//!
//! Bearer auth. The conversation is sent as the `input` array of `{role, content}` items (the
//! system message rides the array, accepted by Responses), `store` is `false`, and
//! `previous_response_id` is never sent — so nothing is persisted or carried server-side. The
//! token cap is `max_output_tokens`. Responses exposes no `seed`, so temperature `0.0` is the
//! only determinism lever. The aggregated `output_text` convenience field is SDK-only and absent
//! from the raw JSON, so the assistant text is gathered from the `output` array's
//! `output_text` parts; the stop reason comes from `status` + `incomplete_details`.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use aionforge_domain::completion::{ChatRole, Completion, CompletionRequest};

use crate::PINNED_TEMPERATURE;
use crate::error::CompleteError;

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: Vec<WireMessage<'a>>,
    temperature: f64,
    max_output_tokens: u32,
    store: bool,
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

/// Build the bearer-authenticated stateless `POST {url}` request for a Responses completion.
pub(crate) fn build(
    client: &reqwest::Client,
    url: &str,
    model: &str,
    api_key: Option<&SecretString>,
    request: &CompletionRequest,
    max_tokens: u32,
) -> reqwest::RequestBuilder {
    let input = request
        .messages
        .iter()
        .map(|m| WireMessage {
            role: role_str(m.role),
            content: &m.content,
        })
        .collect();
    let body = Request {
        model,
        input,
        temperature: PINNED_TEMPERATURE,
        max_output_tokens: max_tokens,
        store: false,
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
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    incomplete_details: Option<IncompleteDetails>,
    #[serde(default)]
    error: Option<ResponseError>,
    #[serde(default)]
    output: Vec<OutputItem>,
}

#[derive(Deserialize)]
struct IncompleteDetails {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct OutputItem {
    // `default` so an item that omits `type` (a future item shape) decodes to an empty kind that
    // the `== "message"` filter skips, rather than failing the whole response decode.
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    content: Vec<ContentPart>,
}

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

/// Parse a Responses body into a [`Completion`], aggregating the `output_text` parts.
pub(crate) fn parse(bytes: &[u8]) -> Result<Completion, CompleteError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|e| CompleteError::Decode(e.to_string()))?;

    // An in-body failure (HTTP 200 with status "failed") is an endpoint problem: degrade.
    if response.status.as_deref() == Some("failed") {
        let detail = response
            .error
            .and_then(|e| e.message)
            .unwrap_or_else(|| "status failed".to_owned());
        return Err(CompleteError::Unavailable(format!(
            "responses request failed: {detail}"
        )));
    }

    let mut content = String::new();
    for item in &response.output {
        if item.kind == "message" {
            for part in &item.content {
                if part.kind == "output_text"
                    && let Some(text) = &part.text
                {
                    content.push_str(text);
                }
            }
        }
    }
    if content.is_empty() {
        return Err(CompleteError::Decode(
            "response carried no output_text".to_owned(),
        ));
    }

    let finish_reason = match response.status.as_deref() {
        Some("completed") => Some("stop".to_owned()),
        Some("incomplete") => Some(normalize_incomplete(
            response.incomplete_details.and_then(|d| d.reason),
        )),
        _ => None,
    };

    Ok(Completion {
        content,
        responding_model: response.model,
        finish_reason,
    })
}

/// Map a Responses incomplete reason to the common vocabulary ([`Completion::finish_reason`]).
fn normalize_incomplete(reason: Option<String>) -> String {
    match reason.as_deref() {
        Some("max_output_tokens") => "length".to_owned(),
        Some("content_filter") => "filter".to_owned(),
        // An incomplete status with no stated reason is, conservatively, a truncation.
        None => "length".to_owned(),
        Some(other) => other.to_owned(),
    }
}
