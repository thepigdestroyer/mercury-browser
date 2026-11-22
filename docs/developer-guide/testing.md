---
title: Testing and Quality
description: Run Mercury unit, adapter, vendored dependency, formatting, and Clippy checks.
---

# Testing and quality

Mercury uses small deterministic tests for browser policy and adapter translation, then reserves manual runs for native integration that needs a real window, GPU, media stack, or website.

## The standard checks

```bash
make qa
```

This runs formatting checks, Clippy with warnings denied, workspace tests, and the vendored `gaol` shared-memory regression test. Use release mode, as everywhere in the project, so the three share one Servo build:

```bash
make qa CARGO_FLAGS=--release
```

Run stages separately while iterating:

```bash
make fmt-check
make lint CARGO_FLAGS=--release
make test CARGO_FLAGS=--release
```

`make test TEST_ARGS="--skip rendering::metal"` passes what follows `--` to the workspace `cargo test` (a filter, `--skip <name>`, `--test-threads=2`); the vendored `gaol` and `servo-net` runs of `make test` do not get it.

`make qa-fast` checks formatting and runs Clippy and the tests for the crates that need no engine; they take minutes rather than a Servo build. Name the crates yourself for a narrower run, for example `cargo test --release -p talaria-core -p mercury-ui`.

Before a change is merged, all of these must pass with no warnings: `cargo fmt --all -- --check`, `cargo clippy --release --workspace --all-targets -- -D warnings`, `cargo test --release --workspace` and `cargo build --release -p mercury-desktop`. Logic is written test first: the failing test, then the code that makes it pass.

## Type-check all targets and features

```bash
make check
```

This checks the complete workspace with all targets and features. It is useful after changing feature flags, target-specific dependencies, or public APIs.

## Test layers

### Talaria Core

Core tests use fake WebViews to cover:

- URL and search classification, and search URLs built from the settings
- Browser state transitions
- Tab creation, selection, cycling, and closure
- Command routing by `TabId`
- High-frequency input coalescing
- Engine-neutral WebView dispatch
- Page requests: dialogs, sign-in, permission prompts, and the permissions refused without one
- Form controls that pages request, and zoom

Prefer adding a core test when the behavior can be expressed without Servo.

### Talaria Servo

Adapter tests cover mappings between Talaria and Servo types, favicon conversion, load metrics, history state, content-filter decisions, runtime options, and content-process argument parsing.

### Desktop composition

Desktop tests exercise event routing, bounded command dispatch, shortcuts, key conversion, IME composition ownership, settings, profiles and the quit sequence without opening the full application where possible.

### The interface

`crates/mercury-ui/tests/page_request_render.rs` and `render_menus.rs` render Mercury's Slint components with Slint's software renderer, so they can be checked without a window. The examples in `crates/mercury-ui/examples/` (`render_settings`, `render_controls`, `render_tabs`) render the settings window, the page controls and the tab strip to PNGs for review. `render_settings_appearance` and `render_settings_tab_groups` render the Appearance and Tab groups sections in every state of their accessibility tests (Mercury and Classic, Orbit and the dial off, the note that the interface is not in yet, a conflict, a dead key, a refused key, keys off, an empty window, 64 groups), in light and dark, at the narrowest window (560 px) and at 900 px: `cargo run --release -p mercury-ui --example render_settings_tab_groups -- <output directory>`. Open the PNGs; nothing may be clipped at 560 px.

The pure models of the two sections are in `crates/mercury-appearance/src/panels/` (`layout_panel.rs`, `key_panel.rs`) with their tests in `crates/mercury-appearance/tests/`; they take seconds, so put new logic there and keep the Slint files to drawing. The accessibility tests of the sections are `crates/mercury-ui/tests/a11y_settings_appearance.rs` and `a11y_settings_tab_groups.rs`, with goldens `a11y-snapshots/settings-appearance-*.txt` and `settings-tab-groups-*.txt`. When a section is added or renamed, every `windows-settings-*.txt` golden changes (they carry the sidebar); bless those by file and read `git diff -U0` to check that only the sidebar lines moved.

