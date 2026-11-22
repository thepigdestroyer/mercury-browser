---
title: Testing Media Source Extensions
description: How to run the Media Source Extensions checks that need a window (the test pages, the player pages for dash.js, Shaka Player and hls.js, and the web-platform-tests media-source tree) with one command, how to read the report, and the by-hand list for YouTube and Twitch.
---

# Testing Media Source Extensions

The Media Source Extensions (MSE) code is tested in pieces without a window (`cargo test -p mercury-mse -p mercury-mse-testkit -p mercury-mse-gst`; see [Media Source Extensions](media-source.md)). What only a window can show is whether the pieces work together inside the browser: the DOM, the element, the pipeline and a page's JavaScript. This page is the tooling for that. It is built so that a run is **short, mechanical and repeatable**, because a window run needs an unlocked screen and a lock costs a run.

What is here:

| Piece | Where | What it does |
|---|---|---|
| The test pages | `tests/pages/mse-*.html` | Small pages that append fixtures and check the DOM and the element; each shows `PASS` and `FAIL` lines and ends its title with `(done)`. |
| The player pages | `tests/pages/mse-players/{dashjs,shaka,hlsjs}.html` | dash.js, Shaka Player and hls.js each play a generated 8 s stream: start, one seek, one forced quality switch, the end, `getVideoPlaybackQuality()`. |
| The WPT tree | fetched, not committed | `media-source/` of web-platform-tests at a pinned commit, run file by file. |
| The expectations | `tests/wpt/media-source.expect.toml` | Every known WPT failure, with a class and a reason. The contract for the flip. |
| The runner | `mse-run` (`crates/mercury-devtools`) | One browser process per test, a watchdog, a journal, a report. |
| The server routes | `serve-tests` (`crates/mercury-devtools/src/serve/`) | `/wpt/`, `/mse-players/`, `/mse-dash/`, `/mse-hls/`, `POST /__report`. |
| The wrapper | `scripts/dev/mse-window-run.sh` | One command. |
| The fetch script | `scripts/dev/fetch-mse-tools.sh` | WPT, dash.js, Shaka Player and hls.js, pinned and checked by SHA-256. |

## Run it

On the build you want to check (the browser is `target/release/mercury-browser`, so `make build-release` first), with **an unlocked screen and nobody else's Mercury running**:

```sh
scripts/dev/fetch-mse-tools.sh                 # once: about 40 MB, a few seconds; --check verifies without the network
scripts/dev/mse-window-run.sh pages            # the test pages: about 6 minutes
scripts/dev/mse-window-run.sh players          # dash.js, Shaka Player, hls.js: about 8 minutes
scripts/dev/mse-window-run.sh wpt              # media-source/: about 25 minutes
scripts/dev/mse-window-run.sh all              # the three, in that order
scripts/dev/mse-window-run.sh all --resume     # after an interruption: continue the journal
```

The browser is started with `MERCURY_DEV_MSE=1` (the preference `dom_media_source_enabled` is off otherwise), `--background`, a fresh profile per test, and without proxy variables. Useful options (`mse-run --help` lists them all):

