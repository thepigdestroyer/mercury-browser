#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#
# Collect the corresponding source of the LGPL libraries in the macOS app (GStreamer, FFmpeg,
# GLib, proxy-libintl, libnice) into one archive published with each release.
#
#   scripts/ci/lgpl-sources.sh <version> <output directory>
#
# Writes <output directory>/Mercury-Browser-<version>-lgpl-sources.tar and its .sha256. The
# archive holds each project's release tarball, checked against the SHA-256 pinned below (the
# checksums cerbero's recipes and the GStreamer project publish), and the whole of cerbero, the
# build system that made the bundled binaries, at the commit pinned below (its recipes name
# every source, configure option and patch). README.txt inside says what is what.
#
# The versions must match THIRD-PARTY-NOTICES.md and scripts/macos/gstreamer.sh; the script
# stops if GStreamer's does not.
set -euo pipefail

die() {
    printf 'lgpl-sources: error: %s\n' "$*" >&2
    exit 1
}

[ "$#" -eq 2 ] || die "usage: $0 <version> <output directory>"
VERSION="$1"
OUT="$2"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$VERSION' is not MAJOR.MINOR.PATCH"

REPO="$(cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
GSTREAMER_VERSION="1.28.9"
CERBERO_URL="https://gitlab.freedesktop.org/gstreamer/cerbero.git"
CERBERO_COMMIT="efda998b6e8032a8ed785a70000a63d337898f82"

pinned="$(sed -n 's/^GSTREAMER_VERSION="\(.*\)"$/\1/p' "$REPO/scripts/macos/gstreamer.sh")"
[ "$pinned" = "$GSTREAMER_VERSION" ] ||
    die "scripts/macos/gstreamer.sh bundles GStreamer '$pinned' but this script collects $GSTREAMER_VERSION; update both"

# file name, SHA-256, URL. GStreamer's checksums are the .sha256sum files published next to
# each tarball; the others are the checksums in cerbero's recipes at CERBERO_COMMIT.
SOURCES="
gstreamer-1.28.9.tar.xz 6dbf744f583131a34845cc641e803843766a9ba80ce00378a9043fd6d8d5fb38 https://gstreamer.freedesktop.org/src/gstreamer/gstreamer-1.28.9.tar.xz
gst-plugins-base-1.28.9.tar.xz c1fda668b520e6aef763c0eb962c032c9fe52bce322e8b00c69b4e9e847bddc1 https://gstreamer.freedesktop.org/src/gst-plugins-base/gst-plugins-base-1.28.9.tar.xz
gst-plugins-good-1.28.9.tar.xz e518cd97483706a252e95e60312a22d23dec6f5e7673af6f5bde719c3a9ca7f4 https://gstreamer.freedesktop.org/src/gst-plugins-good/gst-plugins-good-1.28.9.tar.xz
gst-plugins-bad-1.28.9.tar.xz 4a4e0b7bf84f868378c48f143a7ac3a98923b488a5b74f1a35ba46bc3c43f6b1 https://gstreamer.freedesktop.org/src/gst-plugins-bad/gst-plugins-bad-1.28.9.tar.xz
gst-libav-1.28.9.tar.xz 4b3df21fa70972a80891d2298cf32c14a62c3db4483d545c7ac018230399186b https://gstreamer.freedesktop.org/src/gst-libav/gst-libav-1.28.9.tar.xz
glib-2.82.4.tar.xz 37dd0877fe964cd15e9a2710b044a1830fb1bd93652a6d0cb6b8b2dff187c709 https://download.gnome.org/sources/glib/2.82/glib-2.82.4.tar.xz
ffmpeg-7.1.tar.xz 40973d44970dbc83ef302b0609f2e74982be2d85916dd2ee7472d30678a7abe6 https://ffmpeg.org/releases/ffmpeg-7.1.tar.xz
libnice-0.1.24.tar.gz cfb5e8e778534f2f5b3c6f4958a1eb057c6b95c537c0f100817a537cf5d64fcc https://libnice.freedesktop.org/releases/libnice-0.1.24.tar.gz
proxy-libintl-0.5.tar.gz f7a1cbd7579baaf575c66f9d99fb6295e9b0684a28b095967cfda17857595303 https://github.com/frida/proxy-libintl/archive/refs/tags/0.5.tar.gz
"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 -- "$1" | cut -d ' ' -f 1
    fi
}

NAME="Mercury-Browser-$VERSION-lgpl-sources"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT
STAGE="$WORK/$NAME"
mkdir -p "$STAGE"

while read -r file digest url; do
    [ -n "$file" ] || continue
    printf 'lgpl-sources: %s\n' "$url" >&2
    # https only, also across redirects; the digest check below is what makes the file trusted.
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
        --retry 3 --retry-delay 5 --retry-all-errors --connect-timeout 30 --max-time 300 \
        --output "$STAGE/$file" "$url" || die "cannot download $url"
    actual="$(sha256 "$STAGE/$file")"
    [ "$actual" = "$digest" ] || die "$file has SHA-256 $actual, expected $digest"