### ⌥ group keys

The state machine of the ⌥ group keys is the crate `mercury-option-keys` (no engine, no Slint, time passed in). `tests/machine.rs` has a test per line of its transition table, `tests/spec_table.rs` is an independent reading of the specification, with the five sentences of the design spec as named tests, and `tests/invariants.rs` is a deterministic fuzz of 240,000 events in three capture modes that checks twelve invariants after every call. The card itself is tested in `crates/mercury-ui`: `tests/key_switcher_tiles.rs` (layout, words and models), `key_switcher_render.rs` (software-renderer pictures in light and dark, and clicks), `key_switcher_card.rs` (the real model drawn by the real component) and `a11y_key_switcher.rs` (with `--features test-support`, the reading order frozen in `tests/a11y-snapshots/key-switcher-*.txt`). `apps/mercury-desktop/src/option_keys.rs` holds the winit glue. Nothing calls it yet.

### Accessibility of the interface

Slint's `accessibility` feature is on (the workspace `Cargo.toml`), so VoiceOver, through AccessKit, can read and press Mercury's own windows: the tab strip, toolbar, address bar, menus, sheets and prompts, the history panel, and the Settings, Bookmarks, Import and About windows. A web page is part of that too: the page area is one node, "Page content" (role `main`, no description), and while an assistive technology is attached the engine's accessibility tree for the shown tab is hung from it (see "Accessibility of web pages" below).

**What is checked.** `crates/mercury-ui/tests/a11y_*.rs` show each window or component on Slint's testing platform (no window, no display, no input) and read it the way an assistive technology does, through `ElementHandle`:

- every control and landmark of an area is found by its role and label, so a renamed label or a lost role fails a test;
- the control's default action (`invoke_accessible_default_action`, which is what VoiceOver's "press" runs, and which needs no keyboard focus) does what a click does, and does nothing while the control is disabled;
- the labelling rule below holds in every state the area can be in (`assert_labelled`);
- the reading order of the main states is compared with a golden file in `crates/mercury-ui/tests/a11y-snapshots/`, one line per element, `role | label | description | flags`, with no geometry, so a changed word, a changed order or a glyph read twice shows up as a diff;
- the rectangle each control hands to the assistive technology has an area and lies inside the window and its landmark or overlay; the toolbar buttons are checked to the pixel (36 x 34 px, 8 px apart).

The helper is `crates/mercury-ui/tests/common/a11y.rs` (`init`, `reading_order`, `subtree` for a popup, `control`, `assert_labelled`, `assert_snapshot`, `assert_bounds`), and `tests/a11y_harness.rs` tests the helper itself. A popup's content is read with `subtree` on its own: the testing platform lists a popup before its owner's other children, and a screen reader lists it after them. Slint's debug info, which `ElementHandle` needs, comes only from the `test-support` feature of `mercury-ui`, and a `slint::slint!` block inside a test carries none, so the tests use the real components.

**Run them** with the feature on. A test filter matches test names, not file names, so name the test files:

```bash
cargo test --release -p mercury-ui --features test-support --test 'a11y_*'
```

Add `--no-fail-fast` to see the failures of every file at once. These tests need `--features test-support`, so they run in `make test` and are linted by `make lint` (both use `--all-features`), and not in `make qa-fast`, which builds the engine-free crates without features.

**Bless the golden files** after a change you meant. A missing file fails and prints what it would hold; an existing one fails with a line diff (`-` the golden file, `+` the interface now). Read the new order the way a VoiceOver user would hear it (does it follow the screen, does each button say what it does and not its glyph, is anything read twice), then rewrite the files and review the diff:

```bash
MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-ui --features test-support --test 'a11y_*'
git diff crates/mercury-ui/tests/a11y-snapshots
```

The leading `#` lines of a golden file (its licence lines and the command above) are ignored when comparing.

**The labelling rule.** Every interface component follows it, and the tests enforce it:

