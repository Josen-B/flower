use flower::engine::{EngineError, EngineRouter, Key, KeyEvent, PinyinSession};

const WORDS: &str = "wo\t我\t1000\nai\t爱\t900\nzhong\t中\t800\nguo\t国\t800\nzhong'guo\t中国\t2000\nni\t你\t1000\nhao\t好\t1000\nni'hao\t你好\t2000\nxi\t西\t100\nan\t安\t100\nxian\t先\t1000\nxi'an\t西安\t2000";

fn session() -> PinyinSession {
    PinyinSession::from_lexicon_tsv(WORDS, 9).unwrap()
}
fn type_text(s: &mut PinyinSession, text: &str) {
    for c in text.chars() {
        assert!(
            s.process_key(KeyEvent::plain(Key::Char(c)))
                .unwrap()
                .handled
        );
    }
}
fn choose(s: &mut PinyinSession, text: &str) -> flower::CompositionUpdate {
    for _ in 0..s.page_count() {
        if let Some(index) = s
            .candidates_on_current_page()
            .iter()
            .position(|c| c.text == text)
        {
            return s.select_candidate(index).unwrap();
        }
        s.next_page().unwrap();
    }
    panic!("missing {text}");
}

#[test]
fn combines_words_without_a_hardcoded_sentence_entry() {
    let mut s = session();
    type_text(&mut s, "woaizhongguo");
    let update = choose(&mut s, "我爱中国");
    assert_eq!(update.commit.as_deref(), Some("我爱中国"));
    assert_eq!(s.pending_learning().len(), 3);
}

#[test]
fn partial_selection_keeps_one_composition_and_can_be_cancelled() {
    let mut s = session();
    type_text(&mut s, "nihao");
    let update = choose(&mut s, "你");
    assert_eq!(update.commit, None);
    assert_eq!(update.preedit, "你hao");
    assert_eq!(s.pending_input(), "hao");
    assert!(s.pending_learning().is_empty());
    assert!(s.cancel().unwrap().cancelled);
    assert!(s.pending_learning().is_empty());
    assert_eq!(s.preedit(), "");
}

#[test]
fn partial_selection_then_final_selection_commits_once() {
    let mut s = session();
    type_text(&mut s, "nihao");
    choose(&mut s, "你");
    let final_update = choose(&mut s, "好");
    assert_eq!(final_update.commit.as_deref(), Some("你好"));
    assert_eq!(s.pending_learning().len(), 2);
}

#[test]
fn backspace_can_restore_the_last_confirmed_segment() {
    let mut s = session();
    type_text(&mut s, "nihao");
    choose(&mut s, "你");
    for _ in 0..3 {
        s.process_key(KeyEvent::plain(Key::Backspace)).unwrap();
    }
    assert_eq!(s.preedit(), "你");
    s.process_key(KeyEvent::plain(Key::Backspace)).unwrap();
    assert_eq!(s.preedit(), "ni");
    assert_eq!(s.pending_input(), "ni");
}

#[test]
fn explicit_apostrophe_excludes_a_single_syllable_interpretation() {
    let mut s = session();
    type_text(&mut s, "xi'an");
    assert!(!s
        .candidates_on_current_page()
        .iter()
        .any(|c| c.text == "先"));
    assert_eq!(choose(&mut s, "西安").commit.as_deref(), Some("西安"));
}

#[test]
fn every_partial_choice_consumes_its_following_delimiter() {
    let mut s = session();
    type_text(&mut s, "xi'an");
    let mut matches = 0;
    for _ in 0..s.page_count() {
        for (index, candidate) in s.candidates_on_current_page().iter().enumerate() {
            if candidate.text == "西" {
                let mut selected = s.clone();
                selected.select_candidate(index).unwrap();
                assert_eq!(selected.pending_input(), "an");
                matches += 1;
            }
        }
        s.next_page().unwrap();
    }
    assert_eq!(matches, 1);
}

#[test]
fn enter_never_silently_consumes_only_a_partial_candidate() {
    let mut s = session();
    type_text(&mut s, "nixyz");
    assert_eq!(
        s.process_key(KeyEvent::plain(Key::Enter))
            .unwrap()
            .commit
            .as_deref(),
        Some("nixyz")
    );
    assert!(s.pending_learning().is_empty());
}

#[test]
fn oversized_input_is_rejected_before_mutation_and_preview_does_not_learn() {
    let mut s = session();
    type_text(&mut s, &"z".repeat(128));
    assert_eq!(
        s.process_key(KeyEvent::plain(Key::Char('z'))),
        Err(EngineError::InputTooLong)
    );
    assert_eq!(s.pending_input().len(), 128);
    let mut router = EngineRouter::from_lexicon_tsv(WORDS, 9).unwrap();
    for c in "nihao".chars() {
        router.process_key(KeyEvent::plain(Key::Char(c))).unwrap();
    }
    assert_eq!(router.test_key(KeyEvent::plain(Key::Space)), Ok(true));
    assert!(router.pending_learning().is_empty());
    assert_eq!(router.preedit(), "nihao");
}

#[test]
fn sentence_scoring_keeps_frequency_order_inside_one_logarithm_bucket() {
    let mut s = PinyinSession::from_lexicon_tsv(
        "wo\t我\t1000\nshi\t是\t1000\nshi\t事\t900\nzhong'guo\t中国\t2000\nren\t人\t1000",
        9,
    )
    .unwrap();
    type_text(&mut s, "woshizhongguoren");
    assert_eq!(s.candidates_on_current_page()[0].text, "我是中国人");
}

#[test]
fn an_extreme_external_candidate_index_is_an_error_not_an_overflow() {
    let mut s = session();
    type_text(&mut s, "ni");
    assert!(matches!(
        s.select_candidate(usize::MAX),
        Err(EngineError::CandidateOutOfRange { .. })
    ));
    assert_eq!(s.preedit(), "ni");
}
