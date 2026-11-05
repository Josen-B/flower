//! Bounded word graph: exact results stay complete, composed alternatives are Top-K.
use super::{Candidate, LearningEvent, Lexicon};
use std::collections::HashSet;

const BEAM: usize = 8;
const EDGE_ALTERNATIVES: usize = 4;
const MIXED_DECODE_BUDGET: usize = 16384;

#[derive(Clone, Debug)]
pub(crate) struct Choice {
    pub candidate: Candidate,
    pub consumed: usize,
    pub events: Vec<LearningEvent>,
    score: i64,
    pub(crate) abbreviated: usize,
}

impl Choice {
    pub fn word(candidate: Candidate, consumed: usize, weight: u32) -> Self {
        let events = if candidate.comment.split('\'').count() == candidate.text.chars().count()
            && candidate
                .text
                .chars()
                .all(|c| !c.is_ascii() && !c.is_control())
        {
            vec![LearningEvent {
                pinyin: candidate.comment.clone(),
                text: candidate.text.clone(),
            }]
        } else {
            Vec::new()
        };
        let logarithm = 31 - weight.leading_zeros();
        let bucket = 1u64 << logarithm;
        let fraction = (u64::from(weight) - bucket) * 64 / bucket;
        let score = i64::from(logarithm) * 64
            + fraction as i64
            + candidate.text.chars().count() as i64 * 128
            - 720;
        Self {
            candidate,
            consumed,
            events,
            score,
            abbreviated: 0,
        }
    }

    pub(crate) fn penalize_abbreviations(&mut self) {
        self.score -= self.abbreviated as i64 * 512;
    }
}

#[derive(Clone)]
struct Path {
    text: String,
    reading: String,
    events: Vec<LearningEvent>,
    score: i64,
    words: usize,
    abbreviated: usize,
}

fn trim(paths: &mut Vec<Path>) {
    paths.sort_by(|a, b| {
        a.abbreviated
            .cmp(&b.abbreviated)
            .then_with(|| a.words.cmp(&b.words))
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.text.cmp(&b.text))
            .then_with(|| a.reading.cmp(&b.reading))
    });
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.text.clone()));
    paths.truncate(BEAM);
}

pub(crate) fn decode(lexicon: &Lexicon, raw: &str) -> Vec<Choice> {
    decode_with_budget(lexicon, raw, &mut 0, MIXED_DECODE_BUDGET)
}

