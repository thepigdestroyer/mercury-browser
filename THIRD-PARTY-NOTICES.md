# Third-party notices

This file records the projects that Mercury Browser's source code is derived from or copies code
from, together with their licences. Mercury began as a fork of Hermes Browser, listed first; the
files that came from Hermes, or contain code from it, carry an `SPDX-License-Identifier: MPL-2.0`
header. Every file that contains code copied or adapted from any other project names that source
at the top, has a matching entry here, and stays under the MPL-2.0 (with the upstream licence of
the adapted portions noted where it differs). Vendored dependencies under `third_party/` keep their
own licence files. Mercury's own licences are explained in `LICENSE`, and its copyright notice is
in `NOTICE`.

- Hermes Browser — https://github.com/KaykCaputo/hermes-browser — MPL-2.0, Copyright © 2026 Kayk Caputo — Mercury began as a fork of this project at Hermes commit `a41f18e`; the parts of Mercury derived from it remain under the MPL-2.0.
- Servo `servo-canvas` 0.8.0 — https://github.com/servo/servo, commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/canvas` — MPL-2.0 — vendored in `third_party/servo-canvas` with a small fix to its GPU (vello) backend; see `third_party/servo-canvas/MERCURY.md`.
- Servo `servo-script`, `servo-script-bindings`, `servo-media-streams` and `servo-media-gstreamer` 0.8.0 — https://github.com/servo/servo, commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`, paths `components/script`, `components/script_bindings`, `components/media/streams`, `components/media/backends/gstreamer` — MPL-2.0 (`servo-script-bindings` also BSD-3-Clause for its bundled PLY) — vendored in `third_party/` with Mercury's camera and microphone patch; see `third_party/servo-script/MERCURY.md`.
- Servo, post-0.8.0 changes — https://github.com/servo/servo, pull requests #49541, #47459, #49649, #48253, #49797, #49818, #49884, #49917, #49920, #49632, #49912, #49436, #49981 and #49907 (merge commits are listed in the `MERCURY.md` of each crate) — MPL-2.0 — carried as patches in `third_party/servo-script`, `servo-script-bindings`, `servo-script-webcrypto`, `servo-layout`, `servo-layout-api`, `servo-net` and `servo-webgl` until the next Servo release contains them; each hunk starts with a `Mercury: backport of servo/servo#<n>` comment and each `MERCURY.md` says when it is dropped. #49884, #49917 and #49920 are hand-ported to 0.8.0 APIs. The new file `outdent.rs` (`third_party/servo-script/dom/execcommand/commands/`) is upstream's and keeps its MPL-2.0 header.
- Zervo `patches/servo/0002-script-show-notification-without-resources.patch` — https://github.com/Goddv/Zervo — MPL-2.0 — the idea (show a notification at once when it has no icon, image, badge or action icon to fetch) in `third_party/servo-script/dom/serviceworker/notification.rs`; the code is written from its description, not copied; see `third_party/servo-script/MERCURY.md`.
- Servo `python/servo/gstreamer.py` — https://github.com/servo/servo, tag `v0.8.0` (commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`), functions `package_gstreamer_dylibs` and `load_plugin_libraries_from_text_file` — Apache-2.0 OR MIT, Copyright 2013 The Servo Project Developers — adapted as `scripts/macos/collect-gstreamer.sh`.
- Servo `ports/servoshell/build.rs` — https://github.com/servo/servo, tag `v0.8.0` — MPL-2.0 — the macOS `@executable_path/lib/` rpath in `apps/mercury-desktop/build.rs`.
- Zervo `scripts/bundle-macos.sh` — https://github.com/Goddv/Zervo, commit `86cb7126` — MPL-2.0 — the `lipo` thinning step in `scripts/macos/collect-gstreamer.sh`.
- Slint Servo example — https://github.com/slint-ui/slint, tag `v1.17.1` (commit `cf62c975c311e7036d599ed8ed0b7e6a8386a934`), path `examples/servo/src/webview/rendering_context/metal.rs` — MIT, Copyright © SixtyFPS GmbH <info@slint.dev> — the macOS IOSurface → Metal frame import and its flip pass in `apps/mercury-desktop/src/rendering/metal.rs` are adapted from this file. The MIT text is in `LICENSES/MIT-slint-examples.txt`. `apps/mercury-desktop/src/rendering/surfman.rs` and `apps/mercury-desktop/src/rendering/vulkan.rs` follow the same example's `surfman_context.rs`, `gpu_rendering_context.rs` and `vulkan.rs` step for step.
- Servo `servo-paint-api` 0.8.0 — https://github.com/servo/servo, tag `v0.8.0` (commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`), path `components/shared/paint/rendering_context.rs` — MPL-2.0 — the macOS frame read-back in `apps/mercury-desktop/src/rendering/metal.rs` is adapted from `Framebuffer::read_framebuffer_to_image`; `SurfmanContext` in `apps/mercury-desktop/src/rendering/surfman.rs` follows Servo's `SurfmanRenderingContext`.
- Zervo — https://github.com/Goddv/Zervo, commit `86cb7126` — MPL-2.0 — `scripts/bundle-macos.sh` (deployment-target lookup, Info.plist keys, inside-out signing, disk image) adapted in `scripts/macos/bundle.sh` and `apps/mercury-desktop/resources/macos/Info.plist.in`; `scripts/make-icns.sh` adapted in `scripts/macos/make-icns.sh`; `assets/macos/entitlements.plist` adapted in `apps/mercury-desktop/resources/macos/entitlements.plist`.
- Servo 0.8.0 — https://github.com/servo/servo, tag `v0.8.0` — MPL-2.0 — `ports/servoshell/platform/macos/mod.rs` (the embedded `__TEXT,__info_plist` static) adapted in `apps/mercury-desktop/src/platform_macos.rs`; `ports/servoshell/platform/macos/Info.plist` adapted in `apps/mercury-desktop/resources/macos/Info.plist.in`; `support/macos/Servo.entitlements` adapted in `apps/mercury-desktop/resources/macos/entitlements.plist`.
- Servo 0.8.0 — https://github.com/servo/servo, tag `v0.8.0` — Apache-2.0 OR MIT — `etc/macos_sign.py` (hardened-runtime signing and `codesign --verify --deep --strict`) adapted in `scripts/macos/bundle.sh`.
- Servo servoshell `desktop/keyutils.rs` — https://github.com/servo/servo, tag `v0.8.0`, path `ports/servoshell/desktop/keyutils.rs` — MPL-2.0 — adapted in `apps/mercury-desktop/src/keyutils.rs` (winit → W3C key, code and location tables).
- Zephium — https://github.com/zephium-browser/Zephium — MPL-2.0 — commit `9026a1fffad74d6e201ef97fe8c02c1b01403cd2`. `crates/mercury-import/src/netscape.rs` adapts the Netscape bookmark writer from `crates/zephium-core/src/export.rs` (lines 28-92); `crates/mercury-import/src/{chromium.rs,firefox.rs,snapshot.rs}` adapt the browser-profile importers from `crates/zephium-import/src/{chromium.rs,firefox.rs,snapshot.rs}`.
- Zervo — https://github.com/Goddv/Zervo, commit `86cb71267066f3b1f402abbd5c26faaf24ddf165`, path `src/downloads.rs` — MPL-2.0 — the file-name sanitiser, URL-segment naming, unique naming and byte formatter (with their tests) are adapted in `crates/mercury-downloads/src/naming.rs` and `crates/mercury-downloads/tests/naming.rs`.
- Verso — https://github.com/versotile-org/verso, commit `eb719bdd6c7b4a22b640d33ea55c6d440a5a850d`, paths `src/utils/content_disposition_parser.rs` and `src/download.rs` — Apache-2.0 OR MIT (used under MIT; Copyright (c) 2020-2023 Ngo Iok Ui & Tauri Programme within The Commons Conservancy; the full MIT licence text and permission notice are in `LICENSES/MIT-verso.txt`) — the `Content-Disposition` parser is adapted in `crates/mercury-downloads/src/content_disposition.rs`, and the streaming write loop in `crates/mercury-downloads/src/transfer.rs`.
- Mozilla application-services — https://github.com/mozilla/application-services, commit `5f95eb6bafa36c4a86ff3109dacbf9026029833f`, path `components/places/src/frecency.rs` — MPL-2.0 — the frecency calculation is ported in `crates/mercury-store/src/frecency.rs`.
- Zephium — https://github.com/zephium-browser/Zephium, commit `9026a1fffad74d6e201ef97fe8c02c1b01403cd2`, paths `crates/zephium-store/src/hub/history.rs:617-640` (FTS5 query sanitiser) and `crates/zephium-store/src/migrations.rs:1944-1966` (FTS5 mirror and triggers) — MPL-2.0 — adapted in `crates/mercury-store/src/search.rs` and `crates/mercury-store/src/schema.rs`.
- Zervo — https://github.com/Goddv/Zervo, commit `86cb7126`, path `zervo-core/src/store.rs` (`write_private`, lines 80-123, and the ideas of its tests, lines 125-232) — MPL-2.0 — adapted in `crates/mercury-settings/src/atomic.rs` (exclusive, uniquely named temporary file; directory sync after the rename; see the file header). The settings patterns (a default for every field, first-run detection from a missing file, a migration test) and the starting list of search engines follow `src/settings.rs` at the same commit; no code from it is copied.
- Servo's servoshell 0.8.0 — https://github.com/servo/servo, tag `v0.8.0` (commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`), paths `ports/servoshell/panic_hook.rs`, `ports/servoshell/crash_handler.rs` and `ports/servoshell/backtrace.rs` — MPL-2.0 — adapted in `apps/mercury-desktop/src/panic_hook.rs` and `apps/mercury-desktop/src/crash_handler.rs`.
- Servo's servoshell 0.8.0 — https://github.com/servo/servo, tag `v0.8.0` (commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`), path `ports/servoshell/prefs.rs` lines 29-53 (`EXPERIMENTAL_PREFS`, the list of experimental web-platform preferences) — MPL-2.0 — copied into `crates/talaria-servo/src/runtime.rs`.
- Zervo — https://github.com/Goddv/Zervo, commit `86cb71267066f3b1f402abbd5c26faaf24ddf165` — MPL-2.0 — adapted from its files of the same names: `rust-toolchain.toml`; `deny.toml`; `.github/workflows/ci.yml` (from `.github/workflows/check.yml` and `.github/workflows/macos.yml`); `.github/workflows/release.yml` (macOS signing and publishing, from `.github/workflows/macos.yml` and `.github/workflows/release.yml`); `scripts/ci/check-macos-bundle.sh` (from the "Check the bundle" step of `.github/workflows/macos.yml`).
- Servo `servo-default-resources` 0.8.0 — https://github.com/servo/servo, commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`, path `components/default-resources` — MPL-2.0 — vendored in `third_party/servo-default-resources` without its resource-reader registration, so that Mercury can register its own (site security: Mercury's error pages); see `third_party/servo-default-resources/MERCURY.md`.

