# Flower Windows IME Design

## Goal and scope

Build Flower as an independent Windows Chinese Pinyin input method that can be
selected through Windows Text Services Framework (TSF). The first release
supports basic composition, ranked candidate selection, paging, commit, and
cancel. It is strictly offline, has no telemetry, and processes typed text
locally. The current MVP targets Windows 10/11 x64. x86 host support and
per-user installation remain later milestones.

## Architecture

- **Rust package:** Cargo is the build entry. The root `flower` package is
  organized by responsibility: `src/engine/` contains the platform-independent
  input state machine and lexicon, `src/platform/windows.rs` contains the
  Windows-only COM boundary, and `src/settings.rs` contains the
  platform-neutral JSON settings model.
- **Engine:** A safe-Rust, exact-match no-tone full-Pinyin engine. One
  `PinyinSession` is associated with each TSF input context. It translates key
  events into preedit text, ranked candidates, selection, paging state, commit
  text, and explicit `Result` errors.
- **TSF service:** A Rust COM DLL boundary restricted with
  `cfg(target_os = "windows")`. The `windows` crate generates the class factory
  and text-service interface implementations. The exported server supports the
  Flower CLSID, interface-specific `CreateInstance` queries, object and lock
  counts for `DllCanUnloadNow`, text-service activation, and a pass-through
  key event sink. Repeated activation is rejected, activation failure leaves
  no advised sink, and deactivation is idempotent.
- **Candidate UI:** A Flower-owned, non-activating Win32 popup anchored near the
  caret, showing preedit, numbered candidates, and page position. It must not
  steal focus or write outside the current TSF edit session.
- **Data:** Ship a small, reproducible UTF-8 TSV lexicon at
  `data/pinyin/mvp_lexicon.tsv`. The parser rejects empty or malformed data.
  Keep user settings and learned data in the current user's profile.
- **Settings:** A serde JSON model with candidate page size and theme. A native
  Win32 settings window and Windows profile-path policy are follow-up work.
- **Architectures:** Build and register independent x64 and x86 TSF binaries
  from the same Rust source when x86 support is added. The MVP acceptance
  target is Windows 10/11 x64.

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
    windows.rs
  settings.rs
tests/pinyin_session.rs
data/pinyin/mvp_lexicon.tsv
docs/plans/
```

Only the root package manifest is part of the build. The `windows` dependency
is conditional on `cfg(target_os = "windows")` in `Cargo.toml`.
`data/pinyin/mvp_lexicon.tsv` is embedded by the engine. `docs/plans/` contains
design and implementation records.

## Input and lifecycle

When Flower is active, TSF key events go to the context's Pinyin session.
Composition/preedit and candidate changes are applied through TSF edit
sessions. Number keys 1-9 select candidates; Space or Enter accepts the
highlighted/default candidate; Backspace edits; Escape cancels; PageUp and
PageDown change candidate pages. When not composing, keys pass through.
Context loss and service deactivation release the associated Pinyin session and
popup. A failed engine initialization returns an explicit error and leaves the
focused document unchanged; it must not look like a successful empty candidate
response.

## Installation, privacy, and errors

Provide an idempotent per-user installer and uninstaller. They register only
Flower's per-user COM/TSF entries and files, do not elevate, and remove only
entries owned by Flower. Initialization and registration failures produce a
useful diagnostic without recording typed text, candidate history, or document
content. There is no network access, cloud suggestion feature, or telemetry in
the first release.

## Verification

- Run formatting, tests, and lint checks from the repository root:

  ```text
  cargo fmt --all -- --check
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  cargo check --workspace
  ```

- Check the Windows-targeted package with
  `cargo check --workspace --target x86_64-pc-windows-msvc`.
- Unit-test the Rust engine's composition, candidate ordering, selection,
  paging, cancellation, and error translation.
- Test x64 compilation, COM activation, TSF registration and unregistration,
  repeatable install/uninstall operations, and file/registry scope.
- On Windows, smoke-test in text controls for key pass-through, candidate
  display, commit/cancel, focus changes, and x64 host compatibility.
- Verify package contents and dependency notices against the exact dependency
  and data revisions.

## Implementation sequence

1. Establish the root Cargo package, safe Rust engine, embedded MVP lexicon,
   and engine tests.
2. Add the Windows TSF DLL boundary, class factory, text-service activation,
   and pass-through key event sink.
3. Implement composition lifecycle and the non-activating candidate popup.
4. Add the settings window and per-user registration/install/uninstall flows.
5. Run targeted automated and Windows integration checks; document any
   unavailable real-host verification.
