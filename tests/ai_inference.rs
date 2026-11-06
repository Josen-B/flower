// This test binary, like flower-settings, is intentionally separate from TSF.
#[path = "../src/ai/inference.rs"]
#[allow(dead_code)]
mod inference;

const V: usize = 21128;
fn fixture(text: &str) -> (Vec<String>, Vec<i64>, Vec<f32>) {
    let mut vocab = vec!["[PAD]".into(); V];
    let mut ids = Vec::new();
    let len = text.chars().count();
    let mut logits = vec![-20.0; (len + 2) * V];
    for (i, c) in text.chars().enumerate() {
        let id = 200 + i;
        vocab[id] = c.to_string();
        ids.push(id as i64);
        logits[(i + 1) * V + id] = 20.0;
    }
    (vocab, ids, logits)
}

#[test]
fn unchanged_and_confident_single_substitution() {
    let text = "今天新情很好";
    let (mut vocab, ids, mut logits) = fixture(text);
    let shape = [1, 8, V as i64];
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
    vocab[500] = "心".into();
    logits[3 * V + ids[2] as usize] = 0.0;
    logits[3 * V + 500] = 20.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        Some("今天心情很好".into())
    );
    logits[3 * V + ids[2] as usize] = 17.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
    // Margin >= 4 alone is insufficient when many alternatives carry mass.
    logits[3 * V..4 * V].fill(16.0);
    logits[3 * V + 500] = 20.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
}

#[test]
fn uncertain_or_non_han_prediction_does_not_veto_another_confident_correction() {
    let text = "今天新情很好";
    let (mut vocab, ids, mut logits) = fixture(text);
    let shape = [1, 8, V as i64];
    vocab[500] = "心".into();
    logits[3 * V + ids[2] as usize] = 0.0;
    logits[3 * V + 500] = 20.0;
    vocab[501] = "明".into();
    logits[V + ids[0] as usize] = 19.0;
    logits[V + 501] = 20.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        Some("今天心情很好".into())
    );
    vocab[501] = "##明".into();
    logits[V + ids[0] as usize] = 0.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        Some("今天心情很好".into())
    );
}

#[test]
fn rejects_bad_shape_nonfinite_tokens_and_excess_changes() {
    let text = "今天新情很好";
    let (mut vocab, ids, mut logits) = fixture(text);
    let shape = [1, 8, V as i64];
    assert!(inference::filter_logits(text, &ids, &vocab, &[1, 6, V as i64], &logits).is_err());
    logits[0] = f32::NAN;
    assert!(inference::filter_logits(text, &ids, &vocab, &shape, &logits).is_err());
    logits[0] = -20.0;
    vocab[500] = "##心".into();
    logits[3 * V + ids[2] as usize] = 0.0;
    logits[3 * V + 500] = 20.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
    vocab[500] = "心".into();
    vocab[501] = "明".into();
    logits[V + ids[0] as usize] = 0.0;
    logits[V + 501] = 20.0;
    // Two of six exceeds 25%, although it does not exceed two changes.
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
    assert!(inference::filter_logits("今天A很好", &ids, &vocab, &shape, &logits).is_err());
}

#[test]
fn two_changes_allowed_but_three_always_rejected() {
    let text = "今天新情很好我们一起学习";
    let (mut vocab, ids, mut logits) = fixture(text);
    let shape = [1, 14, V as i64];
    for (position, replacement, c) in [(0, 500, '明'), (2, 501, '心')] {
        vocab[replacement] = c.to_string();
        logits[(position + 1) * V + ids[position] as usize] = 0.0;
        logits[(position + 1) * V + replacement] = 20.0;
    }
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        Some("明天心情很好我们一起学习".into())
    );
    vocab[502] = "正".into();
    logits[5 * V + ids[4] as usize] = 0.0;
    logits[5 * V + 502] = 20.0;
    assert_eq!(
        inference::filter_logits(text, &ids, &vocab, &shape, &logits).unwrap(),
        None
    );
}

#[test]
fn missing_assets_fail_before_ort_initialization() {
    assert!(inference::Corrector::load(std::path::Path::new("relative-ai")).is_err());
    let missing = std::env::current_dir()
        .unwrap()
        .join("nonexistent-fixed-ai-bundle");
    assert!(inference::Corrector::load(&missing).is_err());
}

#[test]
#[ignore = "requires explicit fixed offline bundle; never downloads assets"]
fn real_fixed_model_probe() {
    let bundle = std::env::var_os("FLOWER_AI_TEST_BUNDLE").expect("explicit test bundle");
    let mut model = inference::Corrector::load(std::path::Path::new(&bundle)).unwrap();
    assert_eq!(model.correct("今天心情很好").unwrap(), None);
    for (source, corrected) in [
        ("这个问题很容意解决", "这个问题很容易解决"),
        ("我希忘大家身体健康", "我希望大家身体健康"),
        ("我非常感榭你的帮助", "我非常感谢你的帮助"),
    ] {
        assert_eq!(model.correct(source).unwrap().as_deref(), Some(corrected));
    }
    assert!(model.correct("今天A很好").is_err());
    assert!(model.correct("今天龘龘很好").is_err());
    // Other inputs may legitimately produce no suggestion at the fixed threshold.
    for text in ["今天新情很好", "我喜欢学习中文", "少先队员因该为老人让坐"]
    {
        let result = model.correct(text).unwrap();
        if let Some(corrected) = result {
            assert!(flower::ai::valid_correction(text, &corrected));
        }
    }
}
