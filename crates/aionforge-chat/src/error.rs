//! The chat-completion client error space.

/// An error from the chat-completion client.
///
/// [`Unavailable`](CompleteError::Unavailable) is the graceful-degradation signal: the endpoint
/// could not be reached, timed out, was overloaded (HTTP 429/529), or returned a 5xx, so a
/// caller falls back to the deterministic canonical tier rather than failing (08 §1, the
/// layered-determinism doctrine). Every other variant is a hard error the caller should surface.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum CompleteError {
    /// The endpoint could not be reached, timed out, was overloaded, or returned a 5xx. Degrade.
    #[error("completion endpoint unavailable: {0}")]
    Unavailable(String),

    /// The endpoint returned a non-success status that is not a degrade signal (e.g. a 4xx
    /// other than 429): a bad request, auth failure, or unknown model.
    #[error("completion endpoint returned HTTP status {status}")]
    Status {
        /// The HTTP status code.
        status: u16,
    },

    /// The response body was missing, unparseable, or carried no assistant text.
    #[error("could not decode the completion response: {0}")]
    Decode(String),

    /// The client could not be constructed (an unknown provider or a bad endpoint URL).
    #[error("invalid completion client configuration: {0}")]
    Config(String),
}

impl CompleteError {
    /// Whether this error means the endpoint is unavailable, so a caller should degrade rather
    /// than fail the operation (08 §1).
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}
