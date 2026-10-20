/// A candidate produced for the current Pinyin input.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Candidate {
    pub text: String,
    /// Optional annotation for the candidate window, currently the Pinyin key.
    pub comment: String,
}

impl Candidate {
    pub fn new(text: impl Into<String>, comment: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            comment: comment.into(),
        }
    }
}

/// The complete state change returned after processing a key or selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompositionUpdate {
    /// True when the input session consumed the key.
    pub handled: bool,
    /// Current raw Pinyin input, for example `nihao`.
    pub preedit: String,
    /// Candidates on the current page only.
    pub candidates: Vec<Candidate>,
    /// Zero-based highlighted candidate on the current page.
    pub highlighted: Option<usize>,
    /// Zero-based candidate page.
    pub page: usize,
    /// Number of candidate pages, or zero when there are no candidates.
    pub page_count: usize,
    /// Text to insert into the host document.
    pub commit: Option<String>,
    /// True when Escape or an explicit cancel cleared the composition.
    pub cancelled: bool,
}

impl CompositionUpdate {
    pub fn is_composing(&self) -> bool {
        !self.preedit.is_empty()
    }

    pub fn committed_text(&self) -> Option<&str> {
        self.commit.as_deref()
    }
}
