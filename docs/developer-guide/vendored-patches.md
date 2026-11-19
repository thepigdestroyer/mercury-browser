---
title: Vendored Patches
description: Maintain and eventually remove Mercury's focused gaol, Surfman, servo-canvas and servo-default-resources compatibility patches and its Servo engine patches, and re-vendor them on a Servo upgrade.
---

# Vendored patches

Mercury vendors code when a pinned Servo dependency needs a small, reviewable compatibility fix that cannot be expressed through the public API. Vendoring is a temporary maintenance cost, not a general fork strategy.

## How vendored crates are carried

Every crate Mercury changes lives in `third_party/<crate>/` as **the crate exactly as published on crates.io plus one patch**. Three files record it:

- `third_party/vendor.toml` names each crate, its version, the SHA-256 of the published archive (the one `Cargo.lock` had before the crate was patched), and the upstream repository, commit and directory it was built from.
- `third_party/patches/<crate>.patch` is Mercury's whole change to the crate, as a `git diff`-style patch against the published archive. It is committed, so the complete engine delta can be reviewed in one place.
- `third_party/<crate>/MERCURY.md` explains each change: what it does, why, the upstream file and line, how it is tested and when it can be dropped.

The root `Cargo.toml` `[patch.crates-io]` section redirects the dependency graph to these directories.

The tool `mercury-servo-patches` (`crates/mercury-servo-patches`) maintains all of this. Run it from anywhere in the repository with `cargo run --release -p mercury-servo-patches -- <command>`.

| Command | What it does |
| --- | --- |
| `vendor <crate>@<version>` | Unpacks the published crate unchanged into `third_party/<crate>/` after checking its SHA-256 against `Cargo.lock`. Then records it in `vendor.toml`, adds the `[patch.crates-io]` line, writes a `MERCURY.md` skeleton and an empty patch, and updates `Cargo.lock`. |
| `diff <crate>` | Prints Mercury's changes to the crate as the directory is now, against the published archive. |
| `export` | Rewrites `third_party/patches/<crate>.patch` for every vendored crate. |
| `check` | Fails unless every vendored crate is the published crate plus its exported patch, byte for byte, with a `Mercury:` comment in every changed file, and is wired consistently in `vendor.toml`, `Cargo.toml` and `Cargo.lock`. |
| `upgrade <crate>@<version>` | Moves a crate to a new version and merges Mercury's changes into it (see below). |
| `upgrade --servo <version>` | Does the same for every vendored crate from Servo's repository. |

`make servo-patches-check` runs `check`. So do `make lint-scripts` and the "Vendored crates" CI job.

The published archives come from Cargo's download cache when it has them. Otherwise they are downloaded from crates.io into `target/mercury-servo-patches/crates/`; a patched crate is never in Cargo's cache, so this is usual on a fresh checkout. Every archive is checked against the recorded SHA-256 before use. Set `MERCURY_SERVO_PATCHES_OFFLINE=1` to forbid downloads.

### Changing a vendored crate

1. Reproduce the failure against the exact upstream version.
2. Edit the files in `third_party/<crate>/`, keeping the diff limited to what Mercury needs. Mark every change with a comment containing `Mercury:`, saying why. `check` fails if a changed or new file has no `Mercury:` in its changed lines. A generated file that cannot hold a comment, such as a `Cargo.lock`, goes in the crate's `unmarked` list in `vendor.toml`.
3. Run `mercury-servo-patches export` and commit the patch together with the change. `check` fails if the directory and the exported patch disagree.
4. Add a regression test when the behaviour can be tested locally, and describe the change in the crate's `MERCURY.md`.
5. Preserve the upstream licence files and attribution. The changed files stay under the crate's own licence.
6. Prefer a fix that could be proposed upstream without Mercury-specific concepts.

Avoid opportunistic formatting, dependency upgrades or unrelated cleanup inside a vendored crate: every byte ends up in the patch.

