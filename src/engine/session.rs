use super::{Candidate, CompositionUpdate, EngineError, Key, KeyEvent, Lexicon};

/// Default number of candidates shown on one page.
pub const DEFAULT_PAGE_SIZE: usize = 9;

/// Mutable Pinyin composition state.
#[derive(Debug, Clone)]
pub struct PinyinSession {
    lexicon: Lexicon,
    page_size: usize,
    preedit: String,
    candidates: Vec<Candidate>,
    page: usize,
    highlighted: Option<usize>,
}

/// Stable name for hosts that associate one session with one input context.
pub type EngineSession = PinyinSession;

impl PinyinSession {
    /// Creates a session backed by the embedded MVP lexicon.
    pub fn new() -> Result<Self, EngineError> {
        Self::with_page_size(DEFAULT_PAGE_SIZE)
    }

    /// Creates a session with a custom candidate page size.
    pub fn with_page_size(page_size: usize) -> Result<Self, EngineError> {
        let lexicon = Lexicon::builtin()?;
        Self::from_lexicon(lexicon, page_size)
    }

    /// Creates a session from caller-provided TSV data.
    pub fn from_lexicon_tsv(tsv: &str, page_size: usize) -> Result<Self, EngineError> {
        let lexicon = Lexicon::from_tsv(tsv)?;
        Self::from_lexicon(lexicon, page_size)
    }

    /// Creates a session from an already validated lexicon.
    pub fn from_lexicon(lexicon: Lexicon, page_size: usize) -> Result<Self, EngineError> {
        if page_size == 0 {
            return Err(EngineError::InvalidPageSize { page_size });
        }
        if lexicon.is_empty() {
            return Err(EngineError::EmptyLexicon);
        }

        Ok(Self {
            lexicon,
            page_size,
            preedit: String::new(),
            candidates: Vec::new(),
            page: 0,
            highlighted: None,
        })
    }

