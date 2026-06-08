//! Acceptance tests for the multi-provider chat-completion client (08 §1, M3.T07).
//!
//! Pins, per provider, that a round-trip completion preserves content and records the responding
//! model; that sampling is pinned and (for Anthropic) the system prompt is lifted and the right
//! auth headers are sent; that the stop reason is normalized; and that an unreachable/overloaded
//! endpoint degrades (`Unavailable`) while a 4xx or a malformed body is a hard error.

use std::time::Duration;

use aionforge_chat::{CompleteError, HttpCompleter, Provider};
use aionforge_config::CompleterConfig;
use aionforge_domain::completion::{ChatMessage, CompleterModel, CompletionRequest};
use aionforge_domain::contracts::Completer;
use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn completer(provider: Provider, base: &str, model: &str, api_key: Option<&str>) -> HttpCompleter {
    let identity = CompleterModel {
        family: model.to_owned(),
        version: String::new(),
    };
    HttpCompleter::new(
        provider,
        &format!("{base}/v1"),
        model,
        identity,
        api_key.map(|key| SecretString::from(key.to_owned())),
        Duration::from_secs(5),
        256,
    )
    .expect("build completer")
}

fn user(text: &str) -> CompletionRequest {
    CompletionRequest::new(vec![ChatMessage::user(text)])
}

// --- OpenAI Chat Completions ---------------------------------------------------------------

#[tokio::test]
async fn openai_chat_round_trips_records_model_and_pins_sampling() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-4o-2024-08-06",
        "choices": [
            { "index": 0, "message": { "role": "assistant", "content": "Paris." }, "finish_reason": "stop" }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_partial_json(
            json!({ "temperature": 0.0, "seed": 42, "max_tokens": 256, "stream": false }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = completer(
        Provider::OpenAiChat,
        &server.uri(),
        "gpt-4o",
        Some("sk-test"),
    );
    let out = client
        .complete(&user("capital of France?"))
        .await
        .expect("complete");

    assert_eq!(out.content, "Paris.");
    assert_eq!(
        out.responding_model, "gpt-4o-2024-08-06",
        "records the responding model"
    );
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn openai_chat_length_finish_normalizes() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-4o",
        "choices": [{ "index": 0, "message": { "content": "trunc" }, "finish_reason": "length" }]
    });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "gpt-4o", None);
    let out = client.complete(&user("x")).await.expect("complete");
    assert_eq!(
        out.finish_reason.as_deref(),
        Some("length"),
        "length is the truncation sentinel"
    );
}

// --- OpenAI Responses ----------------------------------------------------------------------

#[tokio::test]
async fn openai_responses_aggregates_output_text_and_is_stateless() {
    let server = MockServer::start().await;
    // Two output items (a reasoning item ahead of the message) to prove we scan, not index [0].
    let body = json!({
        "model": "gpt-5.4",
        "status": "completed",
        "output": [
            { "type": "reasoning", "content": [] },
            { "type": "message", "role": "assistant",
              "content": [ { "type": "output_text", "text": "Paris." } ] }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(
            json!({ "store": false, "temperature": 0.0, "max_output_tokens": 256 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiResponses, &server.uri(), "gpt-5.4", None);
    let out = client.complete(&user("capital?")).await.expect("complete");
    assert_eq!(out.content, "Paris.");
    assert_eq!(out.responding_model, "gpt-5.4");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn openai_responses_incomplete_maps_to_length() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-5.4",
        "status": "incomplete",
        "incomplete_details": { "reason": "max_output_tokens" },
        "output": [ { "type": "message", "content": [ { "type": "output_text", "text": "partial" } ] } ]
    });
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiResponses, &server.uri(), "gpt-5.4", None);
    let out = client.complete(&user("x")).await.expect("complete");
    assert_eq!(out.finish_reason.as_deref(), Some("length"));
}

#[tokio::test]
async fn openai_responses_in_body_failure_is_unavailable() {
    let server = MockServer::start().await;
    let body = json!({ "model": "gpt-5.4", "status": "failed",
        "error": { "code": "server_error", "message": "boom" }, "output": [] });
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiResponses, &server.uri(), "gpt-5.4", None);
    let err = client
        .complete(&user("x"))
        .await
        .expect_err("failed status");
    assert!(err.is_unavailable(), "an in-body failure degrades: {err}");
}

// --- Anthropic Messages --------------------------------------------------------------------

#[tokio::test]
async fn anthropic_lifts_system_sends_headers_and_extracts_text() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "claude-opus-4-8",
        "content": [ { "type": "text", "text": "Paris." } ],
        "stop_reason": "end_turn"
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({
            "system": "be terse",
            "max_tokens": 256,
            "temperature": 0.0,
            "messages": [ { "role": "user", "content": "capital?" } ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(
        Provider::Anthropic,
        &server.uri(),
        "claude-opus-4-8",
        Some("sk-ant"),
    );
    let req = CompletionRequest::new(vec![
        ChatMessage::system("be terse"),
        ChatMessage::user("capital?"),
    ]);
    let out = client.complete(&req).await.expect("complete");
    assert_eq!(out.content, "Paris.");
    assert_eq!(out.responding_model, "claude-opus-4-8");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn anthropic_max_tokens_stop_maps_to_length() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "claude-haiku-4-5",
        "content": [ { "type": "text", "text": "partial" } ],
        "stop_reason": "max_tokens"
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::Anthropic, &server.uri(), "claude-haiku-4-5", None);
    let out = client.complete(&user("x")).await.expect("complete");
    assert_eq!(out.finish_reason.as_deref(), Some("length"));
}

// --- Common error taxonomy -----------------------------------------------------------------

#[tokio::test]
async fn server_error_degrades() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "m", None);
    let err = client.complete(&user("x")).await.expect_err("5xx");
    assert!(err.is_unavailable());
}

