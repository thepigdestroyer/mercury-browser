---
title: Origins and Credits
description: Mercury Browser began as a fork of Hermes Browser by Kayk Caputo. What came from Hermes, what Mercury changed, and how the licence and copyright work.
---

# Origins and credits

Mercury began as a fork of [Hermes Browser](https://github.com/KaykCaputo/hermes-browser), written by Kayk Caputo: a Linux-first browser shell around Servo 0.7, written in Rust with a Slint interface. Thanks to Kayk for the starting point, and to the Hermes contributors.

Mercury is now developed as its own project. The repository is no longer part of Hermes's fork network, and Mercury is not meant to track or replace Hermes.

## What came from Hermes

Hermes's 38 commits are the first in the history of the `mercury` branch. They brought:

- the layering Mercury still uses: an engine-neutral core (`talaria-core`), a Servo adapter (`talaria-servo`), a Slint interface crate and a desktop app. The `talaria-*` crates keep the names they had in Hermes;
- the first tab model, browser state and command controller, and the first address resolution;
- the first bridge to Servo and the Slint window, the new tab page, keyboard and input-method support, and the Linux rendering path through Vulkan;
- network ad and tracker blocking with Brave's filter lists;
- the patches to `gaol` and `surfman` in `third_party/` (see [Vendored patches](vendored-patches.md));
- the winged-sandal logo;
- the first Makefile, CI workflows and this documentation site's structure and styling.

## What Mercury changed and added

Everything since the fork is Mercury's own work. In particular:

- **Platform:** macOS on Apple Silicon first, with frames drawn through Metal, a native menu bar, macOS file panels, an app bundle with its own GStreamer, and an orderly quit on log-out and every other path.
- **Engine:** Servo 0.8 instead of 0.7, with Mercury's own patch to `servo-canvas` for the GPU 2D canvas.
- **New crates:** `mercury-settings`, `mercury-store` (history and bookmarks), `mercury-passwords`, `mercury-downloads` and `mercury-import`.
- **Features:** settings and search engines, page dialogs and permission prompts, HTTP sign-in, form controls and the right-click menu, pop-up handling, more filter lists, proxy support, the HTTP disk cache, clearing data at exit, and the other features on the [user guide's list](../user-guide/index.md#what-is-ready-and-what-isnt).
- **Rewrites:** much of the inherited code, including the desktop app, which was split into focused modules, and the Servo adapter.

The windows still look much like Hermes's. A complete redesign of the interface, far more customisable, is planned and will replace them.

## Licence and copyright

Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors. Portions © 2026 Kayk Caputo (Hermes Browser).

Mercury is source-available: free for any non-commercial use. The files written for Mercury are under the PolyForm Noncommercial License 1.0.0. The files that came from Hermes Browser and the application icon, which is drawn from the Hermes logo, stay under the Mozilla Public License 2.0, the licence Hermes used. Files adapted from other projects keep their upstream licence: MPL-2.0 for code from Servo and other MPL projects, or a permissive licence such as MIT or Apache-2.0, with the upstream copyright notice. Versions published before the change remain available under the MPL-2.0. The repository's `LICENSE` explains the split, `LICENSES/` holds both texts, and `NOTICE` records the copyright, the required notice and Mercury's origin.

The app carries the same information: **About Mercury Browser** shows the copyright and licence lines and "Began as a fork of Hermes Browser by Kayk Caputo.", and the app bundle includes `LICENSE`, `LICENSES/`, `NOTICE` and `THIRD-PARTY-NOTICES.md` in `Contents/Resources`.

Code that Mercury copies or adapts from other projects (Slint's Servo example, Servo, Zervo, Tauri and others) is credited in a header in each file and in `THIRD-PARTY-NOTICES.md`, with the licence texts in `LICENSES/`.