done <<<"$SOURCES"

# cerbero by commit: git checks every object against the commit's hash.
printf 'lgpl-sources: %s at %s\n' "$CERBERO_URL" "$CERBERO_COMMIT" >&2
git init --quiet "$WORK/cerbero" || die "cannot create a git repository in $WORK/cerbero"
fetched=0
for attempt in 1 2 3; do
    if GIT_TERMINAL_PROMPT=0 git -C "$WORK/cerbero" fetch --quiet --depth 1 "$CERBERO_URL" "$CERBERO_COMMIT"; then
        fetched=1
        break
    fi
    printf 'lgpl-sources: fetching cerbero failed (attempt %s of 3)\n' "$attempt" >&2
    sleep 5
done
[ "$fetched" = 1 ] || die "cannot fetch commit $CERBERO_COMMIT from $CERBERO_URL"
[ "$(git -C "$WORK/cerbero" rev-parse FETCH_HEAD)" = "$CERBERO_COMMIT" ] ||
    die "cerbero fetch did not return commit $CERBERO_COMMIT"
git -C "$WORK/cerbero" archive --format=tar.gz --prefix="cerbero-${CERBERO_COMMIT:0:8}/" \
    --output="$STAGE/cerbero-${CERBERO_COMMIT:0:8}.tar.gz" FETCH_HEAD

cat >"$STAGE/README.txt" <<EOF
Mercury Browser $VERSION: source code of the LGPL libraries in the macOS application

Mercury-Browser-$VERSION-macos-arm64.dmg carries these libraries, unmodified except for being
thinned to arm64 and re-signed, as separate dynamic libraries in
"Mercury Browser.app/Contents/MacOS/lib". They come from the official GStreamer
$GSTREAMER_VERSION macOS packages, built by GStreamer's build system cerbero at commit
$CERBERO_COMMIT.

  GStreamer $GSTREAMER_VERSION (LGPL-2.0-or-later / LGPL-2.1-or-later):
    gstreamer, gst-plugins-base, gst-plugins-good, gst-plugins-bad and gst-libav tarballs,
    without patches
  FFmpeg 7.1 (LGPL-2.1-or-later):    ffmpeg-7.1.tar.xz, with the 2 patches in
                                     cerbero's recipes/ffmpeg/ (the Meson build and
                                     fix-textrels.patch)
  GLib 2.82.4 (LGPL-2.0-or-later):   glib-2.82.4.tar.xz, with the 14 patches in
                                     cerbero's recipes/glib/
  proxy-libintl 0.5 (LGPL-2.0-or-later): proxy-libintl-0.5.tar.gz, with the patch in
                                     cerbero's recipes/proxy-libintl/
  libnice 0.1.24 (LGPL-2.1-or-later OR MPL-1.1; Mercury takes it under MPL-1.1):
                                     libnice-0.1.24.tar.gz, without patches

cerbero-${CERBERO_COMMIT:0:8}.tar.gz is the whole build system at that commit: each
recipes/<project>.recipe names the source, its checksum, the configure options and the patches
used, and cerbero/ holds the scripts that control the build. The tarballs are the projects'
release files, each checked when this archive was made against the SHA-256 published by the
project or in cerbero's recipe; SHA256SUMS lists the SHA-256 of every file in this archive.

How to replace a library in the application: see "Bundled libraries" in THIRD-PARTY-NOTICES.md
(https://github.com/thepigdestroyer/mercury-browser/blob/v$VERSION/THIRD-PARTY-NOTICES.md).

Written offer: for at least three years after we last distribute this version, anyone may ask
for a copy of this source on a physical medium, for no more than the cost of making and sending
it, by opening an issue at https://github.com/thepigdestroyer/mercury-browser/issues or writing
to the copyright holder named in NOTICE.
EOF

# The digest goes into a variable first: a failure inside "$(...)" in printf's arguments would
# not stop the script (set -e), and would write a line with an empty digest.
(
    cd "$STAGE"
    for file in *; do
        digest="$(sha256 "$file")"
        printf '%s  %s\n' "$digest" "$file"
    done >"$WORK/SHA256SUMS"
)
mv -- "$WORK/SHA256SUMS" "$STAGE/SHA256SUMS"

# Already-compressed files, so a plain tar. Built beside the staging directory and moved into
# the output directory last, so that a failure leaves nothing half-written there.
tar -cf "$WORK/$NAME.tar" -C "$WORK" "$NAME"
tar_digest="$(sha256 "$WORK/$NAME.tar")"
printf '%s  %s\n' "$tar_digest" "$NAME.tar" >"$WORK/$NAME.tar.sha256"
rm -f -- "$OUT/$NAME.tar" "$OUT/$NAME.tar.sha256"
mv -- "$WORK/$NAME.tar" "$OUT/$NAME.tar"
mv -- "$WORK/$NAME.tar.sha256" "$OUT/$NAME.tar.sha256"
printf 'lgpl-sources: wrote %s\n' "$OUT/$NAME.tar" >&2