## Bundled libraries (macOS application)

`Mercury Browser.app` for macOS carries GStreamer 1.28.9 and the libraries it needs as separate
dynamic libraries in `Contents/MacOS/lib`. They are not part of Mercury Browser's source code
and keep their own licences; the table below lists them.

- **Origin.** They are copied by `scripts/macos/collect-gstreamer.sh` from the official GStreamer
  1.28.9 macOS packages (https://gstreamer.freedesktop.org/data/pkg/osx/1.28.9/,
  `gstreamer-1.0-1.28.9-universal.pkg` and `gstreamer-1.0-devel-1.28.9-universal.pkg`, SHA-256 in
  `scripts/macos/gstreamer.sh`). **They are not modified, except** that each is thinned from a
  universal (x86_64 + arm64) file to its arm64 code with `lipo -thin`, and signed again with the
  application's signature by `scripts/macos/bundle.sh`. The machine code and install names are
  unchanged.
- **Dynamic linking and replacement.** Nothing from them is linked statically into
  `mercury-browser`. The executable loads `libgstreamer`, `libgstplay` and the other libraries it
  names through the rpath `@executable_path/lib/`, and Servo loads the GStreamer plugins at run
  time from the same directory. Any of them can be replaced by a compatible arm64 build with the
  same file name: copy the application out of the disk image (which is read-only) into a folder
  you can write to, then copy the new file over the old one in
  `Mercury Browser.app/Contents/MacOS/lib`. Because the application is signed, changing a file
  invalidates its signature, so the application then has to be signed again, for example ad hoc:
  `codesign --force --sign - "<path to the library>"`, then
  `codesign --force --sign - --options runtime --entitlements apps/mercury-desktop/resources/macos/entitlements-adhoc.plist "Mercury Browser.app/Contents/MacOS/mercury-browser"`
  (the entitlements file is in the source tree of the same version) and the same for
  `"Mercury Browser.app"`. The ad-hoc entitlements turn library validation off, so libraries
  signed by someone else load. A Developer ID build has library validation on and loads only
  libraries signed by its own team; to use a replaced library with it, sign the whole
  application ad hoc as above (it then loses its Developer ID signature and notarisation).
  Finally remove the quarantine flag that macOS puts on downloaded files, from the application
  and from any library you downloaded: `xattr -dr com.apple.quarantine "Mercury Browser.app"`.
  Without that, macOS blocks the first launch of the re-signed (and so not notarised)
  application, and may refuse a quarantined library. `codesign --verify --deep --strict "Mercury Browser.app"`
  checks the result.
