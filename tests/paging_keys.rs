use flower::engine::{keymap::map_windows_virtual_key, Key, KeyEvent, Modifiers, PinyinSession};

#[test]
fn plus_and_minus_page_only_during_composition_without_eating_shortcuts() {
    let mut session =
        PinyinSession::from_lexicon_tsv("shi\t是\t100\nshi\t时\t90\nshi\t事\t80", 1).unwrap();
    for key in [Key::Char('+'), Key::Char('-'), Key::Char('=')] {
        assert!(!session.process_key(KeyEvent::plain(key)).unwrap().handled);
    }
    for letter in "shi".chars() {
        session
            .process_key(KeyEvent::plain(Key::Char(letter)))
            .unwrap();
    }
    assert_eq!(session.page(), 0);
    session
        .process_key(KeyEvent::plain(Key::Char('+')))
        .unwrap();
    assert_eq!(session.page(), 1);
    session
        .process_key(KeyEvent::plain(Key::Char('=')))
        .unwrap();
    assert_eq!(session.page(), 2);
    for modifiers in [Modifiers::CONTROL, Modifiers::ALT, Modifiers::META] {
        let before = session.preedit().to_owned();
        for key in [Key::Char('+'), Key::Char('-')] {
            assert!(
                !session
                    .process_key(KeyEvent::new(key, modifiers))
                    .unwrap()
                    .handled
            );
            assert_eq!(session.page(), 2);
            assert_eq!(session.preedit(), before);
        }
    }
    session
        .process_key(KeyEvent::plain(Key::Char('-')))
        .unwrap();
    assert_eq!(session.page(), 1);
    let commit = session.process_key(KeyEvent::plain(Key::Space)).unwrap();
    assert_eq!(commit.commit.as_deref(), Some("时"));
}

#[test]
fn windows_punctuation_and_numpad_preserve_modifiers() {
    for (vk, modifiers, key) in [
        (0xBB, Modifiers::NONE, Key::PageDown),
        (0xBB, Modifiers::SHIFT, Key::PageDown),
        (0x6B, Modifiers::NONE, Key::PageDown),
        (0xBD, Modifiers::NONE, Key::PageUp),
        (0x6D, Modifiers::NONE, Key::PageUp),
    ] {
        assert_eq!(
            map_windows_virtual_key(vk, modifiers),
            Some(KeyEvent::new(key, modifiers))
        );
    }
    assert_eq!(map_windows_virtual_key(0xBD, Modifiers::SHIFT), None);
    let ctrl_plus = map_windows_virtual_key(0xBB, Modifiers::CONTROL).unwrap();
    assert!(ctrl_plus.is_passthrough());
}
