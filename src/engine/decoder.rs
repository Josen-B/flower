//! Bounded word graph: exact results stay complete, composed alternatives are Top-K.
use super::{Candidate, LearningEvent, Lexicon};
use std::collections::HashSet;

const BEAM: usize = 8;
const EDGE_ALTERNATIVES: usize = 4;

#[derive(Clone, Debug)]
pub(crate) struct Choice {
    pub candidate: Candidate,
    pub consumed: usize,
    pub events: Vec<LearningEvent>,
    score: i64,
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
        }
    }
}

#[derive(Clone)]
struct Path {
    text: String,
    reading: String,
    events: Vec<LearningEvent>,
    score: i64,
    words: usize,
}

fn trim(paths: &mut Vec<Path>) {
    paths.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.words.cmp(&b.words))
            .then_with(|| a.text.cmp(&b.text))
            .then_with(|| a.reading.cmp(&b.reading))
    });
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.text.clone()));
    paths.truncate(BEAM);
}

pub(crate) fn decode(lexicon: &Lexicon, raw: &str) -> Vec<Choice> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut output = lexicon.exact_choices(raw);
    let mut graph: Vec<Vec<Path>> = vec![Vec::new(); raw.len() + 1];
    graph[0].push(Path {
        text: String::new(),
        reading: String::new(),
        events: Vec::new(),
        score: 0,
        words: 0,
    });
    for start in 0..raw.len() {
        if graph[start].is_empty() {
            continue;
        }
        let edges = lexicon.prefix_choices_limited(&raw[start..], EDGE_ALTERNATIVES);
        let mut per_end = std::collections::HashMap::new();
        for edge in edges {
            let count = per_end.entry(edge.consumed).or_insert(0usize);
            if *count >= EDGE_ALTERNATIVES {
                continue;
            }
            *count += 1;
            let end = start + edge.consumed;
            for path in graph[start].clone() {
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
    for path in &graph[raw.len()] {
        if path.words > 1 && seen.insert((path.text.clone(), raw.len())) {
            output.push(Choice {
                candidate: Candidate::new(&path.text, &path.reading),
                consumed: raw.len(),
                events: path.events.clone(),
                score: path.score,
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
