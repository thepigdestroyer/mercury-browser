---
title: Licence Compliance
description: What Mercury Browser must credit and publish when it ships, how the credits page is generated, the notes for counsel on the LGPL libraries, Slint, codecs and patents, and what to check before adding a component.
---

# Licence compliance

**This page is not legal advice.** It records what the repository does to meet the notice and source duties of the software Mercury ships, what was checked and when, and what is not verified. The owner's lawyer decides anything that is a legal question; the draft clause below is for counsel to rewrite. Where a statement comes from the licence audit of 18 November 2026 (R16, kept in the project notes), the text says so, and anything the audit lists as not verified is marked the same way here.

Mercury's own licensing is explained in `LICENSE` and `NOTICE` and summarised in [Origins and credits](origins.md). This page is about the other direction: the licences of what Mercury ships. Mercury is source-available, not open source. The rule behind everything here is that every dependency must allow commercial use and closed redistribution (`make deny` enforces the licence allow-list, see [Dependency policy](continuous-integration.md#dependency-policy)); this page covers the duties that remain even then.

## What is in the application

The macOS application (`Mercury Browser.app`, Apple Silicon) and the Linux preview archive contain:

- Mercury's own code, in files under the PolyForm Noncommercial License 1.0.0 or the MPL-2.0 (each file's SPDX header, or `REUSE.toml`).
- About 880 Rust crates built into the executable: 12 of Mercury's own, 25 vendored under `third_party/` and 845 from crates.io, counted with `cargo tree -p mercury-desktop -e normal,build --target aarch64-apple-darwin` on 19 November 2026. The Linux target has 945 (12, 25 and 908), because it builds the Wayland, X11 and FreeType crates instead of the Apple framework crates. `cargo metadata` alone reports more (962 for the workspace), because it unifies the features of the whole workspace; the set must come from `cargo tree`.
- Native code that `-sys` crates compile into the executable: SpiderMonkey, ANGLE, AWS-LC, HarfBuzz, OTS, SQLite, zstd and others (the list is `credits.toml`, below).
- On macOS only: the GStreamer libraries and plugins, FFmpeg, GLib and others, as separate dynamic libraries in `Contents/MacOS/lib`.
- Data files: the ad and tracker filter lists, Servo's default resources.

## The obligation map

| Duty | Where Mercury meets it | How to check |
| --- | --- | --- |
| Copyright notice and licence text of every Rust crate under MIT, BSD, ISC, Zlib, Unicode, CC0 or Apache-2.0, in a binary distribution | The credits page `credits.html`, generated for each build: `Contents/Resources/credits/credits.html` in the application, `credits/credits.html` in the Linux archive. **About, Third-party notices** opens it. | `make credits-check`; open the page and search it for a crate; the release steps below |
| The `NOTICE` file of every Apache-2.0 crate that has one, and `PATENTS` files (rav1e, for one) | The same page, in blocks of their own. The tool always keeps these. | The page's crate index and its `NOTICE` blocks |
| MPL-2.0 §3.2: say where the source of MPL parts is | The page's source section: a crates.io link for each registry crate that is MPL-2.0, and for each vendored crate the repository tag and upstream commit. **About, Source code** opens the repository. | The `#source` section of the page; **Source code** in About |
| MPL-2.0 for Mercury's own files (the Hermes-derived and adapted files) | The files themselves, published with the release tag in the repository; the tag is the publication | `git grep -l 'SPDX-License-Identifier: MPL-2.0'` lists them; `scripts/ci/check-license-headers.sh` checks that every file has a licence |
| LGPL-2.1 for GStreamer, FFmpeg, GLib, proxy-libintl and libnice (libnice is taken under MPL-1.1) | Separate, replaceable dynamic libraries; the licence text in `LICENSES/LGPL-2.1.txt`; each project's own licence files in `Contents/Resources/gstreamer-licenses/`; the corresponding source attached to every release as `Mercury-Browser-<version>-lgpl-sources.tar`; a written offer valid for three years in `THIRD-PARTY-NOTICES.md` | `scripts/ci/check-macos-bundle.sh --require-media`; the "LGPL library sources" job of `release.yml`; [LGPL sources and credits](releases.md#lgpl-sources-and-credits) |
| Independent JPEG Group credit (libjpeg-turbo): "This software is based in part on the work of the Independent JPEG Group." | `NOTICE`, `THIRD-PARTY-NOTICES.md`, the installation guide, the release notes | `grep -l "Independent JPEG Group" NOTICE THIRD-PARTY-NOTICES.md docs/user-guide/installation.md` |
| Slint Royalty-free License 2.0: show Slint's attribution | `AboutSlint` in the About window, reachable from the application menu and the browser menu | The About window render test in `crates/mercury-ui/tests/render_menus.rs`; look at **About** in the build |
| EasyList and EasyPrivacy under CC-BY-SA-3.0: credit "The EasyList authors (https://easylist.to/)" and keep the share-alike for the lists | `THIRD-PARTY-NOTICES.md` ("Bundled data"), `LICENSES/CC-BY-SA-3.0.txt`, the lists as separate data files with a manifest (`Contents/Resources/filter-lists/`), never compiled into the executable | `scripts/ci/check-macos-bundle.sh` (the executable carries no filter-list text) |
| The PolyForm "Required Notice" and the Hermes credit | `NOTICE`, also in the disk image next to the application | The bundle checks; the tests that read `NOTICE` |
| Vendored crates under `third_party/` keep their own licences | Each directory's own licence files and `MERCURY.md`; `third_party/vendor.toml` records origin and commit; the credits page links the source | `make servo-patches-check`, [Vendored patches](vendored-patches.md) |

