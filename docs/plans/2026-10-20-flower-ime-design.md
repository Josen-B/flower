# Flower Windows IME Design

## Goal and scope

Build Flower as an independent Windows Chinese Pinyin input method that can be
selected through Windows Text Services Framework (TSF). The first release
supports basic composition, ranked candidate selection, paging, commit, and
cancel. It is strictly offline, has no telemetry, and processes typed text
locally. It targets both x64 and x86 host applications and uses per-user
installation without administrator privileges.

Flower will not copy Chatterfly source, executable assets, lexicons, or model
data. Reverse-engineering evidence is used only to establish high-level
integration context. Static analysis of `ChatterflyTSF.dll` observed COM
registration and class-factory exports (Evidence `ev_006b534e19101aacf5e04ac0668c413b073778e56de1f81510a8ee8442fd308e` and
`ev_25d4626dc9afb20bb09325fd54f3dc367a6969d698adb52ccbc040ffdaa11b25`).
`ImeFunc.dll` was identified as a large native x86 DLL (SHA-256
`39a3bf2eaa62ba0ea099fbab53a5822518b25030d161e10568a182a6ea684039`); its
overview reports 36,077 procedures and 13,487 defined strings (Evidence
`ev_befd91d4bc2746d9bf8ce7f2f89b0cae183d3022d5bd4986ff58ef6ff11090dc`).
Detailed key/composition behavior and runtime behavior remain unverified;
Flower will implement the user-approved MVP rather than claim behavioral
parity.

## Architecture

- **TSF service:** Flower-owned native C++ COM text service, class factory,
  per-user registration, TSF activation/deactivation, context and edit-session
  lifecycle, and input-scope handling.
- **Engine adapter:** A narrow C++ boundary around librime's public C API.
  One Rime session is associated with each active TSF composition context.
  The adapter translates key events and returns preedit text, candidates,
  selection, paging state, and commit text without exposing Rime types to TSF
  lifecycle code.
- **Candidate UI:** A Flower-owned, non-activating Win32 popup anchored near
  the caret, showing preedit, numbered candidates, and page position. It must
  not steal focus or write outside the current TSF edit session.
- **Data:** Pin librime and its source dependencies to explicit revisions.
  Package only the runtime libraries and a reviewed upstream Pinyin schema and
  dictionary subset. Keep user settings and learned data in the current user's
  profile; never include Chatterfly data.
- **Architectures:** Build and register independent x64 and x86 TSF binaries
  from the same source. The installer selects both for their matching host
  processes.

## Input and lifecycle

When Flower is active, TSF key events go to the context's Rime session.
Composition/preedit and candidate changes are applied through TSF edit
sessions. Number keys 1–9 select candidates; Space or Enter accepts the
highlighted/default candidate; Backspace edits; Escape cancels; PageUp and
PageDown change candidate pages. When not composing, keys pass through.
Context loss and service deactivation release the associated Rime session and
popup. A failed engine initialization returns an explicit error and leaves
the focused document unchanged; it must not look like a successful empty
candidate response.

## Installation, privacy, and errors

Provide an idempotent per-user installer and uninstaller. They register only
Flower's per-user COM/TSF entries and files, do not elevate, and remove only
entries owned by Flower. Initialization and registration failures produce a
useful diagnostic without recording typed text, candidate history, or
document content. There is no network access, cloud suggestion feature, or
telemetry in the first release.

## Verification

- Unit-test the Rime adapter's composition, candidate ordering, selection,
  paging, cancellation, and error translation.
- Test x64 and x86 compilation, COM activation, TSF registration and
  unregistration, repeatable install/uninstall, and file/registry scope.
- On Windows, smoke-test in text controls for key pass-through, candidate
  display, commit/cancel, focus changes, and x86/x64 host compatibility.
- Verify package contents and third-party notices against the exact pinned
  librime, dependency, schema, and dictionary revisions.

## Implementation sequence

1. Establish the native CMake/MSVC project and pinned librime source/data
   dependencies; verify both architectures build.
2. Implement and test the isolated librime adapter.
3. Implement the TSF COM service, composition lifecycle, and candidate popup.
4. Add per-user registration/install and uninstall flows.
5. Run targeted automated and Windows integration checks; document any
   unavailable real-host verification.
