use flower::engine::keymap::{map_windows_virtual_key, VK_BACK, VK_NEXT, VK_PRIOR, VK_RIGHT};
use flower::engine::{
    CompositionUpdate, EngineError, EngineSession, Key, KeyEvent, Lexicon, Modifiers,
    PinyinSession, DEFAULT_PAGE_SIZE,
};

fn session() -> PinyinSession {
    PinyinSession::new().expect("embedded MVP lexicon must load")
}

fn type_text(session: &mut PinyinSession, text: &str) -> CompositionUpdate {
    let mut update = CompositionUpdate::default();
    for character in text.chars() {
        update = session
            .process_key(KeyEvent::plain(Key::Char(character)))
            .expect("typing a letter cannot fail");
    }
    update
}

#[test]
fn typing_nihao_produces_preedit_and_candidates() {
    let mut session = session();
    let update = type_text(&mut session, "nihao");

    assert!(update.handled);
    assert_eq!(update.preedit, "nihao");
    assert_eq!(update.candidates[0].text, "你好");
    assert_eq!(update.highlighted, Some(0));
    assert_eq!(update.page, 0);
    assert_eq!(update.page_count, 1);
    assert_eq!(session.page_size(), DEFAULT_PAGE_SIZE);
}

#[test]
fn builtin_lexicon_covers_the_required_demo_words() {
    let lexicon = Lexicon::builtin().expect("embedded lexicon");
    for (pinyin, expected) in [
        ("ni", "你"),
        ("hao", "好"),
        ("nihao", "你好"),
        ("zhong", "中"),
        ("guo", "国"),
        ("zhongguo", "中国"),
        ("shi", "是"),
        ("jie", "解"),
    ] {
        let candidates = lexicon.lookup(pinyin);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.text == expected),
            "missing {expected} for {pinyin}"
        );
    }
}

#[test]
fn number_key_selects_and_commits_the_first_candidate() {
    let mut session = session();
    type_text(&mut session, "nihao");

    let update = session
        .process_key(KeyEvent::plain(Key::Char('1')))
        .expect("candidate selection");

    assert!(update.handled);
    assert_eq!(update.commit.as_deref(), Some("你好"));
    assert_eq!(update.preedit, "");
    assert!(update.candidates.is_empty());
    assert!(!session.is_composing());
}

#[test]
fn space_and_enter_commit_the_highlighted_candidate() {
    let mut session = session();
    type_text(&mut session, "zhongguo");
    let update = session
        .process_key(KeyEvent::plain(Key::Space))
        .expect("space commit");
    assert_eq!(update.commit.as_deref(), Some("中国"));

    type_text(&mut session, "shi");
    let update = session
        .process_key(KeyEvent::plain(Key::Enter))
        .expect("enter commit");
    assert_eq!(update.commit.as_deref(), Some("是"));
}

#[test]
fn space_commits_raw_pinyin_when_no_candidate_exists() {
    let mut session = session();
    type_text(&mut session, "xyz");
    let update = session
        .process_key(KeyEvent::plain(Key::Space))
        .expect("raw commit");
    assert_eq!(update.commit.as_deref(), Some("xyz"));
}

#[test]
fn escape_cancels_without_committing() {
    let mut session = session();
    type_text(&mut session, "nihao");
    let update = session
        .process_key(KeyEvent::plain(Key::Escape))
        .expect("cancel");

    assert!(update.handled);
    assert!(update.cancelled);
    assert_eq!(update.commit, None);
    assert_eq!(update.preedit, "");
    assert!(update.candidates.is_empty());
    assert!(!session.is_composing());
}

#[test]
fn backspace_edits_then_clears_the_composition() {
    let mut session = session();
    type_text(&mut session, "ni");

    let update = session
        .process_key(KeyEvent::plain(Key::Backspace))
        .expect("backspace");
    assert_eq!(update.preedit, "n");
    assert!(update.handled);

    let update = session
        .process_key(KeyEvent::plain(Key::Backspace))
        .expect("clear composition");
    assert_eq!(update.preedit, "");
    assert!(update.candidates.is_empty());
}

#[test]
fn page_keys_move_between_candidate_pages() {
    let mut session = session();
    let first = type_text(&mut session, "shi");
    assert!(first.page_count > 1);
    assert_eq!(first.page, 0);
    assert_eq!(first.candidates.len(), DEFAULT_PAGE_SIZE);

    let second = session
        .process_key(KeyEvent::plain(Key::PageDown))
        .expect("next page");
    assert_eq!(second.page, 1);
    assert_eq!(second.candidates.len(), 2);
    assert_ne!(first.candidates[0].text, second.candidates[0].text);

    let first_again = session
        .process_key(KeyEvent::plain(Key::PageUp))
        .expect("previous page");
    assert_eq!(first_again.page, 0);
    assert_eq!(first_again.candidates[0].text, first.candidates[0].text);
}

