#![cfg(windows)]
use flower::engine::LearningEvent;
use flower::user_dictionary::DictionaryStore;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn event() -> LearningEvent {
    LearningEvent {
        pinyin: "ni'hao".into(),
        text: "你好".into(),
    }
}

fn promotion() -> flower::engine::PromotionEvent {
    use flower::engine::{PromotionEvent, RankingCandidate, RankingOrder};
    let candidates: Vec<_> = (0..9)
        .map(|index| RankingCandidate {
            pinyin: "ni".into(),
            text: index.to_string(),
            consumed: 2,
        })
        .collect();
    PromotionEvent {
        selected: candidates[8].clone(),
        order: RankingOrder {
            input: "ni".into(),
            candidates,
        },
    }
}

#[test]
fn process_promotion_writer() {
    let Some(path) = std::env::var_os("FLOWER_PROMOTION_TEST_DICTIONARY") else {
        return;
    };
    DictionaryStore::new(PathBuf::from(path))
        .promote_if_generation(0, &[promotion()])
        .unwrap();
}

#[test]
fn independent_processes_merge_adjacent_steps() {
    let fixture = CrashFixture::new();
    let path = fixture.0.join("dictionary.json");
    let children: Vec<_> = (0..4)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_promotion_writer", "--quiet"])
                .env("FLOWER_PROMOTION_TEST_DICTIONARY", &path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let snapshot = DictionaryStore::new(path).load().unwrap();
    assert_eq!(snapshot.rankings[0].candidates[4].text, "8");
    assert!(snapshot.entries.is_empty());
    assert!(snapshot.learned.is_empty());
}

#[test]
fn process_writer() {
    let Some(path) = std::env::var_os("FLOWER_PROCESS_TEST_DICTIONARY") else {
        return;
    };
    let store = DictionaryStore::new(PathBuf::from(path));
    let generation = store.load().unwrap().generation;
    for _ in 0..20 {
        store.learn_if_generation(generation, &[event()]).unwrap();
    }
}

#[test]
fn independent_processes_merge_counts_without_lost_updates() {
    let root = std::env::var_os("FLOWER_TEST_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = root.join(format!("dictionary-process-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("dictionary.json");
    let store = DictionaryStore::new(&path);
    store.set_learning_enabled(true).unwrap();
    let children: Vec<_> = (0..4)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_writer", "--quiet"])
                .env("FLOWER_PROCESS_TEST_DICTIONARY", &path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let outputs: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    for output in outputs {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(store.load().unwrap().learned[0].count, 80);
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "only invoked as an isolated child by the publication crash test"]
fn process_publish_writer() {
    let path = std::env::var_os("FLOWER_CRASH_TEST_DICTIONARY").unwrap();
    DictionaryStore::new(PathBuf::from(path))
        .set_learning_enabled(true)
        .unwrap();
}

struct CrashFixture(PathBuf);

impl CrashFixture {
    fn new() -> Self {
        let root = std::env::var_os("FLOWER_TEST_SCRATCH")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = root.join(format!("dictionary-crash-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn temporaries(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                name.starts_with("dictionary.json.") && name.ends_with(".tmp")
            })
            .collect()
    }
}

impl Drop for CrashFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ReapChild(std::process::Child);

impl Drop for ReapChild {
    fn drop(&mut self) {
        // Never leave a test writer running, including when an assertion fails.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_writer_before_atomic_publish_preserves_old_bytes_and_allows_recovery() {
    use flower::user_dictionary::UserDictionary;
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    let fixture = CrashFixture::new();
    let store = DictionaryStore::new(fixture.0.join("dictionary.json"));
    store.clear_learning().unwrap();
    let old = std::fs::read(store.path()).unwrap();
    let original = store.load().unwrap();
    assert!(!original.learning_enabled);

    // Deny delete sharing to hold the real production writer before publication.
    // This handle belongs only to the isolated fixture, never a user's dictionary.
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(store.path())
        .unwrap();
    let mut child = ReapChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_publish_writer", "--ignored", "--quiet"])
            .env("FLOWER_CRASH_TEST_DICTIONARY", store.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    let (temporary_path, temporary_handle) = loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "writer exited before the crash checkpoint"
        );
        let temporaries = fixture.temporaries();
        assert!(temporaries.len() <= 1, "temporary payload was rebuilt");
        if let Some(path) = temporaries.first() {
            // Denying WRITE sharing proves the production write handle has
            // closed, after write_all and sync_all. Retain the handle until the
            // child has exited; do not infer the checkpoint from a delay alone.
            if let Ok(handle) = OpenOptions::new().read(true).share_mode(1).open(path) {
                break (path.clone(), handle);
            }
        }
        assert!(
            Instant::now() < deadline,
            "closed temporary was not observed"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    let staged_bytes = std::fs::read(&temporary_path).unwrap();
    let staged: UserDictionary = serde_json::from_slice(&staged_bytes).unwrap();
    assert!(staged.learning_enabled);
    assert_eq!(staged.generation, original.generation + 1);
    assert_eq!(std::fs::read(store.path()).unwrap(), old);

    // Child::kill forcibly terminates this test-owned Windows process. It does
    // not run the writer's normal error cleanup or any Rust destructors.
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert!(!status.success());
    drop(temporary_handle);
    drop(blocker);
    assert_eq!(std::fs::read(store.path()).unwrap(), old);
    assert_eq!(store.load().unwrap().generation, original.generation);
    assert!(!store.load().unwrap().learning_enabled);

    // A complete but unpublished orphan must not be treated as committed data.
    assert_eq!(fixture.temporaries(), vec![temporary_path.clone()]);
    store.set_learning_enabled(true).unwrap();
    let recovered = store.load().unwrap();
    assert!(recovered.learning_enabled);
    assert_eq!(recovered.generation, original.generation + 1);
    assert!(recovered.learned.is_empty());
    assert_eq!(std::fs::read(&temporary_path).unwrap(), staged_bytes);
    assert_eq!(fixture.temporaries(), vec![temporary_path]);
}
