use thiserror::Error;

/// Errors returned by lexicon loading and session state transitions.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    #[error("the pinyin lexicon is empty")]
    EmptyLexicon,

    #[error("the pinyin lexicon is not valid UTF-8: {message}")]
    InvalidUtf8 { message: String },

    #[error("invalid pinyin lexicon at line {line}: {reason}")]
    InvalidLexiconLine { line: usize, reason: String },

    #[error("candidate page size must be greater than zero, got {page_size}")]
    InvalidPageSize { page_size: usize },

    #[error("there is no active pinyin composition")]
    NoActiveComposition,

    #[error(
        "candidate index {index} is out of range for the {candidate_count} candidate(s) \
         on the current page"
    )]
    CandidateOutOfRange {
        index: usize,
        candidate_count: usize,
    },
}

impl EngineError {
    pub(crate) fn invalid_line(line: usize, reason: impl Into<String>) -> Self {
        Self::InvalidLexiconLine {
            line,
            reason: reason.into(),
        }
    }
}
