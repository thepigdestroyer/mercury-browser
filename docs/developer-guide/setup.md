---
title: Development Setup
description: Prepare a Mercury Browser development environment on macOS (Apple Silicon), build and run it, and build this documentation.
---

# Development setup

Mercury is developed on macOS with Apple silicon, and that is where its full path (Metal rendering, the bundled GStreamer, the app bundle, the menu bar) is exercised. Linux builds too; see [Linux](#linux) at the end.

## What you need

- A Mac with Apple silicon and macOS 13 or later.
- The Xcode command-line tools (`xcode-select --install`). Full Xcode is not needed.
- [rustup](https://rustup.rs/). The Rust version is pinned in `rust-toolchain.toml` and installs itself on the first build.
- Python 3.11 or later, or `uv`, on `PATH` (Homebrew's `/opt/homebrew/bin` is enough). Servo's own build scripts need it; Mercury has no Python code of its own.
- Free disk space: a cold build compiles all of Servo and needs 15–20 GB while it runs, plus about 5 GB for the unpacked GStreamer.

## Clone and prepare

```bash
git clone https://github.com/thepigdestroyer/mercury-browser.git
cd mercury-browser
git checkout mercury
make setup     # on macOS: fetches and unpacks GStreamer, checks the tools, adds rustfmt and Clippy, fetches the crates
```

`make setup` runs `make gstreamer-macos` and `make doctor` first, then installs the `rustfmt` and `clippy` components and fetches the locked dependencies with `--locked`. `make doctor` alone reports the Rust toolchain and checks the GStreamer copy.

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

Build, lint and test in release mode, so that one set of compiled Servo artefacts serves all three: pass `CARGO_FLAGS=--release` to `make check`, `make lint` and `make test`, and use `--release` for any `cargo` command you run by hand. Crates that don't depend on Servo (`talaria-core`, `mercury-ui`, `mercury-settings`, `mercury-store`, `mercury-passwords`, `mercury-downloads`, `mercury-import`) build and test in minutes on their own:

```bash
cargo test --release -p talaria-core -p mercury-ui
```

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

Exit statuses: 0 success, 1 failure (or a screenshot that couldn't be saved), 2 a wrong command line, 3 the screenshot's page didn't finish loading, 4 the profile is in use by another Mercury. On `SIGTERM`, `SIGINT` or `SIGHUP` the browser quits in order and then ends by that signal.

A `--screenshot` or `--background` run never touches your own profile. A normal launch uses `~/Library/Application Support/Mercury Browser/Default`; a directory is accepted as a profile only if it is new, empty, or contains the marker file `.mercury-browser-profile`.

## macOS media set-up (GStreamer)

On macOS, Servo's media backend uses the official GStreamer framework packages from
gstreamer.freedesktop.org. Mercury does not install them system-wide and needs no `sudo`:
`scripts/macos/gstreamer.sh` downloads the runtime and development packages, checks their
SHA-256 (a download that fails the check is deleted), and unpacks them into
`~/Library/Caches/mercury-browser/gstreamer/<version>/` (override with `MERCURY_GSTREAMER_CACHE`).
The pinned version and checksums are at the top of that script.

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
- `tests/pages/media.html` checks playback without making a sound; serve it with
  `scripts/serve-tests.py` (a loopback server that honours `Range`); see `tests/media/README.md`.

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

On Linux, Mercury draws Servo's frames through Vulkan, so Slint's Vulkan WGPU backend is needed, and media uses the system's GStreamer. Linux-only code (`#[cfg(target_os = "linux")]`) can't be compiled on a Mac; CI's Linux job checks it.

Windows is not built at the moment.
