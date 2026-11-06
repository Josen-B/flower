//! Fixed, manually authored public diagnostic sentences; no user documents.
#[path = "../src/ai/inference.rs"]
#[allow(dead_code)]
mod inference;

// Frozen before inference. Each source/gold pair has equal Han-character length.
// Categories deliberately include homophones, semantics, two errors and an
// ordinary built-in first-candidate ambiguity. Gold is editorial, not model output.
const CASES: &[(&str, &str, &str)] = &[
    ("homophone", "这个问题很容意解决", "这个问题很容易解决"),
    ("homophone", "我希忘大家身体健康", "我希望大家身体健康"),
    ("homophone", "我非常感榭你的帮助", "我非常感谢你的帮助"),
    ("homophone", "今天新情很好", "今天心情很好"),
    ("homophone", "我们明天在见", "我们明天再见"),
    ("homophone", "祝你生日快了", "祝你生日快乐"),
    ("builtin", "请问现在那里上班", "请问现在哪里上班"),
    ("homophone", "这个方法非常有拥", "这个方法非常有用"),
    ("homophone", "我已经知到答案", "我已经知道答案"),
    ("homophone", "请你把文件发给窝", "请你把文件发给我"),
    ("semantic", "我们应该保护环镜", "我们应该保护环境"),
    ("semantic", "他正在认真学刁", "他正在认真学习"),
    ("semantic", "这个消息令人振愤", "这个消息令人振奋"),
    ("semantic", "他提出了宝贵意建", "他提出了宝贵意见"),
    ("semantic", "我们必须遵守记律", "我们必须遵守纪律"),
    ("semantic", "请大家保持安净", "请大家保持安静"),
    ("semantic", "他取得了优异成积", "他取得了优异成绩"),
    ("semantic", "我们需要提高效绿", "我们需要提高效率"),
    ("two", "少先队员因该为老人让坐", "少先队员应该为老人让座"),
    ("two", "我希忘大家身体建康", "我希望大家身体健康"),
    ("two", "这个问提很容意解决", "这个问题很容易解决"),
    ("two", "我非常感榭你的帮住", "我非常感谢你的帮助"),
    ("two", "我们明天在去医完", "我们明天再去医院"),
    ("semantic", "大家要珍惜宝贵时问", "大家要珍惜宝贵时间"),
];

#[test]
#[ignore = "diagnostic only; requires FLOWER_AI_TEST_BUNDLE; never downloads assets"]
fn fixed_quality_baseline() {
    let bundle = std::env::var_os("FLOWER_AI_TEST_BUNDLE").expect("explicit fixed bundle");
    let start = std::time::Instant::now();
    let mut model = inference::Corrector::load(std::path::Path::new(&bundle)).unwrap();
    println!("load_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    let (mut hit, mut partial, mut missed, mut wrong, mut false_positive) = (0, 0, 0, 0, 0);
    for &(category, source, gold) in CASES {
        assert_ne!(source, gold);
        assert_eq!(source.chars().count(), gold.chars().count());
        for (kind, input) in [("error", source), ("control", gold)] {
            let start = std::time::Instant::now();
            let result = model.correct(input).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if kind == "error" {
                match result.as_deref() {
                    Some(s) if s == gold => hit += 1,
                    None => missed += 1,
                    Some(s) if only_gold_changes(input, s, gold) => partial += 1,
                    Some(_) => wrong += 1,
                }
            } else if result.is_some() {
                false_positive += 1;
            }
            println!("{kind}\t{category}\t{input}\t{gold}\t{result:?}\t{ms:.3}ms");
        }
    }
    println!(
        "summary errors={} controls={} gold_hit={hit} partial={partial} missed={missed} wrong={wrong} false_positive={false_positive}",
        CASES.len(), CASES.len()
    );
}

// Partial means at least one edit, all edits agree with the human gold, and
// at least one original error remains. Counts are diagnostic, never locked.
fn only_gold_changes(source: &str, output: &str, gold: &str) -> bool {
    output != source
        && output != gold
        && output.chars().count() == source.chars().count()
        && source
            .chars()
            .zip(output.chars())
            .zip(gold.chars())
            .all(|((s, o), g)| o == s || o == g)
}

#[test]
#[ignore = "strict real model regression; requires FLOWER_AI_TEST_BUNDLE; never downloads assets"]
fn high_confidence_edit_survives_without_lowering_thresholds() {
    let bundle = std::env::var_os("FLOWER_AI_TEST_BUNDLE").expect("explicit fixed bundle");
    let mut model = inference::Corrector::load(std::path::Path::new(&bundle)).unwrap();
    let source = "他取得了优异成积";
    let gold = "他取得了优异成绩";
    let corrected = model.correct(source).unwrap();
    assert_eq!(corrected.as_deref(), Some(gold));
    println!("strict\t{source}\t{corrected:?}");
    for control in [gold, "他正在认真学习", "他提出了宝贵意见"] {
        let result = model.correct(control).unwrap();
        assert_eq!(result, None, "correct control {control}");
        println!("strict_control\t{control}\t{result:?}");
    }
}

#[test]
#[ignore = "real builtin chain; requires FLOWER_AI_TEST_BUNDLE; never downloads assets"]
fn builtin_quality_examples() {
    use flower::engine::{EngineRouter, Key, KeyEvent};
    let bundle = std::env::var_os("FLOWER_AI_TEST_BUNDLE").expect("explicit fixed bundle");
    let mut model = inference::Corrector::load(std::path::Path::new(&bundle)).unwrap();
    for (raw, source, gold) in [
        (
            "qingwenxianzainalishangban",
            "请问现在那里上班",
            "请问现在哪里上班",
        ),
        (
            "qingwennixianzainalishangban",
            "请问你现在那里上班",
            "请问你现在哪里上班",
        ),
        (
            "qingwenninxianzainalishangban",
            "请问您现在那里上班",
            "请问您现在哪里上班",
        ),
        ("zhunishengrikuaile", "住你生日快乐", "祝你生日快乐"),
        ("zhuninshengrikuaile", "住您生日快乐", "祝您生日快乐"),
        ("zhunishentijiankang", "住你身体健康", "祝你身体健康"),
        ("zhuninshentijiankang", "住您身体健康", "祝您身体健康"),
        ("zhunitiantiankaixin", "住你天天开心", "祝你天天开心"),
        ("zhunixinniankuaile", "住你新年快乐", "祝你新年快乐"),
        ("zhuningongzuoshunli", "住您工作顺利", "祝您工作顺利"),
    ] {
        let mut router = EngineRouter::new().unwrap();
        for c in raw.chars() {
            router.process_key(KeyEvent::plain(Key::Char(c))).unwrap();
        }
        let input = router.ai_input().expect("eligible builtin first candidate");
        assert_eq!(input.source, source, "{raw}");
        let corrected = model.correct(source).unwrap();
        assert_eq!(corrected.as_deref(), Some(gold), "{raw}");
        let update = router
            .apply_ai_candidate(&input, gold)
            .expect("AI candidate");
        assert!(update
            .candidates
            .iter()
            .any(|c| c.is_ai() && c.text == gold));
        assert_eq!(model.correct(gold).unwrap(), None, "correct control {gold}");
        println!("builtin\t{raw}\t{source}\t{gold}\tinserted=true\tcontrol=None");
    }
}