The first two rows are the ones the credits tool exists for. The rest were met earlier by the release and documentation work and are listed so that one page shows all of them.

## The credits page and its tool

`crates/mercury-credits` is a Rust tool (package and binary `mercury-credits`, in the workspace, built in the light pool because it does not depend on Servo or Slint). It writes one self-contained page with the licence text, copyright line, NOTICE and MPL source pointer of every crate that is built into the application, and `check` fails when a crate cannot be credited.

### What it reads

- **The set of crates**: `cargo tree --locked -p mercury-desktop -e normal,build --target <triple> --prefix none --format '{p}'`. With `--runtime-only` the build edges are dropped; the default keeps them, because build scripts compile native sources and crediting a few build-only crates too is the safe side.
- **The details** (licence expression, repository, authors, location of the package on disk): `cargo metadata --locked`, unfiltered. `--filter-platform` is not used because it drops packages that `cargo tree` still lists.
- `deny.toml`, for which licences Mercury may use (below), `third_party/vendor.toml` for the origin of each vendored crate, `Cargo.lock` (its digest is printed in the page), `NOTICE` and `THIRD-PARTY-NOTICES.md` (embedded in the page), `LICENSES/<SPDX id>.txt` for standard texts, and `crates/mercury-credits/credits.toml`.

The sources of every crate must be on disk: run `cargo fetch --locked` first on a clean machine. The tool never touches the network.

### Generate and check

From the repository root:

```bash
cargo run --release --locked -p mercury-credits -- check --package mercury-desktop --target aarch64-apple-darwin
cargo run --release --locked -p mercury-credits -- generate --package mercury-desktop --target aarch64-apple-darwin --out dist/credits
```

(through `scripts/dev/build-slot.sh`, as every build is.) `make credits-check` runs `check` for the target of the host it runs on (`aarch64-apple-darwin` on a Mac, `x86_64-unknown-linux-gnu` on Linux), and `make credits` runs `generate` into `target/<profile>/credits`; to check the other target, run the command above by hand. `generate` writes `<out>/credits.html` through a temporary file and a rename, and writes nothing when there is a problem. Both commands exit 0 (warnings allowed), 1 (problems) or 2 (usage), and print messages starting `mercury-credits:` on standard error. The last line of a successful run is the count of crates credited, how many are registry, vendored and Mercury's own, how many distinct licence texts the page holds, and how many crates are on a standard text.

The two targets in use are `aarch64-apple-darwin` (the application) and `x86_64-unknown-linux-gnu` (the Linux archive). The set differs: the Linux target has the Wayland, X11 and bundled FreeType crates, the macOS one the Apple framework crates.

### How the licences are chosen

The tool parses each crate's SPDX expression (`OR`, `AND`, `WITH`, parentheses, `LicenseRef-*`, and the old `/` for `OR`) and compares every term with `deny.toml`. A term is taken when it is on the global allow-list (`[licenses] allow`) or in the crate's own `[[licenses.exceptions]]` entry. So `deny.toml` is the single place where the owner's policy is written:

