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

`crates/mercury-ui/tests/page_request_render.rs` and `render_menus.rs` render Mercury's Slint components with Slint's software renderer, so they can be checked without a window. The examples in `crates/mercury-ui/examples/` (`render_settings`, `render_controls`, `render_tabs`) render the settings window, the page controls and the tab strip to PNGs for review.

### Menus and the About window

`apps/mercury-desktop/src/menus/model.rs` holds the menu bar as a table. Its tests check that every item shows the key that runs its action in `shortcuts.rs`, and that each key has exactly one owner: either the window's key path or the native menu. `decide_key_equivalent` holds the main menu's whole decision for a key equivalent; it is tested as a table that covers the browser window, other Mercury windows and native panels, with and without auto-repeat. `crates/mercury-ui/tests/render_menus.rs` renders the browser menu and the About window with Slint's software renderer, with the chrome theme set to Light, Dark and System. It also opens the browser menu in the browser window and drives it with the arrow keys and Return. Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.

The macOS menu bar cannot be opened without input. To read it back, use the diagnostic `MERCURY_MENU_DIAGNOSTICS`. It is compiled only with the `menu-diagnostics` feature of `mercury-desktop`, which `make build-release` and the app bundle leave off; build a binary for it with `cargo build --release -p mercury-desktop --features menu-diagnostics` in the GStreamer environment, then run `make build-release` again afterwards. Run a background launch with `MERCURY_MENU_DIAGNOSTICS=<directory>`. The log then says whether Mercury's menu is the main menu after start-up, while the About window is open and after it closes, and while the settings window is open and after it closes. The About window, drawn by the app's renderer, is saved as `about-window.png` in that directory. Use `RUST_LOG=mercury_browser=info` or more to see the log lines.

### Clipboard

`crates/talaria-core/src/clipboard.rs` decides which clipboard writes and reads of a page go through, and which keys and context-menu items are the user's own Cut, Copy and Paste; `crates/talaria-servo/src/clipboard.rs` tests one tab's handling with a fake answer object. Both are tested without starting the engine. Automated runs (`--screenshot`, `--background`, `--import-from`) and tests use a clipboard in memory, never the user's. `MERCURY_TEST_CLIPBOARD_TEXT=<text>` gives that clipboard a starting text; a normal run ignores it. Pages do not get `navigator.clipboard` yet (`ASYNC_CLIPBOARD_OFFERED`), and `crates/talaria-core/src/clipboard.rs`, `crates/talaria-servo/src/runtime.rs` and `crates/talaria-core/tests/page_requests.rs` each have a test that fails if that, or a page's right to read the clipboard, is turned on without the checklist in the constants' doc comments. The user's own Cut, Copy and Paste do depend on `talaria-servo` telling the tab's clipboard about each command before Servo performs it; `web_view.rs` has the functions that do (`send_keyboard_input`, `send_clipboard_command`, `answer_control_in_tab`, the code behind `ServoWebView::keyboard_input`, `::clipboard` and `::answer_control`), and its tests drive them with a recorder in place of the engine and check that the next read of the clipboard gets the text. To check the page side, serve `tests/` with `scripts/serve-tests.py` and take a `--screenshot` of `tests/pages/clipboard.html`, once as it is and once with `MERCURY_TEST_CLIPBOARD_TEXT=seed`: both must show `navigator.clipboard: missing`. Real Cut, Copy and Paste through the engine and the system clipboard need input and are in the hands-on checklist.

### Vendored dependencies

Each Mercury-specific upstream regression needs a focused test close to the patch. The `gaol` shared-memory test protects the `memfd` and `ftruncate` behavior that originally killed isolated Servo children with `SIGXFSZ`.

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

