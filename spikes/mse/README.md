# GstMSE spike

A research prototype, not part of Mercury Browser and not wired into the app. It answers one question for
the media lane (MED-1, R14a EA1): **is GStreamer's own MSE library (`libgstmse-1.0`, plugin `mse`,
element `msesrc`, in the bundled GStreamer 1.28.9) good enough to be Mercury's `MediaSource` backend?**
The findings are in `.superpowers/research/R17-mse-spike.md` (private notes); the lane brief is
`.superpowers/sdd/lane-sp-mse-brief.md`.

This directory is a Cargo workspace of its own. The repository's root workspace never includes it, so
`make check`, `make lint`, `make test` and the CI do not build it.

| Crate | What it is |
|---|---|
| `crates/mse-segments` | Pure Rust: splits fragmented MP4 and WebM into an initialisation segment and media segments; the committed fixtures (`fixtures/`) |
| `crates/mse-rig` | Private GStreamer set-up, a thin backend over `gstreamer-mse` (the sketch for MED-1), pipelines with counting sinks, the event log, the report types |
| `crates/mse-conformance` | Scenarios C01..C14: what an MSE player does (split A/V, seeks, quality switches, removal, end of stream) |
| `crates/mse-stress` | Scenarios T01..T12: threads, re-entrancy, teardown, throughput, memory, decoders, protection events |

## Run it

macOS only, with the unpacked GStreamer (`make gstreamer-macos` once). Nothing makes a sound, opens a
window or touches the real profile.

```sh
spikes/mse/run.sh test                          # unit tests of the crates
spikes/mse/run.sh run -p mse-conformance -- --report /tmp/c.json
spikes/mse/run.sh run -p mse-stress -- --report /tmp/t.json
```

`run.sh` loads the GStreamer environment (`scripts/macos/gstreamer.sh env`), sets
`DYLD_FALLBACK_LIBRARY_PATH` for the test and run binaries, uses `<worktree>/target` as the target
directory (so the disk guard deletes it) and passes everything else to `cargo`.
