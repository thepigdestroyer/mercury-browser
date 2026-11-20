---
title: PDF viewer
description: How PDF files reach Mercury's own viewer, where PDFium comes from and how it is pinned, the helper process and its protocol, the limits, and what is not protected.
---

# PDF viewer

A link to a PDF file opens in the tab, in a viewer Mercury draws itself, from pages rendered by
**PDFium** (the PDF library of Chromium) in a helper process. Servo cannot show a PDF: it shows
its "Unknown content type" page, which Mercury notices (`talaria-servo/src/download_offers.rs`)
and hands to `apps/mercury-desktop/src/downloads/`. This page describes the parts that supply
PDFium and hand a PDF to the viewer. The viewer's own pieces are in `crates/mercury-pdf` (the
protocol, the helper's serve loop, the client, the model of what to draw), `crates/mercury-pdfium`
(the PDFium backend and `locate_library`) and `apps/mercury-desktop/src/pdf/` (the Slint viewer).

```text
click / typed address / script
  -> Servo loads application/pdf -> "Unknown content type" page
  -> DownloadSignal::Offered { url, mime_type, referrer, posted }
  -> downloads/mod.rs  offered():   pdf_handoff decides  (setting web.pdf, facts, viewer available?)
       viewer           fetch into <profile>/pdf-view/<random>.pdf  -> PdfViewerSink::open(tab, url, file)
       system viewer    download as usual, then /usr/bin/open <file>   (only if the user started it)
       download         the ordinary download rules
  viewer -> PdfClient <-pipe-> mercury-browser --pdf-worker -> libpdfium.dylib
```

## The setting and the decision