- `A OR B`: every taken branch is credited, and no branch Mercury may not take. A superset is harmless. `r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`) gets MIT and Apache-2.0; its LGPL text is left out and counted in the summary.
- `A AND B`: every term must be taken, and every one is credited (`servo-script-bindings`: `MPL-2.0 AND BSD-3-Clause`).
- Slint's `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`: only the Royalty-free branch is taken, because only that is in the crate's exception. The GPL text is never credited.
- No term taken is a problem: the crate would also fail `make deny`.

The licence files it reads in each package are those in the package root, in `LICENSES/` or `licenses/`, and the paths `credits.toml` names. A file name starting with LICENSE, LICENCE, COPYING, UNLICENSE, NOTICE, COPYRIGHT or PATENTS counts, in any case and with any suffix (a source file such as `license.rs` does not). The tool classifies each text by its words (Apache-2.0, MIT, BSD-2 and BSD-3, ISC, Zlib, MPL-2.0, Unicode-3.0, CC0, Unlicense, BSL-1.0, NCSA, CDLA-Permissive-2.0, Slint, GPL, LGPL, AGPL). A text of a licence that is not taken is left out and counted. A text it cannot classify is kept as it is and reported as a warning until a person has read it and added the crate to `[accept_unclassified]`.

When a crate has no file for a taken licence (139 of the 870 non-Mercury crates ship no licence file at all, and on 19 November 2026 140 crates of the macOS target and 124 of the Linux target needed the standard text), the tool uses the standard text from `LICENSES/<id>.txt` and builds the copyright line from `credits.toml`, else from the package's `authors`, else "the <name> authors (<repository>)". The page marks those crates as standard-text crates. The MIT, BSD, ISC and Zlib texts in `LICENSES/` are templates with exactly one line, `Copyright (c) <year> <copyright holders>`, which the tool replaces; the other standard texts are verbatim. This is the conservative position without network access, and is what tools such as cargo-about do. A later round with network access can replace the standard texts with each upstream's own file, which carries the true copyright line.

### credits.toml

The file is `crates/mercury-credits/credits.toml`. Its comments say what each entry is for.

| Table | Use it when |
| --- | --- |
| `[[extra]]` `crate`, `files`, `note` | A package bundles native code with its own licence files deeper than the package root (`mozjs_sys`, `aws-lc-sys`, `mozangle`, `fontsan`, `harfbuzz-sys`, `zstd-sys`, `libz-sys`, `glslopt` and a few others). `files` are relative to the package root; every path must exist. Name only projects that are compiled in, never a package's build or test tooling. An entry with `files = []` records that a crate with a `links` key was read and bundles nothing else; without any entry, `check` warns about such a crate. |
| `[[clarify]]` `crate`, `licence`, `copyright`, `source` | A package has no licence file, or its `authors` do not name the holder, and a person has read the holder in a README or a source header. `copyright` is the line to put in the standard text; `source` is where it was read (for maintainers, not shown). Never invent a holder: a crate with neither authors nor a line a person has read stays on the tool's fallback line. |
| `[[ignore]]` `crate`, `reason` | A crate in the set that is not credited, such as a build tool whose output holds none of its code. The reason is printed as a warning on every run. None is used today. |
| `[accept_unclassified]` `crates` | A crate whose licence files have wording the tool does not know, read and accepted by a person. Every entry needs a comment saying what was read. |

An entry for a crate that is not in the application for the target being checked draws a warning ("names crates that are not in the app for ..."). The entries for the Linux-only crates (FreeType, the Wayland protocols, Fontconfig) therefore warn when checking the macOS target, and the one for `objc-sys` when checking the Linux target. The warning is expected; the comments in `credits.toml` mark which entries are for which target.

### When `check` fails

A failure after a Servo, Slint or dependency bump is how the page stays honest, and it has a small number of causes:

- **A new crate has no licence in `Cargo.toml`.** Add a `[[clarify]]` entry with `licence` after reading the package.
- **No licence of a crate is allowed.** The crate fails `make deny` too. It is a decision for the owner whether to add the licence to the allow-list; never add it just to make the check pass.
- **A taken licence has no file and no standard text.** Add `LICENSES/<SPDX id>.txt`, taking the wording from a copy in the registry.
- **An `[[extra]]` file is not in the package.** A bump moved or removed it (`mozjs_sys` reorganises its tree on every release). Look at the new package, find the new path, check that the bundled project is still compiled in, and update the entry.
- **A path crate is neither a workspace member nor in `third_party/vendor.toml`.** A vendored crate was added without the tool that records it (`mercury-servo-patches`).
- **New warnings.** A new unclassified file: read it, then accept the crate. A new crate with `links` and no `[[extra]]`: read the package for bundled projects before silencing. A stale entry: remove it.