- **Corresponding source.** Every bundled library was built by GStreamer's build system,
  cerbero, at commit `efda998b6e8032a8ed785a70000a63d337898f82` ("Build 1.28.9 release",
  https://gitlab.freedesktop.org/gstreamer/cerbero/-/tree/efda998b6e8032a8ed785a70000a63d337898f82);
  its `recipes/<project>.recipe` names each project's source, checksum and patches, and the
  patches themselves are in `recipes/<project>/` at that commit. The versions in the table below
  are those the packages record in `share/versions.txt`. For the LGPL projects:
  - GStreamer 1.28.9, built from these releases without patches:
    https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-1.28.9.tar.xz,
    https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-1.28.9.tar.xz,
    https://gstreamer.freedesktop.org/src/gst-plugins-good/gst-plugins-good-1.28.9.tar.xz,
    https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-1.28.9.tar.xz and
    https://gstreamer.freedesktop.org/src/gst-libav/gst-libav-1.28.9.tar.xz.
  - GLib 2.82.4: https://download.gnome.org/sources/glib/2.82/glib-2.82.4.tar.xz (the recipe's SHA-256
    `37dd0877fe964cd15e9a2710b044a1830fb1bd93652a6d0cb6b8b2dff187c709`), **with 14 patches** from
    cerbero's `recipes/glib/` (GIO module paths, platform build fixes).
  - FFmpeg 7.1: https://ffmpeg.org/releases/ffmpeg-7.1.tar.xz (the recipe's SHA-256
    `40973d44970dbc83ef302b0609f2e74982be2d85916dd2ee7472d30678a7abe6`), **with 2 patches** in
    cerbero's `recipes/ffmpeg/` (the Meson build and `fix-textrels.patch`). The bundled build records
    `-Dnonfree=disabled -Dversion3=disabled` and reports its licence as "LGPL version 2.1 or later".
  - libnice 0.1.24 (LGPL-2.1-or-later or MPL-1.1):
    https://libnice.freedesktop.org/releases/libnice-0.1.24.tar.gz (the recipe's SHA-256
    `cfb5e8e778534f2f5b3c6f4958a1eb057c6b95c537c0f100817a537cf5d64fcc`), without patches.
  - proxy-libintl 0.5: https://github.com/frida/proxy-libintl/archive/refs/tags/0.5.tar.gz
    (the recipe's SHA-256 `f7a1cbd7579baaf575c66f9d99fb6295e9b0684a28b095967cfda17857595303`), with one patch
    from cerbero's `recipes/proxy-libintl/`.

  These sources, with the whole of cerbero at that commit, are attached to every GitHub Release
  as `Mercury-Browser-<version>-lgpl-sources.tar` (collected by `scripts/ci/lgpl-sources.sh`,
  which checks each tarball against the SHA-256 above). **Written offer:** for at least three
  years after a version was last distributed, anyone may ask for this source on a physical
  medium, for no more than the cost of making and sending it, by opening an issue at
  https://github.com/thepigdestroyer/mercury-browser/issues or writing to the copyright holder
  named in `NOTICE`. libnice is used under its MPL-1.1 option.

  The other bundled projects are under permissive licences (see the table) with no source
  obligation.
- **libjpeg-turbo (IJG).** This software is based in part on the work of the Independent JPEG
  Group.
- **Licence texts.** The LGPL version 2.1 is in `LICENSES/LGPL-2.1.txt` (the LGPL 2.1 permits use
  under any later version, which covers the "LGPL-2.0-or-later" projects too). Each project's
  own licence files, as the packages ship them under `share/licenses/<project>/`, are copied into
  `Contents/Resources/gstreamer-licenses/<project>/` of the application. `LICENSE`, `NOTICE`,
  this file and `LICENSES/` are in `Contents/Resources` too, and `LICENSE` and `NOTICE` are also
  next to the application in the disk image.

Bundled libraries by project (generated from the collected set; the collector fails if this
table does not match what it collected):

<!-- gstreamer-libraries:begin (generated by scripts/macos/gstreamer-notices.sh; do not edit) -->
| Project (cerbero recipe) | Version | Licence | Bundled files |
|---|---|---|---|
| bzip2 | 1.0.8 | bzip2-1.0.6 | `libbz2.1.dylib` |
| ffmpeg | 7.1 | LGPL-2.1-or-later | `libavcodec.61.dylib`, `libavfilter.10.dylib`, `libavformat.61.dylib`, `libavutil.59.dylib`, `libswresample.5.dylib`, `libswscale.8.dylib` |
| glib | 2.82.4 | LGPL-2.0-or-later | `libgio-2.0.0.dylib`, `libglib-2.0.0.dylib`, `libgmodule-2.0.0.dylib`, `libgobject-2.0.0.dylib` |
| graphene | 1.10.8 | MIT | `libgraphene-1.0.0.dylib` |
| gst-libav-1.0 | 1.28.9 | LGPL-2.1-or-later | `libgstlibav.dylib` |
| gst-plugins-bad-1.0 | 1.28.9 | LGPL-2.0-or-later (usrsctp in libgstsctp: BSD-3-Clause) | `libgstapplemedia.dylib`, `libgstaudiobuffersplit.dylib`, `libgstcodecparsers-1.0.0.dylib`, `libgstdtls.dylib`, `libgstid3tag.dylib`, `libgstplay-1.0.0.dylib`, `libgstproxy.dylib`, `libgstsctp-1.0.0.dylib`, `libgstvideoparsersbad.dylib`, `libgstvulkan-1.0.0.dylib`, `libgstwebrtc-1.0.0.dylib`, `libgstwebrtc.dylib`, `libgstwebrtcnice-1.0.0.dylib` |
| gst-plugins-base-1.0 | 1.28.9 | LGPL-2.0-or-later | `libgstapp-1.0.0.dylib`, `libgstapp.dylib`, `libgstaudio-1.0.0.dylib`, `libgstaudioconvert.dylib`, `libgstaudioresample.dylib`, `libgstfft-1.0.0.dylib`, `libgstgio.dylib`, `libgstgl-1.0.0.dylib`, `libgstopengl.dylib`, `libgstopus.dylib`, `libgstpbutils-1.0.0.dylib`, `libgstplayback.dylib`, `libgstriff-1.0.0.dylib`, `libgstrtp-1.0.0.dylib`, `libgstsdp-1.0.0.dylib`, `libgsttag-1.0.0.dylib`, `libgsttheora.dylib`, `libgsttypefindfunctions.dylib`, `libgstvideo-1.0.0.dylib`, `libgstvideoconvertscale.dylib`, `libgstvolume.dylib`, `libgstvorbis.dylib` |
| gst-plugins-good-1.0 | 1.28.9 | LGPL-2.1-or-later | `libgstaudiofx.dylib`, `libgstaudioparsers.dylib`, `libgstautodetect.dylib`, `libgstdeinterlace.dylib`, `libgstid3demux.dylib`, `libgstinterleave.dylib`, `libgstisomp4.dylib`, `libgstmatroska.dylib`, `libgstosxaudio.dylib`, `libgstosxvideo.dylib`, `libgstrtp.dylib`, `libgstrtpmanager.dylib`, `libgstvideofilter.dylib`, `libgstvpx.dylib`, `libgstwavparse.dylib` |
| gstreamer-1.0 | 1.28.9 | LGPL-2.0-or-later | `libgstbase-1.0.0.dylib`, `libgstcontroller-1.0.0.dylib`, `libgstcoreelements.dylib`, `libgstnet-1.0.0.dylib`, `libgstreamer-1.0.0.dylib` |
| libffi | 3.2.9999.5 | MIT | `libffi.7.dylib` |
| libjpeg-turbo | 3.1.0 | IJG AND BSD-3-Clause AND Zlib | `libjpeg.8.3.2.dylib` |
| libnice | 0.1.24 | LGPL-2.1-or-later OR MPL-1.1 | `libgstnice.dylib`, `libnice.10.dylib` |
| libogg | 1.3.5 | BSD-3-Clause | `libogg.0.dylib` |
| libpng | 1.6.56 | libpng-2.0 | `libpng16.16.56.0.dylib` |
| libtheora | 1.1.1 | BSD-3-Clause | `libtheoradec.1.dylib`, `libtheoraenc.1.dylib` |
| libvorbis | 1.3.7 | BSD-3-Clause | `libvorbis.0.dylib`, `libvorbisenc.2.dylib` |
| libvpx | v1.14.1 | BSD-3-Clause (with the WebM patent grant) | `libvpx.9.dylib` |
| moltenvk | 1.3.283.0 | Apache-2.0 | `libMoltenVK.dylib` |
| openssl | 3.5.0 | Apache-2.0 | `libcrypto.3.dylib`, `libssl.3.dylib` |
| opus | 1.5.2 | BSD-3-Clause | `libopus.0.dylib` |
| orc | 0.4.42 | BSD-2-Clause | `liborc-0.4.0.dylib` |
| pcre2 | 10.42 | BSD-3-Clause WITH PCRE2-exception | `libpcre2-8.0.dylib` |
| proxy-libintl | 0.5 | LGPL-2.0-or-later | `libintl.8.dylib` |
| zlib | 1.3.1 | Zlib | `libz.1.dylib` |
<!-- gstreamer-libraries:end -->

## Bundled data

The filter lists in `crates/talaria-servo/resources/adblock/` are data files read by the content
blocker, redistributed unmodified from the Brave mirror snapshot commit named in
`crates/talaria-servo/resources/adblock/README.md`; they are not part of Mercury Browser's source
code and keep their own licences, with their own headers. They ship as separate files beside the
program, never compiled into it: `Contents/Resources/filter-lists/` in the macOS application and
`filter-lists/` in the Linux archive, with `manifest.json` (name, file, source URL, licence, version
or date, SHA-256 of each list). In the Linux archive `filter-lists/` also holds `NOTICE.md` (each
list's name, credit, licence and source) and `LICENSES/` with the licence texts of the lists, and
the archive carries this file and the repository's `LICENSES/` as the macOS application does.
Mercury Browser downloads newer versions of each list in the background, at most once a day
(retrying sooner, after 1, 2, 4, 8 and 16 hours, only when a check could not reach a publisher:
no network, a time-out or a server error), from the URL given below ("Fetched from") and keeps
them in the user's profile (`filter-lists/`); only their network rules are used. The engine
compiled from them is built on the user's computer and never distributed. No GPL-only list is
bundled or fetched: the uBlock filters (uBlockOrigin/uAssets, GPL-3.0) were removed on 18 Nov 2026 so that everything
Mercury Browser ships can be used in a commercial product.

- EasyList — The EasyList authors, https://easylist.to/ (source repository https://github.com/easylist/easylist) — local file `easylist.txt` — fetched from https://easylist.to/easylist/easylist.txt — used under Creative Commons Attribution-ShareAlike 3.0 Unported (CC-BY-SA-3.0); the EasyList authors offer it under that licence or any later version, or alternatively under the GNU GPL version 3 or later, which Mercury Browser does not use. Credit: "The EasyList authors (https://easylist.to/)". Stated at https://easylist.to/pages/licence.html, the page named by the list's `! Licence:` header. Text: `LICENSES/CC-BY-SA-3.0.txt`.
- EasyPrivacy — The EasyList authors, https://easylist.to/ (source repository https://github.com/easylist/easylist) — local file `easyprivacy.txt` — fetched from https://easylist.to/easylist/easyprivacy.txt — used under CC-BY-SA-3.0 like EasyList, with the same credit: "The EasyList authors (https://easylist.to/)". Stated at https://easylist.to/pages/licence.html, the page named by the list's `! Licence:` header. Text: `LICENSES/CC-BY-SA-3.0.txt`.
- Brave Firstparty specific filters — Brave, https://github.com/brave/adblock-lists (`brave-lists/brave-firstparty.txt`) — local file `brave-firstparty.txt` — fetched from https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-firstparty.txt — MPL-2.0, the repository's `LICENSE` file (read at commit `a190fe07fe27b697dc796d54a54b58dfd75e7b1c`); the list itself has no licence line. Text: `LICENSES/MPL-2.0.txt`.
- Brave Specific — Brave, https://github.com/brave/adblock-lists (`brave-lists/brave-specific.txt`) — local file `brave-specific.txt` — fetched from https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-specific.txt — MPL-2.0, the repository's `LICENSE` file (read at commit `a190fe07fe27b697dc796d54a54b58dfd75e7b1c`); the list itself has no licence line. Text: `LICENSES/MPL-2.0.txt`.
- Brave unbreak list (its only line is `! Brave desktop filters`) — Brave, https://github.com/brave/adblock-lists (`brave-lists/brave-unbreak.txt`) — local file `brave-unbreak.txt` — fetched from https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-unbreak.txt — MPL-2.0, the repository's `LICENSE` file (read at commit `a190fe07fe27b697dc796d54a54b58dfd75e7b1c`); the list itself has no licence line. Text: `LICENSES/MPL-2.0.txt`.

The HSTS preload list is data too, but it is compiled into the executable, not shipped as a file
beside it: the host names that sites asked browsers, in advance, to open only over HTTPS, in the
`fst` map format Servo reads. `crates/talaria-servo/src/resources.rs` serves it to Servo's network
stack in place of the copy in the vendored `servo-default-resources`. It is regenerated before
releases by the `hsts-preload` tool (`crates/mercury-devtools`) from Chromium's published list;
`crates/talaria-servo/resources/hsts_preload.manifest.json` records where the data came from, the
date it was fetched and the SHA-256 of the input and of the data file. Until the first
regeneration it is the copy that Servo 0.8.0 publishes (made by the Servo project from the same
Chromium list), which the manifest says.

- Chromium HSTS preload list — The Chromium Authors, https://www.chromium.org/ (file `net/http/transport_security_state_static.json` of https://chromium.googlesource.com/chromium/src, the list maintained with https://hstspreload.org/) — local file `crates/talaria-servo/resources/hsts_preload.fstmap`, compiled into the executable — regenerated by `hsts-preload` from that JSON as fetched on the date in `hsts_preload.manifest.json`; the host names and the include-subdomains flag are the data, the file format is the `fst` crate's — BSD-3-Clause, Copyright 2015 The Chromium Authors. Text: `LICENSES/BSD-3-Clause-chromium.txt`.

## Build tools

Libraries linked only into Mercury's own build tools, never into the browser or its app bundle.

- resvg 0.48 — https://github.com/linebender/resvg — Apache-2.0 OR MIT (used under MIT; Copyright 2017 the Resvg Authors; the text is in `LICENSES/MIT-resvg.txt`) — the SVG renderer of `crates/mercury-iconset`, which `scripts/macos/make-icns.sh` runs to render the app icon's iconset. Mercury's source contains none of its code.

## PDF viewer (PDFium)

Mercury Browser draws PDF files with **PDFium**, the PDF library of the Chromium project. PDFium is
C++ and is not part of Mercury Browser's source code: it is a separate dynamic library that the
browser loads when a PDF is opened in its own viewer, and it keeps its own licences. **The build is
without V8 and without XFA** (no V8, no XFA): no JavaScript inside a PDF file runs, and XFA forms are not supported.

- **Origin.** The prebuilt library of the pdfium-binaries project, https://github.com/bblanchon/pdfium-binaries,
  release `chromium/8165` (PDFium 160.0.8165.0, published 16 November 2026;
  https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium%2F8165). The packaging scripts of that project are
  MIT, Copyright 2014-2025 Benoit Blanchon (`LICENSES/MIT-pdfium-binaries.txt`). `scripts/macos/pdfium.sh` downloads
  `pdfium-mac-arm64.tgz` (SHA-256 `112e74d717f3e0310273b70f3c8780e8a61ff11b34834124465c8305c841d350`) and, for
  development on an Intel Mac, `pdfium-mac-x64.tgz` (SHA-256 `07285915b564e46a432a9909dbc1f15892885ca6489e3faeefc70dee77ed371a`),
  stops on any other checksum, and checks that the tarball's `args.gn` says `pdf_enable_v8 = false` and
  `pdf_enable_xfa = false`. The `pdfium-v8-*` builds (which run form JavaScript) are never used. The library inside
  the arm64 tarball, `lib/libpdfium.dylib`, has SHA-256 `d421e426bd487de02fa3bd1f2aeaf56947261f790e6116951e26f729ced1b018`.
- **Unmodified, and separate.** `Mercury Browser.app/Contents/MacOS/lib/libpdfium.dylib` (arm64) is that file byte for
  byte, except that `scripts/macos/bundle.sh` signs it again with the application's signature (its install name,
  `@rpath/libpdfium.dylib`, is the one it was built with). Nothing from it is linked into `mercury-browser`: the
  `mercury-pdfium` crate loads it at run time, in a helper process of its own (`mercury-browser --pdf-worker`). A
  compatible build with the same file name can replace it, signed again as described for the GStreamer libraries above;
  none of its licences requires that.
- **Licence texts.** Each component's text, as the tarball ships it, is in `LICENSES/` (named after its licence and
  project, below) and, under the tarball's own file names, in `Contents/Resources/pdfium-licenses/` of the application.
  The texts were copied from the tarball, not retyped; `scripts/macos/test-pdfium.sh --online` checks that they still
  are identical to it.