| Option | Meaning |
|---|---|
| `--timeout SECONDS` | The watchdog of one test (default 60; a WPT file that asks for the long timeout gets twice). |
| `--only TEXT` | Only the tests whose name contains the text, for example `--only mse-seek` or `--only changetype`. |
| `--list` | Print what would run and exit. |
| `--browser PATH` | Another browser binary, or a stub script (see "Dry run"). |
| `--out DIR`, `--journal FILE`, `--report FILE` | Where the browser logs, the journal and the report go (default: the lane's log directory, else a temporary one; `MSE_RUN_OUT` sets it). |
| `--no-lock-check` | Skip the lock check. For a stub browser only. |

**Before it starts**, `mse-run` checks that the browser, the fetched tools and the expectations file exist and are valid, and that the screen is not locked. It reads `CGSSessionScreenIsLocked` from `ioreg -n Root -d1`; "cannot tell" (nobody is logged in at the screen) is refused too. A run that is refused starts nothing.

**While it runs**, each test is one browser process. The page reports to the server in the same process: a small script that the server inserts into each page of `tests/` opened by the runner (and Mercury's own `testharnessreport.js` for WPT) posts the `PASS`/`FAIL` lines or the subtest results to `/__report`. The browser has no command that prints a page's title, which is why. A watchdog ends a test that does not finish: SIGTERM, a grace period of 5 s, then SIGKILL on the whole process group. A browser in state `U` (uninterruptible; a GPU call hung on a locked screen) cannot be ended by any signal: the runner records the test as **stuck**, does not wait for it, and stops; only a restart of the Mac clears such a process. The journal gets one line after **every** test, so what has run is never lost.

**A lock in the middle of a run** costs one test. The runner checks the lock before each test and again after a test that timed out, crashed or never loaded; if the screen is locked then, that test is recorded as `INTERRUPTED` (not a failure), the run stops and says so, and `--resume` runs that test again and the rest. Tests that finished (PASS, FAIL, TIMEOUT, NOLOAD, CRASH) are kept by `--resume`.

## Reading the report

The report is `window-run-<UTC>.md` in the output directory: a header (browser, journal, the tools fetched), the verdict, and one section per phase.

- **Pages and players**: a table with each test's result (`PASS`, `FAIL`, `TIMEOUT` = started and did not finish, `NOLOAD` = never reported, `CRASH` = the browser ended first), PASS and FAIL line counts, time, and the **peak resident memory** of the browser process (sampled every half second). The `FAIL` and `INFO` lines of every test that did not pass are listed below the table: the `INFO` lines carry the measurements (seek latencies, colours, frame counts). A page passes only when its title ends with `(done)`, no line starts with `FAIL`, at least one starts with `PASS`, and the title's own count agrees with its lines. Script errors are noted and do not fail a passing page.
- **WPT**: the number of files run (loaded, never loaded, crashed, past the watchdog), the subtests run and the share that PASS (the floor of flip condition F3 is 85%; "met" or "NOT met" is printed), every **unexpected result** against the expectations file, the files that did not finish normally, the known failures by class, the entries that name no test of this checkout, and "F3": the `mse-core` and `dom` entries in the core areas (append, remove, abort, duration, end of stream, buffered, seek, `changeType`, `timestampOffset`, append windows) that have no `issue`. Each of those blocks the flip until it is fixed or given an issue-level reason.
- **The exit status** is 0 only when nothing unexpected happened **and** nothing was left out. 1: unexpected results, an unexpected pass included. 2: a usage or preflight error. 3: the run is incomplete (a lock, a stuck browser, tests without a result); that wins over 1, because an incomplete run proves nothing.

## The failure classes

Every failure is classified before anything is fixed. The classes are the `class` of the entries in the expectations file and the order in which a failure is looked at.

| Class | Meaning | Who fixes it |
|---|---|---|
| `mse-core` | Behaviour of `crates/mercury-mse` (the algorithms below the DOM). | The lane: fix in the crate, with a test that fails without the fix. |
| `dom` | The carried DOM (`third_party/servo-script/dom/mse/`) or the element hooks. | The lane. |
| `player` | `servo-media-gstreamer`, `mercury-mse-gst` or GStreamer itself (playback, seeking, decoding). | The lane. |
| `engine-gap` | Servo 0.8 lacks an API the page or the library uses (`Intl`, a `Worker` from a Blob URL, `MediaCapabilities`, `ResizeObserver`). | **Not the lane.** Recorded in `sdd/med-1b-logs/engine-gaps.md` for the lead, with the library or test that hit it. |
| `test-bug` | The test is wrong, or needs a server feature the runner does not have (`wptserve` substitution). | Fix the test page; for WPT, record why. |
| `not-planned` | Mercury does not do this: MSE in workers, `ManagedMediaSource`, WebCodecs input, IAMF. | Nobody; the entry says so, and it may be `skip = true`. |

## The WPT tree and its expectations

`scripts/dev/fetch-mse-tools.sh` makes a **sparse checkout** of `web-platform-tests/wpt` at one pinned commit (`media-source`, `resources`, `common`, `interfaces`) into `~/Library/Caches/mercury-browser/ci-tools/mse/wpt` (the one cache directory the disk guard keeps; `MSE_TOOLS_DIR` moves it). Nothing is committed to the repository, so no WPT licence text enters it. The script prints a problem and exits 1 on any mismatch (`--check` verifies without the network); the three players are one file each at a pinned version, checked against a pinned SHA-256 (a download that differs is deleted).

`serve-tests` serves the checkout at `/wpt/` and, because WPT pages name their helpers by absolute path, `/resources/`, `/common/`, `/interfaces/` and `/media-source/` at the root. Its `resources/testharnessreport.js` is Mercury's. It also does what `wptserve` does for `.any.js` and `.window.js` files: a request for `x.any.html` or `x.window.html` makes the page from the `.js` file and its `// META:` lines. What it does **not** do: the worker variants of `.any.js` files, reftests (a page with `<link rel=match>` is left out and counted), `.sub.` substitution and `.headers` files. `media-source/` uses none of them (checked at the pinned commit).

`tests/wpt/media-source.expect.toml` says what each file is expected to do. A file with no entry must finish `OK` with every subtest `PASS`. The format is described at the top of the file and in `crates/mercury-devtools/src/mse_run/expect.rs`. An entry **without a class or a reason is refused** (the run does not start). The file is the contract of flip condition F3: after a run, everything that differs is in the report.

To update it after a run: for each **unexpected failure**, decide the class, fix it or add the subtest to the entry's `fail` list with a `reason`; for each **unexpected pass**, remove the subtest (or the whole entry); delete stale entries. Never add an entry to hide a failure without a reason a reader can check.

The starting list is Servo PR #47269's `tests/wpt/meta/media-source/`, which lists what Servo's own MSE fails. Mercury carries that PR's DOM but a different data path, so the first run will contradict it in both directions. Every entry says "provisional" until a window run has confirmed it.

## The player pages

Each page loads one library from `/mse-players/` and plays a stream that `serve-tests` generates from the committed fixtures of `crates/mercury-mse-testkit`, so no segment and no manifest is committed:

- `/mse-dash/h264/manifest.mpd`: DASH, numbered segments with a timeline, two H.264 renditions (320x180 and 640x360) and AAC audio; `/mse-dash/vp9/manifest.mpd`: VP9 and Opus in WebM (Shaka plays it for 2 s at the end of its page);
- `/mse-hls/h264/master.m3u8`: HLS with fMP4 segments, two variants and an AAC audio rendition, `#EXT-X-MAP` for the init segment.

Init segment followed by the media segments is **byte for byte the fixture's file**; a test of the server checks that. The player pages' steps are in `tests/pages/mse-players/common.js`. They fail with a `FAIL` line, never a hang, when a step does not happen (each wait has a limit and the page has its own 48 s watchdog). hls.js runs with its transmuxing worker off: the stream is fMP4 and a `Worker` from a Blob URL is a separate engine question.

A player that fails is classified (above) before it is fixed; a failure of the library on something unrelated to MSE is an `engine-gap` and goes to `engine-gaps.md`. By hand, the same streams open in any player from the URLs above and the index `http://127.0.0.1:<port>/pages/mse-players/index.html`.

## Dry run (no browser)

`mse-run` itself is tested without a browser. `crates/mercury-devtools/tests/stub-browser.sh` is a stub that takes the browser's arguments and posts canned reports to the server. The whole pipeline, with a complete report, runs like this:

```sh
scripts/dev/mse-window-run.sh all --no-lock-check --browser crates/mercury-devtools/tests/stub-browser.sh
```

By default every page passes and every WPT file finishes `OK` with one passing subtest, so the report shows what an unexpected pass looks like against the provisional expectations. `STUB_CANNED=<dir>` makes individual tests fail, hang, crash or stay silent (see the header of the script). The unit and integration tests (`cargo test --release -p mercury-devtools`) cover the report parser, the journal and resume, the expectations logic, the lock check with a fake `ioreg`, the watchdog and the kill order with a process that ignores SIGTERM, and the routes.

## By-hand checks of real sites

These need a person with the browser and no tools. Start it with `MERCURY_DEV_MSE=1` until the preference is on. For each site write down what happened (plays, no picture, no sound, stalls, an error on the page), the time to the first frame, whether a quality change by the menu works, whether a seek by the progress bar works, the video's codec if the site shows it, and a screenshot of any error. The result is recorded; only a crash, a hang or runaway memory in them reopens the decision to turn MSE on.

- **YouTube** (logged out; a normal 5 to 10 minute video, then a 1080p one): start; "Stats for nerds" (right-click the player: the codecs line `avc1` / `vp09` / `av01`, "Dropped frames", "Buffer health"); quality auto, then 1080p, then 360p and back; seek forward, back and near the end; pause for a minute and resume; play 10 minutes in a background tab and return; let it end and see if the next video starts; a live stream (weaker: the DVR window); a Short; an ad at the start (ads use MSE too). Expected: AV1 is not offered on Apple Silicon; no 4K if software VP9 is too slow; Widevine content is not available.
- **Twitch**: a live channel for 5 minutes (latency, a quality change, pause and resume, a reload), a VOD (seek, speed 1.25x), a clip.
- **Others, 2 minutes each**: Vimeo (HLS or DASH), Reddit and X video (hls.js), the Shaka demo (`shaka-player-demo.appspot.com`), the dash.js reference player (`reference.dashif.net/dash.js/`), a bitmovin demo page, Internet Archive video. Netflix, Disney+ and anything that says "protected content" are **expected to fail** (they need EME): note the message, do not report it as a bug.
- **If the browser freezes or the page stays black**: note the time and whether the Mercury window still responds; run `pgrep -fl mercury-browser`. The time is what lets the lane find the log line; a screenshot of a frozen window is not.

## Continuous integration

`.github/workflows/mse.yml` builds and tests `mercury-mse`, `mercury-mse-testkit` and `mercury-mse-gst` on Ubuntu 22.04 (GStreamer 1.20, the `v1_18` features only) and on Apple Silicon (the bundled GStreamer, VideoToolbox), on pushes to `mercury` and `master`, on pull requests and by hand (never on a lane branch: the train integrator runs it once at the merge). The `#[ignore]` VideoToolbox and AV1 route tests run there as a **record-only** step: they print what the hardware answers and never fail the run.
