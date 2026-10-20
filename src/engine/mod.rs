#![forbid(unsafe_code)]
//! Platform-independent Pinyin input state and candidate generation.
//!
//! The platform layer translates Windows virtual keys into [`KeyEvent`] values
//! and renders the [`CompositionUpdate`] values produced by [`PinyinSession`].

mod error;
mod key;
mod lexicon;
mod session;
mod types;

pub mod keymap;

pub use error::EngineError;
pub use key::{Key, KeyEvent, Modifiers};
pub use keymap::map_windows_virtual_key;
pub use lexicon::Lexicon;
pub use session::{EngineSession, PinyinSession, DEFAULT_PAGE_SIZE};
pub use types::{Candidate, CompositionUpdate};