- **Credits that licences ask for.** FreeType (used under the FreeType Licence, FTL): "Portions of this software are
  copyright © 2026 The FreeType Project (www.freetype.org). All rights reserved." (2026 is the year of the FreeType
  snapshot that PDFium 160.0.8165.0 is built from.) libjpeg-turbo (IJG): "This software is based in part on the work
  of the Independent JPEG Group."

What the library contains (all as built into PDFium 160.0.8165.0; the project of each is the Chromium copy named by PDFium's
`DEPS`; versions are those pinned there):

| Component | Used for | Licence | Text |
|---|---|---|---|
| PDFium, Copyright 2014 The PDFium Authors | the PDF engine | BSD-3-Clause AND Apache-2.0 (both texts are in its licence file) | `LICENSES/BSD-3-Clause-pdfium.txt` |
| Abseil C++ | utility library | Apache-2.0 | `LICENSES/Apache-2.0-abseil.txt` |
| Anti-Grain Geometry 2.3 (agg23), Copyright 2002-2005 Maxim Shemanarev | 2D rasteriser | AGG 2.3 permission notice (copy, use, modify, sell and distribute, if the notice stays) | `LICENSES/LicenseRef-agg23.txt` |
| dragonbox | floating-point to text | BSL-1.0 OR Apache-2.0 WITH LLVM-exception | `LICENSES/BSL-1.0-dragonbox.txt`, `LICENSES/Apache-2.0-LLVM-exception-dragonbox.txt` |
| fast_float | text to floating-point | MIT (upstream: Apache-2.0 OR MIT OR BSL-1.0) | `LICENSES/MIT-fast_float.txt` |
| FreeType | fonts | FTL (upstream: FTL OR GPL-2.0-only; **used under the FTL**, with the credit above) | `LICENSES/FTL-freetype.txt` |
| HarfBuzz | text shaping | "Old MIT" | `LICENSES/MIT-harfbuzz.txt` |
| Highway | SIMD helpers | Apache-2.0 (upstream: Apache-2.0 OR BSD-3-Clause) | `LICENSES/Apache-2.0-highway.txt` |
| ICU | Unicode text handling | Unicode-3.0 (the file also carries the notices of the data it includes) | `LICENSES/Unicode-3.0-icu.txt` |
| Little CMS (lcms2) | colour management | MIT | `LICENSES/MIT-lcms.txt` |
| libjpeg-turbo | JPEG images | IJG AND BSD-3-Clause AND Zlib, with the credit above | `LICENSES/IJG-libjpeg-turbo.txt`, `LICENSES/BSD-3-Clause-libjpeg-turbo.md` |
| OpenJPEG | JPEG 2000 images | BSD-2-Clause | `LICENSES/BSD-2-Clause-openjpeg.txt` |
| libpng | PNG images | libpng-2.0 | `LICENSES/libpng-2.0-libpng.txt` |
| llvm-libc | C library functions | Apache-2.0 WITH LLVM-exception | `LICENSES/Apache-2.0-LLVM-exception-llvm-libc.txt` |
| simdutf | UTF conversion | MIT (upstream: Apache-2.0 OR MIT) | `LICENSES/MIT-simdutf.txt` |
| zlib | compression | Zlib | `LICENSES/Zlib-zlib.txt` |