1. Every control has an `accessible-role`, an `accessible-label` and an `accessible-action-default`. The role is a constant, because Slint cannot bind it to a state: `button`, `tab`, `list-item`, `checkbox`, `radio-button`, `switch`, `combobox`, `slider` or `text-input`. The label says what the control does in words ("Back", "Close tab"), never its glyph ("‹", "×", "⋮"); it may be a binding. The default action does what a click does, and nothing while the control is disabled.
2. State goes in properties: `accessible-enabled` (mirror `enabled`), `accessible-checkable` with `accessible-checked`, `accessible-item-selectable` with `accessible-item-selected`, `accessible-item-index` and `accessible-item-count` for rows and tabs, `accessible-expandable` with `accessible-expanded`, and `accessible-value`. A state that has no property (loading, playing sound, pinned) goes in `accessible-description`.
3. What is inside a labelled control is hidden. `Text`, `Image` and `Path` are accessible by default, so the glyph or caption inside a button, and every decorative image, separator and focus ring, carries `accessible-role: none`; otherwise the control is read twice.
4. A text field has an `accessible-label` for its purpose (not its current text) and, when it draws a placeholder, the same words as `accessible-placeholder-text`, with the drawn placeholder hidden.
5. Slint 1.17 has no toolbar, dialog or heading role. The toolbar is a `navigation` landmark, the address bar a `search` landmark, the page a `main` landmark, a tab strip a `tab-list`, a menu or a list a `list`, and a sheet, prompt or popover a `groupbox` whose label is its heading (keyboard focus moves into it when it opens). Every landmark has a label. A window has one `main`: while a PDF is open the page area of the PDF viewer is a `region`, and the web view's "Page content" node (which stays in the tree under the viewer) has no description (`tests/a11y_pdf.rs`).
6. Within one window state no two controls share both role and label; rows and tabs with an item index are exempt, and static text may repeat. What a page causes, such as a dialog, a prompt or a pop-up notice, names the page's origin in its label or description, as it does on screen. Some tests find a control by its label (the desktop tests look for "Retry" and "Dismiss"), so a label a test uses stays as it is.
7. Toolbar buttons carry no shortcut text in their label or description: `shortcuts.rs` is the one source. Menu rows keep their shortcut as the description.

### Accessibility of web pages

