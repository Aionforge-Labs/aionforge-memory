//! The multi-provider HTTP chat-completion client.

use std::sync::Once;
use std::time::Duration;

use aionforge_config::CompleterConfig;
use aionforge_domain::completion::{CompleterModel, Completion, CompletionRequest};
use aionforge_domain::contracts::Completer;
use secrecy::SecretString;

use crate::error::CompleteError;
use crate::provider::Provider;
use crate::{anthropic, openai_chat, openai_responses};

/// Install the ring crypto provider as the process default exactly once.
///
/// reqwest is built with `rustls-no-provider`, so a `Client` constructed before a provider is
/// installed panics. Mirrors the embedding client; the install is process-global and
/// first-writer-wins, so it composes with the embedder doing the same.
fn ensure_crypto_provider() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// A multi-provider chat-completion client over one declared provider and model.
///
/// Sampling is pinned for reproducibility; the responding model is recorded on every
/// [`Completion`]; and an unreachable or overloaded endpoint surfaces as
/// [`CompleteError::is_unavailable`] so a caller can degrade to the deterministic canonical tier.
#[derive(Debug)]
pub struct HttpCompleter {
    client: reqwest::Client,
    provider: Provider,
    url: String,
    request_model: String,
    identity: CompleterModel,
    api_key: Option<SecretString>,
    max_tokens: u32,
}

impl HttpCompleter {
    /// Build a client against `endpoint` (a base URL such as `https://host/v1`) for `provider`.
    ///
    /// `request_model` is the model id sent in each request; `identity` is the declared
    /// [`CompleterModel`] recorded for the cross-family guard. `api_key`, when set, is sent as the
    /// provider's auth credential and never logged. `max_tokens` is the default output-token cap
    /// (required by the Anthropic provider; an upper bound for the OpenAI providers).
    ///
    /// # Errors
    /// Returns [`CompleteError::Config`] if `endpoint`'s transport is not allowed (must be
    /// `https://` unless the host is localhost) or the HTTP client cannot be built.
    pub fn new(
        provider: Provider,
        endpoint: &str,
        request_model: impl Into<String>,
        identity: CompleterModel,
        api_key: Option<SecretString>,
        timeout: Duration,
        max_tokens: u32,
    ) -> Result<Self, CompleteError> {
        // Enforce the transport rule at construction too, so building a client directly cannot
        // slip past the config-time check (mirrors the embedding client, §8.4).
        if !aionforge_config::endpoint_transport_is_allowed(endpoint) {
            return Err(CompleteError::Config(format!(
                "endpoint {endpoint} must use https:// unless the host is localhost"
            )));
        }
        ensure_crypto_provider();
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| CompleteError::Config(error.to_string()))?;
        let base = endpoint.trim_end_matches('/');
        Ok(Self {
            client,
            provider,
            url: format!("{base}{}", provider.path_suffix()),
            request_model: request_model.into(),
            identity,
            api_key,
            max_tokens,
        })
    }

    /// Build a client from a [`CompleterConfig`] and an already-resolved API key.
    ///
    /// # Errors
    /// Returns [`CompleteError::Config`] if the configured provider is unknown, the endpoint's
    /// transport is not allowed, or the HTTP client cannot be built.
    pub fn from_config(
        config: &CompleterConfig,
        api_key: Option<SecretString>,
    ) -> Result<Self, CompleteError> {
        let provider: Provider = config.provider.parse()?;
        let identity = CompleterModel {
            family: config.model.clone(),
            version: String::new(),
        };
        Self::new(
            provider,
            &config.endpoint,
            config.model.clone(),
            identity,
            api_key,
            Duration::from_millis(config.timeout_ms),
            config.max_tokens,
        )
    }

    /// Send a built request and read the body, mapping transport and status to the error space.
    ///
    /// A connection failure, a 5xx, or an overload (HTTP 429/529) is `Unavailable` (degrade);
    /// any other non-success is a hard `Status`. A body-read failure after a success status is a
    /// severed stream — a transport problem, so `Unavailable`, not a decode error.
    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<Vec<u8>, CompleteError> {
        let response = builder
            .send()
            .await
            .map_err(|error| CompleteError::Unavailable(error.to_string()))?;
        let status = response.status();
        if status.is_server_error() || matches!(status.as_u16(), 429 | 529) {
            return Err(CompleteError::Unavailable(format!("HTTP status {status}")));
        }
        if !status.is_success() {
            return Err(CompleteError::Status {
                status: status.as_u16(),
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CompleteError::Unavailable(error.to_string()))?;
        Ok(bytes.to_vec())
    }
}

impl Completer for HttpCompleter {
    type Error = CompleteError;

    async fn complete(&self, request: &CompletionRequest) -> Result<Completion, Self::Error> {
        let max_tokens = request.max_tokens.unwrap_or(self.max_tokens);
        let key = self.api_key.as_ref();
        let builder = match self.provider {
            Provider::OpenAiChat => openai_chat::build(
                &self.client,
                &self.url,
                &self.request_model,
                key,
                request,
                max_tokens,
            ),
            Provider::OpenAiResponses => openai_responses::build(
                &self.client,
                &self.url,
                &self.request_model,
                key,
                request,
                max_tokens,
            ),
            Provider::Anthropic => anthropic::build(
                &self.client,
                &self.url,
                &self.request_model,
                key,
                request,
                max_tokens,
            ),
        };
        let bytes = self.send(builder).await?;
        match self.provider {
            Provider::OpenAiChat => openai_chat::parse(&bytes),
            Provider::OpenAiResponses => openai_responses::parse(&bytes),
            Provider::Anthropic => anthropic::parse(&bytes),
        }
    }

    fn model(&self) -> &CompleterModel {
        &self.identity
    }
}