Rust crates that load and wrap the library are compiled into `mercury-browser` and keep their own licences:

- `pdfium-render` 0.9.5 — https://github.com/ajrcarey/pdfium-render — MIT OR Apache-2.0 (used under MIT; Copyright (c)
  Alastair Carey, the crate's author; `LICENSES/MIT-pdfium-render.txt`) — the safe wrapper over PDFium's C interface, with
  the features `pdfium_8125` and `thread_safe`; its `pdfium_enable_v8` and `pdfium_enable_xfa` features are off. The
  crate and its repository carry no `LICENSE-MIT` and no `LICENSE-APACHE` (their `LICENSE.md` says "either of Apache-2.0
  or MIT, at your option" and points at two files that were never published; checked in the repository's whole history on
  20 Nov 2026). The file in `LICENSES/` is therefore the standard MIT text (SPDX `MIT`) with the copyright lines the
  crate does give: its `authors` field ("Alastair Carey") and the header of its `build.rs` ("Copyright 2022 - 2024,
  pdfium-render Developers" and "Copyright 2021, pdfium-sys Developers"). The crate's `include/` folder holds PDFium's own
  headers (BSD-3-Clause, "The PDFium Authors"), read only when its bindings are regenerated (its `bindings` feature,
  off here); the bindings it ships were made from them, and PDFium's own licence is listed above.
- `maybe-owned` 0.3.4 — https://github.com/rustonaut/maybe-owned — MIT OR Apache-2.0 (used under MIT; Copyright (c)
  2016 Philipp Korber; `LICENSES/MIT-maybe-owned.txt`) — a dependency of `pdfium-render`.
