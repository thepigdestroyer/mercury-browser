# Patched i-slint-backend-winit 1.17.1

This directory vendors `i-slint-backend-winit` 1.17.1 under its own licence (`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` in its `Cargo.toml`).
It is the crate exactly as published on crates.io (checksum
`a27c7e8f53418d3017362465367b6612b6e439a68097d627d1b91c94b885f0a6`), built from https://github.com/slint-ui/slint at commit
`cf62c975c311e7036d599ed8ed0b7e6a8386a934`, path `internal/backends/winit`,
plus Mercury's changes in `third_party/patches/i-slint-backend-winit.patch`.
`third_party/vendor.toml` records where it came from.

`mercury-servo-patches diff i-slint-backend-winit` shows the changes. Each changed file carries
a `Mercury:` comment at every change; `make servo-patches-check` fails without one.

## Licence

Slint offers this crate under GPL-3.0-only, OR the Slint Royalty-free Desktop,
Mobile and Web Applications License 2.0, OR the Slint Software License 3.0.
Mercury uses it under the **Royalty-free 2.0** licence, as it does the rest of
Slint (see `deny.toml` and `docs/developer-guide/continuous-integration.md`).
That licence allows modifying the Software and distributing it, modified or
not, as part of an application (`LICENSES/LicenseRef-Slint-Royalty-free-2.0.md`
in this directory, sections on the grant and on distribution). Its conditions,
which every change here must keep:

- Never remove or alter any licence notice in the source: the copyright lines,
  the `SPDX-License-Identifier` headers, the disclaimers of warranty and the
  limitations of liability. Mercury's changes add `Mercury:` comments next to
  them and leave them as they are.
- Keep this crate's `LICENSES/` directory as published.
- The application must show Slint's attribution (the AboutSlint widget or the
  "Made with Slint" badge), as Mercury already does for Slint as a whole.

Mercury's changes to this crate stay under the same Slint licence; they are not
relicensed under PolyForm or the MPL.

## Changes

Changes made by lane a11y-1 (21 Nov 2026). The patch stays under the Slint licence.

### Grafting a web page's accessibility tree into the window's tree (EE1) — `accesskit.rs`, `lib.rs`, `winitwindowadapter.rs`, new `mercury_graft.rs`

- **What:** the window's AccessKit tree has a node for the page ("Page content", role `main`). The backend marks the node
  matching a configured `(role, label)` as a graft, pushes the page's trees (the web view's, the document's, iframes')
  as subtrees only after a graft names them, routes an action aimed at another tree to the embedder, tells the embedder when
  assistive technology attaches or detaches, and moves Slint's focus to the page node on a `Focus` request. Every call
  that gives foreign data to `accesskit_winit` is wrapped in `catch_unwind`; a panic switches page accessibility off for
  the window. A work-around keeps `accesskit_consumer` 0.38 from panicking when the window gains or loses focus while the
  focus is inside a page (`before_host_focus_change`). The new public API is one sealed trait,
  `MercuryPageAccessibility`, implemented for `slint::Window` (`mercury_graft.rs`); the order logic is in
  `mercury_a11y::graft` (PolyForm, tested without winit), behind the crate's `accessibility` feature. `accesskit.rs`
  has about 14 hook sites, `lib.rs` one, `winitwindowadapter.rs` three, each tagged `Mercury: a11y-1`. The idea of
  holding a subtree until its graft is in is that of servoshell's graft queue (`ports/servoshell/desktop/gui.rs`, v0.8.0,
  MPL-2.0); no line of it was used.
- **Why:** Slint's backend builds the window's tree itself and has no way to hang a foreign subtree from it (OD-3).
- **Rebase cost on a Slint upgrade:** the hook sites above; Slint is held at 1.17.
- **Tested by:** `mercury-a11y` graft and bridge tests (checked against `accesskit_consumer`), the `mercury-ui` page-node
  tests (compile the backend with the feature on). **The graft through the real adapter has not been run** (it opens a window).
- **Drop when:** Slint offers a foreign-subtree API (to be asked upstream), and `accesskit_consumer` fixes the host-focus panic.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/i-slint-backend-winit.patch` and this directory
once Mercury no longer needs to change the crate.
