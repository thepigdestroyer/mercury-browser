# Contributing to Mercury Browser

Bug reports, reduced test cases, tests, documentation fixes and focused pull requests are welcome. The
developer guide in [`docs/developer-guide/`](docs/developer-guide/) explains how to set up, build and
test; run `make qa` before you send a change.

## Code rules

- Product code is Rust and Slint only; see [`AGENTS.md`](AGENTS.md) for the full rules, which apply to
  people as well as coding agents.
- Every new file starts with a licence header:

  ```rust
  // SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
  // SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
  ```

  Use `#` instead of `//` in shell scripts, TOML, YAML and Makefiles (after the `#!` line, if there is
  one). Files that cannot hold a comment, such as Markdown with front matter, JSON or images, are
  listed in [`REUSE.toml`](REUSE.toml) instead.
- A file that already says `SPDX-License-Identifier: MPL-2.0` stays under the MPL-2.0, and so do your
  changes to it.
- A new file that copies or adapts code from another project takes `MPL-2.0` (or the upstream
  licence, if MPL-2.0 cannot cover it), names its source at the top, and gets an entry in
  [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).
- `make lint-scripts` checks that every file has a header or a `REUSE.toml` entry.

## Contributor terms

Mercury Browser is source-available: free for non-commercial use, with commercial licences
available from Mrinal Joshi (see [`LICENSE`](LICENSE)). For that to keep working once other people
contribute, every contribution comes with the following terms. By opening a pull request, or by
adding a `Signed-off-by:` line to your commits, you agree to them for the contributions you send.

1. **Your licence to everyone.** Your contribution is licensed under the licence of the file it goes
   into: the PolyForm Noncommercial License 1.0.0 or the Mozilla Public License 2.0.
2. **Your licence to the project owner.** You also grant Mrinal Joshi, and anyone he licenses
   Mercury Browser to, a perpetual, worldwide, non-exclusive, no-charge, royalty-free and
   irrevocable licence to use, copy, modify, distribute, sublicense and relicense your contribution,
   under any terms, including commercial licences. This includes a patent licence, on the same terms,
   for any patent claims you can license that your contribution would otherwise infringe.
3. **You keep your copyright.** These terms do not transfer ownership; you may use your own
   contribution however you like.
4. **You have the right to contribute it.** The contribution is your own work, or you have the right
   to submit it under these terms. If your employer has rights in it, you have their permission. If
   it contains anyone else's work, you say so in the pull request and name its source and licence.
5. **No warranty.** You provide your contribution as is.

If you can't agree to these terms, open an issue instead and describe the change; someone else can
write it.