After a Slint bump, also re-read Slint's terms (see [Slint's terms](#slints-terms)); the check can only see that the licence expression is still on the exception list.

### Determinism and why the page is not committed

The output is sorted, has no timestamps, and shows the workspace version and the first 12 hex digits of the `Cargo.lock` digest, so a tag reproduces it. It is generated at package time and not committed because `Cargo.lock` changes in almost every change, and a committed generated file of two to four megabytes would conflict in every merge and go stale. The page has no JavaScript and no external resources, escapes every string, and opens in Mercury itself from a `file://` address, because licence texts from a package are untrusted input.

### Limits

The tool shows whole files. Notices that exist only in a source-file header, not in a licence file, cannot be shown, and `THIRD-PARTY-NOTICES.md` lists the ones that were found (SpiderMonkey's WebAssembly, VIXL, Apple-derived, Blink decimal and Float16 files, and ICU). It does not read the code to find every bundled project: `credits.toml` is only as complete as the person who read each `-sys` package. It does not check patents. It does not check data files (Servo's default resources, the filter lists): those are in `THIRD-PARTY-NOTICES.md`.

## The LGPL libraries and the commercial licence terms

The GStreamer packages Mercury bundles carry LGPL-2.0-or-later and LGPL-2.1-or-later libraries (GStreamer, FFmpeg 7.1, GLib, proxy-libintl), with libnice taken under its MPL-1.1 option. Mercury meets the duties as follows (the "Bundled libraries" section of `THIRD-PARTY-NOTICES.md` is the authoritative text): the libraries are loaded at run time from `Contents/MacOS/lib` (the shared-library mechanism of LGPL-2.1 section 6b), any of them can be replaced, the licence text and each project's licence files ship, and the source is a release asset with a three-year written offer (section 6c and 6d).

### What a commercial licence or EULA must not do

LGPL-2.1 section 6 allows a work that uses the library to be distributed "under terms of your choice, provided that the terms permit modification of the work for the customer's own use and reverse engineering for debugging such modifications". The audit (R16, P4.2) reads this as three requirements on any commercial licence or end-user licence agreement for Mercury:

1. It must permit modifying the application for the user's own use.
2. It must permit reverse engineering the application to debug such modifications.
3. It must not forbid replacing the bundled LGPL dylibs with a modified, interface-compatible version.

**Mercury has no end-user licence agreement today.** The files written for Mercury are under the PolyForm Noncommercial License 1.0.0, which has no clause about reverse engineering (a search of the licence text and of `LICENSE` for "reverse", "decompil" and "disassembl" found nothing on 19 November 2026) and none about replacing libraries, and the bundle's replacement steps are in `THIRD-PARTY-NOTICES.md`. The requirements bite when the owner writes the commercial licence. This draft is for counsel, not legal advice:

> Notwithstanding any other term of this licence, you may modify the Software for your own use, and you may reverse engineer the Software to the extent needed to debug those modifications. You may replace any library that is distributed with the Software as a separate file and is covered by the GNU Lesser General Public License with a modified or other interface-compatible version of that library. Nothing in this licence restricts those rights.

Points for counsel that are not settled by the audit and are **not verified**:

- LGPL-2.1 section 6 also asks for "prominent notice with each copy of the work that the Library is used in it", and, when the work displays copyright notices while it runs, the library's copyright notice and a reference to the licence. The About window names Mercury's copyright and links **Third-party notices**; whether that satisfies the sentence, or whether the window should add a line naming the LGPL libraries, is for counsel. Adding one line is cheap.
- A Developer ID build has library validation on, so it loads only libraries signed by its own team. A replaced library needs the application to be signed again (ad hoc is the documented route, which gives up the Developer ID signature and notarisation). Whether that satisfies section 6b, "will operate properly with a modified version of the library, if the user installs one", is for counsel.
- A Mac App Store release is a separate question and is not planned: its terms and library validation clash with the replaceability that the LGPL asks for (R16, P4.4). Stay with Developer ID distribution.
- libnice is taken under MPL-1.1 so as to avoid the LGPL duties for that library. `THIRD-PARTY-NOTICES.md` says so.

## Slint's terms

Slint 1.17.1 (held at `~1.17.1` in `Cargo.toml`) is offered under `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`. Mercury takes the **Slint Royalty-free Desktop, Mobile and Web Applications License 2.0**, a free proprietary licence (it is not open source; the open-source branch is the GPL, which Mercury does not use). `deny.toml` allows it for the named Slint crates only. The audit (R16 section 3) reads the terms as follows. The Royalty-free text is in each Slint crate's `LICENSES/` directory and is the authority.

- **Grant:** a worldwide, royalty-free, non-exclusive licence to use, modify and distribute Slint as part of a desktop, mobile or web application. It contains no non-commercial limit, so a paid, closed-source application is allowed.
- **Attribution:** the `AboutSlint` widget in an About dialog reachable from the top-level menu, or the "Made with Slint" badge on the public download page. Mercury does the first: `AboutSlint` is in the About window, reached from the macOS application menu and the browser menu. Keep it there.
- **No embedded systems.** A kiosk, car or other embedded build of Mercury would need the paid licence.
- **No exposure of Slint's APIs**, in part or in total. Never ship themes, extensions or scripting that let third parties write `.slint` code or call Slint through Mercury. This is why `i-slint-backend-winit` is vendored in `third_party/` (so that Mercury's changes stay inside the application) and why nothing of Slint is exposed to themes or extensions.
- Slint may not be distributed on its own, and its licence notices must stay in its source: the vendored backend keeps its `LICENSES/` directory and every notice as published (see `third_party/i-slint-backend-winit/MERCURY.md`).
- **The grant attaches to the version received.** Slint can change the terms of later releases, so re-read them at every Slint upgrade and record the result in the pull request. R16's options if the terms change or "fully open source" becomes a requirement: stay on the Royalty-free licence; buy a paid plan (per-seat, with a startup plan for small companies; check the current price page, which R16 read on 18 November 2026); or move to a permissively licensed toolkit.

