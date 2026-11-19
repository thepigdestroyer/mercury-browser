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

None yet. For each change, say what it does, why Mercury needs it, the upstream
file and line it changes, how it is tested and when it can be dropped.

## Removing this directory

Remove the `[patch.crates-io]` entry in the root `Cargo.toml`, the entry in
`third_party/vendor.toml`, `third_party/patches/i-slint-backend-winit.patch` and this directory
once Mercury no longer needs to change the crate.
