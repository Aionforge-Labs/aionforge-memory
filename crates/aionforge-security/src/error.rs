//! The typed error space for the security subsystem.

/// An error from the security subsystem.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum SecurityError {
    /// A filter pattern failed to compile. Carries the pattern id so a bad custom
    /// pattern is identifiable; the regex syntax error is the source.
    #[error("filter pattern `{id}` is not a valid regex")]
    InvalidPattern {
        /// The id of the pattern that failed to compile.
        id: String,
        /// The underlying regex compilation error.
        #[source]
        source: regex::Error,
    },
}
