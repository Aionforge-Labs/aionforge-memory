//! The capture-side privacy and prompt-injection filter (04 §1, 07 §2).
//!
//! [`CaptureFilter`] runs on the capture hot path before an episode is committed: it
//! redacts configured sensitive spans and detects/strips known prompt-injection
//! markers, recording what it did in the [`FilterOutcome`] that the capture path
//! folds into `Episode.origin` (02 §6.1). It is local and synchronous, so it adds no
//! network round-trip to capture.
//!
//! The filter is deliberately conservative in v1.0 — a small, low-false-positive
//! default pattern set that "raises the bar" (07 §2). Hardening it against a
//! published injection corpus and measuring block / false-positive rates is M6.T03;
//! callers can supply their own pattern sets via [`CaptureFilter::new`] in the
//! meantime.
//!
//! Redaction spans are reported as byte offsets into the *original* content (the
//! `Redaction.span` contract), and the matched text is replaced with a typed
//! `[redacted:<kind>]` placeholder; injection markers are stripped from the cleaned
//! content and their ids collected into `injection_flags`. Matches are applied as a
//! single deterministic, non-overlapping edit pass: the earliest start wins, the
//! longer match breaks a tie, and a later overlapping match is dropped.

use aionforge_domain::nodes::episodic::Redaction;
use aionforge_domain::{FilterOutcome, PrivacyFilter};
use regex::Regex;

use crate::error::SecurityError;

/// A configured redaction rule: a regex whose matches are recorded and replaced.
#[derive(Debug, Clone)]
pub struct RedactionPattern {
    id: String,
    kind: String,
    regex: Regex,
}

impl RedactionPattern {
    /// Compile a redaction rule. `id` names the rule (recorded as `pattern_id`),
    /// `kind` labels the sensitive-data class, and `pattern` is its regex.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidPattern`] if `pattern` is not a valid regex.
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        pattern: &str,
    ) -> Result<Self, SecurityError> {
        let id = id.into();
        let regex = Regex::new(pattern).map_err(|source| SecurityError::InvalidPattern {
            id: id.clone(),
            source,
        })?;
        Ok(Self {
            id,
            kind: kind.into(),
            regex,
        })
    }
}

/// A known prompt-injection marker: a regex whose matches are flagged and stripped.
#[derive(Debug, Clone)]
pub struct InjectionMarker {
    id: String,
    regex: Regex,
}

impl InjectionMarker {
    /// Compile an injection marker. `id` names the marker (recorded in
    /// `injection_flags`); `pattern` is its regex (use the `(?i)` flag for
    /// case-insensitivity).
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidPattern`] if `pattern` is not a valid regex.
    pub fn new(id: impl Into<String>, pattern: &str) -> Result<Self, SecurityError> {
        let id = id.into();
        let regex = Regex::new(pattern).map_err(|source| SecurityError::InvalidPattern {
            id: id.clone(),
            source,
        })?;
        Ok(Self { id, regex })
    }
}

/// The capture-side privacy/injection filter (07 §2).
#[derive(Debug, Clone)]
pub struct CaptureFilter {
    redactions: Vec<RedactionPattern>,
    markers: Vec<InjectionMarker>,
}

impl CaptureFilter {
    /// Build a filter from explicit redaction and injection-marker rule sets.
    #[must_use]
    pub fn new(redactions: Vec<RedactionPattern>, markers: Vec<InjectionMarker>) -> Self {
        Self {
            redactions,
            markers,
        }
    }

    /// Build a filter with the conservative v1.0 default pattern set.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidPattern`] only if a built-in pattern fails to
    /// compile, which the unit tests guard against.
    pub fn with_defaults() -> Result<Self, SecurityError> {
        let redactions = DEFAULT_REDACTIONS
            .iter()
            .map(|&(id, kind, pattern)| RedactionPattern::new(id, kind, pattern))
            .collect::<Result<Vec<_>, _>>()?;
        let markers = DEFAULT_MARKERS
            .iter()
            .map(|&(id, pattern)| InjectionMarker::new(id, pattern))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(redactions, markers))
    }
}

/// One planned change to the content, before overlap resolution.
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
    redaction: Option<Redaction>,
    flag: Option<String>,
}

impl PrivacyFilter for CaptureFilter {
    type Error = SecurityError;

    fn filter(&self, content: &str) -> Result<FilterOutcome, Self::Error> {
        let mut edits: Vec<Edit> = Vec::new();

        for pattern in &self.redactions {
            for m in pattern.regex.find_iter(content) {
                edits.push(Edit {
                    start: m.start(),
                    end: m.end(),
                    replacement: format!("[redacted:{}]", pattern.kind),
                    redaction: Some(Redaction {
                        pattern_id: pattern.id.clone(),
                        span: (m.start(), m.end()),
                        kind: pattern.kind.clone(),
                    }),
                    flag: None,
                });
            }
        }
        for marker in &self.markers {
            for m in marker.regex.find_iter(content) {
                edits.push(Edit {
                    start: m.start(),
                    end: m.end(),
                    replacement: String::new(),
                    redaction: None,
                    flag: Some(marker.id.clone()),
                });
            }
        }

        // Deterministic, non-overlapping edit order: earliest start first, longer
        // match first on a tie. The walk below drops any later edit that overlaps an
        // applied one.
        edits.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));

        let mut cleaned = String::with_capacity(content.len());
        let mut redactions = Vec::new();
        let mut injection_flags: Vec<String> = Vec::new();
        let mut cursor = 0usize;

        for edit in edits {
            if edit.start < cursor {
                continue; // overlaps an already-applied edit
            }
            cleaned.push_str(&content[cursor..edit.start]);
            cleaned.push_str(&edit.replacement);
            cursor = edit.end;
            if let Some(redaction) = edit.redaction {
                redactions.push(redaction);
            }
            if let Some(id) = edit.flag
                && !injection_flags.contains(&id)
            {
                injection_flags.push(id);
            }
        }
        cleaned.push_str(&content[cursor..]);

        Ok(FilterOutcome {
            cleaned,
            redactions,
            injection_flags,
        })
    }
}

/// Default redaction rules: `(id, kind, regex)`. Conservative to keep the benign
/// false-positive rate low (07 §2 acceptance); M6.T03 expands and tunes these.
const DEFAULT_REDACTIONS: &[(&str, &str, &str)] = &[
    (
        "email",
        "email",
        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
    ),
    (
        "us_phone",
        "phone",
        r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b",
    ),
    ("credit_card", "card", r"\b(?:\d[ -]?){13,16}\b"),
    ("secret_key", "secret", r"\bsk-[A-Za-z0-9_-]{20,}\b"),
];

/// Default injection markers: `(id, regex)`. All case-insensitive. A starting set
/// of well-known override phrases; M6.T03 hardens against a published corpus.
const DEFAULT_MARKERS: &[(&str, &str)] = &[
    (
        "ignore_previous",
        r"(?i)ignore\s+(?:all\s+)?(?:previous|prior|the\s+above|above)\s+(?:instructions?|prompts?)",
    ),
    (
        "disregard_above",
        r"(?i)disregard\s+(?:all\s+)?(?:previous|prior|the\s+above|above)",
    ),
    ("system_prompt", r"(?i)system\s+prompt\s*:"),
    (
        "new_instructions",
        r"(?i)(?:new|updated)\s+instructions\s*:",
    ),
    ("you_are_now", r"(?i)you\s+are\s+now\b"),
    (
        "override_instructions",
        r"(?i)override\s+(?:your\s+)?(?:previous\s+)?(?:instructions|system|prompt)",
    ),
];