`web.pdf` in `settings.json` (`mercury_settings::PdfAction`, shown in Settings > Downloads as "PDF
files") is `open_in_mercury` (the default), `open_in_system_viewer` or `download`. A value this
version does not know reads as the default and nothing else in the file is reset (`PdfAction` has
its own deserialiser: an undecodable settings file would otherwise be moved aside whole).

The rule is one pure function, `mercury_downloads::pdf_disposition(preference, viewer_available,
&Facts)`, with its whole table tested in `crates/mercury-downloads/tests/disposition.rs`. In order:

| # | Rule |
|---|---|
| 1 | Only `application/pdf` and `application/x-pdf` (parameters and case ignored) are PDFs; anything else is left to the ordinary rules. |
| 2 | `Content-Disposition: attachment` (or any disposition type but `inline`) is a download, whatever the setting. The header is learned from the response: the viewer's fetch reports it on `DownloadEvent::Started::attachment`; then the offer goes through the ordinary rules again. |
| 3 | The setting `download` is a download. |
| 4 | `open_in_mercury` with no viewer (no PDFium library) behaves as `open_in_system_viewer`. |
| 5 | `open_in_system_viewer` is a download followed by `/usr/bin/open <file>`, **only** when the user started the navigation, and **never** in an automated run (`--screenshot`, `--background`: the log says "would open"). Otherwise it is a plain download: a page cannot launch an application. |
| 6 | The viewer: a tab in the background shows nothing unless the user started it (a Ctrl-click: the file is fetched and the viewer opens when the tab is first shown); only `http` and `https` are fetched, plus `file:` for an address the user typed or opened (read where it is, never copied or deleted); `blob:`, `data:` and the rest are refused; a form's POST (`posted == Some(true)`) is refused. |

A refusal is not the hand-off's: the ordinary rules refuse the same offers, with their notes and
their going back to the previous page. For the viewer there is **no** going back: the tab stays on
the PDF's address.

## How the file reaches the viewer

`downloads/pdf_handoff.rs` carries the decision out. It does no network work itself; it answers
with actions that `downloads/mod.rs` performs (the block marked `pdf-viewer`).

- The viewer's **fetch** is an ordinary `mercury-downloads` transfer with the request a download
  gets: the engine's user agent, the cookies of the PDF's own URL, the origin as referrer. It goes
  into `<profile>/pdf-view/` (mode 0700, files 0600) and is **never listed** as a download: its
  events are taken out of the queue before the list sees them.
- **One fetch per tab**; a newer offer for the tab cancels the older one, whose file is deleted.
- The finished file must hold `%PDF-` in its first 1,024 bytes, or it is deleted and the offer goes
  through the ordinary rules with a one-line note. An attachment drops the copy the same way.
- If the tab no longer shows the address when the file arrives, the file is deleted; a closed tab
  cancels its fetch and removes its file.
- A fetch that fails leaves a line in the downloads list. A second request for a one-time link can
  fail ("the site did not send the file again"): that is the downloads limitation, fixed when the
  engine hands over its own copy.
- **The fetch is bounded.** The request carries `DownloadRequest::max_bytes`
  (`PRIVATE_COPY_MAX_BYTES`, the viewer's own `limits::MAX_FILE_BYTES`, 512 MiB). The download
  crate refuses a response whose `Content-Length` (or `Content-Range` total) is over it before any
  file is made, and stops a body that grows past it (a server with no length, or one that never
  ends) before the byte that would cross it is written; the transfer then fails, its partial file
  is removed, and the hand-off turns the `Failed` event into the note "The PDF could not be
  opened: the file is larger than the 512 MiB limit". The tab stays on the PDF's address. Tests:
  `mercury-downloads/tests/pdf_fetch.rs` (`/endless.pdf` is a chunked body that never ends).
- **What removes a private file.** A closed tab, a tab that left the address, a newer offer, an
  attachment, a body that is no PDF, a failed or cancelled fetch (as above); **the end of the run**
  (`pdf::release()` in `release_window` of `post_run.rs`: every session is dropped, which stops its
  helper and deletes its copy, then `clear_private_folder` removes what no session owns, such as a
  fetch that finished as the run ended); and **the next start** (`sweep_at_start`: every file in
  `pdf-view/` that is older than this run's start, which is all of them, because the profile is
  locked by one Mercury and this run has fetched nothing yet). A crash or a forced quit therefore
  leaves a copy only until the next start, not for a day. The copies "Open in Preview" makes in
  `$TMPDIR/mercury-browser-pdf-open/` are the exception: another app may be reading them, so the
  end of the run keeps them, and the next start (of any profile) removes those over a day old.

The seam to the viewer is `PdfViewerSink` (`is_available`, `open(tab, url, PdfFile { path, owned })`,
`tab_closed`) and `install_viewer`, both in `pdf_handoff.rs`; the viewer registers itself once at
start-up. With no sink installed PDFs behave as `open_in_system_viewer`. `owned` means a private
cache copy that the viewer deletes when it is done.

## PDFium: where it comes from

`scripts/macos/pdfium.sh` (modelled on `gstreamer.sh`) fetches and checks one release of
[pdfium-binaries](https://github.com/bblanchon/pdfium-binaries), **without V8 and without XFA**
(`pdfium-mac-*`, never `pdfium-v8-*`: about three times the size, and it runs form JavaScript).

```sh
make pdfium-macos                                   # this Mac's architecture
scripts/macos/pdfium.sh --arch arm64 fetch unpack check
eval "$(scripts/macos/pdfium.sh env)"               # MERCURY_PDFIUM_LIBRARY, MERCURY_PDFIUM_PREFIX
```

| Command | What it does |
|---|---|
| `fetch` | downloads `pdfium-mac-<arch>.tgz`; a checksum that is not the pinned one deletes the download and stops |
| `unpack` | checks the checksum again, refuses an archive that holds anything but files and folders or a path that climbs out, expands it, then runs `check` on the result before it is put in place |
| `check` | statically: the release (`VERSION`), `args.gn` says `pdf_enable_v8 = false` and `pdf_enable_xfa = false` (and the right `target_cpu`), the library's SHA-256, `lipo -archs`, the install name `@rpath/libpdfium.dylib`, only system libraries are loaded, and the licence files are exactly the ones THIRD-PARTY-NOTICES.md lists. Nothing is executed, so an arm64 build can be checked on an Intel Mac |
| `env`, `prefix`, `library` | print the exports, the unpacked directory, the library's path |

The architecture is `--arch arm64|x64`, else `MERCURY_PDFIUM_ARCH`, else this Mac's. The cache is
`~/Library/Caches/mercury-browser/pdfium/8165/<arch>/` (`MERCURY_PDFIUM_CACHE` replaces the root),
the tarball unpacked as it is (`lib/libpdfium.dylib`, `include/`, `licenses/`, `LICENSE`, `VERSION`,
`args.gn`) beside a hidden marker, with the download next to it. `MERCURY_PDFIUM_URL_BASE` replaces
the download address (a mirror, or a `file://` directory, which is how `test-pdfium.sh` works); the
pins are checked either way. The disk guard deletes the cache when no Mercury is running; the script
fetches the 4 MB again.

`mercury_pdfium::locate_library()` looks, in order, at `MERCURY_PDFIUM_LIBRARY`, at
`<executable dir>/lib/libpdfium.dylib` (the bundle) and, on macOS, at that cache. A build run from
`target/` therefore finds PDFium after one `make pdfium-macos`, and `target/<profile>/lib` stays
GStreamer's alone (its collector compares the directory with a digest, so a foreign file would make
it rebuild the directory).

### The pins (release `chromium/8165`, PDFium 160.0.8165.0)

Checked against real downloads on 19 November 2026. Bundle builds use arm64; the Intel development
Mac runs x64; `univ` is pinned for later and never used.

| Asset | Bytes | SHA-256 of the tarball | SHA-256 of `lib/libpdfium.dylib` |
|---|---|---|---|
| `pdfium-mac-arm64.tgz` | 3,640,054 | `112e74d717f3e0310273b70f3c8780e8a61ff11b34834124465c8305c841d350` | `d421e426bd487de02fa3bd1f2aeaf56947261f790e6116951e26f729ced1b018` |
| `pdfium-mac-x64.tgz` | 3,872,424 | `07285915b564e46a432a9909dbc1f15892885ca6489e3faeefc70dee77ed371a` | `fcbfee9dc7f895289fbfb8f7d1f0d5d643f60fcb790708425daaa0cfa64f330d` |
| `pdfium-mac-univ.tgz` | 7,394,247 | `80d88c4bec1876e04448523b76d1a70ece00bbc33e15301419d4fb9d2f8cc895` | `fdc694fe7e2681bf239c3462d036d4ff88c41b6f74c1533c25f36c33cfea4482` |

The library needs macOS 13.0 (the application's minimum), loads only system libraries (AppKit,
CoreGraphics, CoreFoundation, Foundation, libSystem) and already carries the install name
`@rpath/libpdfium.dylib`, so the bundle copies it unmodified (`collect-pdfium.sh` verifies, never
rewrites). Listing its exports shows every function that `pdfium-render` 0.9.5 names for the
`pdfium_8125` feature (leaving out the Skia, V8, XFA and Windows-only ones) is there; the real load in
`mercury-pdfium`'s tests is the authority.

### Bumping PDFium

1. Pick a release of pdfium-binaries; read the GitHub release API for the three `pdfium-mac-*`
   digests, download the three tarballs, hash them and the `lib/libpdfium.dylib` inside each.
2. In `scripts/macos/pdfium.sh` change the one pin block: `PDFIUM_RELEASE`, `PDFIUM_VERSION`, the six
   digests, and `PDFIUM_LICENSE_FILES` if `licenses/` changed (the script refuses a tarball whose
   licence files differ from the list: that is the reminder to update the notices).
3. Replace the texts under `LICENSES/` and the table in `THIRD-PARTY-NOTICES.md` for any component
   whose licence changed (`scripts/macos/test-pdfium.sh --online` compares the copies with the
   tarball), and the FreeType year in the credit sentence.
4. If `pdfium-render` has a newer binding feature for the release, move the `pdfium_*` feature in the
   workspace `Cargo.toml` with it; the real-library tests in `mercury-pdfium` fail when a symbol
   went missing (pdfium-render loads every symbol at once).
5. `make pdfium-macos`, `scripts/macos/test-pdfium.sh --online`, `make lint-scripts`, then the
   `mercury-pdf` and `mercury-pdfium` tests with the real library.
6. Update the table above and the release in the user guide's troubleshooting if it names one.

## In the application bundle

`bundle.sh` calls `scripts/macos/collect-pdfium.sh <prefix> Contents/MacOS/lib Contents/Resources`
after the media libraries, with the arm64 build (`--pdfium-prefix` or `pdfium.sh --arch arm64
prefix`; the bundle refuses to build without it). The collector verifies the whole prefix against
the pins first, refuses anything but arm64 for a bundle, and writes
`Contents/MacOS/lib/libpdfium.dylib` and `Contents/Resources/pdfium-licenses/` (the build's `LICENSE`
and one file per component, flat). The existing inside-out signing loop signs the library with the
rest. `scripts/ci/check-macos-bundle.sh --require-media` asserts the library (arm64 only, install
name) and the licence files. In the release workflow's signing job PDFium is downloaded afresh into
`$RUNNER_TEMP/pdfium` and checked against the same pins, like GStreamer.

## The helper process and its protocol

One helper per open document (`mercury-browser --pdf-worker`, the same executable), at most four live
at once: a crash or a leak dies with the file and PDFium's global state is never shared. Inside, one
thread owns the PDFium backend (PDFium is not thread-safe) and a reader thread feeds it. The
browser's `PdfClient` starts it with a **cleared environment** (only `MERCURY_PDFIUM_LIBRARY`), no
network use, stderr inherited for the log; its standard output is the protocol stream and nothing
else may write there. It is not restarted when it dies: the viewer reports it and offers Try again,
Save a copy and Open in Preview, so a file that crashes PDFium cannot loop.

Frames are `[u32 LE header length][u32 LE blob length][header JSON][blob]`, in both directions, the
header at most 64 KiB and the blob at most 64 MiB, both checked before anything is allocated. The
helper's first message is `Ready { protocol_version, engine }`. Tiles are 512 x 512 device pixels of
opaque RGBA in the blob; page text travels as 20-byte records. A malformed frame is answered with one
error and an orderly exit. The authoritative description is the header of
`crates/mercury-pdf/src/protocol.rs`.

Limits (`mercury_pdf::types::limits`): a file of at most 512 MiB and 20,000 pages, a tile of at most
2,048 pixels a side and 4 mega-pixels, a scale from 0.05 to 16, at most 1,000,000 characters of text and 5,000
links per page, 5,000 outline entries, a find query of at most 256 characters. The
client's time limits are 60 seconds to open and 20 seconds for any other request; then the helper is
killed and the viewer says so.

Passwords are asked in the viewer, passed to the helper in the `Open` message only, never logged or
stored, and zeroed after use.

## What is not protected

There is **no operating-system sandbox** around the helper yet. The Servo content-process sandbox is
off in Mercury today; the plan for one (EC1 in the engine plan) must run `--pdf-worker` under its
profile when it lands. PDFium is C++ reading hostile input: process isolation, the cleared
environment, the file-size, page and tile limits and the time limits reduce the damage, but they do
not stop a memory-corruption exploit that runs as the user. The build without V8 and XFA removes the
JavaScript engine from the attack surface and the library runs no scripts, but not PDFium's parsers.

## Not in this version

Form filling and saving, annotations, signing, printing, outline and thumbnail sidebars, a context
menu, Select All, text-to-speech, VoiceOver reading of the page text, `blob:`, `data:` and POST PDFs,
an "Ask" action, Mercury as the system's default PDF app, and Linux and Windows downloads (a Linux
build uses a `libpdfium.so` named by `MERCURY_PDFIUM_LIBRARY` or placed in `<executable dir>/lib`).

## Troubleshooting

- **A PDF is downloaded instead of shown.** The viewer is not available (no library found: run
  `make pdfium-macos`, or check `MERCURY_PDFIUM_LIBRARY`), the setting is not the default, or the site
  sent it as an attachment. The log has one line saying which.
- **`pdfium.sh` says a checksum does not match.** The download is deleted; do not work around it. The
  release asset changed, or a mirror (`MERCURY_PDFIUM_URL_BASE`) serves something else.
- **`pdfium.sh check` says the licence files are not the ones the notices list.** PDFium added or
  dropped a component: update `PDFIUM_LICENSE_FILES`, `LICENSES/` and `THIRD-PARTY-NOTICES.md` together.
- **`bundle.sh` says there is no unpacked arm64 PDFium.** Run `make pdfium-macos` (it fetches arm64
  for `app-macos` and `package-macos`), or pass `--pdfium-prefix`.
- **The Intel development Mac cannot run the arm64 library.** Only static checks run on it (`lipo`,
  `otool`); the first Apple Silicon run is a hands-on check for the owner.
- **"The PDF viewer stopped working on this file".** The helper died or timed out. Run the file's
  tab again, or `mercury-browser --pdf-worker` is not the executable that started it (a copied binary
  without its `lib/` folder).
- **`pgrep -fl pdf-worker` shows helpers after the browser quit.** It should not: a helper ends when its
  standard input closes. Report it with the log.
