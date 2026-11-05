use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::{Candidate, EngineError};

const BUILTIN_LEXICON: &str = concat!(
    include_str!("../../data/pinyin/base_lexicon.tsv"),
    "\n",
    include_str!("../../data/pinyin/phrases/phrase_lexicon.tsv"),
);

// Project-maintained everyday vocabulary, NOT corpus frequency or new readings.
// Only existing canonical, multi-syllable entries are classified at builtin load.
const EVERYDAY_WORDS: &str = "\
你好 您好 谢谢 再见 我们 你们 他们 什么 怎么 为什么 可以 需要 知道 喜欢 \
今天 明天 现在 时间 工作 学习 电脑 手机 输入法 中国 大家 自己 一个 这个 那个 \
这些 那些 这里 那里 这样 那样 多少 时候 已经 还是 因为 所以 如果 但是 而且 \
一起 一直 一定 可能 应该 没有 不是 不要 不能 不好 非常 真的 觉得 希望 \
朋友 家人 孩子 老师 学生 学校 公司 生活 世界 北京 上海 早上 晚上 下午 上午 \
中午 昨天 以后 以前 开始 结束 继续 等待 帮助 问题 事情 方法 结果 消息 电话 \
地址 名字 号码 邮件 文件 软件 系统 网络 网站 密码 设置 打开 关闭 保存 选择 \
使用 安装 更新 语言 中文 拼音 文字 音乐 电影 游戏 天气 快乐 高兴 开心 健康 \
安全 吃饭 喝水 回家 出门 上班 下班 休息 睡觉 购买 价格 生日 欢迎 对不起 \
没关系 不客气 请问";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LexiconEntry {
    text: String,
    weight: u32,
    reading: String,
    everyday: bool,
}

fn rank_entries(left: &LexiconEntry, right: &LexiconEntry) -> std::cmp::Ordering {
    right
        .everyday
        .cmp(&left.everyday)
        .then_with(|| right.weight.cmp(&left.weight))
        .then_with(|| left.text.cmp(&right.text))
        .then_with(|| left.reading.cmp(&right.reading))
}

/// A deterministic, exact-match Pinyin lexicon.
///
/// TSV rows use three tab-separated UTF-8 fields:
/// `lowercase-pinyin<TAB>candidate-text<TAB>weight`.
/// Empty lines and lines whose first non-space character is `#` are ignored.
#[derive(Debug, Clone)]
pub struct Lexicon {
    entries: Arc<HashMap<String, Vec<LexiconEntry>>>,
    initials: Arc<HashMap<String, Vec<LexiconEntry>>>,
}

static BUILTIN: OnceLock<Result<Lexicon, EngineError>> = OnceLock::new();

