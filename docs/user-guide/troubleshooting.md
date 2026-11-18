---
title: Troubleshooting
description: Diagnose start-up, blank-page, canvas, media, build and website problems in Mercury Browser.
---

# Troubleshooting

Start with the first unexpected message in the log. Later messages, such as a `Broken pipe` warning, usually describe a process that had already failed.

## Collect a log

Start Mercury from Terminal; it writes its log there:

```bash
RUST_LOG="talaria_servo=debug,servo=info" \
    "/Applications/Mercury Browser.app/Contents/MacOS/mercury-browser"
```

From a source checkout, `make run` keeps Mercury's messages visible and Servo's quieter, and `make run RUST_LOG="talaria_servo=debug,servo=info"` shows more.

In a report, include the Mercury version (**About Mercury Browser**, or `mercury-browser --version`), your macOS version and Mac model, the page address, what you did, and the first relevant error. Remove personal addresses and tokens first. Crash reports, if any, are in **Console → Crash Reports** under `mercury-browser`.

## macOS won't open the app

macOS blocks the first launch of an app that isn't notarised. See [the first launch](installation.md#the-first-launch-allowing-mercury-past-gatekeeper).

## Mercury quits at once with status 4

Another Mercury is already using the profile. Only one copy can use a profile at a time; quit the other one, or start this one with `--profile-dir <an empty folder>`.

The other exit statuses: 0 success, 1 a start-up failure (or a screenshot that couldn't be saved), 2 a wrong command line (`--help` lists the options), 3 a screenshot's page didn't finish loading.

## A page stays blank

Check whether the tab's title or address changed.

- **Nothing changes:** look in the log for network, address, TLS or start-up errors.
- **The title changes but the page stays blank:** the page loaded but wasn't drawn. Look for messages about the rendering context or the first frame.

If you refused a "Leave this page?" prompt while the page was still loading, the page can stay stuck in its loading state: Servo has already stopped the load by the time it asks Mercury. Reload the page.

## Canvas drawings are missing or wrong

Mercury draws 2D canvases on the GPU. When the GPU canvas cannot start, canvases are drawn on the CPU instead and the log shows one warning, `GPU 2D canvas unavailable (…); drawing 2D canvases on the CPU`. If a canvas still looks wrong, turn off **Settings → Advanced → Draw canvases on the GPU** (it applies at the next start) or start Mercury with `--disable-gpu-canvas`, and say in your report whether that changes anything.

## Media does not play

1. Try a direct MP4 or WebM file. If it plays, the site most likely streams through Media Source Extensions, which Servo doesn't have yet ([Media playback](media.md#video-sites)).
2. Ogg files don't play on macOS for now; that is expected.
3. Start Mercury with media logging ([Media playback](media.md#diagnose-playback)) and look for the first GStreamer error.

Every start logs this warning, which is expected while Ogg is left out, and media still works:

```text
WARN servo::servo::media_platform] Error initializing GStreamer: ErrorLoadingPlugins(["libgstogg.dylib"]). Trying fallback initialization.
```

A real failure looks different: more files than `libgstogg.dylib` in that list (a damaged or incomplete `lib` folder in the app), or `dyld: Library not loaded: @rpath/libgst…` before anything else. Reinstall the app in either case.

## A website is broken

Mercury shows the web as Servo renders it. Try the page in Servo's own browser, `servoshell` (from [servo.org](https://servo.org/download/)), at the same Servo version. If it breaks there too, the fix belongs in Servo and the report is most useful there. If `servoshell` works and Mercury doesn't, say so in the Mercury issue.

## A build fails

- Run `make doctor`. It checks Rust, Cargo, rustup and, on macOS, the GStreamer copy that `make gstreamer-macos` unpacks.
- Keep the Rust version from `rust-toolchain.toml`; `rustup show active-toolchain` shows what is active.
- Check free disk space. A cold build compiles all of Servo and needs 15–20 GB free while it runs.
- Use the locked dependencies (`Cargo.lock` unchanged).

On Linux, rerun `make install-linux-deps` on a Debian or Ubuntu system. Mesa and the desktop session choose the graphics adapter; Mercury removes an inherited `DRI_PRIME` value before graphics or media start, and the vendored Surfman patch avoids setting a GPU preference for the whole process.
