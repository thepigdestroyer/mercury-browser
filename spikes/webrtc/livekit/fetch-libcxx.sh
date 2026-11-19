#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# The libc++ headers that C++ must be compiled with to call the pinned libwebrtc (route L of the
# WebRTC spike). Shiguredo's archive is built against Chromium's libc++ in the inline namespace
# std::__Cr, and its include/ tree keeps only the libc++ files that end in .h (1459 of them): the
# C++ standard headers without an extension (<vector>, <string>, <memory>) and __config_site are not
# in it. The same revision is named in the archive's VERSIONS file:
#   WEBRTC_SRC_THIRD_PARTY_LIBCXX_SRC_COMMIT=5abc7f839700f0f17338434e1c1c6a8c87c00c11  (libcxx)
#   WEBRTC_SRC_BUILDTOOLS_COMMIT=17495e454aae81b581e8b3caccbb53054509b280              (buildtools)
# so this script takes exactly that revision from Chromium's own git mirrors (headers only; the
# licence is Apache-2.0 with LLVM-exception, the same as the files already in the archive).
#
#   ./fetch-libcxx.sh fetch      download and unpack (a no-op when the cache is already verified)
#   ./fetch-libcxx.sh check      re-hash the unpacked files against the pins below
#   ./fetch-libcxx.sh env        print `export MERCURY_LIBCXX_DIR=...` (fails unless `check` passes)
#   ./fetch-libcxx.sh compare    also compare with the .h files of the archive (webrtc.sh path libwebrtc)
#
# The tarballs gitiles produces differ byte for byte between downloads (gzip and tar metadata), so
# the pin is the hash of the unpacked content: SHA-256 of the sorted "sha256  ./path" lines of every
# file. A mismatch deletes the download and stops; the pins are never edited to fit.
# The tarball of buildtools also holds Python files (generate_libcxx_headers.py, sync_dir.py); only
# __config_site and __assertion_handler are taken out of it, nothing else is unpacked or run.
#
# Cache: ${MERCURY_WEBRTC_CACHE:-$HOME/Library/Caches/mercury-browser/webrtc}/libcxx/<commit>/
#   include/  config/  .mercury-libcxx-verified

set -eu

LIBCXX_COMMIT="5abc7f839700f0f17338434e1c1c6a8c87c00c11"
LIBCXX_URL="https://chromium.googlesource.com/external/github.com/llvm/llvm-project/libcxx.git/+archive/${LIBCXX_COMMIT}/include.tar.gz"
LIBCXX_FILES="1686"
LIBCXX_TREE_SHA256="40b02ecde0423c01d1687f1a9847dfd0222aed727b83d46273bdf3e25592b969"

BUILDTOOLS_COMMIT="17495e454aae81b581e8b3caccbb53054509b280"
BUILDTOOLS_URL="https://chromium.googlesource.com/chromium/src/buildtools/+archive/${BUILDTOOLS_COMMIT}/third_party/libc++.tar.gz"
CONFIG_SITE_SHA256="fcac9dc8ea709997ab8b43af6c4e4e2a3674531c6f6303ca42eaf1e3fe189064"
ASSERTION_HANDLER_SHA256="6f9934bea04c9e23f264a82725925354274af6ca2651a1056a737f96a73d0193"

cache_root="${MERCURY_WEBRTC_CACHE:-${HOME}/Library/Caches/mercury-browser/webrtc}"
dir="${cache_root}/libcxx/${LIBCXX_COMMIT}"
marker="${dir}/.mercury-libcxx-verified"

say() { printf '%s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }

sha256_of() { shasum -a 256 "$1" | cut -d ' ' -f 1; }

# The hash of a directory's content, independent of the file times and the tar that made it.
tree_sha256() {
    (
        cd "$1"
        find . -type f | LC_ALL=C sort | xargs shasum -a 256 | shasum -a 256 | cut -d ' ' -f 1
    )
}

# The pinned values are for this host's tools only in the sense that shasum and tar exist.
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }

check() {
    [ -f "$marker" ] || { say "libc++ headers not unpacked: ${dir}"; return 1; }
    [ "$(sha256_of "${dir}/config/__config_site")" = "$CONFIG_SITE_SHA256" ] || { say "__config_site changed"; return 1; }
    [ "$(sha256_of "${dir}/config/__assertion_handler")" = "$ASSERTION_HANDLER_SHA256" ] || { say "__assertion_handler changed"; return 1; }
    [ "$(find "${dir}/include" -type f | wc -l | tr -d ' ')" = "$LIBCXX_FILES" ] || { say "libc++ file count changed"; return 1; }
    [ "$(tree_sha256 "${dir}/include")" = "$LIBCXX_TREE_SHA256" ] || { say "libc++ include/ content changed"; return 1; }
    say "ok: libc++ ${LIBCXX_COMMIT} verified (${LIBCXX_FILES} files, tree sha256 ${LIBCXX_TREE_SHA256})"
}

fetch() {
    need curl
    need tar
    need shasum
    if check 2>/dev/null; then
        return 0
    fi
    work="$(mktemp -d "${TMPDIR:-/tmp}/mercury-libcxx.XXXXXX")"
    trap 'rm -rf -- "$work"' EXIT
    mkdir -p "$work/include" "$work/bt" "$work/config"

    say "downloading libc++ headers ${LIBCXX_COMMIT}"
    curl -sS -L --fail --retry 3 -o "$work/include.tar.gz" "$LIBCXX_URL" || die "download failed: $LIBCXX_URL"
    tar -xzf "$work/include.tar.gz" -C "$work/include"
    [ "$(tree_sha256 "$work/include")" = "$LIBCXX_TREE_SHA256" ] \
        || die "libc++ headers from ${LIBCXX_URL} do not match the pinned content hash; nothing was kept"

    say "downloading buildtools libc++ configuration ${BUILDTOOLS_COMMIT}"
    curl -sS -L --fail --retry 3 -o "$work/bt.tar.gz" "$BUILDTOOLS_URL" || die "download failed: $BUILDTOOLS_URL"
    tar -xzf "$work/bt.tar.gz" -C "$work/bt" __config_site __assertion_handler
    [ "$(sha256_of "$work/bt/__config_site")" = "$CONFIG_SITE_SHA256" ] || die "__config_site does not match its pin; nothing was kept"
    [ "$(sha256_of "$work/bt/__assertion_handler")" = "$ASSERTION_HANDLER_SHA256" ] || die "__assertion_handler does not match its pin; nothing was kept"
    mv "$work/bt/__config_site" "$work/bt/__assertion_handler" "$work/config/"

    rm -rf -- "$dir"
    mkdir -p "$dir"
    mv "$work/include" "$dir/include"
    mv "$work/config" "$dir/config"
    printf '%s\n' "$LIBCXX_TREE_SHA256" > "$marker"
    check
}

compare() {
    check
    webrtc_dir="${MERCURY_WEBRTC_DIR:-}"
    [ -n "$webrtc_dir" ] || die "MERCURY_WEBRTC_DIR is not set (eval \"\$(scripts/macos/webrtc.sh env libwebrtc)\")"
    archive_include="${webrtc_dir}/include/third_party/libc++/src/include"
    same=0
    other=0
    while IFS= read -r file; do
        if cmp -s "${archive_include}/${file}" "${dir}/include/${file}"; then
            same=$((same + 1))
        else
            other=$((other + 1))
            say "differs or missing: ${file}"
        fi
    done <<EOF
$(cd "$archive_include" && find . -type f | LC_ALL=C sort)
EOF
    say "archive's libc++ files identical to the pinned revision: ${same}; different or missing: ${other}"
    [ "$other" -eq 0 ] || exit 1
}

case "${1:-}" in
    fetch) fetch ;;
    check) check ;;
    env)
        check || die "run ./fetch-libcxx.sh fetch first"
        printf 'export MERCURY_LIBCXX_DIR=%s\n' "$dir"
        ;;
    compare) compare ;;
    *) die "usage: fetch-libcxx.sh fetch | check | env | compare" ;;
esac