impl Lexicon {
    /// Loads the shared single-character base and core phrases embedded at build time.
    pub fn builtin() -> Result<Self, EngineError> {
        BUILTIN
            .get_or_init(|| {
                let mut lexicon = Self::from_tsv(BUILTIN_LEXICON)?;
                let words: std::collections::HashSet<_> =
                    EVERYDAY_WORDS.split_whitespace().collect();
                let entries = Arc::make_mut(&mut lexicon.entries);
                for candidates in entries.values_mut() {
                    for entry in candidates.iter_mut() {
                        let syllables = entry.reading.split('\'').count();
                        entry.everyday = syllables >= 2
                            && syllables == entry.text.chars().count()
                            && words.contains(entry.text.as_str());
                    }
                    candidates.sort_by(rank_entries);
                }
                lexicon.initials = Arc::new(Self::build_initials(entries));
                Ok(lexicon)
            })
            .clone()
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

            if !super::segment::valid_reading(key) {
                return Err(EngineError::invalid_line(
                    line_number,
                    "pinyin must contain lowercase ASCII syllables separated by apostrophes",
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
                .entry(key.replace('\'', ""))
                .or_default()
                .push(LexiconEntry {
                    text: text.to_owned(),
                    weight,
                    reading: key.to_owned(),
                    everyday: false,
                });
        }

        if entries.is_empty() {
            return Err(EngineError::EmptyLexicon);
        }

        for candidates in entries.values_mut() {
            // Group equal text before weight ordering: equal text with different
            // weights need not be adjacent in a weight-first sorted list.
            candidates.sort_by(|left, right| {
                left.text
                    .cmp(&right.text)
                    .then_with(|| left.reading.cmp(&right.reading))
                    .then_with(|| right.weight.cmp(&left.weight))
            });
            candidates
                .dedup_by(|left, right| left.text == right.text && left.reading == right.reading);
            candidates.sort_by(rank_entries);
        }

        Ok(Self {
            initials: Arc::new(Self::build_initials(&entries)),
            entries: Arc::new(entries),
        })
    }

    fn build_initials(
        entries: &HashMap<String, Vec<LexiconEntry>>,
    ) -> HashMap<String, Vec<LexiconEntry>> {
        let mut initials: HashMap<String, Vec<LexiconEntry>> = HashMap::new();
        for entry in entries.values().flatten() {
            let syllables: Vec<_> = entry.reading.split('\'').collect();
            if syllables.len() < 2 || syllables.len() != entry.text.chars().count() {
                continue;
            }
            let key: String = syllables.iter().map(|s| s.as_bytes()[0] as char).collect();
            initials.entry(key).or_default().push(entry.clone());
        }
        for words in initials.values_mut() {
            words.sort_by(rank_entries);
        }
        initials
    }

    pub(crate) fn initial_choices(&self, raw: &str) -> Vec<super::decoder::Choice> {
        if raw.contains('\'') {
            return Vec::new();
        }
        self.initials
            .get(raw)
            .into_iter()
            .flatten()
            .map(|entry| {
                super::decoder::Choice::word(
                    Candidate::new(&entry.text, &entry.reading),
                    raw.len(),
                    entry.weight,
                )
            })
            .collect()
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
        self.exact_choices(pinyin)
            .into_iter()
            .map(|choice| choice.candidate)
            .collect()
    }

    /// Make one immutable activation snapshot with explicitly supplied local words.
    pub fn with_overlay(&self, tsv: &str) -> Result<Self, EngineError> {
        if tsv.trim().is_empty() {
            return Ok(self.clone());
        }
        let overlay = Self::from_tsv(tsv)?;
        let mut result = self.clone();
        let table = Arc::make_mut(&mut result.entries);
        for (key, additions) in overlay.entries.iter() {
            let entries = table.entry(key.clone()).or_default();
            for addition in additions {
                if let Some(existing) = entries
                    .iter_mut()
                    .find(|entry| entry.text == addition.text && entry.reading == addition.reading)
                {
                    existing.weight = existing.weight.max(addition.weight);
                } else {
                    entries.push(addition.clone());
                }
            }
            entries.sort_by(rank_entries);
        }
        result.initials = Arc::new(Self::build_initials(table));
        Ok(result)
    }

    /// Bounded local adjustment; never creates a word absent from the active dictionary.
    pub fn with_learning(&self, events: &[(super::LearningEvent, u32)]) -> Self {
        if events.is_empty() {
            return self.clone();
        }
        let mut result = self.clone();
        let table = Arc::make_mut(&mut result.entries);
        for (event, count) in events {
            if let Some(entries) = table.get_mut(&event.pinyin.replace('\'', "")) {
                for entry in entries
                    .iter_mut()
                    .filter(|e| e.text == event.text && e.reading == event.pinyin)
                {
                    entry.weight = entry
                        .weight
                        .saturating_add(count.saturating_mul(512).min(8192));
                }
                entries.sort_by(rank_entries);
            }
        }
        result.initials = Arc::new(Self::build_initials(table));
        result
    }

    pub(crate) fn exact_choices(&self, raw: &str) -> Vec<super::decoder::Choice> {
        self.exact_choices_limited(raw, usize::MAX)
    }

    fn exact_choices_limited(&self, raw: &str, limit: usize) -> Vec<super::decoder::Choice> {
        let compact = raw.replace('\'', "");
        let mut seen = std::collections::HashSet::new();
        self.entries
            .get(&compact)
            .into_iter()
            .flatten()
            .filter(|entry| super::segment::boundaries_match(raw, &entry.reading))
            .filter(|entry| seen.insert(entry.text.clone()))
            .take(limit)
            .map(|entry| {
                super::decoder::Choice::word(
                    Candidate::new(&entry.text, &entry.reading),
                    raw.len(),
                    entry.weight,
                )
            })
            .collect()
    }

    pub(crate) fn prefix_choices(&self, raw: &str) -> Vec<super::decoder::Choice> {
        self.prefix_choices_limited(raw, usize::MAX)
    }

    pub(crate) fn prefix_choices_limited(
        &self,
        raw: &str,
        per_prefix: usize,
    ) -> Vec<super::decoder::Choice> {
        let mut choices = Vec::new();
        for end in 1..=raw.len() {
            // Raw composition is restricted to ASCII before this function is called.
            if !raw.is_char_boundary(end) {
                continue;
            }
            // The delimiter belongs to the preceding span. Never offer a
            // prefix whose selection strands a leading apostrophe in the tail.
            if raw.as_bytes().get(end) == Some(&b'\'') {
                continue;
            }
            choices.extend(self.exact_choices_limited(&raw[..end], per_prefix));
        }
        choices
    }

    /// Number of distinct Pinyin keys in this lexicon.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyday_layer_only_classifies_existing_builtin_words_without_changing_weights() {
        let raw = Lexicon::from_tsv(BUILTIN_LEXICON).unwrap();
        let builtin = Lexicon::builtin().unwrap();
        assert_eq!(raw.len(), builtin.len());
        let mut classified = std::collections::HashSet::new();
        for (key, entries) in raw.entries.iter() {
            let ranked = &builtin.entries[key];
            assert_eq!(entries.len(), ranked.len());
            for entry in entries {
                let actual = ranked
                    .iter()
                    .find(|e| e.text == entry.text && e.reading == entry.reading)
                    .unwrap();
                assert_eq!(actual.weight, entry.weight);
                assert!(!entry.everyday, "custom TSV must remain unclassified");
                if actual.everyday {
                    assert!(actual.reading.contains('\''));
                    assert_eq!(
                        actual.reading.split('\'').count(),
                        actual.text.chars().count()
                    );
                    classified.insert(actual.text.clone());
                }
            }
            assert!(ranked
                .windows(2)
                .all(|w| !rank_entries(&w[0], &w[1]).is_gt()));
        }
        assert!(
            (80..=150).contains(&classified.len()),
            "{}",
            classified.len()
        );
        for words in builtin.initials.values() {
            assert!(words
                .windows(2)
                .all(|w| !rank_entries(&w[0], &w[1]).is_gt()));
        }
    }

    #[test]
    fn builtin_snapshots_keep_classes_and_user_weights_separate() {
        let base = Lexicon::builtin().unwrap();
        let before = base.lookup("nihao");
        let overlay = base
            .with_overlay("ni'hao\t你好\t4294967295\nni'hao\t私词\t4294967295")
            .unwrap();
        let entries = &overlay.entries["nihao"];
        let hello = entries.iter().find(|e| e.text == "你好").unwrap();
        let private = entries.iter().find(|e| e.text == "私词").unwrap();
        assert!(hello.everyday);
        assert!(!private.everyday);
        assert_eq!(hello.weight, u32::MAX);
        assert_eq!(private.weight, u32::MAX);
        assert_eq!(overlay.lookup("nihao")[0].text, "你好");
        let learned = overlay.with_learning(&[(
            super::super::LearningEvent {
                pinyin: "ni'hao".into(),
                text: "私词".into(),
            },
            u32::MAX,
        )]);
        assert_eq!(learned.entries["nihao"], overlay.entries["nihao"]);
        assert_eq!(base.lookup("nihao"), before);
        assert_eq!(Lexicon::builtin().unwrap().lookup("nihao"), before);
    }

    #[test]
    fn builtin_and_clones_share_the_read_only_table() {
        let first = Lexicon::builtin().unwrap();
        let second = Lexicon::builtin().unwrap();
        let snapshot = first.clone();
        assert!(Arc::ptr_eq(&first.entries, &second.entries));
        assert!(Arc::ptr_eq(&first.entries, &snapshot.entries));
        assert!(Arc::ptr_eq(&first.initials, &second.initials));
        assert!(Arc::ptr_eq(&first.initials, &snapshot.initials));
        let custom = Lexicon::from_tsv("ni\t你\t1").unwrap();
        assert!(!Arc::ptr_eq(&first.entries, &custom.entries));
    }

    #[test]
    fn duplicate_text_keeps_highest_weight_even_when_separated_by_other_weights() {
        let lexicon = Lexicon::from_tsv("ni\t你\t100\nni\t尼\t80\nni\t你\t50").unwrap();
        let candidates = lexicon.lookup("ni");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].text, "你");
        assert_eq!(lexicon.entries["ni"][0].weight, 100);
    }

    #[test]
    fn every_baseline_character_is_reachable_through_the_actual_lookup() {
        use std::collections::HashSet;
        let lexicon = Lexicon::builtin().unwrap();
        let reachable: HashSet<_> = lexicon
            .entries
            .keys()
            .flat_map(|key| lexicon.lookup(key))
            .filter(|candidate| candidate.text.chars().count() == 1)
            .map(|candidate| candidate.text.chars().next().unwrap())
            .collect();
        let mut baseline = HashSet::new();
        for line in include_str!("../../data/pinyin/sources/kMandarin_8105.txt").lines() {
            if !line.starts_with("U+") {
                continue;
            }
            let token = line.split(':').next().unwrap();
            let char = char::from_u32(u32::from_str_radix(&token[2..], 16).unwrap()).unwrap();
            assert!(baseline.insert(char), "duplicate baseline character");
            assert!(
                reachable.contains(&char),
                "unreachable baseline character {token}"
            );
        }
        assert_eq!(baseline.len(), 8105);
        assert!(reachable.len() > 40000);
    }

    #[test]
    fn multi_readings_umlaut_and_supplementary_characters_are_available() {
        let lexicon = Lexicon::builtin().unwrap();
        for (key, text) in [
            ("chong", "重"),
            ("zhong", "重"),
            ("hang", "行"),
            ("xing", "行"),
            ("lv", "吕"),
            ("nv", "女"),
            ("lve", "略"),
            ("nve", "虐"),
            ("he", "𠀀"),
        ] {
            assert!(
                lexicon
                    .lookup(key)
                    .iter()
                    .any(|candidate| candidate.text == text),
                "missing {text} for {key}"
            );
        }
    }
}