    /// Processes one key and returns a snapshot of the resulting composition.
    ///
    /// Command-style shortcuts are never consumed. Digit keys `1..=9` select
    /// candidates on the current page. Space and Enter commit the highlighted
    /// candidate, or the raw Pinyin text if no candidate exists.
    pub fn process_key(&mut self, event: KeyEvent) -> Result<CompositionUpdate, EngineError> {
        if event.is_passthrough() {
            return Ok(self.snapshot(false));
        }

        match event.key {
            Key::Char(character) if character.is_ascii_alphabetic() => {
                self.preedit.push(character.to_ascii_lowercase());
                self.refresh_candidates();
                Ok(self.snapshot(true))
            }
            Key::Char(character) if character.is_ascii_digit() && character != '0' => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }

                let index = usize::from(character as u8 - b'1');
                let candidate_count = self.current_page().len();
                if index >= candidate_count {
                    return Ok(self.snapshot(true));
                }
                self.select_candidate(index)
            }
            Key::Backspace => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }

                self.preedit.pop();
                if self.preedit.is_empty() {
                    self.clear_composition();
                } else {
                    self.refresh_candidates();
                }
                Ok(self.snapshot(true))
            }
            Key::Enter | Key::Space => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.commit_current()
            }
            Key::Escape => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.clear_composition();
                let mut update = self.snapshot(true);
                update.cancelled = true;
                Ok(update)
            }
            Key::PageDown => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.change_page(1);
                Ok(self.snapshot(true))
            }
            Key::PageUp => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.change_page(-1);
                Ok(self.snapshot(true))
            }
            Key::Left | Key::Up => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.move_highlight(-1);
                Ok(self.snapshot(true))
            }
            Key::Right | Key::Down => {
                if !self.is_composing() {
                    return Ok(self.snapshot(false));
                }
                self.move_highlight(1);
                Ok(self.snapshot(true))
            }
            Key::Char(_) | Key::Other => Ok(self.snapshot(false)),
        }
    }

    /// Selects and commits a zero-based candidate from the current page.
    pub fn select_candidate(&mut self, index: usize) -> Result<CompositionUpdate, EngineError> {
        if !self.is_composing() {
            return Err(EngineError::NoActiveComposition);
        }

        let candidates = self.current_page();
        let candidate_count = candidates.len();
        let text = candidates
            .get(index)
            .map(|candidate| candidate.text.clone())
            .ok_or(EngineError::CandidateOutOfRange {
                index,
                candidate_count,
            })?;

        self.clear_composition();
        let mut update = self.snapshot(true);
        update.commit = Some(text);
        Ok(update)
    }

    /// Commits the highlighted candidate or the raw Pinyin input.
    pub fn commit(&mut self) -> Result<CompositionUpdate, EngineError> {
        if !self.is_composing() {
            return Err(EngineError::NoActiveComposition);
        }
        self.commit_current()
    }

    /// Cancels the active composition without committing text.
    pub fn cancel(&mut self) -> Result<CompositionUpdate, EngineError> {
        if !self.is_composing() {
            return Err(EngineError::NoActiveComposition);
        }
        self.clear_composition();
        let mut update = self.snapshot(true);
        update.cancelled = true;
        Ok(update)
    }

    /// Moves to the next candidate page, if one exists.
    pub fn next_page(&mut self) -> Result<CompositionUpdate, EngineError> {
        if !self.is_composing() {
            return Err(EngineError::NoActiveComposition);
        }
        self.change_page(1);
        Ok(self.snapshot(true))
    }

    /// Moves to the previous candidate page, if one exists.
    pub fn previous_page(&mut self) -> Result<CompositionUpdate, EngineError> {
        if !self.is_composing() {
            return Err(EngineError::NoActiveComposition);
        }
        self.change_page(-1);
        Ok(self.snapshot(true))
    }

    /// Changes the candidate page size while preserving valid page state.
    pub fn set_page_size(&mut self, page_size: usize) -> Result<(), EngineError> {
        if page_size == 0 {
            return Err(EngineError::InvalidPageSize { page_size });
        }

        self.page_size = page_size;
        let page_count = self.page_count();
        if page_count == 0 {
            self.page = 0;
            self.highlighted = None;
        } else {
            self.page = self.page.min(page_count - 1);
            self.highlighted = Some(0);
        }
        Ok(())
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn page_count(&self) -> usize {
        if self.candidates.is_empty() {
            0
        } else {
            self.candidates.len().div_ceil(self.page_size)
        }
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn is_composing(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// Candidates on the current page.
    pub fn candidates_on_current_page(&self) -> &[Candidate] {
        self.current_page()
    }

    fn commit_current(&mut self) -> Result<CompositionUpdate, EngineError> {
        let text = self
            .highlighted
            .and_then(|index| self.current_page().get(index))
            .map(|candidate| candidate.text.clone())
            .unwrap_or_else(|| self.preedit.clone());

        self.clear_composition();
        let mut update = self.snapshot(true);
        update.commit = Some(text);
        Ok(update)
    }

    fn refresh_candidates(&mut self) {
        self.candidates = self.lexicon.lookup(&self.preedit);
        self.page = 0;
        self.highlighted = if self.candidates.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn current_page(&self) -> &[Candidate] {
        if self.candidates.is_empty() {
            return &[];
        }

        let start = self.page.saturating_mul(self.page_size);
        let end = start
            .saturating_add(self.page_size)
            .min(self.candidates.len());
        if start >= end {
            &[]
        } else {
            &self.candidates[start..end]
        }
    }

    fn change_page(&mut self, delta: isize) {
        let page_count = self.page_count();
        if page_count == 0 {
            self.page = 0;
            self.highlighted = None;
            return;
        }

        if delta < 0 {
            self.page = self.page.saturating_sub(delta.unsigned_abs());
        } else {
            self.page = self.page.saturating_add(delta as usize).min(page_count - 1);
        }
        self.highlighted = if self.current_page().is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn move_highlight(&mut self, delta: isize) {
        let candidate_count = self.current_page().len();
        if candidate_count == 0 {
            self.highlighted = None;
            return;
        }

        let current = self.highlighted.unwrap_or(0).min(candidate_count - 1);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(candidate_count - 1)
        };
        self.highlighted = Some(next);
    }

    fn clear_composition(&mut self) {
        self.preedit.clear();
        self.candidates.clear();
        self.page = 0;
        self.highlighted = None;
    }

    fn snapshot(&self, handled: bool) -> CompositionUpdate {
        CompositionUpdate {
            handled,
            preedit: self.preedit.clone(),
            candidates: self.current_page().to_vec(),
            highlighted: self.highlighted,
            page: self.page,
            page_count: self.page_count(),
            commit: None,
            cancelled: false,
        }
    }
}
