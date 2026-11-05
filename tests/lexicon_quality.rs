use flower::engine::{LearningEvent, Lexicon};

#[test]
fn actual_embedded_lexicon_keeps_all_fixed_phrase_cases_at_top_one() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../data/pinyin/phrases/quality-cases.json")).unwrap();
    let lexicon = Lexicon::builtin().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let reading = case["pinyin"].as_str().unwrap();
        let word = case["text"].as_str().unwrap();
        let candidates = lexicon.lookup(reading);
        assert_eq!(
            candidates.first().map(|c| c.text.as_str()),
            Some(word),
            "{reading}"
        );
    }
}

#[test]
fn local_overlay_and_bounded_learning_do_not_mutate_the_shared_base() {
    let base = Lexicon::from_tsv("shi\t是\t1000\nshi\t事\t900").unwrap();
    let overlay = base.with_overlay("shi\t式\t2000").unwrap();
    assert_eq!(overlay.lookup("shi")[0].text, "式");
    let learned = base.with_learning(&[(
        LearningEvent {
            pinyin: "shi".into(),
            text: "事".into(),
        },
        2,
    )]);
    assert_eq!(learned.lookup("shi")[0].text, "事");
    assert_eq!(base.lookup("shi")[0].text, "是");
    let not_invented = base.with_learning(&[(
        LearningEvent {
            pinyin: "shi".into(),
            text: "虚构".into(),
        },
        u32::MAX,
    )]);
    assert_eq!(not_invented.lookup("shi"), base.lookup("shi"));
}
