use flower::engine::{Key, KeyEvent, Lexicon, PinyinSession};
use flower::user_dictionary::DictionaryStore;
use std::path::PathBuf;
use std::sync::Arc;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn selected_mapping_moves_one_position_and_survives_a_new_store_and_session() {
    let root = std::env::var_os("FLOWER_TEST_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let fixture = Fixture(root.join(format!("adaptive-input-{}-{nonce}", std::process::id())));
    std::fs::create_dir(&fixture.0).unwrap();
    let store = DictionaryStore::new(fixture.0.join("dictionary.json"));
    let initial = store.load().unwrap();
    assert!(initial.adaptive_ranking_enabled);
    // Explicit second-position fixture, independent of upstream frequency changes.
    let mut session =
        PinyinSession::from_lexicon_tsv("ying'she\t影射\t200\nying'she\t映射\t100", 9).unwrap();
    session.set_candidate_width(Some(544));
    for letter in "yingshe".chars() {
        session
            .process_key(KeyEvent::plain(Key::Char(letter)))
            .unwrap();
    }
    assert_eq!(session.candidates_on_current_page()[1].text, "映射");
    let committed = session.select_candidate(1).unwrap();
    assert_eq!(committed.commit.as_deref(), Some("映射"));
    let events = session.pending_promotions().to_vec();
    assert_eq!(events.len(), 1);
    store
        .promote_if_generation(initial.generation, &events)
        .unwrap();
    drop(store);

    let reopened = DictionaryStore::new(fixture.0.join("dictionary.json"));
    let persisted = reopened.load().unwrap();
    assert_eq!(persisted.schema_version, 2);
    // The saved stable identity also applies when the real embedded dictionary
    // includes additional candidates not present in the original fixture.
    assert!(Lexicon::builtin()
        .unwrap()
        .lookup("yingshe")
        .iter()
        .any(|c| c.text == "映射"));
    let mut restarted = PinyinSession::from_lexicon(Lexicon::builtin().unwrap(), 9).unwrap();
    restarted.set_ranking_orders(Arc::new(persisted.rankings));
    restarted.set_candidate_width(Some(544));
    for letter in "yingshe".chars() {
        restarted
            .process_key(KeyEvent::plain(Key::Char(letter)))
            .unwrap();
    }
    assert_eq!(restarted.candidates_on_current_page()[0].text, "映射");
    assert_eq!(
        restarted.select_candidate(0).unwrap().commit.as_deref(),
        Some("映射")
    );
}
