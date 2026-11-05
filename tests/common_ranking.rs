use flower::engine::{Key, KeyEvent, Lexicon, PinyinSession};

#[test]
fn frequent_primary_readings_precede_rare_and_secondary_readings() {
    let lexicon = Lexicon::builtin().unwrap();
    for (pinyin, expected) in [
        ("de", "的"),
        ("yi", "一"),
        ("shi", "是"),
        ("wo", "我"),
        ("ni", "你"),
    ] {
        assert_eq!(lexicon.lookup(pinyin)[0].text, expected, "{pinyin}");
    }
    let yi = lexicon.lookup("yi");
    assert!(yi.iter().any(|candidate| candidate.text == "也"));
    assert_ne!(yi[0].text, "也", "text frequency is not reading frequency");
    let de = lexicon.lookup("de");
    let common = de.iter().position(|c| c.text == "的").unwrap();
    for rare in ["嘚", "锝"] {
        assert!(de.iter().position(|c| c.text == rare).unwrap() > common);
    }
    for (pinyin, expected) in [
        ("shijie", "世界"),
        ("beijing", "北京"),
        ("xuexi", "学习"),
        ("yinyue", "音乐"),
        ("nihao", "你好"),
        ("women", "我们"),
        ("shenme", "什么"),
        ("keyi", "可以"),
    ] {
        assert_eq!(lexicon.lookup(pinyin)[0].text, expected, "{pinyin}");
    }
}

#[test]
fn common_words_are_reachable_with_initials_in_real_session() {
    for (raw, expected, canonical) in [
        ("nh", "你好", "ni'hao"),
        ("wm", "我们", "wo'men"),
        ("sm", "什么", "shen'me"),
        ("ky", "可以", "ke'yi"),
        ("zg", "中国", "zhong'guo"),
        ("xx", "学习", "xue'xi"),
    ] {
        let mut session = PinyinSession::from_lexicon(Lexicon::builtin().unwrap(), 9).unwrap();
        for letter in raw.chars() {
            session
                .process_key(KeyEvent::plain(Key::Char(letter)))
                .unwrap();
        }
        assert!(
            session
                .candidates_on_current_page()
                .iter()
                .any(|candidate| candidate.text == expected && candidate.comment == canonical),
            "{raw}: {expected} must be on the first nine-candidate page"
        );
        let mut selected = None;
        for _ in 0..session.page_count() {
            if let Some(index) = session
                .candidates_on_current_page()
                .iter()
                .position(|candidate| candidate.text == expected)
            {
                assert_eq!(
                    session.candidates_on_current_page()[index].comment,
                    canonical
                );
                selected = Some(session.select_candidate(index).unwrap());
                break;
            }
            session.next_page().unwrap();
        }
        let result = selected.unwrap_or_else(|| panic!("{raw} missing {expected}"));
        assert_eq!(result.commit.as_deref(), Some(expected));
    }
}
