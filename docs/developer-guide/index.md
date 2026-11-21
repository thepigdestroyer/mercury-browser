---
title: Developer Guide
description: How the Mercury Browser codebase is organised, how to build and test it, and how to prepare a focused contribution.
---

# Developer Guide

Mercury is written in Rust, with its interface in Slint, on the published Servo 0.8 engine, plus a few small patches kept as exported patches against vendored crates. It builds on macOS, on Apple Silicon and on Intel Macs. The browser logic doesn't know about Servo, and the interface doesn't know about the browser logic's internals, so most behaviour can be tested without starting the engine or opening a window. This guide shows where Mercury ends, where Servo begins, and how to test the seam between them.

Work happens on the `mercury` branch. Each feature is developed on its own branch, starts with tests, is reviewed by someone other than its author, and merges once the full checks pass.

## A useful reading order

1. [Development setup](setup.md): build the app on macOS (Apple Silicon or Intel), with its GStreamer copy and the build slots.
2. [Architecture](architecture.md): follow commands from Slint to a web view and events back to the interface.
3. [Servo integration](servo-integration.md): rendering through Metal, media, content blocking, process isolation and shutdown.
4. [Testing and quality](testing.md): the checks to run for the area you changed.
5. [Hands-on checklist](hands-on-checklist.md): what to try by hand, because automated runs can't click.
6. [Vendored patches](vendored-patches.md): the rules for the few patched dependencies.
7. [Continuous integration](continuous-integration.md): what CI and the release workflow check, and the dependency policy.
8. [Licence compliance](licence-compliance.md): what Mercury must credit and publish when it ships, how the credits page is generated, and the notes on the LGPL libraries, Slint, codecs and patents.
9. [Origins and credits](origins.md): what came from Hermes Browser and what Mercury added.
10. [Saved logins](passwords.md): the Keychain store, the password manager and the worker that keeps Keychain calls off the interface thread.
11. [Camera and microphone](media-capture.md): how pages get them, and testing with fake devices only.
12. [Releases](releases.md): the macOS disk image, the Linux archive, the update manifest and the documentation site.
13. [Contributing](contributing.md): how to prepare a focused, reviewable change.

## Repository map