- `piston-float` 1.0.1 — https://github.com/pistondevelopers/float — MIT (Copyright (c) 2015 PistonDevelopers;
  `LICENSES/MIT-piston-float.txt`) — a dependency of `pdfium-render` (through `vecmath`).
- `utf16string` 0.2.0 — https://github.com/getsentry/utf16string — MIT OR Apache-2.0 (used under MIT; Copyright (c)
  2020 Armin Ronacher, 2019 Manish Goregaokar; `LICENSES/MIT-utf16string.txt`) — a dependency of `pdfium-render`.
- `vecmath` 1.0.0 — https://github.com/pistondevelopers/vecmath — MIT (Copyright (c) 2014 PistonDevelopers;
  `LICENSES/MIT-vecmath.txt`) — a dependency of `pdfium-render`.

## Vendored crates (`third_party/`)

Mercury Browser carries 26 crates in `third_party/`. Each is the crate exactly as published on
crates.io plus Mercury's changes in `third_party/patches/<name>.patch`; `third_party/vendor.toml`
records, for each, the upstream repository, commit and path, and its `MERCURY.md` says what was
changed and why. They keep their own licences, and `make servo-patches-check` fails when a
directory and its patch disagree. The credits page (see "Rust crates" below) lists every one of
them with its licence text and a pointer to its source.