Vendored crates are never workspace members and never path dependencies of one, so `cargo fmt --all` and `cargo clippy --workspace` leave them alone. `check` fails if that changes, and the licence-header check skips the directories under `third_party/`. Some crates ship a `.gitignore` that hides their own files (surfman's hides its `Cargo.lock`). `check` fails on such files; add them with `git add -f`.

### Vendoring a new crate

```bash
cargo run --release -p mercury-servo-patches -- vendor servo-script@0.8.0
```

The crate must already be in `Cargo.lock`, taken from crates.io. Commit the unchanged crate on its own first, then make Mercury's changes in later commits.

### Upgrading

`upgrade` first refuses to run if a crate has edits that are not exported. It then takes the new published archive and, for each file Mercury changes, does a three-way merge of the old published file, Mercury's version and the new published file. It writes the merged crate in place and updates `vendor.toml`, then reports each file as:

- applied cleanly;
- already upstream: Mercury's change is no longer needed;
- **CONFLICT**: diff3 conflict markers written into the file;
- **GONE**: upstream removed a file Mercury changes.

It exits with status 1 if anything conflicts. Then:

1. For `--servo`, set the root `servo = "=<version>"` requirement.
2. Resolve each conflict (search for `<<<<<<<`) and decide what each GONE change becomes.
3. Run `export` and update each `MERCURY.md`.
4. Run `cargo update -p <crate>…` and `make servo-patches-check`, then the full gates.

`check` fails until the patch is exported again and no conflict marker is left. Git keeps the previous state if you need to start over.

### Removing a crate

When an equivalent fix reaches the Servo dependency graph Mercury uses:

1. Update the pinned dependency version.
2. In one commit, remove the `[patch.crates-io]` entry, the `vendor.toml` entry, `third_party/patches/<crate>.patch` and the directory.
3. Regenerate `Cargo.lock` intentionally.
4. Run the original regression test and full QA.

The removal commit should say which upstream release absorbed the patch.

## Patch inventory

| Directory | Upstream version | Why Mercury carries it |
| --- | --- | --- |
| `third_party/gaol` | 0.2.1 | In the Linux sandbox profile: system calls that current Rust, glibc, SpiderMonkey and Servo need, and no zero file-size limit, which killed content processes sizing their shared-memory IPC regions |
| `third_party/surfman` | 0.14.0 | Avoid process-global `DRI_PRIME=1` selection and its race with media initialization |
| `third_party/servo-canvas` | 0.8.0 | Make Servo's GPU 2D-canvas backend (`vello` feature) build against wgpu 30, fall back to the CPU backend when it cannot start, survive failed renders and read-backs, share one GPU device, and fix GPU drawing defects (lost drawing, clips, oversized canvases, shapes with shadows) |
| `third_party/servo-default-resources` | 0.8.0 | Stop it registering Servo's default resource reader, so that Mercury's own reader (its error and certificate-error pages, Servo's files for everything else) is the only one; see its `MERCURY.md` |
| `third_party/servo-script` | 0.8.0 | Home of Mercury's engine patches. Camera and microphone: ask before opening a device, expose device names only with permission, `MediaStreamTrack.stop()`, release devices when a document goes away, camera and microphone `'self'` in frames and refused to workers |
| `third_party/servo-script-bindings` | 0.8.0 | WebIDL for the above (`stop()`, `readyState`, `[SecureContext]` `mediaDevices`); WebRTC moved behind `dom_webrtc_transceiver_enabled` so capture can be on without it |
| `third_party/servo-media-streams` | 0.8.0 | `MediaStream::stop()`, so a device can be released |
| `third_party/servo-media-gstreamer` | 0.8.0 | Its GStreamer implementation, and a caps-field fix for fractional constraints |

The gaol and Surfman patches date from before the fork and Mercury continues to carry them ([Origins and credits](origins.md)). Mercury added the servo-canvas and servo-default-resources patches and the Servo engine patches (`servo-script` and the three crates vendored with it, see [The Servo engine patches](#the-servo-engine-patches-servo-script-and-friends)). The `gaol` and Surfman patches concern Linux. The `gaol` patch adjusts the Linux sandbox profile (its seccomp allow-list and file-size limit), which takes effect only when Servo's `sandbox` option is on; Mercury keeps it off on every platform (see [Process isolation](servo-integration.md#process-isolation)). The Surfman patch changes how Mesa's GPU is chosen.

## Validate the gaol patch

```bash
cargo test \
  --target-dir target \
  --manifest-path third_party/gaol/Cargo.toml \
  --test shared-memory
```

The separate target directory avoids leaving an untracked build tree inside the vendored crate.

Media playback uses Servo's upstream GStreamer player without a local patch (the changes to `servo-media-gstreamer` concern camera and microphone capture only). It still needs a manual playback check because unit tests cannot prove that the host's plugins, decoder selection, audio sink, frame transfer, and isolated content process work together.

## The servo-canvas patch

Servo 0.8.0's GPU 2D canvas (`vello` feature, preference `dom_canvas_backend = "vello"`) does not compile as published: in wgpu 30, which vello 0.11 requires, `BufferSlice::get_mapped_range()` returns a `Result`. The vendored copy handles the error in `VelloDrawTarget::map_read` the way that function already handles a failed `map_async`.

The vendored copy also makes the GPU backend robust:

- **CPU fallback.** When a canvas cannot get a GPU adapter or device, or vello's renderer cannot be created, that canvas is drawn by the CPU backend (the one `--disable-gpu-canvas` selects). One warning, `GPU 2D canvas unavailable (<reason>); drawing 2D canvases on the CPU`, is logged per process, and later canvases go straight to the CPU backend without trying the GPU again.
- **Failed renders** are logged once (`VELLO RENDER ERROR …`) instead of panicking Servo's Canvas thread, which would leave the session without any 2D canvas.
- **Failed read-backs** unmap the buffer, so the next read-back on that canvas can succeed.
- **Shared GPU resources.** All GPU canvases share one wgpu device and one vello renderer (whose buffers take about 175 MB); a canvas larger than 2048×2048 in area gets a renderer of its own, and one created or resized beyond what vello can draw (8192 pixels on a side, or more than 256 bins of 256×256 pixels) is drawn on the CPU. If the shared device is lost, which blanks the GPU canvases of every tab, canvases created or resized afterwards are drawn on the CPU (this path has only been exercised by simulation).
- **Correctness fixes** found with `tests/pages/canvas-parity.html`: earlier drawing is kept when a canvas is drawn again, non-rectangular and `evenodd` clips are applied (including through a full-canvas `clearRect`), and shapes drawn with a shadow appear (without the shadow, which neither backend implements; in both backends).

**Costs of the GPU canvas** (measured on an M-series Mac under load, Task 4d; figures in `third_party/servo-canvas/MERCURY.md`):

- The shared renderer holds about 175 MB of GPU memory from the first GPU canvas until the process exits. Fifty 200×100 canvases: about 0.45 GB process footprint against 0.28 GB on the CPU; two hundred: about 0.56 GB against 0.36 GB.
- **Displaying a GPU canvas is a read-back.** WebRender receives canvases as CPU pixels, so every frame that changes a GPU canvas is rendered, copied back from the GPU and un-premultiplied on the Canvas thread: the same work as `getImageData`, `toDataURL` or `drawImage` from a canvas. Up to about 3 megapixels an animated canvas runs as fast on the GPU as on the CPU (52.9 against 57.1 frames per second for a full 2360×1304 window; 51.2 against 46.1 at 800×600); above that the GPU is about 35–40 % slower (3200×1800: 28.5 against 44.1).

`third_party/servo-canvas/MERCURY.md` lists each change, why it is needed and when it can be dropped. `third_party/patches/servo-canvas.patch` holds the whole patch against the published crate.

Mercury draws 2D canvases on the GPU by default. `--disable-gpu-canvas` selects Servo's CPU backend instead. Check both modes with `tests/pages/canvas-readback.html`, whose read-back lines must all end in `PASS`:

```bash
./target/release/mercury-browser --screenshot gpu.png --window-size 1000x800 \
  "file://$PWD/tests/pages/canvas-readback.html"
./target/release/mercury-browser --disable-gpu-canvas --screenshot cpu.png --window-size 1000x800 \
  "file://$PWD/tests/pages/canvas-readback.html"
```

To confirm which backend drew, run with `RUST_LOG=vello=debug,wgpu_core=trace`. When the GPU canvas draws, a page that creates a 2D canvas logs vello's labelled GPU resources and pipelines, for example `Device::create_buffer("vello.scene")`, `vello.flatten` and `vello.fine_area`. With `--disable-gpu-canvas` no `vello.` label appears. After the CPU fallback warning, no further `vello.` resources are created (a failed renderer creation may log some pipeline labels just before the warning). A secondary signal is the device id format: vello's wgpu 30 logs `Created Device DeviceId(…)`, while Slint's wgpu 29 logs `Created Device Id(…)`.

`tests/pages/canvas-parity.html` draws one case per small canvas (paths, line styles, transforms, clips, compositing, gradients, patterns, shadows, text, `drawImage`, image data, drawing over time, serialisation, unusual sizes) and lists the pixels it reads back with the values the specification gives. It also puts the list on one line in the document title, which the window title log carries out of the browser:

```bash
RUST_LOG=warn,winit::window=debug ./target/release/mercury-browser --screenshot gpu.png \
  --window-size 1400x1600 --screenshot-delay 2500 "file://$PWD/tests/pages/canvas-parity.html" 2>&1 \
  | grep -o 'title="PARITY-BEGIN .* PARITY-END' | perl -pe 's/ ;; /\n/g'
```

Run it in both modes and compare line by line: `i` samples must be identical, `e` samples may differ by 2 per channel, `b` text boxes by 1 pixel. The differences known today are listed in `third_party/servo-canvas/MERCURY.md`.

`tests/pages/canvas-many.html` creates 50 canvases (`?n=200` for more), reads a pixel back from each and prints the timings; use it to compare the two backends' cost of creating canvases.

## The Servo engine patches (`servo-script` and friends)

`third_party/servo-script`, `servo-script-bindings`, `servo-media-streams` and `servo-media-gstreamer` were vendored together for camera and microphone capture. Like every crate in `third_party/vendor.toml` they are the published `.crate` plus `third_party/patches/<crate>.patch`, checked by `make servo-patches-check`. `servo-script` is also where later engine patches go. Each crate's `MERCURY.md` lists its changes; `third_party/servo-script/MERCURY.md` describes the capture patch as a whole and the rustc warnings the vendored crate shows with Mercury's feature set and how each is silenced at its own site.

Rules on top of [Changing a vendored crate](#changing-a-vendored-crate):

- **List every change** in that crate's `MERCURY.md` with the upstream file and line it changes. Mark each hunk with a `// Mercury:` comment where a comment fits (also in WebIDL); `make servo-patches-check` requires one in every changed file.
- **No formatting churn.** The vendored files follow Servo's rustfmt settings, which differ from Mercury's; `cargo fmt --all` does not touch `third_party/`. Format new code with `rustfmt --edition 2024 --config match_block_trailing_comma=true,binop_separator=Back,imports_granularity=Module,group_imports=StdExternalCrate,style_edition=2024 <file>` (these leave the upstream files unchanged).
- **Zero warnings.** Cargo does not cap lints for path dependencies, so warnings from the vendored crates appear in `cargo build`. Silence one only at its own site and record it in `MERCURY.md`; never with a crate-wide `[lints]` entry, which would hide warnings in Mercury's own changes.

### Re-vendor on a Servo upgrade

Run [`upgrade --servo`](#upgrading), which merges Mercury's changes into the new published crates, and follow its steps. Then, for these four crates:

1. Redo the warning table in `third_party/servo-script/MERCURY.md` from a clean build's output: new upstream warnings are silenced at their site, and silencing that is no longer needed is removed.
2. Update the versions, checksums, commits and line numbers in each `MERCURY.md`. Drop a change whose "drop when" condition the new release meets.
3. Run the capture tests (`make test-capture-macos`) and the full gate.
