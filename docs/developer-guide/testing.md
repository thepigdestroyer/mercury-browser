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

### Certificate exceptions

A certificate exception is accepted only for the pair of host and certificate that failed there ([the patch](vendored-patches.md#the-servo-net-certificate-exception-patch)). The rule is tested in three places, and all of the tests are Rust. None touches a real trust store, the keychain, another host on the network or your profile.

**The engine patch** is tested in `third_party/servo-net/tests/certificate_exceptions.rs`, part of the crate's `main` test target (the crate's library has no tests of its own; they all live under `tests/`). Most of them need no network: which pairs of name and certificate are accepted (the same certificate under another name, another certificate under the same name, a subdomain and a parent name are refused), how host names are normalised (case, a trailing dot, IPv6 literals, garbage), each refusal of `accept_failed_certificate`, HSTS hosts (learned and preloaded), the bound of the record of failures, revoke, and the refusal of `chrome:allowcert`. The others use a loopback TLS server with a self-signed certificate: a fetch fails with the server's certificate, succeeds under the accepted name after the user's decision, still fails under the server's other name, fails again on a new connection after a revoke, and an IPv6 literal gets a certificate error and can be accepted. A third group is about HSTS, which Mercury keeps between sessions: the same server answers `Strict-Transport-Security`, the list learns the host from a connection that needed no exception (the control), does not learn it from one that only the user's exception made valid, and the user can accept again afterwards; hosts that have had an exception (in every spelling, revoked or not) teach nothing while their neighbours do; and a scan of the crate's sources fails if anything but `HttpState::learn_hsts_from_response` updates the list from a response, or if the HTTP fetch or the WebSocket handshake stops calling it. The revoke test covers new connections only: a revoke does not close connections that are already open.

`servo-net` is a vendored dependency with dev-dependencies, so `cargo test -p servo-net` from the workspace root does not work (cargo refuses a package that is not a member). The root `Cargo.toml` lists it under `exclude`, as it does `gaol`, so cargo accepts the crate's own manifest: **`make test-servo-net`** runs the whole `main` target on that manifest with the crate's own `Cargo.lock`, and builds the crate against the published sibling crates (`servo-net-traits` and the rest), which are the same code as Mercury's copies because only `servo-net` is changed here. It does not use Mercury's vendored sibling crates or Mercury's `Cargo.lock`. That costs nothing here: of the crates `servo-net` links against, `servo-net` itself is the only one Mercury changes in a way that matters to it (the others it uses are byte-identical to the published ones, and the one patched crate in its test graph, `servo-default-resources`, differs only in not registering the default resource reader, which the tests need registered). `make test` runs it (so the gates do), and so does the CI job "Vendored servo-net tests"; neither needs GStreamer or a window. Two tests of the crate fail on the unpatched `7f1a78e` copy as well, with nothing of Mercury's involved, and the target skips them by name: `fetch::test_file` reads `../../components/net/tests/test.css`, a path that exists only in Servo's own repository, and `filemanager_thread::test_filemanager` panics on the process-wide async runtime when it runs with the other tests. To run only the certificate-exception tests while you work, from the repository root:

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