- `gaol` 0.2.1 — https://github.com/servo/gaol, commit `4544946c5c922ee619a932ae5c3fd1d26f143384` — MIT/Apache-2.0, used under the MIT licence (Copyright (c) 2012-2013 Mozilla Foundation); the texts are in `third_party/gaol/LICENSE-MIT` and `third_party/gaol/LICENSE-APACHE`.
- `surfman` 0.14.0 — https://github.com/servo/surfman, commit `ca64434fb50cab45d9eda38a00f01fa0ddc43fe4` — MIT OR Apache-2.0 OR MPL-2.0, used under the MIT licence (Copyright (c) 2010 The Rust Project Developers) or the Apache-2.0 licence; the texts are in `third_party/surfman/LICENSE-MIT`, `LICENSE-APACHE` and `LICENSE-MPL`.
- `i-slint-backend-winit` 1.17.1 — https://github.com/slint-ui/slint, commit `cf62c975c311e7036d599ed8ed0b7e6a8386a934`, path `internal/backends/winit` — offered under `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`, used under the Slint Royalty-free Desktop, Mobile and Web Applications License 2.0 like the rest of Slint (see "Slint" below). Mercury's changes to it stay under that licence. Its `LICENSES/` directory is kept as published; `third_party/i-slint-backend-winit/MERCURY.md` has the conditions every change must keep.
- The Servo crates `servo`, `servo-canvas`, `servo-config`, `servo-constellation`, `servo-constellation-traits`, `servo-default-resources`, `servo-embedder-traits`, `servo-layout`, `servo-layout-api`, `servo-media`, `servo-media-gstreamer`, `servo-media-player`, `servo-media-streams`, `servo-media-webrtc`, `servo-net`, `servo-net-traits`, `servo-script`, `servo-script-bindings`, `servo-script-webcrypto`, `servo-storage`, `servo-storage-traits` and `servo-webgl` (all 0.8.0, from https://github.com/servo/servo, commit `4e136d8ef31b7c4748fbfac606ca463f3efd7537`) and `stylo_static_prefs` 0.23.0 (from https://github.com/servo/stylo, commit `2dca34592ced37c6aa01733e94206a767194cf9d`) — MPL-2.0 (`servo-script-bindings` also BSD-3-Clause for the PLY library its build script runs, which is not part of the executable). Where Mercury changed one of them, the changed files stay under the MPL-2.0 and are published with the release tag in the Mercury repository (https://github.com/thepigdestroyer/mercury-browser, `third_party/`), as the MPL-2.0 requires; the unchanged files are at the upstream commit named above.

## Slint

Mercury Browser's interface is built with Slint 1.17.1 (https://slint.dev). Slint is offered
under `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`;
Mercury Browser uses it under the Slint Royalty-free Desktop, Mobile and Web Applications
License 2.0 (`deny.toml` allows that licence for the Slint crates only). The licence requires
the application to show Slint's attribution, which Mercury Browser does with the `AboutSlint`
widget in its About window (**About Mercury Browser**, in the application menu and in the
browser menu). Mercury Browser does not expose Slint's APIs to themes, extensions or scripts.
The licence text is in each Slint crate's `LICENSES/` directory and on the credits page.

## Rust crates

Mercury Browser is built from about 880 Rust crates (the exact number depends on the platform).
Their copyright notices and licence texts, with the `NOTICE` files of the Apache-2.0 crates, are
in one page that is generated for each build from the crates actually built into it, by
`mercury-credits` (`crates/mercury-credits`): `credits/credits.html`, which is
`Contents/Resources/credits/credits.html` in the macOS application and `credits/credits.html` in
the Linux archive. **About Mercury Browser, Third-party notices** opens it. For each crate that is
under the MPL-2.0 the page also says where its source is. For a crate that offers several
licences, the page shows the permissive ones Mercury Browser may use, as `deny.toml` lists them;
copyleft alternatives are not used. Where a crate ships no licence file, the page gives the
standard text of its licence with a copyright line taken from the crate's metadata, and says so.

Several of these crates compile native code into the application, whose licence files the page
includes: SpiderMonkey (`mozjs_sys`, with LZ4, xsum, Zydis, irregexp, double-conversion, fdlibm,
{fmt}, function2, gemmology and xsimd), ANGLE (`mozangle`), AWS-LC (`aws-lc-sys`), HarfBuzz
(`harfbuzz-sys`), the OpenType Sanitiser and LZ4 (`fontsan`), Zstandard (`zstd-sys`) and zlib
(`libz-sys`).

- **SQLite** (`libsqlite3-sys`) is in the public domain (https://www.sqlite.org/copyright.html) and has no licence text.
- **The Linux archive** also contains FreeType 2.13.3 (bundled by the `freetype-sys` crate), used under the FreeType License. Portions of this software are copyright © 2024 The FreeType Project (www.freetype.org). All rights reserved.
- **SpiderMonkey source files with their own notice.** These notices are in the files' headers, not in licence files, so they are reproduced here. The text of the Apache License 2.0 is in `LICENSES/Apache-2.0.txt`, and the Unicode License v3 in `LICENSES/Unicode-3.0.txt`.
  - `js/src/wasm/` and a few related files — Copyright 2014 to 2025 Mozilla Foundation — Licensed under the Apache License, Version 2.0.
  - ICU, in `intl/icu/source` — © 2016 and later: Unicode, Inc. and others, as its files say; they point to https://www.unicode.org/copyright.html for the licence, and the Unicode License v3 is in `LICENSES/Unicode-3.0.txt`.
  - `js/src/jit/arm64/vixl/` (the ARM64 assembler, used by the Apple Silicon build) — Copyright 2013 to 2019, ARM Limited and the VIXL authors — BSD-3-Clause, with this text:

    ```text
    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions are met:

      * Redistributions of source code must retain the above copyright notice,
        this list of conditions and the following disclaimer.
      * Redistributions in binary form must reproduce the above copyright notice,
        this list of conditions and the following disclaimer in the documentation
        and/or other materials provided with the distribution.
      * Neither the name of ARM Limited nor the names of its contributors may be
        used to endorse or promote products derived from this software without
        specific prior written permission.

    THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS CONTRIBUTORS "AS IS" AND
    ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
    WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
    DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE
    FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
    DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
    SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
    CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
    OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
    OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
    ```

  - `js/src/jit/ExecutableAllocator.cpp` and `.h`, `js/src/jit/x86-shared/AssemblerBuffer-x86-shared.h` and `BaseAssembler-x86-shared.h` (Copyright (C) 2008 Apple Inc. All rights reserved.), and the parts of `js/src/vm/BigIntType.cpp` taken from WebKit (Copyright (C) 2017 Caio Lima and Copyright (C) 2017-2018 Apple Inc. All rights reserved.) — BSD-2-Clause, with this text:

    ```text
    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions
    are met:
    1. Redistributions of source code must retain the above copyright
       notice, this list of conditions and the following disclaimer.
    2. Redistributions in binary form must reproduce the above copyright
       notice, this list of conditions and the following disclaimer in the
       documentation and/or other materials provided with the distribution.

    THIS SOFTWARE IS PROVIDED BY APPLE INC. ``AS IS'' AND ANY
    EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
    IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
    PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL APPLE INC. OR
    CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
    EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
    PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
    PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
    OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
    (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
    OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
    ```

  - `js/src/vm/BigIntType.cpp` also contains portions taken from V8 (Copyright 2017 the V8 project authors; BSD-3-Clause, with the same wording as the V8 text of `irregexp` on the credits page), from Dart (Copyright (c) 2014 the Dart project authors) and from Go (Copyright 2009 The Go Authors), the last two under BSD-style licences that the file's header refers to (https://github.com/dart-lang/sdk/blob/master/LICENSE and https://golang.org/LICENSE).
  - `mozglue/misc/decimal/` (Blink's decimal arithmetic) — Copyright (C) 2012 Google Inc. All rights reserved. — BSD-3-Clause, with this text:

    ```text
    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions are
    met:

        * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.
        * Redistributions in binary form must reproduce the above
    copyright notice, this list of conditions and the following disclaimer
    in the documentation and/or other materials provided with the
    distribution.
        * Neither the name of Google Inc. nor the names of its
    contributors may be used to endorse or promote products derived from
    this software without specific prior written permission.

    THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
    "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
    LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
    A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
    OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
    SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
    LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
    DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
    THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
    (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
    OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
    ```

  - `js/src/vm/Float16.h` (the `half` library, version 2.2.0) — Copyright (c) 2012-2021 Christian Rau — MIT, with this text:

    ```text
    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
    THE SOFTWARE.
    ```

## Codecs and patents

The macOS application bundles codec libraries as separate dynamic libraries, listed in the table
under "Bundled libraries by project" above: FFmpeg's `libavcodec` and its companions, libvpx, opus,
libvorbis, libtheora and libogg for audio and video, and libjpeg-turbo and libpng for images. The
Linux archive uses the codec libraries of the system's GStreamer instead. Mercury Browser grants
no patent licence for any codec. Some codecs in those libraries may be covered by patents owned by
third parties, in some countries; the user or distributor of the software is responsible for any
licence that a codec needs.
