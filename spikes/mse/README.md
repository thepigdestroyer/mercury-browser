# GstMSE spike

A research prototype, not part of Mercury Browser and not wired into the app. It answers one question for
the media lane (MED-1, R14a EA1): **is GStreamer's own MSE library (`libgstmse-1.0`, plugin `mse`,
element `msesrc`, in the bundled GStreamer 1.28.9) good enough to be Mercury's `MediaSource` backend?**
The findings are in `.superpowers/research/R17-mse-spike.md` (private notes); the lane brief is
`.superpowers/sdd/lane-sp-mse-brief.md`.

This directory is a Cargo workspace of its own. The repository's root workspace never includes it, so
`make check`, `make lint`, `make test` and the CI do not build it.

## Layout

| Path | What it is |
|---|---|
| `run.sh` | The only way to build and run the spike: `run.sh <cargo-subcommand> [args]` |
| `crates/mse-segments` | Pure Rust: splits fragmented MP4 and WebM into an initialisation segment and media segments, and loads the committed fixtures |
| `fixtures/` | The committed test media (8 s, 25 fps, a red picture then a blue one, a 440 Hz tone), `gen.sh`, checksums |
| `crates/mse-rig` | Private GStreamer set-up (`env`), a thin backend over `gstreamer-mse` (`backend`, the sketch for MED-1), the two pipeline shapes (`rig`), counting sinks (`frames`), the event log (`events`), the scenario runner (`harness`) and the report types (`report`) |
| `crates/mse-conformance` | Scenarios C01..C15: what an MSE player does (split A/V, seeks, quality switches, removal, end of stream) |
| `crates/mse-stress` | Scenarios T01..T12: threads, re-entrancy, teardown, throughput, memory, decoders, protection events |

## Run it

macOS only, with the unpacked GStreamer (`make gstreamer-macos` once). Nothing makes a sound, opens a
window, uses the network or touches the real profile or keychain: sinks are `appsink`s, all media is a
local file, and every run uses a private scratch directory (`$TMPDIR/opencode/lanes/sp-mse/`) that is
removed at the end. Builds go through the disk guard, in the light pool (the spike has no Servo in it):

```sh
GUARD=~/mercury-browser/.superpowers/tools/guarded.sh
MERCURY_SLOT_POOL=light $GUARD "$PWD" 'RUST_TEST_THREADS=2 spikes/mse/run.sh test -p mse-rig'
MERCURY_SLOT_POOL=light $GUARD "$PWD" 'spikes/mse/run.sh clippy --all-targets -- -D warnings'
spikes/mse/run.sh fmt -- --check
spikes/mse/run.sh run -p mse-rig --example minimal -- --muxed               # the minimal prototype
spikes/mse/run.sh run -p mse-conformance -- --report /tmp/c.json
spikes/mse/run.sh run -p mse-stress -- --only T01,T04 --report /tmp/t.json
```

`run.sh` changes to the repository root, loads the GStreamer environment (`scripts/macos/gstreamer.sh
env`), sets `DYLD_FALLBACK_LIBRARY_PATH` to the framework's `lib` directory for cargo and so for the test and
example binaries it starts, adds `--manifest-path spikes/mse/Cargo.toml` and `--target-dir target` (the
build output is `<worktree>/target`, which the disk guard deletes; `fmt` takes no target directory), and
execs cargo. It stops with a message naming `make gstreamer-macos` when the framework is not unpacked. A
program started from `target/` by hand needs the same `DYLD_FALLBACK_LIBRARY_PATH` (macOS removes `DYLD_*`
variables when a protected program such as `/bin/sh` or `perl` starts).

### The examples (`crates/mse-rig/examples`)

| Example | What it prints |
|---|---|
| `minimal` | The minimal prototype. Appends the init segment and four media segments of each fixture, plays, calls `endOfStream`, then prints the GStreamer version and plugin mode, whether EOS arrived, the `msesrc` track counts, per-sink counts (buffers, bytes, first and last timestamp, out-of-order, largest gap, flushes, caps changes), the colour of the picture per second (`RRRRBBBB`), the decoders chosen, `buffered`, the first events in order and the threads they came on, the teardown time. Exit 0 only if 200 (+-2) video frames arrived. Options: `--shape bare\|playbin`, `--plugin-mode system\|servo`, `--glib-main-loop`, `--sync`, `--muxed` (one source buffer, `h264-aac`), `--wait N` |
| `split_av_probe` | Variations of the append sequence for audio and video in two source buffers (order, delays, play first, one track only), one timed line per phase: the experiments behind the stall described below |
| `api_probe` | What GstMSE does at the edges of the API, one `Qnn` line per observation: negative `timestampOffset`, `+Infinity` duration and window, removal, error classes, `QuotaExceeded`. The answers are in the doc comments of `backend.rs`. Sections: `offset duration window remove errors quota lag bisect types` |

### The scenario binaries (`mse-conformance`, `mse-stress`)

