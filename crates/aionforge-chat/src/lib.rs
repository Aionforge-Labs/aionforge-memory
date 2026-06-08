//! Multi-provider chat-completion client (08 §1, M3.T07).
//!
//! [`HttpCompleter`] implements the domain
//! [`Completer`](aionforge_domain::contracts::Completer) contract over three wire formats
//! behind one provider-agnostic seam:
//!
//! - [`Provider::OpenAiChat`] — OpenAI Chat Completions (`/chat/completions`), which is also
//!   the open de-facto standard every OpenAI-compatible local/self-hosted server speaks
//!   (vLLM, Ollama, LM Studio, llama.cpp).
//! - [`Provider::OpenAiResponses`] — OpenAI's Responses API (`/responses`), used **statelessly**
//!   (`store: false`, no `previous_response_id`) so no server-side conversation state leaks in.
//! - [`Provider::Anthropic`] — Anthropic's Messages API (`/messages`), with `x-api-key` +
//!   `anthropic-version` auth, a top-level `system` field, and a required `max_tokens`.
//!
//! A deployment **declares one** provider and model; there is no cost-first auto-routing, so the
//! responding model family stays verifiable (the cross-family guard, M6). The client pins
//! sampling for reproducibility, records the model the endpoint *actually* responded with, and
//! normalizes each provider's stop reason to one vocabulary. When the endpoint is unreachable or
//! overloaded, [`CompleteError::is_unavailable`] is true so a caller degrades to the
//! deterministic canonical tier rather than failing (the layered-determinism doctrine).
//!
//! The substrate runs no inference itself; this crate is the boundary to a provider.

mod anthropic;
mod client;
mod error;
mod openai_chat;
mod openai_responses;
mod provider;

pub use client::HttpCompleter;
pub use error::CompleteError;
pub use provider::Provider;

/// The pinned sampling temperature sent on every request: `0.0`, the most deterministic setting
/// every provider honors. Sampling is not caller-configurable — reproducibility is a property of
/// the client, not a per-call choice (08 §1; layered-determinism doctrine in 00 principle 9).
pub(crate) const PINNED_TEMPERATURE: f64 = 0.0;

/// The pinned RNG seed sent where the provider supports one (OpenAI Chat Completions). It makes
/// reproducible outputs best-effort on that provider; Responses and Anthropic expose no seed, so
/// there temperature `0.0` is the only lever. Determinism is best-effort across all providers,
/// never guaranteed — the canonical tier, not this client, is the byte-deterministic path.
pub(crate) const PINNED_SEED: i64 = 42;