#[test]
fn arrow_keys_move_the_highlight_and_space_commits_it() {
    let mut session = session();
    let typed = type_text(&mut session, "shi");
    let expected = typed.candidates[1].text.clone();

    let moved = session
        .process_key(KeyEvent::plain(Key::Right))
        .expect("move highlight");
    assert_eq!(moved.highlighted, Some(1));

    let committed = session
        .process_key(KeyEvent::plain(Key::Space))
        .expect("commit highlighted candidate");
    assert_eq!(committed.commit.as_deref(), Some(expected.as_str()));
}

#[test]
fn control_and_alt_shortcuts_are_passed_through() {
    let mut session = session();
    type_text(&mut session, "ni");

    for modifiers in [
        Modifiers::CONTROL,
        Modifiers::ALT,
        Modifiers::META,
        Modifiers::CONTROL.with_shift(),
    ] {
        let update = session
            .process_key(KeyEvent::new(Key::Char('c'), modifiers))
            .expect("pass-through");
        assert!(!update.handled);
        assert_eq!(update.preedit, "ni");
        assert_eq!(session.preedit(), "ni");
    }
}

#[test]
fn keys_without_an_active_composition_pass_through() {
    let mut session = session();
    for key in [
        Key::Backspace,
        Key::Enter,
        Key::Space,
        Key::Escape,
        Key::PageUp,
        Key::PageDown,
        Key::Left,
        Key::Right,
        Key::Char('1'),
    ] {
        let update = session
            .process_key(KeyEvent::plain(key))
            .expect("pass-through");
        assert!(!update.handled, "{key:?} should pass through");
    }
}

#[test]
fn windows_virtual_keys_map_to_the_core_api() {
    let letter = map_windows_virtual_key(b'A' as u32, Modifiers::SHIFT).expect("letter key");
    assert_eq!(letter.key, Key::Char('a'));
    assert_eq!(letter.modifiers, Modifiers::SHIFT);

    assert_eq!(
        map_windows_virtual_key(VK_BACK, Modifiers::NONE)
            .expect("backspace")
            .key,
        Key::Backspace
    );
    assert_eq!(
        map_windows_virtual_key(VK_PRIOR, Modifiers::NONE)
            .expect("page up")
            .key,
        Key::PageUp
    );
    assert_eq!(
        map_windows_virtual_key(VK_NEXT, Modifiers::NONE)
            .expect("page down")
            .key,
        Key::PageDown
    );
    assert_eq!(
        map_windows_virtual_key(VK_RIGHT, Modifiers::NONE)
            .expect("right")
            .key,
        Key::Right
    );
    assert!(map_windows_virtual_key(0x70, Modifiers::NONE).is_none());
}

#[test]
fn selection_state_errors_are_results_not_panics() {
    let mut session = session();
    assert_eq!(
        session.select_candidate(0),
        Err(EngineError::NoActiveComposition)
    );
    assert_eq!(session.commit(), Err(EngineError::NoActiveComposition));
    assert_eq!(session.cancel(), Err(EngineError::NoActiveComposition));

    type_text(&mut session, "nihao");
    assert!(matches!(
        session.select_candidate(99),
        Err(EngineError::CandidateOutOfRange { .. })
    ));
    assert_eq!(session.preedit(), "nihao");
}

#[test]
fn empty_and_malformed_lexicons_are_rejected() {
    assert!(matches!(
        Lexicon::from_tsv("# comments only\n\n"),
        Err(EngineError::EmptyLexicon)
    ));
    assert!(matches!(
        Lexicon::from_tsv("ni\t你\n"),
        Err(EngineError::InvalidLexiconLine { line: 1, .. })
    ));
    assert!(matches!(
        Lexicon::from_tsv("Ni\t你\t1\n"),
        Err(EngineError::InvalidLexiconLine { line: 1, .. })
    ));
    assert!(matches!(
        Lexicon::from_tsv_bytes(&[0xFF, 0xFE]),
        Err(EngineError::InvalidUtf8 { .. })
    ));
}

#[test]
fn invalid_page_size_is_rejected() {
    assert!(matches!(
        PinyinSession::with_page_size(0),
        Err(EngineError::InvalidPageSize { page_size: 0 })
    ));
}

#[test]
fn engine_session_alias_uses_the_same_api() {
    let mut session = EngineSession::new().expect("engine session");
    type_text(&mut session, "ni");
    assert_eq!(session.preedit(), "ni");
}