The tree of a web page is built by the vendored Servo layout (`third_party/servo-layout/accessibility/`), from the answers of one engine-free crate, `crates/mercury-a11y` (roles, accessible names and descriptions on the `accname` crate, states, values, relations, the advertised actions, the order in which a page's trees may reach AccessKit, and the per-tab routing). The crate builds and tests in the light pool, without Servo:

```bash
cargo test --release -p mercury-a11y
```

- `tests/roles.rs`, `names.rs`, `states.rs`, `relations.rs` read tables written from the specifications (`tests/data/`) and a hand-built DOM (`mercury_a11y::testing::TestDom`, which can parse a small HTML string).
- `tests/graft.rs` and `bridge.rs` feed exactly what the window's adapter would push into `accesskit_consumer::Tree`, which panics where AccessKit would, so an update that arrives before its graft is caught.
- `tests/snapshot.rs` and `tests/snapshots/*.txt` pin the text format of a tree (`mercury_a11y::snapshot::dump`) and a golden of a hand-built page with iframes; bless with `MERCURY_A11Y_BLESS=1 cargo test --release -p mercury-a11y --test snapshot`.
- `crates/mercury-ui/tests/a11y_page_node.rs` checks that exactly one `main` node labelled "Page content" exists in every state of the main window.

The tests that start a real headless Servo and read a live page's tree (`crates/talaria-servo/tests/a11y_pages.rs`, `a11y_actions.rs`, `a11y_frames.rs`, pages in `tests/pages/a11y/`) are named `headless_servo_<what>` and are `#[ignore]`d: they need a GL context, so `make test` skips them. Run them where Servo can render, one thread at a time:

```bash
make test-a11y-pages
```

The macOS CI job runs the same target. A person with VoiceOver does the last check (`hands-on-checklist.md`, "VoiceOver on web pages").
8. A new component gets a headless test that finds its controls by role and label, presses each default action, runs `assert_labelled` in every state and compares its reading order with a golden file. A test file starts with `#![cfg(feature = "test-support")]` and `mod common;`.
9. Edits to `main-window.slint` stay line-level and carry a `// a11y-base` comment, so the layout rewrite can find them and move them.
10. A field that hides what is typed never hands its text on. Slint's std `LineEdit` binds `accessible-value` to its text, and AccessKit passes that on for a password field as well: macOS gives it unmasked to any program with accessibility access, as the value of a "secure text field". A password field is therefore a `SecretField` (`ui/passwords/secret-field.slint`) or a `RequestField` (`ui/dialogs/page-request-parts.slint`), which set `accessible-value` to nothing, and never a `LineEdit` with `input-type: password`. `tests/a11y_secrets.rs` fails for a password input type anywhere else, and `a11y_windows.rs` and `a11y_page_ui.rs` type into both components and read the value back. (A password the person asked to see, with Show in Settings > Passwords, is read aloud by design; a generated one too.)
11. A `ListView` builds only the rows that are on screen, and only existing rows are in the accessibility tree. Slint's adapter answers no scroll action, so an assistive technology reaches the first screenful of a long list (the history list, the bookmarks manager, the select popup, the bookmarks overflow menu) and the rest only after the page has been scrolled with the wheel or trackpad. `a_long_history_gives_assistive_technology_only_the_rows_on_screen` freezes this. The follow-up is keyboard scrolling (arrow and page keys, scrolling to the focused row); until then the search fields narrow the lists. A row's `accessible-item-count` counts what the list exposes: the history list counts visits only (its day headings and "Show more" are not counted), the select popup counts its option group headings, and the bookmarks overflow menu counts its separators.

The tests use Slint's testing platform, which has no AccessKit adapter. `cargo run --release -p mercury-ui --example a11y_real_adapter` (macOS, a login session, no GPU needed) opens the real main window, the Settings window on its Passwords section and a control window on Slint's winit backend with the software renderer, asks each window's view for its `accessibilityChildren` as VoiceOver does, prints the tree the adapter builds, and checks that the controls are named and that a password set through `setAccessibilityValue:` reads back empty. It does not replace trying VoiceOver itself.

What the tests cannot say is whether VoiceOver reads the result well on a real Mac. That is the VoiceOver section of the [hands-on checklist](hands-on-checklist.md).

### Menus and the About window

`apps/mercury-desktop/src/menus/model.rs` holds the menu bar as a table. Its tests check that every item shows the key that runs its action in `shortcuts.rs`, and that each key has exactly one owner: either the window's key path or the native menu. `decide_key_equivalent` holds the main menu's whole decision for a key equivalent; it is tested as a table that covers the browser window, other Mercury windows and native panels, with and without auto-repeat. `crates/mercury-ui/tests/render_menus.rs` renders the browser menu and the About window with Slint's software renderer, with the chrome theme set to Light, Dark and System. It also opens the browser menu in the browser window and drives it with the arrow keys and Return. Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.

The macOS menu bar cannot be opened without input. To read it back, use the diagnostic `MERCURY_MENU_DIAGNOSTICS`. It is compiled only with the `menu-diagnostics` feature of `mercury-desktop`, which `make build-release` and the app bundle leave off; build a binary for it with `cargo build --release -p mercury-desktop --features menu-diagnostics` in the GStreamer environment, then run `make build-release` again afterwards. Run a background launch with `MERCURY_MENU_DIAGNOSTICS=<directory>`. The log then says whether Mercury's menu is the main menu after start-up, while the About window is open and after it closes, and while the settings window is open and after it closes. The About window, drawn by the app's renderer, is saved as `about-window.png` in that directory. Use `RUST_LOG=mercury_browser=info` or more to see the log lines.

### Clipboard

`crates/talaria-core/src/clipboard.rs` decides which clipboard writes and reads of a page go through, and which keys and context-menu items are the user's own Cut, Copy and Paste; `crates/talaria-servo/src/clipboard.rs` tests one tab's handling with a fake answer object. Both are tested without starting the engine. Automated runs (`--screenshot`, `--background`, `--import-from`) and tests use a clipboard in memory, never the user's. `MERCURY_TEST_CLIPBOARD_TEXT=<text>` gives that clipboard a starting text; a normal run ignores it. Pages do not get `navigator.clipboard` yet (`ASYNC_CLIPBOARD_OFFERED`), and tests in `crates/talaria-core/src/clipboard.rs` (both), `crates/talaria-servo/src/runtime.rs` (the API) and `crates/talaria-core/tests/page_requests.rs` (the right to read) fail if that, or a page's right to read the clipboard (`CLIPBOARD_READ_FRAME_GATE`), is turned on without the checklist in the constants' doc comments. The user's own Cut, Copy and Paste do depend on `talaria-servo` telling the tab's clipboard about each command before Servo performs it; `web_view.rs` has the functions that do (`send_keyboard_input`, `send_clipboard_command`, `answer_control_in_tab`, the code behind `ServoWebView::keyboard_input`, `::clipboard` and `::answer_control`), and its tests drive them with a recorder in place of the engine and check that the next read of the clipboard gets the text. To check the page side, serve `tests/` with `scripts/serve-tests.sh` and take a `--screenshot` of `tests/pages/clipboard.html`, once as it is and once with `MERCURY_TEST_CLIPBOARD_TEXT=seed`: both must show `navigator.clipboard: missing`. Real Cut, Copy and Paste through the engine and the system clipboard need input and are in the hands-on checklist.

### Vendored dependencies

Each Mercury-specific upstream regression needs a focused test close to the patch. The `gaol` shared-memory test protects the `memfd` and `ftruncate` behavior that originally killed isolated Servo children with `SIGXFSZ`.

The vendored Servo crates are not workspace members, so `make test` and `cargo test --workspace` do not run their own unit tests (`servo-net` is the exception: `make test` also runs `make test-servo-net`, see [Certificate exceptions](#certificate-exceptions)). `make vendored-tests` runs the ones that need neither GStreamer nor a window (`servo-config`, `stylo_static_prefs`, `servo-embedder-traits`, `servo-constellation-traits`: the preferences, the Stylo preference bridge and the message types) in the light build pool, and the `Fast checks` jobs of CI run it. The ones that build the engine are run by hand with `RUST_TEST_THREADS=2` on a Servo upgrade (see [Re-vendor on a Servo upgrade](vendored-patches.md#re-vendor-on-a-servo-upgrade) and [Testing a vendored crate](vendored-patches.md#testing-a-vendored-crate)). The behaviour of the backports is checked with the pages in `tests/pages/` in a real run: `compat-features.html`, `compat-editing.html`, `webgl2-textures.html`, `svg-deep-nesting.html` and `notification-no-icon.html`, each printing `PASS`, `FAIL` or `SKIP` lines.

The tests in `apps/mercury-desktop` that create a Metal device (`rendering::metal::tests`) hang while the macOS screen is locked, in a state that `kill -9` does not end, and so does every `--screenshot` run. Unlock the screen and keep it from locking before `make test`, the page runs and the screenshot tests. A hung test binary also keeps its pipe open, so a gate run whose output goes through a pipe can stop short without its summary.

### Certificate exceptions

A certificate exception is accepted only for the pair of host and certificate that failed there ([the patch](vendored-patches.md#the-servo-net-certificate-exception-patch)). The rule is tested in three places, and all of the tests are Rust. None touches a real trust store, the keychain, another host on the network or your profile.

**The engine patch** is tested in `third_party/servo-net/tests/certificate_exceptions.rs`, part of the crate's `main` test target (the crate's library has `test = false`, so every test lives under `tests/`; `tests/mercury_unit.rs` includes `mercury_log.rs` and `mercury_svg.rs` with `#[path]`, which runs the 30 unit tests inside those two files there). Most of them need no network: which pairs of name and certificate are accepted (the same certificate under another name, another certificate under the same name, a subdomain and a parent name are refused), how host names are normalised (case, a trailing dot, IPv6 literals, garbage), each refusal of `accept_failed_certificate`, HSTS hosts (learned and preloaded), the bound of the record of failures, revoke, and the refusal of `chrome:allowcert`. The others use a loopback TLS server with a self-signed certificate: a fetch fails with the server's certificate, succeeds under the accepted name after the user's decision, still fails under the server's other name, fails again on a new connection after a revoke, and an IPv6 literal gets a certificate error and can be accepted. A third group is about HSTS, which Mercury keeps between sessions: the same server answers `Strict-Transport-Security`, the list learns the host from a connection that needed no exception (the control), does not learn it from one that only the user's exception made valid, and the user can accept again afterwards; hosts that have had an exception (in every spelling, revoked or not) teach nothing while their neighbours do; and a scan of the crate's sources fails if anything but `HttpState::learn_hsts_from_response` updates the list from a response, or if the HTTP fetch or the WebSocket handshake stops calling it. The revoke test covers new connections only: a revoke does not close connections that are already open.

`servo-net` is a vendored dependency with dev-dependencies, so `cargo test -p servo-net` from the workspace root does not work (cargo refuses a package that is not a member). The root `Cargo.toml` lists it under `exclude`, as it does `gaol`, so cargo accepts the crate's own manifest: **`make test-servo-net`** runs the whole `main` target on that manifest with the crate's own `Cargo.lock`, and builds the crate against the published sibling crates (`servo-net-traits` and the rest). It does not use Mercury's vendored sibling crates or Mercury's `Cargo.lock`, so Mercury's K0 changes to `servo-embedder-traits` and `servo-config` are not in this build. That costs nothing here: of the crates `servo-net` links against, `servo-net` itself is the only one Mercury changes in a way that matters to it (the others it uses are byte-identical to the published ones, and the one patched crate in its test graph, `servo-default-resources`, differs only in not registering the default resource reader, which the tests need registered). `make test` runs it (so the gates do), and so does the CI job "Vendored servo-net tests"; neither needs GStreamer or a window. The target adds the feature `fonts/bundled_freetype`, which builds FreeType from source, because the crate's own build needs FreeType 24.3.18 or later and Ubuntu 22.04 (the CI runner) has 24.1.18; it does nothing on macOS. Two tests of the crate fail on the unpatched `7f1a78e` copy as well, with nothing of Mercury's involved, and the target skips them by name: `fetch::test_file` reads `../../components/net/tests/test.css`, a path that exists only in Servo's own repository, and `filemanager_thread::test_filemanager` panics on the process-wide async runtime when it runs with the other tests. To run only the certificate-exception tests while you work, from the repository root:

```bash
RUST_TEST_THREADS=2 cargo test --release --locked --target-dir target \
  --manifest-path third_party/servo-net/Cargo.toml \
  --features test-util,devtools --test main certificate_exceptions
```

The first run builds the published dependencies (about 11 minutes on a 12-thread Intel Mac, a few hundred MB under `target/`, which the separate `--target-dir` keeps out of the vendored crate). Format only the files you changed with the command under "No formatting churn" in [Vendored patches](vendored-patches.md#the-servo-engine-patches-servo-script-and-friends), then run `export` and `make servo-patches-check`.

**Mercury's rules** need no engine and run in minutes: `cargo test --release -p talaria-core -p mercury-site-security`. They cover the one form of a host name that the exception set, the address-bar mark and the request parser share, the rule that offers an exception only for local names, the request parser and its refusals, and the certificate page.

**End to end,** `scripts/macos/test-certificate-exceptions.sh` (or `make test-certificate-exceptions-macos`) runs the release browser against the loopback TLS test server (`crates/mercury-site-security/examples/tls_test_server.rs`). It needs the release build, so run `make build-release` first; it builds the test server itself (a small build without the engine). It uses only `127.0.0.1`, and each run is an automated `--screenshot` run with its own temporary profile, so it touches no trust store, keychain or profile of yours and synthesises no input. The server's one certificate has two names, `127.0.0.1` and `localhost`, and both reach the server. With `MERCURY_TEST_PROCEED_ON_CERTIFICATE_ERROR=1`, which makes the certificate page continue by itself in an automated run, the script opens `https://127.0.0.1:<port>/host-binding`. That page loads an image from its own name and one from the other name, and puts `CERTEXC same=<loaded|failed>; other=<loaded|failed>` in its title. The check passes only when the title reads `same=loaded; other=failed`, which means the exception holds for the name it was made for and not for the same certificate under the other name; an engine that accepts the certificate for every host loads both. A second run opens the page as `localhost` and must read the same, with the log showing an exception for `localhost` and none for `127.0.0.1`. A control run without the variable must show the certificate page ("Certificate not valid") and accept nothing. The script prints one line per check, ends with `all checks passed` and exits non-zero if any check fails. A run that misses the browser's 30-second page-load limit (exit status 3), which can happen on a heavily loaded machine, is repeated once. `MERCURY_TEST_CERTEXC_KEEP=<directory>` copies the screenshots and logs there before the temporary directory is removed, and `MERCURY_TEST_CERTEXC_DELAY=<milliseconds>` changes how long each run waits before its screenshot (10000 by default). Run it again after any Servo upgrade and after any change to `third_party/servo-net`.

## Manual browser checks

Automated runs on the shared development Mac never synthesise keyboard or mouse input and never take focus. They use `--screenshot`, which captures the window once the page has loaded and exits, or `--background`:

```bash
target/release/mercury-browser --screenshot /tmp/js.png --window-size 1000x800 \
    "file://$PWD/tests/pages/js.html"
```

Before merging a rendering or media change, check at least:

1. The new tab page paints.
2. A simple HTTPS page loads, paints and updates its title.
3. The relevant pages in `tests/pages/` (for example `colors.html`, `canvas-readback.html`, `media.html`) show the expected result. Serve the media pages with `scripts/serve-tests.sh`, which supports `Range` requests, rather than from `file://`.
4. Quitting leaves no `mercury-browser` process behind and no repeated broken-pipe errors in the log.

What only a person can try (clicking, typing, logging out, real devices) is in the [hands-on checklist](hands-on-checklist.md).

### Quit paths

Nobody may synthesise input on the shared test machine, so the quits a user performs are exercised from inside the process with the diagnostic options `--exit-after <milliseconds>` and `--exit-by close-request|quit|terminate|logout`. `close-request` dispatches Slint's close request to the window, the same call the winit backend makes for the title bar's close button; `quit` calls `slint::quit_event_loop()`, as the quit shortcut does. On macOS, `terminate` sends `-[NSApplication terminate:]` from the run loop, as a Quit menu item does, and `logout` sends the browser's own process (never another one) the quit Apple event of a log-out (`keyAEQuitReason` = `kAEReallyLogOut`); an event addressed to the current process is dispatched directly to its handler and never leaves the process. Neither can log the user out. `logout` exists only in builds with the `quit-diagnostics` cargo feature (`cargo build --release -p mercury-desktop --features quit-diagnostics`, with the GStreamer environment); `make lint` and `make check` compile it through `--all-features`, released builds never contain it. They work with `--background` (exit status 0) and with `--screenshot`, where a quit before the capture exits with status 1. Signals are tested by sending `SIGTERM`, `SIGINT` or `SIGHUP` to a `--background` run: it quits in order and ends by that signal.

```bash
target/release/mercury-browser --background --exit-after 3000 --exit-by close-request \
    "file://$PWD/tests/pages/animation.html"
```

After any quit, no `mercury-browser` process of the run may be left, and a temporary profile (`$TMPDIR/mercury-browser-profile-<pid>-*`) must be gone. `SLINT_DESTROY_WINDOW_ON_HIDE=1` makes hiding destroy the window on macOS too, which exercises the Wayland teardown path.

For an engine-level website failure, reproduce it in the pinned Servo `servoshell`. Record whether the same revision succeeds or fails there.

## Documentation checks

```bash
make docs
```

MkDocs runs in strict mode, so missing navigation entries, broken internal references, and configuration warnings fail the build.