## Codecs and patents

This section records facts. Patent law is outside what a repository can settle: **lawyer sign-off on codec patents stays with the owner** before a commercial release.

### What the bundle contains today

The macOS bundle's FFmpeg is the GStreamer 1.28.9 package's FFmpeg 7.1 with cerbero's patches (`libavcodec.61.dylib` and the other `libav*` libraries), loaded through the plugin `libgstlibav.dylib`. Checked on 19 November 2026 on the unpacked packages in `~/Library/Caches/mercury-browser/gstreamer/1.28.9/`:

- `strings` on `libavcodec.61.dylib` shows "libavcodec license: LGPL version 2.1 or later", and the build's meson options record `-Dnonfree=disabled -Dversion3=disabled`. The copyright side is clean: an LGPL build, with no GPL or non-free parts and no x264, x265 or fdk-aac.
- The audit (R16 P2, `nm` and `strings` on the same library, 18 November 2026) found 181 encoders and 515 decoders, including software decoders for H.264, HEVC, VVC, AAC, AC-3, E-AC-3, DTS, TrueHD, MPEG-2, MPEG-4 Part 2 and MP3, and encoders for AAC, AC-3, E-AC-3, DTS, TrueHD, H.263, MPEG-2 and MPEG-4. A browser never needs the encoders.
The element ranks, read with `gst-inspect-1.0` from the same GStreamer 1.28.9 packages on 19 November 2026 (the x86_64 slice of the full framework; Mercury's bundle holds a subset of the plugins, including `libgstlibav`, `libgstapplemedia` and `libgstosxaudio`):

| Element | Plugin | Rank |
| --- | --- | --- |
| `vtdec_hw` (VideoToolbox, hardware) | `applemedia` | primary + 1 (257) |
| `vtdec` (VideoToolbox, software) | `applemedia` | secondary (128) |
| `avdec_h264`, `avdec_h265`, `avdec_aac`, `avdec_mpeg2video` (FFmpeg) | `libav` | primary (256) |
| `atdec` (AudioToolbox) | `osxaudio` | marginal (64) |
| `avdec_ac3`, `avdec_eac3`, `avdec_dca`, `avdec_truehd`, `avdec_mp3` | `libav` | marginal (64) |

By rank, H.264 and HEVC go to VideoToolbox first, falling back to FFmpeg when VideoToolbox refuses a stream, and AAC goes to FFmpeg's `avdec_aac`, because `atdec` ranks lower. The vendored `servo-media-gstreamer` sets no ranks (searched on 19 November 2026): it plays through GStreamer's `GstPlay`, a `playbin3` pipeline, whose `decodebin3` picks decoders by rank.

### Which of those are patent-encumbered

H.264, HEVC, VVC and AAC are covered by patent pools, and MP3, MPEG-2 and AC-3 patents have expired (R16 section 4; not re-checked). The status of E-AC-3, DTS and TrueHD patents is **not verified**. VP8, VP9, AV1, Opus, Vorbis, FLAC and Theora are royalty-free with the WebM, AOM and Xiph grants, although pools such as Sisvel assert VP9 and AV1 claims, which browsers generally ignore.

### What is not verified, and what the fix is

- Whether Servo's pipeline always picks `vtdec_hw` over `avdec_h264`. The ranks say it does unless VideoToolbox refuses; no end-to-end check was made.
- `atdec`'s behaviour for the AAC streams that browsers meet. Its rank is marginal.
- Whether Servo 0.8 runs without `libgstlibav.dylib`. The collector can leave the plugin out (`MERCURY_GSTREAMER_EXCLUDE_PLUGINS=libav`, tested in `scripts/macos/test-media.sh`), but no media playback test was made without it.
- The patent status of E-AC-3, DTS and TrueHD.

The audit's fix (R16 P2) is to route H.264, HEVC and AAC through Apple only (`atdec` above `avdec_aac`; `vtdec_hw` and `vtdec` above `avdec_h26x`), to remove the encumbered codecs and all encoders from FFmpeg (either by leaving `libgstlibav` out or by building a royalty-free-only FFmpeg in cerbero), and to drop or license VVC, E-AC-3, DTS and TrueHD. **That routing change is separate work and has not landed.** Until it has, no shipped notice says anything about which decoder is used, and `THIRD-PARTY-NOTICES.md` states only that Mercury grants no patent licence.

### The release-checklist test for when the routing lands

Play `tests/media/clip-h264-aac.mp4` (through `tests/pages/media.html`) with `GST_DEBUG=*:3` and confirm that only `vtdec*` and `atdec` decoders appear in the log, and no `avdec_*`. `scripts/macos/test-media.sh` checks the media scripts today; the codec test is not written and should go beside it. Add the result to the release checklist.

## MPL-2.0 files

Mercury's own MPL-2.0 files are the Hermes-derived files and the files adapted from Servo, Zervo, Zephium and Mozilla's application-services (105 files carry an MPL-2.0 SPDX header on 19 November 2026, and `REUSE.toml` lists more; R16 counted 99 on 18 November). The owner cannot relicense them to closed source. MPL-2.0 allows a closed commercial product, with these duties (R16 P7):

- **Publish the source of those files, with Mercury's changes, for each release.** The release tag in the public repository is the publication, and the credits page says so and links the tag. Do not delete a tag.
- **The binary must say where the source is** (MPL-2.0 §3.2): **About, Source code** and the page's source section.
- **Keep proprietary logic out of MPL files.** New closed code goes in new files. The only way to move a Hermes-derived file out of MPL is to rewrite it from scratch (clean room).
- A vendored Servo crate patched by Mercury stays MPL-2.0 and its changes are published with the tag (`third_party/patches/`, [Vendored patches](vendored-patches.md)). `i-slint-backend-winit` is the exception: Slint's licence, not MPL.

## Before adding a component

First the generic checks, for any new dependency, library or data file:

1. Is the licence on the `deny.toml` allow-list? If not, it is the owner's decision, never an edit to make `make deny` pass.
2. Does the crate bundle native code with other licences? Read the package and add an `[[extra]]` entry.
3. Does it ship prebuilt binaries? Pin each by SHA-256 for both arm64 and x86_64 (or a universal build), and say so when only one architecture exists.
4. Add it to `THIRD-PARTY-NOTICES.md` and its licence text to `LICENSES/` (and make the bundle carry it) if it is not a Rust crate; run `make credits-check` if it is.
5. LGPL libraries are linked dynamically only. GPL libraries are never linked or bundled.

The planned components that the audit (R16 section 1f and P8) already looked at:

| Component | What the audit found | Before you add it |
| --- | --- | --- |
| **PDFium** (bblanchon/pdfium-binaries, `chromium/8165`; no V8, no XFA; run in a helper process) | BSD-3-Clause, with bundled abseil, agg23, dragonbox, fast_float, **FreeType (FTL)**, HarfBuzz, ICU, highway, lcms2, **libjpeg-turbo (IJG)**, OpenJPEG, libpng, llvm-libc, simdutf and zlib. `pdfium-render` is MIT OR Apache-2.0. | Carry every text in the tarball's `licenses/` folder in `LICENSES/`, in `THIRD-PARTY-NOTICES.md` and in the bundle. Add the FreeType credit line ("Portions of this software are copyright © <year> The FreeType Project (www.freetype.org). All rights reserved.", with the year of the version) and keep the IJG sentence. Pin the SHA-256 for arm64 and x86_64. |
| **libwebrtc** | LiveKit's prebuilt binary is **not usable as shipped**: it links FFmpeg statically (LGPL) and compiles OpenH264 from source (H.264 patent exposure), and its build script does not verify the download. Its Rust crates (`libwebrtc`, `webrtc-sys`) are Apache-2.0. | Use the Shiguredo prebuilt without H.264 and H.265, pinned by SHA-256 (the audit did not examine that build: check its licence files and run `nm` for `ff_`, `avcodec_` and `Wels` symbols first; **not verified**), or GStreamer's `webrtcbin`. Carry WebRTC's `PATENTS`, libvpx's `PATENTS` and the AOM patent licence. H.264 for WebRTC only through VideoToolbox, never OpenH264 from source and never x264. |
| **Bergamot translator** | MPL-2.0; marian-dev MIT; its own dependencies (intgemm, ruy, sentencepiece, ssplit, pcre2) were **not verified** item by item. | Read the full dependency list and its licence files before adopting. Never `slimt` (GPL-2.0). |
| **Translation models** | The archived `mozilla/firefox-translations-models` repository is MPL-2.0. The licence of the current Google-hosted models and of their training data is **not verified**; Mozilla's CDN terms for other clients are **not verified**. | Use models from a source with a stated MPL-2.0 licence, re-hosted on Mercury's own release servers. Ask Mozilla about the newer ones. |
| **Hunspell** | MPL-1.1 OR GPL-2.0+ OR LGPL-2.1+; take MPL-1.1 (file-level source availability). `hunspell-sys` is MIT/Apache and `hunspell-rs` Apache-2.0. | Say in the notices that MPL-1.1 is taken. Audit every dictionary (many LibreOffice dictionaries are GPL, LGPL or MPL; en_US is permissive), or use macOS `NSSpellChecker` through `objc2` and ship none. |
| **ngtcp2 and nghttp3 crates** | The C libraries are MIT. The crates.io `ngtcp2` and `ngtcp2-sys` 0.2.0 declare a "non-standard" licence. | Read that licence first, or write the binding yourself. |
| **slimt** | GPL-2.0. | Never. |
| **EasyList and EasyPrivacy** | GPL-3.0-or-later or CC-BY-SA-3.0-or-later; Mercury takes CC-BY-SA. Attribution "The EasyList authors (https://easylist.to/)", and share-alike on the lists and on adaptations of them, though not on the program while the lists stay separate data files. | Keep the credit and the lists as data files, never compiled into the executable. The uBlock filters (GPL-3.0 only) were dropped on 18 November 2026; do not add them back. |
| **gst-plugins-rs** (`webrtc`, `rtp`) | MPL-2.0 for those two; other plugins vary. | Check each plugin before bundling it. |
| **GstMSE** | LGPL-2.1; dynamic. | The LGPL duties above apply to it too. |
| **Skia renderer** (Slint's, in the lock file only) | rust-skia downloads a prebuilt binary that includes FreeType (FTL) and ICU. | Keep the renderer off, or audit that binary first. |

## Release steps for the credits page

These use the names that the Makefile, `bundle.sh` and the workflows agree on. A macOS bundle cannot be built on an Intel Mac (`scripts/macos/bundle.sh` is Apple Silicon only), so CI builds it; the check can run anywhere.

1. **Check.** `make credits-check` runs `mercury-credits check` for the host's target. The "Fast checks" job of `ci.yml` runs it on both of its legs (`aarch64-apple-darwin` on macOS, `x86_64-unknown-linux-gnu` on Linux) after `cargo fetch --locked`, so a new dependency, or a Servo or Slint bump, that adds a crate nothing can credit fails in the pull request and not at the release. It must exit 0 with no unexplained warning.
2. **Generate.** `make app-macos` and `make package-macos` generate the page for `aarch64-apple-darwin` into `target/release/credits` and pass it to `scripts/macos/bundle.sh --credits <dir>` (or `MERCURY_CREDITS_DIR`). `bundle.sh` refuses to build a bundle without a usable page (`credits_problem` in `scripts/macos/lib.sh`: present, at least 100,000 bytes, written by `mercury-credits`, at least 500 crates, no `<script`), because a bundle without credits is a compliance bug, not a degraded build. `make package-linux` generates the Linux page into the archive's `credits/` directory and applies the same rule.
3. **Carry it to signing.** The macOS build job records the SHA-256 of `target/release/credits/credits.html` as the job output `credits-sha256` and uploads the file as the artefact `credits-macos-arm64`. The signing job has no cargo by design: it downloads that artefact by exact name, checks that it holds only the page and that its digest equals `credits-sha256`, passes it to `bundle.sh --credits`, and after building checks that the page in the disk image has the same digest. The publishing job does not take the page on its own; it is inside the disk image and the Linux archive. See [Continuous integration](continuous-integration.md#releaseyml-tags-v-manual-runs).
4. **Look at it.** In the finished build, **About** shows the licence line, **Third-party notices** opens the bundled page (a `cargo run` build, which has none, falls back to the notices on GitHub), **Source code** opens the repository, and `AboutSlint` is visible. [Releases](releases.md#before-the-first-public-release) has the checklist item. The page is `Contents/Resources/credits/credits.html` in the application, `credits/credits.html` in the Linux archive, and `target/<profile>/credits/credits.html` after `make credits`.
5. **After any dependency change**, expect `credits.toml` to need an update (see "When `check` fails"), and re-read Slint's terms on a Slint bump.

## Open items and what is not verified

- **Notices that live only in source headers.** SpiderMonkey's WebAssembly code (Apache-2.0), VIXL, the Apple-derived files, Blink's decimal code and `Float16.h`, and ICU are reproduced in `THIRD-PARTY-NOTICES.md`, not in the page's licence blocks, because the tool shows whole files. The ICU `LICENSE` file, which lists ICU's own third-party parts, is not in the `mozjs_sys` package, and neither are the Dart and Go licence files that `BigIntType.cpp` refers to; they were not fetched (no download was allowed). A round with network access should read Mozilla's and ICU's own notices. These notices were searched for under `js/`, `mfbt/`, `mozglue/`, `modules/` and `memory/` of `mozjs_sys` 153.6.0-1; other trees were not.
- **Standard-text crates.** Crates without a licence file are credited with a standard text and a built copyright line, flagged on the page (140 on the macOS target, 124 on the Linux target on 19 November 2026; 77 of the macOS ones are MPL-2.0, which has no copyright line). Crates with neither a licence file nor `authors`, and no holder in a README or header, are on the tool's fallback line "the <name> authors (<repository>)": the `objc2-*` framework crates, `nom-rfc8288`, `ohos-deviceinfo-sys`, `rspolib` and `zune-inflate` (no repository either). Replace all of them with upstream's own file when network access allows.
- **The Linux archive.** `wayland-protocols-plasma` (via winit, Linux only) embeds KDE protocol definitions that are LGPL-2.1-or-later, although the crate is declared MIT, and its generated code is compiled into the Linux executable. The macOS application does not contain it. The Linux archive is a preview artefact; settle this before it is distributed as a product. The bundled FreeType is credited under the FreeType Licence, with its credit line in `THIRD-PARTY-NOTICES.md`.
- **Licences not on the allow-list.** `regex-syntax`'s Unicode tables are under the Unicode-DFS-2016 text, and `vello_shaders`' shaders under the Unlicense, although `deny.toml` does not list those licences (cargo-deny reads only the `license` field of `Cargo.toml`). They are credited because `credits.toml` names the files, and the tool says so in a warning on every run. The Wayland protocols' `COPYING` (Linux) is the X.org "MIT Expat" wording, which the tool classifies as MIT-0.
- **Data files** in the vendored `servo-default-resources` (the public suffix list, the HSTS preload list, the GATT blocklist from the Web Bluetooth Community Group's registries, whose header names its licence file) have origins that were read in their headers but whose licences were not checked.
- **Native code not compiled in on these targets** was left out on purpose: Intel's ittnotify (SpiderMonkey, VTune builds only), NSPR, the dav1d-derived assembly of `rav1e` (its `asm` feature is off), SQLCipher, zlib-ng and the Vulkan loader in ANGLE's tree. Re-check the list on every Servo bump.
- The patent questions, the services' terms, and the not-verified items in the tables above.
