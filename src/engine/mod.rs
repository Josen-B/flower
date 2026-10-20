#![forbid(unsafe_code)]
//! Platform-independent Pinyin input state and candidate generation.
//!
//! The platform layer translates Windows virtual keys into [`KeyEvent`] values
//! and routes them through [`EngineRouter`]. The resulting
//! [`CompositionUpdate`] values are rendered by the platform layer.

mod error;
mod key;
mod lexicon;
mod router;
mod session;
mod types;

pub mod keymap;

pub use error::EngineError;
pub use key::{Key, KeyEvent, Modifiers};
pub use keymap::map_windows_virtual_key;
pub use lexicon::Lexicon;
pub use router::EngineRouter;
pub use session::{EngineSession, PinyinSession, DEFAULT_PAGE_SIZE};
pub use types::{Candidate, CompositionUpdate};