fn decode_with_budget(
    lexicon: &Lexicon,
    raw: &str,
    expanded: &mut usize,
    limit: usize,
) -> Vec<Choice> {
    if raw.is_empty()
        || !raw.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')
        || raw.starts_with('\'')
        || raw.contains("''")
    {
        return Vec::new();
    }
    let mut output = lexicon.exact_choices(raw);
    let mut whole_mixed = Vec::new();
    let mut graph: Vec<Vec<Path>> = vec![Vec::new(); raw.len() + 1];
    graph[0].push(Path {
        text: String::new(),
        reading: String::new(),
        events: Vec::new(),
        score: 0,
        words: 0,
        abbreviated: 0,
    });
    for start in 0..raw.len() {
        if graph[start].is_empty() {
            continue;
        }
        let mixed = lexicon.mixed_prefix_choices(&raw[start..], expanded, limit);
        if start == 0 {
            whole_mixed = mixed
                .iter()
                .filter(|c| c.consumed == raw.len())
                .cloned()
                .collect();
        }
        let mut edges = lexicon.graph_choices(&raw[start..], EDGE_ALTERNATIVES);
        edges.extend(mixed);
        // Exact/initial candidates keep their existing rank within equal abbreviation
        // counts. Sort before the per-end cap so less abbreviated mixed edges survive.
        edges.sort_by_key(|edge| edge.abbreviated);
        let mut per_end = std::collections::HashMap::new();
        let mut edge_seen = HashSet::new();
        let paths = graph[start].clone();
        for edge in edges {
            let count = per_end.entry(edge.consumed).or_insert(0usize);
            if *count >= EDGE_ALTERNATIVES {
                continue;
            }
            if !edge_seen.insert((
                edge.consumed,
                edge.candidate.text.clone(),
                edge.candidate.comment.clone(),
            )) {
                continue;
            }
            *count += 1;
            let end = start + edge.consumed;
            for path in &paths {
                let mut events = path.events.clone();
                events.extend(edge.events.clone());
                graph[end].push(Path {
                    text: format!("{}{}", path.text, edge.candidate.text),
                    reading: if path.reading.is_empty() {
                        edge.candidate.comment.clone()
                    } else {
                        format!("{}'{}", path.reading, edge.candidate.comment)
                    },
                    events,
                    score: path.score + edge.score,
                    words: path.words + 1,
                    abbreviated: path.abbreviated + edge.abbreviated,
                });
            }
            trim(&mut graph[end]);
        }
    }
    let mut seen: HashSet<_> = output
        .iter()
        .map(|c| (c.candidate.text.clone(), c.consumed))
        .collect();
    output.extend(
        lexicon
            .initial_choices(raw)
            .into_iter()
            .filter(|c| seen.insert((c.candidate.text.clone(), c.consumed))),
    );
    output.extend(
        whole_mixed
            .into_iter()
            .filter(|c| seen.insert((c.candidate.text.clone(), c.consumed))),
    );
    for path in &graph[raw.len()] {
        if path.words > 1 && seen.insert((path.text.clone(), raw.len())) {
            output.push(Choice {
                candidate: Candidate::new(&path.text, &path.reading),
                consumed: raw.len(),
                events: path.events.clone(),
                score: path.score,
                abbreviated: path.abbreviated,
            });
        }
    }
    let mut partial: Vec<_> = lexicon
        .prefix_choices(raw)
        .into_iter()
        .filter(|c| c.consumed < raw.len())
        .collect();
    partial.sort_by(|a, b| {
        b.consumed
            .cmp(&a.consumed)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.candidate.text.cmp(&b.candidate.text))
    });
    output.extend(
        partial
            .into_iter()
            .filter(|c| seen.insert((c.candidate.text.clone(), c.consumed))),
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_shares_one_instrumented_mixed_budget_and_preserves_exact_initials() {
        let lexicon = Lexicon::builtin().unwrap();
        for raw in ["shenmeys".to_owned(), "shi".repeat(42), "s".repeat(128)] {
            let mut expanded = 0;
            decode_with_budget(&lexicon, &raw, &mut expanded, MIXED_DECODE_BUDGET);
            assert!(expanded <= MIXED_DECODE_BUDGET, "{raw}: {expanded}");
            let mut exhausted = 0;
            decode_with_budget(&lexicon, &raw, &mut exhausted, 7);
            assert_eq!(exhausted, 7, "{raw}");
        }
        for (raw, expected) in [("nihao", "你好"), ("nh", "你好")] {
            let choices = decode_with_budget(&lexicon, raw, &mut 0, 0);
            assert!(choices
                .iter()
                .any(|c| c.candidate.text == expected && c.consumed == raw.len()));
        }
    }

    #[test]
    fn mixed_rejects_missing_initials_partial_syllables_and_false_boundaries() {
        let lexicon = Lexicon::from_tsv("shen'me'yi'si\t什么意思\t100").unwrap();
        for raw in [
            "shenme",
            "shenmey",
            "shenmeyi",
            "sheme ys",
            "shenme!ys",
            "'shenmeys",
            "shen''meys",
            "she'nmeys",
            "shen'm'eys",
            "shenme'yi'sh",
        ] {
            assert!(
                !decode(&lexicon, raw)
                    .iter()
                    .any(|c| c.candidate.text == "什么意思"),
                "{raw}"
            );
        }
        for raw in ["shenmeys", "shen'me'y's", "shenmeys'"] {
            let choices = decode(&lexicon, raw);
            assert_eq!(choices[0].candidate.text, "什么意思", "{raw}");
            assert_eq!(choices[0].consumed, raw.len());
        }
    }

    #[test]
    fn every_full_initial_product_is_matched_without_storing_products() {
        let lexicon = Lexicon::from_tsv("shen'me'yi'si\t什么意思\t100").unwrap();
        for mask in 0..16 {
            let raw: String = ["shen", "me", "yi", "si"]
                .iter()
                .enumerate()
                .map(|(i, s)| if mask & (1 << i) == 0 { *s } else { &s[..1] })
                .collect();
            assert_eq!(
                decode(&lexicon, &raw)[0].candidate.text,
                "什么意思",
                "{raw}"
            );
        }
    }
}
