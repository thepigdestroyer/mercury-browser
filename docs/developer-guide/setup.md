---
title: Development Setup
description: Prepare a Mercury Browser development environment on macOS (Apple Silicon or Intel), build and run it, and build this documentation.
---

# Development setup

Mercury is developed on macOS, on Apple silicon and on Intel Macs, and that is where its full path (Metal rendering, the bundled GStreamer, the app bundle, the menu bar) is exercised. Linux builds too; see [Linux](#linux) at the end.

Both kinds of Mac build, lint, test and run the browser from source in the same way. The app bundle and the disk image are Apple silicon only: `make app-macos` and `make package-macos` build an arm64 app; on an Intel Mac they compile the release build and then stop at `scripts/macos/bundle.sh`, which refuses an executable that isn't arm64. Use `make run` there.

## What you need

- A Mac with Apple silicon or an Intel processor, and macOS 13 or later.
- The Xcode command-line tools (`xcode-select --install`). Full Xcode is not needed.
- [rustup](https://rustup.rs/). The Rust version is pinned in `rust-toolchain.toml` and installs itself on the first build.
- Python 3.11 or later, or `uv`, on `PATH` (on Apple silicon, Homebrew's `/opt/homebrew/bin` is enough; without Homebrew, as on an Intel Mac that has none, install `uv` with [its standalone installer](https://docs.astral.sh/uv/getting-started/installation/) and check that `uv --version` works in a new shell). Servo's own build scripts need it: they try `uv run python` first, then `python3`, then `python`. Mercury's own tools are Rust or POSIX shell, not Python.
- Free disk space: a cold build compiles all of Servo and needs 15–20 GB while it runs, plus about 5 GB for the unpacked GStreamer.

## Clone and prepare

```bash
git clone https://github.com/thepigdestroyer/mercury-browser.git
cd mercury-browser
git checkout mercury
make setup     # on macOS: fetches and unpacks GStreamer, checks the tools, adds rustfmt and Clippy, fetches the crates
```

`make setup` runs `make gstreamer-macos` and `make doctor` first, then installs the `rustfmt` and `clippy` components and fetches the locked dependencies with `--locked`. `make doctor` alone reports the Rust toolchain and checks the GStreamer copy. It does not look for `uv` or Python, so check `uv --version` (or `python3 --version`) yourself: with neither, the first engine build stops in a Servo build script with "No suitable python found!".

`make lint-scripts` and `make deny` need shellcheck, actionlint and cargo-deny. `make ci-tools` downloads the pinned versions into `target/ci-tools/bin` (`CI_TOOLS_DIR` changes the place), checks each download's SHA-256 and installs nothing system-wide. The pins cover Linux x86-64 and macOS on both Apple silicon and Intel.

## Build and run

```bash
make run              # build the optimised browser and start it
make app-macos        # build dist/Mercury Browser.app (signed ad hoc)
make build-release    # build the whole workspace in release mode
```

Pass arguments to the browser through `ARGS`, and change logging through `RUST_LOG`:

```bash
make run ARGS="https://servo.org/" RUST_LOG="talaria_servo=debug,servo=info"
```

`make run-release` is an alias for `make run`. `make run-debug` builds the development profile, which keeps debug information for Mercury's code; use it for debugging, not for judging speed or media.

The ad and tracker filter lists are data files, not compiled into the program. `make run`, `make run-debug`, `make build` and `make build-release` copy them to `target/<profile>/filter-lists` with the `mercury-filter-lists` tool, checking each file's SHA-256 against the manifest. A binary started without that step logs that it found no `filter-lists` folder.

Build, lint and test in release mode, so that one set of compiled Servo artefacts serves all three: pass `CARGO_FLAGS=--release` to `make check`, `make lint` and `make test`, and use `--release` for any `cargo` command you run by hand. Crates that don't depend on Servo (`talaria-core`, `mercury-ui`, `mercury-settings`, `mercury-store`, `mercury-omnibox`, `mercury-passwords`, `mercury-password-manager`, `mercury-downloads`, `mercury-import`, `mercury-filter-lists`, `mercury-site-security`, `mercury-tab-snapshots`, `mercury-option-keys`, `mercury-proxy`, `mercury-devtools`, `mercury-release` and `mercury-servo-patches`) build and test in minutes on their own:

```bash
cargo test --release -p talaria-core -p mercury-ui
```

`make qa-fast` checks formatting, then lints and tests only the crates that need neither Servo nor Slint; `scripts/ci/light-crates.sh --explain` says which those are and why.

`make test` runs the whole workspace, and on a busy Mac a few of the desktop app's largest tests (the 10,000-entry import tests) can time out waiting for their service thread although nothing is wrong. They pass when run alone. When several builds share the machine, limit the test threads:

```bash
RUST_TEST_THREADS=2 make test CARGO_FLAGS=--release
```

CI runs the macOS checks on Apple silicon only, so an Intel Mac is checked by hand.

### Build slots

Servo builds are heavy, so the AI coding agents' builds on the development Mac, an Intel Core i7 with 12 threads and 32 GB, run at most six at once. `scripts/dev/build-slot.sh` holds the slots; nothing in the Makefile calls it, so a build you start yourself isn't slotted. A build that goes through it and compiles Servo waits until one of the **six heavy slots** is free and at least 40 GB of disk is free, then runs with `CARGO_BUILD_JOBS` set to the Mac's cores divided by the number of slots, never fewer than two: **two jobs each** here. The cores are shared out by slot rather than by the builds running now, so a build that starts alone doesn't keep every core for its whole run. Builds that name only crates without Servo (`cargo test --release -p talaria-core …`) use a separate pool of four light slots and need 10 GB free, so they never queue behind a Servo build. A build that has waited six hours gives up with exit status 75.

The OpenCode disk guard in `.opencode/plugins/` sends the AI coding agents' build commands (for example `cargo build`, `test`, `clippy`, `check`, `run`, `doc`; `make lint`, `test`, `build-release`, `app-macos`, `run`) through the script, and deletes build output after every round, as `.opencode/skills/mercury-build-hygiene/SKILL.md` describes. The limits are `MERCURY_BUILD_SLOTS` (6), `MERCURY_MIN_FREE_GB` (40), `MERCURY_LIGHT_SLOTS` (4), `MERCURY_LIGHT_MIN_FREE_GB` (10) and `MERCURY_SLOT_WAIT_MINUTES` (360), read from the environment with these defaults; change them if your Mac differs.

## Command-line options

`mercury-browser --help` lists them. The ones used most in development:

| Option | What it does |
| --- | --- |
| `[URL…]` | Opens each URL in its own tab |
| `--screenshot <file.png>` | Saves a PNG of the window once the page has loaded, then exits. Never takes focus |
| `--screenshot-delay <ms>` | Waits this long after the page has loaded (default 1000) |
| `--window-size <W>x<H>` | Sets the window's logical size, for example `1280x800` |
| `--background` | Starts without taking focus, for automated runs |
| `--profile-dir <directory>` | Uses this profile. Screenshot and background runs otherwise use a temporary profile that is removed at exit |
| `--disable-gpu-canvas` | Draws 2D canvases on the CPU |
| `--disable-experimental-web-features` | Leaves Servo's experimental web-platform features off |
| `--exit-after <ms>`, `--exit-by …` | Diagnostics: quit from inside the browser, as the close button, the quit shortcut or the Quit menu item would ([Testing](testing.md#quit-paths)) |
| `--download <URL>`, `--download-dir <directory>`, `--download-offer-answer <answer>` | Diagnostics: download an address once the browser is up, save into this directory, and answer every "wants to download" bar the same way (`download` or `block`) |
| `--filter-list-server <http://127.0.0.1:port/path/>` | Diagnostics: check for filter-list updates once at start-up from this loopback server only, also in screenshot and background runs, which otherwise never update |

Exit statuses: 0 success, 1 failure (or a screenshot that couldn't be saved), 2 a wrong command line, 3 the screenshot's page didn't finish loading, 4 the profile is in use by another Mercury. On `SIGTERM`, `SIGINT` or `SIGHUP` the browser quits in order and then ends by that signal.

A `--screenshot` or `--background` run never touches your own profile. A normal launch uses `~/Library/Application Support/Mercury Browser/Default`; a directory is accepted as a profile only if it is new, empty, or contains the marker file `.mercury-browser-profile`.

## macOS media set-up (GStreamer)

On macOS, Servo's media backend uses the official GStreamer framework packages from
gstreamer.freedesktop.org. Mercury does not install them system-wide and needs no `sudo`:
`scripts/macos/gstreamer.sh` downloads the runtime and development packages, checks their
SHA-256 (a download that fails the check is deleted), and unpacks them into
`~/Library/Caches/mercury-browser/gstreamer/<version>/` (override with `MERCURY_GSTREAMER_CACHE`).
The pinned version and checksums are at the top of that script. The packages are the universal ones, so the same step serves Apple silicon and Intel Macs, and the collector below copies only the executable's own architecture.

```bash
make gstreamer-macos      # fetch, unpack, check
make doctor               # also checks the GStreamer prefix on macOS
```

Disk space (measured): the two packages take 0.9 GiB, the unpacked framework 4.0 GiB, so the
cache holds **4.9 GiB** afterwards; while `unpack` runs it needs up to **9.7 GiB**, because the
expanded development package and the assembled framework exist at the same time. Downloading
took about a minute here and unpacking between 1 and 11 minutes, depending on disk load.

The Makefile's build, check, lint, test and run targets load the build environment themselves
and stop if it cannot be set up (so they never build against another GStreamer that
`pkg-config` happens to find). For a plain shell, use:

```bash
gst_env="$(scripts/macos/gstreamer.sh env)" && eval "$gst_env"   # MERCURY_GSTREAMER_PREFIX, PKG_CONFIG, PKG_CONFIG_LIBDIR, PATH
cargo build --release -p mercury-desktop
scripts/macos/collect-gstreamer.sh "$MERCURY_GSTREAMER_PREFIX" target/release/lib
```

Every cargo command that compiles Servo needs that environment, including `cargo check` from an
editor.

Notes:

- The executable links GStreamer directly and finds it through the rpath
  `@executable_path/lib/`, so `target/<profile>/lib` must exist before it starts; without it the
  program stops in the dynamic loader. `collect-gstreamer.sh` fills it: the libraries the
  executable links, the plugins `scripts/macos/gstreamer-plugins.sh` names (Servo's list from the
  resolved `servo` crate, minus the excluded plugins), and their dependencies, thinned to the
  executable's architecture. A marker records what was collected; an intact, up-to-date directory
  is not copied again, a damaged one is rebuilt, and a directory without a marker is never
  replaced. `make run`, `make run-debug`, `make build`, `make build-release` and `make test` call it.
- **Ogg is left out** (`DEFAULT_EXCLUDED` in `scripts/macos/gstreamer-plugins.sh`; see the user
  guide). `MERCURY_GSTREAMER_EXCLUDE_PLUGINS` overrides the list for one run (`""` = none). If the
  set of collected files changes, the collector stops until the library table in
  `THIRD-PARTY-NOTICES.md` is regenerated: rerun it with `MERCURY_GSTREAMER_UPDATE_NOTICES=1` and
  commit the table.
- The first launch after a fresh copy is slow (2 to 10 seconds before media is ready) while macOS
  checks each new library; later launches load the plugins in about 0.3 seconds. Users see the
  same delay on their first launch after installing or updating Mercury. Several worktrees can
  share one collected directory through a symlink (`ln -s <shared lib> target/release/lib`);
  it stays warm across them.
- Test binaries have no rpath to the libraries. `make test` builds the executable, collects
  `target/<profile>/lib` and runs the tests with `DYLD_FALLBACK_LIBRARY_PATH` pointing at that
  directory, on the `cargo test` command line itself: macOS removes `DYLD_*` variables when it
  starts SIP-protected programs such as `make` and `/bin/sh`, so setting it in a parent shell
  script or in `make` does not work. Use `make test CARGO_FLAGS=--release` (likewise `check` and
  `lint`) for release-mode runs.
- **Where GStreamer and GLib write.** At the top of `main()`,
  `apps/mercury-desktop/src/media_isolation.rs` points GStreamer's registry and plugin paths at
  the application's own files, and `apps/mercury-desktop/src/media_buffers.rs` creates a private,
  locked run directory under `<dirs::cache_dir()>/io.github.thepigdestroyer.mercury-browser/media-buffers/`
  and points `XDG_CACHE_HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_RUNTIME_DIR` into it,
  so GStreamer's download buffers (copies of played media) land there. It is removed at a normal
  quit, and leftovers of killed runs at the next start. `scripts/macos/test-media.sh` plays the
  media page with a scratch `HOME` and fails if anything appears outside that directory.
- **Expected start-up warning while Ogg is excluded.** Every launch logs
  `WARN servo::servo::media_platform] Error initializing GStreamer: ErrorLoadingPlugins(["libgstogg.dylib"]). Trying fallback initialization.`
  Servo loads its plugins by name and treats the missing Ogg plugin as an error; it then starts
  the backend through its fallback, with the other plugins it had already loaded. Media works.
  A real failure looks different: more than `libgstogg.dylib` in that list (a damaged or
  incomplete `lib`), `dyld: Library not loaded: @rpath/libgst…` before anything else (no `lib`
  at all), or `canPlayType('video/mp4; codecs="avc1.42E01E"')` answering `''` in a page.
- `make test-media-macos` runs `scripts/macos/test-media.sh`, the tests for these scripts.
- `make test-capture-macos` checks camera and microphone capture with fake devices only, from a copy of the release build without the real capture plugins; see [Camera and microphone](media-capture.md).
- `make test-certificate-exceptions-macos` checks that a certificate exception holds for the name it was made for and not for the same certificate under another name (loopback only, needs `make build-release`); see [Testing](testing.md#certificate-exceptions).
- `make test-servo-net` runs the tests of the vendored `servo-net` (the certificate-exception rules and the HSTS guard among them) on the crate's own manifest; `make test` includes it. See [Testing](testing.md#certificate-exceptions).
- `tests/pages/media.html` checks playback without making a sound; serve it with
  `scripts/serve-tests.sh` (a loopback server that honours `Range`); see `tests/media/README.md`.

## Build the documentation

This site is built with MkDocs and the Material theme, an existing external tool. Install it in a virtual environment so it stays out of the system's Python:

```bash
python3 -m venv .venv-docs
source .venv-docs/bin/activate
python -m pip install -r docs/requirements.txt
```

Then:

```bash
make docs-serve   # preview with live reload
make docs         # build into target/mkdocs with strict checks
```

`make docs` runs MkDocs in strict mode: a broken link, a page missing from the navigation or a configuration warning fails the build. `docs/superpowers/` holds design notes and plans and is excluded from the site.

The published site is <https://thepigdestroyer.github.io/mercury-browser/>. `.github/workflows/pages.yml` builds it with the same strict `make docs` and deploys it whenever a push to `master` changes `docs/` (the repository's Pages source has to be set to GitHub Actions). **Mercury Browser Help** in the Help menu opens it.

Screenshots in `docs/screenshots/` are taken from a release build with `--screenshot`, for example:

```bash
"dist/Mercury Browser.app/Contents/MacOS/mercury-browser" --screenshot docs/screenshots/mercury-servo.png \
    --window-size 1280x800 --screenshot-delay 3000 https://servo.org/
```

## Linux

Linux builds from source and CI tests it, but it gets less attention than macOS. On Debian or Ubuntu:

```bash
make install-linux-deps   # the packages CI uses (needs sudo)
make setup
make run
```

On Linux, Mercury draws Servo's frames through Vulkan, so Slint's Vulkan WGPU backend is needed, and media uses the system's GStreamer. Linux-only code (`#[cfg(target_os = "linux")]`) can't be compiled on a Mac; CI's Linux job checks it. The merge trains' own checks ran on macOS, so the recent camera and microphone, saved logins, clipboard and tab-strip work has not been verified on Linux beyond what CI's Linux job reports.

Windows is not built at the moment.