Both are one line of `main`: `harness::main_for(tool, SCENARIOS)`. Options: `--list`, `--only C01,C04`,
`--shape bare|playbin` (default `playbin`), `--plugin-mode system|servo` (default `system`),
`--glib-main-loop`, `--report FILE`, `--scratch DIR`, `--iterations N`, `--seed N`.

**Every scenario runs in a child process** of the same executable (`--child <ID>`), so that a crash, an
abort or a hang inside GStreamer costs one scenario and not the report; the parent kills a child at its
timeout (60 s unless the scenario says otherwise) and never initialises GStreamer itself. One extra child
first runs only `env::init`: it fills the private plugin registry (so the scenarios' timeouts do not
include a plugin scan) and tells the parent the GStreamer version.

What gets printed on standard output (the lines are an example of the form):

```
mse-conformance: GStreamer 1.28.9 x86_64, plugins system-path, shape playbin, glib main loop off, seed 1234
PASS  C02   split A/V, fMP4  (2210 ms)
FAIL  C07   seek to an unbuffered range  (11034 ms)
ERROR C13   abort and errors  (60001 ms)
```

`PASS` behaved as an MSE player needs. `FAIL` is a finding about GstMSE: the scenario worked and GstMSE
misbehaved (what happened is in the JSON `notes`). `SKIP` was not run (the reason is in `notes`). `ERROR`
means the harness broke: a panic, a set-up failure, or a child that crashed, aborted or timed out (the exit
status or signal is in `notes`, with the last lines of its standard error). The JSON report (`--report`)
holds the run (`tool`, GStreamer version, architecture, plugin mode, shape, seed, plugins that failed to
load; no host name, user name or path) and, per scenario, `id`, `title`, `status`, `metrics`, `notes` and
`elapsed_ms`. Exit status: 0 unless some scenario is `ERROR` (then 1); 2 for a usage error.

## What the rig does

* **Private GStreamer** (`env.rs`), as `apps/mercury-desktop/src/media_isolation.rs` does it: a registry file
  in the scratch directory, no inherited `GST_*` variables (except `GST_DEBUG*`), no plugin scanner process,
  `XDG_*` and `GIO_*` inside the scratch directory. `system` mode scans the framework's plugins in process
  through a directory of symlinks that leaves out `libgstvulkan` (a fresh scan hung twice in its
  `plugin_init`, inside MoltenVK and the AMD driver, on this Mac) and `libgstpython`. `servo` mode loads the
  files Mercury ships today (Servo's plugin lists minus `ogg`) plus `libgstmse.dylib` one by one with
  `Plugin::load_file`, as `init_with_plugins` does; a file that fails to load is listed in the report.
* **Two shapes** (`rig.rs`): `playbin` is how Servo does it (`gstreamer_play::Play` over `playbin3`,
  `uri = mse://...`, the `source-setup` signal attaches the `MediaSource`, `PlaySignalAdapter::new_sync_emit`,
  no GLib main loop unless asked); `bare` is `msesrc ! decodebin3 ! appsinks`.
* **Counting sinks** (`frames.rs`): `appsink`s that count buffers, bytes, timestamps, caps changes, flushes and
  arrival times, and take the mean colour of the whole picture about once per second of media time.
* **The event log** (`events.rs`): every callback of GstMSE and GstPlay with the time and the OS name of the
  thread it came on (the thread is a result, so nothing is marshalled before logging); `add_hook` runs a
  closure on the emitting thread for re-entrancy experiments.
* **The backend** (`backend.rs`): `MseBackend` over `gstreamer-mse` 0.25, with no `gstreamer_mse` type in
  its public signatures. It is what MED-1 will put behind servo-media's traits.
* **Watchdogs.** GstMSE 1.28.9 can hang a state change (below), so `Rig::shutdown` runs the teardown on a
  helper thread and returns `RigError::Timeout` after 15 s instead of hanging, and a dropped rig sets NULL
  the same way.

## What building the rig found (details and numbers in R17)

* One source buffer holding audio and video plays: 200 video frames and 375 audio buffers, in both shapes,
  both plugin modes, with and without a GLib main loop. `vtdec_hw` decodes the H.264 and FFmpeg's
  `avdec_aac` the AAC (R16 P2 matters here).
* **Audio and video in two source buffers (how DASH delivers them) deadlock** in GstMSE 1.28.9: the second
  source buffer's track never comes up, the element stays `HaveNothing`, and `set_state(NULL)` then hangs
  in `gst_task_join`. See the ignored test in `crates/mse-rig/tests/gstmse_cycle.rs` and
  `examples/split_av_probe.rs`.
* The append window's start setter checks the wrong way round, `UpdateEnd` comes before `buffered` shows
  the data, there are no negative `timestampOffset` values, and `QuotaExceeded("buffer is full")` comes at
  about 100 MiB: the doc comments of `backend.rs` have the measured answers.
