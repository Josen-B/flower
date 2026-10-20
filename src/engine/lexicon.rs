use std::collections::HashMap;

use super::{Candidate, EngineError};

const BUILTIN_LEXICON: &str = include_str!("../../data/pinyin/mvp_lexicon.tsv");

#[derive(Debug, Clone, PartialEq, Eq)]
struct LexiconEntry {
    text: String,
    weight: u32,
}

/// A deterministic, exact-match Pinyin lexicon.
///
/// TSV rows use three tab-separated UTF-8 fields:
/// `lowercase-pinyin<TAB>candidate-text<TAB>weight`.
/// Empty lines and lines whose first non-space character is `#` are ignored.
#[derive(Debug, Clone)]
pub struct Lexicon {
    entries: HashMap<String, Vec<LexiconEntry>>,
}

impl Lexicon {
    /// Loads the small lexicon embedded in Flower at compile time.
    pub fn builtin() -> Result<Self, EngineError> {
        Self::from_tsv(BUILTIN_LEXICON)
    }

    /// Parses and validates a UTF-8 TSV lexicon.
    pub fn from_tsv(tsv: &str) -> Result<Self, EngineError> {
        let tsv = tsv.strip_prefix('\u{feff}').unwrap_or(tsv);
        let mut entries: HashMap<String, Vec<LexiconEntry>> = HashMap::new();

        for (index, raw_line) in tsv.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim_end_matches('\r');
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let mut fields = line.split('\t');
            let key = fields.next().unwrap_or_default().trim();
            let text = match fields.next() {
                Some(value) => value.trim(),
                None => {
                    return Err(EngineError::invalid_line(
                        line_number,
                        "expected pinyin, text, and weight fields",
                    ))
                }
            };
            let weight = match fields.next() {
                Some(value) => value.trim(),
                None => {
                    return Err(EngineError::invalid_line(
                        line_number,
                        "missing weight field",
                    ))
                }
            };
            if fields.next().is_some() {
                return Err(EngineError::invalid_line(
                    line_number,
                    "expected exactly three tab-separated fields",
                ));
            }

            if key.is_empty() || !key.bytes().all(|byte| byte.is_ascii_lowercase()) {
                return Err(EngineError::invalid_line(
                    line_number,
                    "pinyin must contain lowercase ASCII letters only",
                ));
            }
            if text.is_empty() {
                return Err(EngineError::invalid_line(
                    line_number,
                    "candidate text must not be empty",
                ));
            }
            if text.chars().any(char::is_control) {
                return Err(EngineError::invalid_line(
                    line_number,
                    "candidate text must not contain control characters",
                ));
            }

            let weight: u32 = weight.parse().map_err(|_| {
                EngineError::invalid_line(line_number, "weight must be an unsigned 32-bit integer")
            })?;
            if weight == 0 {
                return Err(EngineError::invalid_line(
                    line_number,
                    "weight must be greater than zero",
                ));
            }

            entries
                .entry(key.to_owned())
                .or_default()
                .push(LexiconEntry {
                    text: text.to_owned(),
                    weight,
                });
        }

        if entries.is_empty() {
            return Err(EngineError::EmptyLexicon);
        }

        for candidates in entries.values_mut() {
            candidates.sort_by(|left, right| {
                right
                    .weight
                    .cmp(&left.weight)
                    .then_with(|| left.text.cmp(&right.text))
            });
            candidates.dedup_by(|left, right| left.text == right.text);
        }

        Ok(Self { entries })
    }

    /// Parses a byte slice, reporting invalid UTF-8 as an engine error.
    pub fn from_tsv_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        let text = std::str::from_utf8(bytes).map_err(|error| EngineError::InvalidUtf8 {
            message: error.to_string(),
        })?;
        Self::from_tsv(text)
    }

    /// Returns ranked candidates for an exact Pinyin key.
    pub fn lookup(&self, pinyin: &str) -> Vec<Candidate> {
        self.entries
            .get(pinyin)
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| Candidate::new(entry.text.clone(), pinyin))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Number of distinct Pinyin keys in this lexicon.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