| Path | Purpose |
| --- | --- |
| `apps/mercury-desktop` | The application: window, macOS menu bar, shortcuts, file panels, settings window glue, start-up and an orderly quit on every path, and the glue for history, bookmarks, downloads, find in page, sessions, saved logins, camera and microphone, site security, the filter lists and Now Playing |
| `crates/mercury-ui` | The Slint interface: tab strip and tab search, address bar, new tab page, settings window, dialogs, pickers, browser menu, About window, the history, bookmarks, import and downloads windows, find bar, site information popover |
| `crates/talaria-core` | Browser logic with no engine inside: tabs (pinned, restored), navigation, address resolution, page requests, find, the HTTPS policy, the clipboard rules, commands and events, tested with fake web views |
| `crates/talaria-servo` | The bridge to Servo: web views, content blocking, permission and dialog requests, media and the media session, downloads, find in page, error pages, the HTTP cache, proxies, clearing data at exit |
| `crates/mercury-settings` | Settings, search engines and their keywords, stored as `settings.json` in the profile, and the saved session (`session.json`) |
| `crates/mercury-appearance` | How Mercury looks, as data: the palettes and the contrast rules each one must pass, the tab-group colours, the one-letter group-key map, and the resolving of the stored layout, palette and tab-group settings into values for the interface. No engine and no Slint |
| `crates/mercury-store` | History, bookmarks and the downloads list in SQLite, with search and address-bar suggestions |
| `crates/mercury-passwords` | Saved logins in the macOS Keychain, a password generator, CSV import and export ([details](passwords.md)) |
| `crates/mercury-password-manager` | The logic between `mercury-passwords` and the interface: the list in Settings, editing, the authentication gate before a password is shown or copied, manual fill, saving HTTP sign-ins ([details](passwords.md#the-password-manager-in-the-browser)) |
| `crates/mercury-downloads` | The download engine: fetching files, resuming after an interruption, stopping cleanly at quit |
| `crates/mercury-import` | Reading history and bookmarks from Chromium-based browsers, Firefox and bookmark files |
| `crates/mercury-filter-lists` | The ad and tracker filter lists as data files with a manifest, the engine built from them at start-up, and the background updates |
| `crates/mercury-site-security` | Internationalised names shown safely, Mercury's network, certificate and crash pages, and the certificate-exception request |
| `crates/mercury-tab-snapshots` | Small pictures of the open tabs, for the Orbit view, kept in memory only: the shrink to at most 480 by 300 pixels, the store under a 128 MB cap and the rules for when a picture is taken ([details](servo-integration.md#tab-snapshots)) |
| `crates/mercury-option-keys` | The ⌥ group-key state machine, a pure module with injected time: quick ⌥ + letter passes to the page, a 0.4 s hold opens the "Switch group" card, letter then digit then ⌥ up commits. Nothing wires it into the keyboard yet |
| `crates/mercury-release` | Writes, signs and checks the update manifest (`update.json`) that each release carries |
| `crates/mercury-credits` | The credits tool: writes the page of licence texts, copyright lines, notices and MPL source pointers for every crate built into the app, and checks that every crate can be credited ([details](licence-compliance.md#the-credits-page-and-its-tool)) |
| `crates/mercury-servo-patches` | The tool that vendors, patches, checks and upgrades the crates in `third_party/` |
| `crates/mercury-iconset` | A build tool: renders the app icon's SVG into the macOS iconset |
| `crates/mercury-devtools` | Development and CI tools in Rust: the loopback server for `tests/`, the smoke test's screenshot checker, the CI scripts' helpers |
| `third_party` | Vendored crates, each the published crate plus Mercury's exported patch in `third_party/patches/`: `gaol`, `surfman`, `servo-canvas`, `servo-default-resources` (so that Mercury's own error pages can be used), the `servo-script`, `servo-script-bindings`, `servo-media-streams` and `servo-media-gstreamer` crates patched for camera and microphone, the engine base (`servo`, `servo-config`, `stylo_static_prefs`, the three message crates, `servo-net`, `servo-webgl`, `servo-layout`, `servo-layout-api`: preferences, the message scaffold, backports from post-0.8 Servo, SpiderMonkey 153.7.0 through `servo-script-webcrypto`), and the other crates (Servo's, and Slint's winit backend), vendored unchanged and kept ready for patches. `vendor.toml` lists them, `make servo-patches-check` checks them, and each has a `MERCURY.md` explaining the change |
| `scripts/macos` | GStreamer set-up, the app bundle, signing and their tests |
| `scripts/ci`, `.github/workflows` | Continuous integration, release builds and the documentation deploy |
| `scripts/dev`, `.opencode` | The build slots (`build-slot.sh`) and the disk guard plugin and skill that the AI coding agents work under ([setup](setup.md#build-slots)) |
| `tests/pages` | Local test pages for rendering, media, dialogs, controls and canvas |
| `docs` | This MkDocs site and its screenshots |

`mercury-store`, `mercury-passwords`, `mercury-password-manager`, `mercury-downloads` and `mercury-import` are libraries with their own tests, and the app's history, bookmarks, downloads, saved-logins and import windows use them.

## Rules that apply everywhere

- Product code is Rust and Slint only. No new Python, Node or other scripting-language code, in the browser or in its tooling: write tools in Rust, or in POSIX shell when it is a few lines of glue. JavaScript is allowed only where it must run inside a page through Servo. The repository's own tools are in `crates/mercury-devtools`. No Swift or Objective-C source either: call Apple frameworks from Rust through `objc2`.
- Reuse first. Before writing a feature, look for working code: a library, a Servo backport or pull request, or another browser's code under a compatible licence. Don't rewrite a working library just to change its language. C and C++ libraries (GStreamer and SpiderMonkey, for example) are linked and called from Rust through a binding crate or FFI; each gets an entry in `THIRD-PARTY-NOTICES.md` and its licence text in `LICENSES/`. LGPL libraries are linked dynamically only, and GPL libraries are never linked or bundled.
- Only `talaria-servo` names Servo types, apart from the rendering and keyboard glue in `mercury-desktop` that already does.
- `talaria-core` depends on nothing but `url`, and does no I/O and never reads the clock.
- Every new file names its licence: an SPDX header for PolyForm Noncommercial 1.0.0, or an entry in `REUSE.toml` for files that can't hold a comment. Never change an existing file's licence. `CONTRIBUTING.md` in the repository has the exact rules. Call Mercury source-available, never open source.
- Code copied or adapted from another project goes in a file of its own that names the source repository, tag or commit, path and licence at the top, gets an entry in `THIRD-PARTY-NOTICES.md`, and keeps the upstream licence: MPL-2.0, or a permissive one (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause) with the upstream copyright notice kept and the licence text in `LICENSES/`. Never port GPL or LGPL code; take ideas only. Never paste outside code into a PolyForm file.
- Build, lint and test in release mode, so that one set of Servo artefacts serves all three.
- Make platform limits visible instead of claiming support that isn't there.
