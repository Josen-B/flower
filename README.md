# Flower

Flower is an offline Windows 10/11 x64 Pinyin input method. The implementation
is a single Rust Cargo package named `flower`.

## Current status

- `flower::engine` is a safe-Rust, platform-independent Pinyin engine.
- `flower::platform` contains the Windows TSF/COM boundary. Its `windows`
  dependency is enabled only for Windows targets.
- `flower::settings` provides the serde JSON settings model and file store.
- `EngineRouter` is the platform-neutral boundary used by the TSF layer.
- `DllGetClassObject` exposes the Flower class factory and accepts only the
  Flower text-service CLSID. The active text service keeps one engine session
  per TSF context and applies preedit/commit/cancel updates through synchronous
  edit sessions.
- Candidate rendering, profile registration, the settings UI, and a real
  Windows host smoke test remain future work.

## Repository layout

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
README.md
src/
  lib.rs
  engine/
    mod.rs
    error.rs
    key.rs
    keymap.rs
    lexicon.rs
    session.rs
    types.rs
  platform/
    mod.rs
    composition.rs
    windows.rs
  settings.rs
tests/
  engine_router.rs
  pinyin_session.rs
data/pinyin/mvp_lexicon.tsv
docs/plans/
```

The staged implementation and acceptance criteria are tracked in
[`docs/plans/2026-10-20-flower-rust-implementation-roadmap.md`](docs/plans/2026-10-20-flower-rust-implementation-roadmap.md).

## Public API

The primary API lives in three modules:

- `flower::engine`: keys, key events, key mapping, lexicon loading, composition,
  candidates, and session errors.
- `flower::settings`: `Settings`, `SettingsStore`, `Theme`, and settings errors.
- `flower::platform`: platform support, TSF metadata, and the Windows COM
  exports.

Common engine and settings items are also re-exported from the crate root:

```rust
use flower::{keymap, EngineRouter, Key, KeyEvent, Modifiers};

let event = keymap::map_windows_virtual_key(b'A' as u32, Modifiers::SHIFT)
    .expect("supported key");
assert_eq!(event.key, Key::Char('a'));

let mut router = EngineRouter::new().expect("embedded MVP lexicon");
assert!(!router.is_composing());

let letter = KeyEvent::plain(Key::Char('n'));
assert!(router.test_key(letter).expect("key preview"));
assert!(!router.is_composing());

let update = router.process_key(letter).expect("key processing");
assert_eq!(update.preedit, "n");
```

On Windows, `FLOWER_TEXT_SERVICE_CLSID`, `DllGetClassObject`, and
`DllCanUnloadNow` are available through both `flower::platform` and the crate
root.

## Build and test

Run these commands from the repository root:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
```

The Windows TSF boundary can be checked from the root with the MSVC target
installed:

```text
cargo check --workspace --target x86_64-pc-windows-msvc
```

On non-Windows targets, `flower::platform::is_supported_platform()` is still
available and returns `false`; the Windows COM implementation is not compiled.

## Input behavior

- A-Z starts or extends a no-tone full-Pinyin composition and is normalized to
  lowercase.
- Digits 1-9 select a candidate on the current page.
- Space or Enter commits the highlighted candidate, or the raw Pinyin input
  when no candidate exists.
- Backspace edits the composition, Escape cancels it, and PageUp/PageDown change
  candidate pages.
- Left/Right and Up/Down move the highlighted candidate.
- Ctrl, Alt, and Windows/meta combinations pass through to the host.
- The default page size is 9, and candidates are ranked by lexicon weight.

## Lexicon

The reproducible MVP lexicon is
[`data/pinyin/mvp_lexicon.tsv`](data/pinyin/mvp_lexicon.tsv). It is embedded at
compile time and uses three tab-separated UTF-8 fields:

```text
lowercase-pinyin<TAB>candidate-text<TAB>weight
```

Blank lines and lines beginning with `#` are ignored. Weights are positive
`u32` values, with higher weights sorting first. Malformed rows, non-lowercase
Pinyin keys, invalid UTF-8, zero weights, and an empty lexicon are rejected
with `EngineError`.

## Settings

`flower::settings` stores an app-chosen JSON path with
`candidate_page_size` (1-99) and `theme` (`system`, `light`, or `dark`). It does
not choose a Windows profile path or provide a UI.

Flower performs no network requests, collects no telemetry, and includes no
external input data or models.
