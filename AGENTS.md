# Instructions for coding agents

These rules apply to every agent working in this repository.

## Languages

Mercury is a lightweight browser written in **Rust**, with its interface in **Slint**. Keep it that way.

- Product code is Rust and Slint only.
- No new Python, Node, Ruby or other scripting-language code anywhere in the repository: not in the browser, and not in build, test or CI tooling. Write tools in Rust (a binary in a workspace crate) or, when it is a few lines of glue, POSIX shell.
- JavaScript is allowed only where it must run inside a web page through Servo, such as the find-in-page and media helper scripts. Keep these scripts small and test them from Rust where possible.
- Code vendored unchanged from upstream (for example Servo's own Python bindings generator in `third_party/servo-script-bindings`) stays as upstream wrote it.
- The development tools are Rust, in `crates/mercury-devtools`: the test page server (`scripts/serve-tests.sh`) and the screenshot checker. `scripts/serve-tests.py` is kept only until the lanes in progress have moved to `scripts/serve-tests.sh`, then deleted. The inline `python3` in `scripts/ci/light-crates.sh` and `scripts/ci/github-env.sh` is still to be replaced. Do not extend any of them.

## Builds and disk space

Load the project skill `mercury-build-hygiene` before you build, test, run the browser or create files. The disk guard plugin in `.opencode/plugins/` enforces it: build output is deleted after every round, and work is safe only once it is pushed to GitHub.
