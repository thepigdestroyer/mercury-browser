# Instructions for coding agents

These rules apply to every agent working in this repository.

## Languages

Mercury is a lightweight browser written in **Rust**, with its interface in **Slint**. Keep it that way.

- Product code is Rust and Slint only.
- No new Python, Node, Ruby or other scripting-language code anywhere in the repository: not in the browser, and not in build, test or CI tooling. Write tools in Rust (a binary in a workspace crate) or, when it is a few lines of glue, POSIX shell.
- JavaScript is allowed only where it must run inside a web page through Servo, such as the find-in-page and media helper scripts. Keep these scripts small and test them from Rust where possible.
- Code vendored unchanged from upstream (for example Servo's own Python bindings generator in `third_party/servo-script-bindings`) stays as upstream wrote it.
- The few existing Python helpers (`scripts/serve-tests.py`, `scripts/ci/check-colors-screenshot.py` and two inline `python3` snippets in `scripts/macos/`) are being replaced with Rust. Do not extend them.

## Licences

Mercury's own files are under the PolyForm Noncommercial License 1.0.0; files from Hermes Browser or adapted from other projects stay under the MPL-2.0 (see `LICENSE` and `CONTRIBUTING.md`).

- Every new file starts with `SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0` and `SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors`, as comments (`//` in Rust and Slint, `#` in shell, TOML, YAML and Makefiles, after any `#!` line). Files that cannot hold a comment (Markdown, JSON, images, HTML) go in the PolyForm list in `REUSE.toml`.
- Never change the licence of an existing file. A file marked `MPL-2.0` stays MPL-2.0, whoever edits it.
- A new file that copies or adapts code from another project takes `MPL-2.0`, names its source at the top and gets an entry in `THIRD-PARTY-NOTICES.md`. Do not paste Hermes or other MPL-2.0 code into a PolyForm file; put it in an MPL-2.0 file instead.
- When moving or splitting a file, the new files keep the original's licence.
- Call Mercury "source-available", never "open source". `scripts/ci/check-license-headers.sh` (run by `make lint-scripts` and CI) fails when a file has no licence.

## GitHub and the Mac

- Never create a GitHub repository. Code lives in `thepigdestroyer/mercury-browser`: `mercury`, `master` and one `lane/<name>` branch per lane of work in progress. Notes, snapshots and finished lane branches go to the private `thepigdestroyer/mercury-browser-archive`.
- Nothing stays on the Mac that is not on GitHub, and nothing stays on the Mac once it is on GitHub and no longer needed. A finished lane's scratch, worktree and local branch are deleted after it merges.

## Builds and disk space

Load the project skill `mercury-build-hygiene` before you build, test, run the browser or create files. The disk guard plugin in `.opencode/plugins/` enforces it: build output is deleted after every round, and work is safe only once it is pushed to GitHub.
