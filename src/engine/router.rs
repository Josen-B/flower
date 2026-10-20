use super::{CompositionUpdate, EngineError, KeyEvent, Lexicon, PinyinSession, DEFAULT_PAGE_SIZE};

/// Routes platform-independent key events into one owned Pinyin session.
///
/// The router is the only mutable entry point for key events. Read-only
/// callers can preview a key with [`EngineRouter::test_key`] without changing
/// the active composition.
#[derive(Debug, Clone)]
pub struct EngineRouter {
    session: PinyinSession,
}

impl EngineRouter {
    /// Creates a router backed by the embedded MVP lexicon.
    pub fn new() -> Result<Self, EngineError> {
        Self::with_page_size(DEFAULT_PAGE_SIZE)
    }

    /// Creates a router with a custom candidate page size.
    pub fn with_page_size(page_size: usize) -> Result<Self, EngineError> {
        PinyinSession::with_page_size(page_size).map(Self::from_session)
    }

    /// Creates a router from caller-provided TSV data.
    pub fn from_lexicon_tsv(tsv: &str, page_size: usize) -> Result<Self, EngineError> {
        PinyinSession::from_lexicon_tsv(tsv, page_size).map(Self::from_session)
    }

    /// Creates a router from an already validated lexicon.
    pub fn from_lexicon(lexicon: Lexicon, page_size: usize) -> Result<Self, EngineError> {
        PinyinSession::from_lexicon(lexicon, page_size).map(Self::from_session)
    }

    /// Reports whether a key would be handled without changing the session.
    pub fn test_key(&self, event: KeyEvent) -> Result<bool, EngineError> {
        let mut preview = self.session.clone();
        preview.process_key(event).map(|update| update.handled)
    }

    /// Processes one key and advances the active composition.
    pub fn process_key(&mut self, event: KeyEvent) -> Result<CompositionUpdate, EngineError> {
        self.session.process_key(event)
    }

    /// Returns true while a composition is active.
    pub fn is_composing(&self) -> bool {
        self.session.is_composing()
    }

    /// Returns the current raw Pinyin input.
    pub fn preedit(&self) -> &str {
        self.session.preedit()
    }

    /// Returns the owned session for read-only inspection.
    pub fn session(&self) -> &PinyinSession {
        &self.session
    }

    fn from_session(session: PinyinSession) -> Self {
        Self { session }
    }
}
