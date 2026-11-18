# Instructions for coding agents

These rules apply to every agent working in this repository.

## Languages

Mercury is a fast, lightweight browser. Its own code is **Rust**, its interface is **Slint**, and it reuses existing code in fast compiled languages rather than rewriting it.

- **Reuse first.** Before writing a feature, look for working code to reuse: a library, a Servo backport or pull request, or another browser's code under a compatible licence (see "Licences"). Do not rewrite a working library just to change its language.
- **Languages Mercury may ship:** Rust, Slint, and libraries in fast compiled languages, chiefly C and C++ (for example GStreamer, PDFium, Bergamot, SpiderMonkey). Link them as libraries, prebuilt or built through Cargo build scripts, and call them from Rust through a binding crate or FFI. Any other language needs to be known for speed and efficiency, and the owner's agreement.
- **Mercury's own new code** is Rust, with Slint for interface. C or C++ written by us only where a reused library needs a small shim, kept minimal.
- **Not allowed:** Swift or Objective-C source (call Apple frameworks from Rust through `objc2`), and new Python, Node, Ruby or other scripting-language code anywhere in the repository, including build, test and CI tooling. Write tools in Rust (a binary in a workspace crate) or, when it is a few lines of glue, POSIX shell.
- JavaScript is allowed only where it must run inside a web page through Servo, such as the find-in-page and media helper scripts. Keep these scripts small and test them from Rust where possible.
- Code vendored unchanged from upstream (for example Servo's own Python bindings generator in `third_party/servo-script-bindings`) stays as upstream wrote it.
- The few existing Python helpers (`scripts/serve-tests.py`, `scripts/ci/check-colors-screenshot.py` and two inline `python3` snippets in `scripts/macos/`) are being replaced with Rust. Do not extend them.

## Licences

Mercury's own files are under the PolyForm Noncommercial License 1.0.0; files from Hermes Browser or adapted from other projects stay under the MPL-2.0 (see `LICENSE` and `CONTRIBUTING.md`).

- Every new file starts with `SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0` and `SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors`, as comments (`//` in Rust and Slint, `#` in shell, TOML, YAML and Makefiles, after any `#!` line). Files that cannot hold a comment (Markdown, JSON, images, HTML) go in the PolyForm list in `REUSE.toml`.
- Never change the licence of an existing file. A file marked `MPL-2.0` stays MPL-2.0, whoever edits it.
- A new file that copies or adapts code from another project names its source at the top, gets an entry in `THIRD-PARTY-NOTICES.md`, and keeps the upstream licence: `MPL-2.0`, or a permissive one (`Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`) with the upstream copyright notice kept and the licence text in `LICENSES/`. Never port GPL or LGPL code; ideas only. Do not paste outside code into a PolyForm file; put it in its own file instead.
- Linked libraries (C, C++ or Rust) keep their own licences. Record each in `THIRD-PARTY-NOTICES.md` with its licence text in `LICENSES/`, and make sure the app bundle carries those texts. LGPL libraries are linked dynamically only; GPL libraries are never linked or bundled.
- When moving or splitting a file, the new files keep the original's licence.
- Call Mercury "source-available", never "open source". `scripts/ci/check-license-headers.sh` (run by `make lint-scripts` and CI) fails when a file has no licence.

## GitHub and the Mac

- Never create a GitHub repository. Code lives in `thepigdestroyer/mercury-browser`: `mercury`, `master` and one `lane/<name>` branch per lane of work in progress. Notes, snapshots and finished lane branches go to the private `thepigdestroyer/mercury-browser-archive`.
- Nothing stays on the Mac that is not on GitHub, and nothing stays on the Mac once it is on GitHub and no longer needed. A finished lane's scratch, worktree and local branch are deleted after it merges.

## Builds and disk space

Load the project skill `mercury-build-hygiene` before you build, test, run the browser or create files. The disk guard plugin in `.opencode/plugins/` enforces it: build output is deleted after every round, and work is safe only once it is pushed to GitHub.
