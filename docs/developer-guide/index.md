---
title: Developer Guide
description: How the Mercury Browser codebase is organised, how to build and test it, and how to prepare a focused contribution.
---

# Developer Guide

Mercury is written in Rust, with its interface in Slint, on the published Servo 0.8 engine. The browser logic doesn't know about Servo, and the interface doesn't know about the browser logic's internals, so most behaviour can be tested without starting the engine or opening a window. This guide shows where Mercury ends, where Servo begins, and how to test the seam between them.

Work happens on the `mercury` branch. Each feature is developed on its own branch, starts with tests, is reviewed by someone other than its author, and merges once the full checks pass.

## A useful reading order

1. [Development setup](setup.md): build the app on macOS, with its GStreamer copy.
2. [Architecture](architecture.md): follow commands from Slint to a web view and events back to the interface.
3. [Servo integration](servo-integration.md): rendering through Metal, media, content blocking, process isolation and shutdown.
4. [Testing and quality](testing.md): the checks to run for the area you changed.
5. [Hands-on checklist](hands-on-checklist.md): what to try by hand, because automated runs can't click.
6. [Vendored patches](vendored-patches.md): the rules for the few patched dependencies.
7. [Continuous integration](continuous-integration.md): what CI and the release workflow check, and the dependency policy.
8. [Origins and credits](origins.md): what came from Hermes Browser and what Mercury added.

## Repository map

| Path | Purpose |
| --- | --- |
| `apps/mercury-desktop` | The application: window, macOS menu bar, shortcuts, file panels, settings window glue, start-up and an orderly quit on every path |
| `crates/mercury-ui` | The Slint interface: tab strip, address bar, new tab page, settings window, dialogs, pickers, browser menu, About window |
| `crates/talaria-core` | Browser logic with no engine inside: tabs, navigation, address resolution, page requests, commands and events, tested with fake web views |
| `crates/talaria-servo` | The bridge to Servo: web views, content blocking, permission and dialog requests, media, the HTTP cache, proxies, clearing data at exit |
| `crates/mercury-settings` | Settings, search engines and their keywords, stored as `settings.json` in the profile |
| `crates/mercury-store` | History and bookmarks in SQLite, with search and address-bar suggestions (the windows that use it are being merged) |
| `crates/mercury-passwords` | Saved logins in the macOS Keychain, a password generator, CSV import and export ([details](passwords.md)) |
| `crates/mercury-downloads` | The download engine: fetching files, resuming after an interruption, stopping cleanly at quit |
| `crates/mercury-import` | Reading history and bookmarks from Chromium-based browsers, Firefox and bookmark files |
| `third_party` | Patched copies of `gaol`, `surfman` and `servo-canvas`, each with a `MERCURY.md` explaining the change |
| `scripts/macos` | GStreamer set-up, the app bundle, signing and their tests |
| `scripts/ci`, `.github/workflows` | Continuous integration and release builds |
| `tests/pages` | Local test pages for rendering, media, dialogs, controls and canvas |
| `docs` | This MkDocs site and its screenshots |

`mercury-store`, `mercury-passwords`, `mercury-downloads` and `mercury-import` are finished libraries with their own tests; the app does not use them yet.

## Rules that apply everywhere

- Product code is Rust and Slint only. No new Python, Node or other scripting-language code, in the browser or in its tooling: write tools in Rust, or in POSIX shell when it is a few lines of glue. JavaScript is allowed only where it must run inside a page through Servo.
- Only `talaria-servo` names Servo types, apart from the rendering and keyboard glue in `mercury-desktop` that already does.
- `talaria-core` depends on nothing but `url`, and does no I/O and never reads the clock.
- Every new file names its licence: an SPDX header for PolyForm Noncommercial 1.0.0, or an entry in `REUSE.toml` for files that can't hold a comment. Never change an existing file's licence. `CONTRIBUTING.md` in the repository has the exact rules.
- Code copied or adapted from another project goes in a file of its own that names the source repository, tag or commit, path and licence at the top, gets an entry in `THIRD-PARTY-NOTICES.md`, and keeps the upstream licence: MPL-2.0, or a permissive one (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause) with the upstream copyright notice kept and the licence text in `LICENSES/`. Never port GPL or LGPL code; take ideas only. Never paste outside code into a PolyForm file.
- Build, lint and test in release mode, so that one set of Servo artefacts serves all three.
- Make platform limits visible instead of claiming support that isn't there.
