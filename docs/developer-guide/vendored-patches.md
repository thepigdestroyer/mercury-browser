---
title: Vendored Patches
description: Maintain and eventually remove Mercury's focused gaol, Surfman, servo-canvas and servo-default-resources compatibility patches and its Servo engine patches (including the servo-net certificate-exception patch), and re-vendor them on a Servo upgrade.
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
| `third_party/servo-script` | 0.8.0 | Home of Mercury's engine patches. Camera and microphone: ask before opening a device, expose device names only with permission, `MediaStreamTrack.stop()`, release devices when a document goes away, camera and microphone `'self'` in frames and refused to workers. K0: editing and WebGL2 backports from post-0.8 Servo, notifications without an icon, SpiderMonkey 153.7.0. txt-1: the document's highlight set, IME composition in text controls and in contenteditable (EB11), the text input state message, servo/servo#49613 and #49885 |
| `third_party/servo-script-bindings` | 0.8.0 | WebIDL for the above (`stop()`, `readyState`, `[SecureContext]` `mediaDevices`); WebRTC moved behind `dom_webrtc_transceiver_enabled` so capture can be on without it. K0: the `DomSlice` rename (servo/servo#49649), SpiderMonkey 153.7.0 |
| `third_party/servo-media-streams` | 0.8.0 | `MediaStream::stop()`, so a device can be released |
| `third_party/servo-media-gstreamer` | 0.8.0 | Its GStreamer implementation, and a caps-field fix for fractional constraints |
| `third_party/servo` | 0.8.0 | K0: delegate hooks and `Servo::send_mercury_message` for Mercury's lane messages (`mercury/mod.rs`), see [Messages from the engine to a lane](servo-integration.md#messages-from-the-engine-to-a-lane) ; txt-1: the IME tests (`tests/input_methods.rs`) |
| `third_party/servo-config` | 0.8.0 | K0: twenty preferences for the first two waves in one block, and `stylo_prefs`, a name list that turns Stylo preferences on without a Servo edit (`mercury_stylo.rs`) |
| `third_party/stylo_static_prefs` | 0.23.0 | K0: `set_bool` and `get_bool`, generated by the build script (`mercury_bridge.rs`), so a Stylo boolean preference can be set by name at run time |
| `third_party/servo-embedder-traits`, `servo-constellation-traits`, `servo-constellation` | 0.8.0 | K0: one `Mercury` message variant per channel, with one file per lane in `mercury/`, and the constellation's router. txt-1: the IME events (`Commit` from servo/servo#49400, `SetMarkedText`, `InsertText`) and the text lane's `TextMsg::InputState` in `servo-embedder-traits` |
| `third_party/servo-net` | 0.8.0 | Accept a certificate exception only for the host and the certificate that failed there (Servo accepts a bare certificate for every host), refuse it for HSTS hosts, and refuse Servo's own `chrome:allowcert` entry point; new code in `certificate_exceptions.rs`, see [The servo-net certificate-exception patch](#the-servo-net-certificate-exception-patch). K0: HTTP/2 header list of 64 KiB (YouTube, servo/servo#49981), cookie and credential values out of the logs, a guard against deeply nested SVG images (more than 500 levels, counted on the parsed document, nested `data:` SVGs included, parsed on a large stack) |
| `third_party/servo-webgl` | 0.8.0 | K0: zero-initialised `texStorage2D` and `texStorage3D` storage (servo/servo#49436) |
| `third_party/servo-layout`, `servo-layout-api` | 0.8.0 | K0: the caret in contenteditable elements (servo/servo#48253). txt-1: typed highlights for text runs (EB7), the counter pass and counter styles (ED5: ordered lists show their numbers), `text-overflow: ellipsis` hand-ported from servo/servo#42394 (ED7), selection on replaced elements (servo/servo#49613) and UA widgets not selected (servo/servo#49885) |
| `third_party/servo-script-webcrypto` | 0.8.0 | Vendored unchanged except `mozjs =0.27.0` (SpiderMonkey 153.7.0): `servo-script`, `servo-script-bindings` and this crate pin `mozjs` exactly, so all three had to be vendored to move it |

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

### Backports from post-0.8 Servo (K0)

Servo 0.8.0 is a tagged release; fixes and features that landed on Servo's `main` afterwards are carried as patches until the next Servo bump (B1). A backport is **one upstream pull request, one commit**, with the subject `fix(script): backport servo#<n>, <upstream title>` and a body that names the upstream merge commit and says "drop at the bump that contains it". Rules:

- **Apply the diff, do not retype it.** Map the upstream path `components/<x>` to `third_party/<crate>` with `third_party/vendor.toml` and `git apply` it. A pull request that does not apply because its context belongs to a pull request Mercury does not take is hand-ported only when the port is small and only in whole chains (never part of a chain); otherwise it is dropped and written into `MERCURY.md` as "arrives with B1".
- **One `Mercury: backport of servo/servo#<n> (<merge commit>)` comment in every changed file**, so `make servo-patches-check` passes and the upgrade can tell the hunk from Mercury's own.
- **Read the added lines against 0.8** for API drift (names, signatures, `CanGc` and `JSContext` forms) before building.
- **Say in `MERCURY.md` which upstream test covers it and which Mercury page does.** Upstream's `tests/wpt` is not vendored; the pages under `tests/pages/` (`compat-editing.html`, `webgl2-textures.html`, `svg-deep-nesting.html`, ...) are what exercise the backports in a real run.
- **New code goes in new files** (`mercury_log.rs`, `mercury_svg.rs`, `mercury_stylo.rs`, `mercury/`), and upstream files get hook lines only, so an upgrade has a few lines to re-apply instead of a merge to resolve.

K0 carries: the editing chain (#49541, #47459, #49649, #48253, #49797, #49818, and the hand-ported #49884, #49917, #49920), the WebGL2 fixes (#49632, #49912, #49436), the HTTP/2 header list (#49981) and SpiderMonkey 153.7.0 (#49907, as the `mozjs =0.27.0` pin). Taken in place of an upstream change: a Mercury guard for deeply nested SVG images instead of #49835, which needs resvg 0.50 (`mercury_svg.rs`; see `third_party/servo-net/MERCURY.md`). Not taken: #49404 and #49902 (they change `servo-canvas-traits`, which is not vendored). Each crate's `MERCURY.md` has the details and the drop condition.

### The SpiderMonkey pin

`servo-script`, `servo-script-bindings` and `servo-script-webcrypto` pin `mozjs` with `=`, so a SpiderMonkey update changes the three `Cargo.toml` files together, with a `# Mercury:` comment in each. `servo-script-webcrypto` is vendored for that reason. After the change, `cargo update -p mozjs` and check that `Cargo.lock` moved only `mozjs`, `mozjs_sys` and the `mozjs_*` glue crates: a plain `cargo update -p` can also re-point unrelated dependency edges, and the lock should then be edited to the intended packages only (`cargo metadata --locked` must accept it unchanged). Run `make deny`. SpiderMonkey's cold build adds to the first Servo build after the change.

### Testing a vendored crate

A vendored crate is not a workspace member, so `cargo test --workspace` and `make test` never run its own tests. What runs, and how:

- **No dev-dependencies** (`servo-config`, `stylo_static_prefs`, `servo-embedder-traits`, `servo-constellation-traits`, `servo-constellation`, `servo-script`, `servo-webgl`): `cargo test --release -p <crate> --lib`. `make vendored-tests` runs the first four in the light build pool, and CI runs it. Add `--lib`: some crates ship doc comments that are examples for the whole of Servo and do not compile as doc tests on their own. `servo-script` and `servo-webgl` build the engine; run them by hand, with `RUST_TEST_THREADS=2` and `eval "$(scripts/macos/gstreamer.sh env)"` first.
- **With dev-dependencies** (`servo`, `servo-net`, `servo-layout`): cargo refuses `cargo test -p servo-net` because the crate "requires dev-dependencies and is not a member of the workspace". Either use the gaol pattern, `cargo test --release --target-dir target --manifest-path third_party/servo-net/Cargo.toml --lib` (the crate then resolves with its own `Cargo.lock`, builds from scratch and is not checked against Mercury's lock), or, for a new file of Mercury's that depends on little (`servo-net`'s `mercury_log.rs` and `mercury_svg.rs`, which need only `http`, `log`, `resvg` and, for the tests, `base64` and `flate2`), include it in a scratch crate: a `Cargo.toml` with `[workspace]` and those dependencies, and a `lib.rs` of `#[path = "<worktree>/third_party/servo-net/mercury_svg.rs"] mod mercury_svg;` lines, then `cargo test --release --offline`. The 30 tests of those two files (10 and 20) were run that way in K0, and their guard tests read the real `cookie_storage.rs`, `cookie.rs` and `http_loader.rs` with `include_str!`. Since the certificate-exception patch, the root `Cargo.toml` lists `servo-net` under `exclude` (as it does `gaol`), so the manifest-path form works for it and `make test-servo-net` runs its whole `main` test target ([Testing](testing.md#certificate-exceptions)); the crate's library target has `test = false`, so `--lib` runs nothing there: the `main` target is what runs, and the unit tests inside `mercury_log.rs` and `mercury_svg.rs` still need the scratch crate.
- **What cannot be run from here** at all: the two `deliver` tests in `servo/mercury/mod.rs`. Mercury's own tests of the same path are in `talaria-servo`.

### The servo-net certificate-exception patch

`third_party/servo-net` binds a certificate exception to a host and a certificate. Servo 0.8 keeps a list of bare certificates, and its verifier accepts a certificate in that list for any server name, so one exception made for one site covered every site. [`third_party/servo-net/MERCURY.md`](https://github.com/thepigdestroyer/mercury-browser/blob/mercury/third_party/servo-net/MERCURY.md) lists each change with the upstream file and line, how it is tested and when it can be dropped; this is the short version.

- **The new file is `third_party/servo-net/certificate_exceptions.rs`** (module `net::certificate_exceptions`). It holds the store of accepted pairs and the bounded record of the certificate each host last failed with (`CertificateExceptionStore`), the host-to-name function (`server_name_for_host`), the refusals (`CertificateExceptionRefusal`, each with a stable `code()`), `CertificateErrorOverrideManager::add_override(host, certificate)`, and `HttpState::accept_failed_certificate`, `revoke_certificate_exception` and `certificate_exceptions`. Following the new-file rule, upstream files get hook lines only.
- **The hooks** are in `connector.rs`: the override manager keeps a `CertificateExceptionStore` instead of a list of bare certificates; the one-argument `add_override` is removed, so a stale caller does not compile; `verify_server_cert` asks the store whether the pair of server name and certificate is accepted; `remove_certificate_failing_verification` still removes the failure when the error page is built (a failure left behind would turn a later, unrelated network error into a certificate error) but also copies it into the bounded record, and now handles IPv6 literals; and `create_tls_config` takes its TLS session cache from the override manager, because rustls does not verify a certificate again when it resumes a session, so a revoke has to drop the host's session tickets too. In `fetch/methods.rs`, `handle_allowcert_request` is deleted and Servo's `chrome:allowcert` entry point is refused with a warning, because it pushed a bare certificate. In `lib.rs` there is one `pub mod` line. In `http_loader.rs` and `websocket_loader.rs` the HSTS update after a response goes through `HttpState::learn_hsts_from_response`, which does nothing for a host that has had an exception in this process: a response over a connection that only the user's exception made valid must not teach the HSTS list (RFC 6797, section 8.1), or a self-signed device that sends the header would be refused for good from the next session on, and whoever holds its key could plant an HSTS entry that Mercury keeps. A revoke does not close connections that are already open (HTTP keep-alive and HTTP/2), which is why Mercury has no Remove button yet.
- **Mercury is the only caller.** The protocol handler in `crates/talaria-servo/src/certificate_exceptions.rs` forwards the user's decision to `accept_failed_certificate` and holds no policy of its own. The engine accepts only the certificate that the host actually failed with, never one the page supplies, and refuses HSTS hosts.
- **Drop it when** Servo binds overrides to a host and refuses HSTS hosts itself.

The patch file is generated. After any merge or rebase that touches `third_party/servo-net`, run `cargo run --release -p mercury-servo-patches -- export` again and then `make servo-patches-check`; never merge `third_party/patches/servo-net.patch` by hand. How to run its tests is in [Testing](testing.md#certificate-exceptions): `make test-servo-net` (also part of `make test` and of the CI job "Vendored servo-net tests"; the root `Cargo.toml` lists the crate under `exclude`, as it does gaol, so that cargo runs it on its own manifest).

### Re-vendor on a Servo upgrade

Run [`upgrade --servo`](#upgrading), which merges Mercury's changes into the new published crates, and follow its steps. Then, for every vendored Servo crate that carries a change:

1. Redo the warning table in `third_party/servo-script/MERCURY.md` from a clean build's output: new upstream warnings are silenced at their site, and silencing that is no longer needed is removed.
2. Update the versions, checksums, commits and line numbers in each `MERCURY.md`. Drop a change whose "drop when" condition the new release meets.
3. Run the capture tests (`make test-capture-macos`) and the full gate.
4. Run the vendored crates' own tests (see [Testing a vendored crate](#testing-a-vendored-crate)): `make vendored-tests` (light, also in CI), `cargo test --release -p servo-script --lib` and `-p servo-webgl --lib` with `RUST_TEST_THREADS=2` after `eval "$(scripts/macos/gstreamer.sh env)"`, and the `servo-net` and `servo` tests by the pattern described there.
5. Take out every backport whose pull request the new release contains (each `MERCURY.md` says "drop at the Servo bump that contains PR #n"), re-check the hand-ported chain #49884, #49917, #49920 (they need #49552 upstream) and the SVG guard (`mercury_svg.rs`: when resvg 0.50 arrives, set `max_depth: 500` in `usvg::Options` and delete the file), and move the `mozjs` pin if the new release moved it.