#[tokio::test]
async fn rate_limit_and_overload_degrade() {
    for code in [429_u16, 529] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(code))
            .mount(&server)
            .await;
        let client = completer(Provider::Anthropic, &server.uri(), "m", None);
        let err = client.complete(&user("x")).await.expect_err("overload");
        assert!(err.is_unavailable(), "HTTP {code} should degrade");
    }
}

#[tokio::test]
async fn client_error_is_a_hard_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "m", None);
    let err = client.complete(&user("x")).await.expect_err("4xx");
    assert!(
        matches!(err, CompleteError::Status { status: 400 }),
        "got {err}"
    );
}

#[tokio::test]
async fn malformed_body_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{not json"))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "m", None);
    let err = client.complete(&user("x")).await.expect_err("bad body");
    assert!(matches!(err, CompleteError::Decode(_)), "got {err}");
}

#[tokio::test]
async fn unknown_provider_from_config_is_a_config_error() {
    let config = CompleterConfig {
        enabled: true,
        provider: "vertex".to_owned(),
        endpoint: "https://example.com/v1".to_owned(),
        model: "m".to_owned(),
        ..CompleterConfig::default()
    };
    let err = HttpCompleter::from_config(&config, None).expect_err("unknown provider");
    assert!(matches!(err, CompleteError::Config(_)));
}

#[tokio::test]
async fn non_local_plaintext_endpoint_is_rejected() {
    let identity = CompleterModel {
        family: "m".to_owned(),
        version: String::new(),
    };
    let err = HttpCompleter::new(
        Provider::OpenAiChat,
        "http://api.example.com/v1",
        "m",
        identity,
        None,
        Duration::from_secs(5),
        256,
    )
    .expect_err("plaintext remote endpoint");
    assert!(matches!(err, CompleteError::Config(_)));
}

// --- Forward compatibility & empty output (serde defaults degrade, not panic) ---------------

#[tokio::test]
async fn openai_chat_missing_finish_reason_decodes_to_none() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-4o",
        "choices": [{ "message": { "content": "ok" } }]
    });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "gpt-4o", None);
    let out = client.complete(&user("x")).await.expect("complete");
    assert_eq!(out.content, "ok");
    assert!(
        out.finish_reason.is_none(),
        "an absent finish_reason is None, not a decode error"
    );
}

#[tokio::test]
async fn openai_chat_missing_choices_is_a_clean_decode_error() {
    let server = MockServer::start().await;
    // No `choices` key at all: serde defaults it to empty, and parse reports a clear Decode.
    let body = json!({ "model": "gpt-4o" });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "gpt-4o", None);
    let err = client.complete(&user("x")).await.expect_err("no choices");
    assert!(matches!(err, CompleteError::Decode(_)), "got {err}");
}

#[tokio::test]
async fn openai_chat_empty_content_is_a_decode_error() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "gpt-4o",
        "choices": [{ "message": { "content": "" }, "finish_reason": "stop" }]
    });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiChat, &server.uri(), "gpt-4o", None);
    let err = client
        .complete(&user("x"))
        .await
        .expect_err("empty content");
    assert!(
        matches!(err, CompleteError::Decode(_)),
        "an empty completion is rejected: {err}"
    );
}

#[tokio::test]
async fn openai_responses_skips_an_unknown_output_item_type() {
    let server = MockServer::start().await;
    // An output item missing `type` (a future shape) must be skipped, not fail the whole decode.
    let body = json!({
        "model": "gpt-5.4",
        "status": "completed",
        "output": [
            { "content": [] },
            { "type": "message", "content": [ { "type": "output_text", "text": "ok" } ] }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = completer(Provider::OpenAiResponses, &server.uri(), "gpt-5.4", None);
    let out = client.complete(&user("x")).await.expect("complete");
    assert_eq!(out.content, "ok");
}
