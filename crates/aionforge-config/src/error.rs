//! The configuration error space.

/// An error from loading or validating configuration.
///
/// The [`Missing`](ConfigError::Missing), [`Invalid`](ConfigError::Invalid), and
/// [`SecretEnvMissing`](ConfigError::SecretEnvMissing) messages name the offending key
/// or env-var **name**, never a value. [`Load`](ConfigError::Load) relays the loader's
/// own message, which may quote a malformed value — but a secret is never deserialized
/// into the config (only the env-var name is), so one cannot reach a log through any
/// variant.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configuration source (a file, the environment, or a merged layer) could not be
    /// read or parsed.
    #[error("could not load configuration: {0}")]
    Load(String),

    /// A required key is missing or empty.
    #[error("missing required configuration: {0}")]
    Missing(String),

    /// A key is present but its value is not allowed.
    #[error("invalid configuration for {key}: {reason}")]
    Invalid {
        /// The dotted key path, e.g. `embedder.dimension`.
        key: String,
        /// Why the value is rejected (the reason never quotes a secret value).
        reason: String,
    },

    /// An API key was requested through an `api_key_env` setting, but that environment variable
    /// is not set. Only the variable's name appears here, never a value. The same variant covers
    /// the embedder and the completer, so it names the variable rather than a fixed config key.
    #[error("the environment variable {0} named by an `api_key_env` setting is not set")]
    SecretEnvMissing(String),
}

impl ConfigError {
    /// Construct a [`ConfigError::Missing`].
    pub(crate) fn missing(key: impl Into<String>) -> Self {
        Self::Missing(key.into())
    }

    /// Construct a [`ConfigError::Invalid`].
    pub(crate) fn invalid(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid {
            key: key.into(),
            reason: reason.into(),
        }
    }
}
