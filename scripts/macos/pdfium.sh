#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Obtain the PDFium library that Mercury's PDF viewer renders with, without sudo.
#
#   scripts/macos/pdfium.sh [--arch arm64|x64] fetch [unpack] [check] ...
#
#   fetch     download pdfium-mac-<arch>.tgz and verify its SHA-256 against the pin
#   unpack    check the pin again, then expand the tarball into the cache and verify
#             what came out (see `check`)
#   check     verify an unpacked cache statically: release, build variant, the
#             library's digest, architecture and install name, and the licence files.
#             Nothing is executed, so an arm64 build can be checked on an Intel Mac
#   env       print the exports a shell needs: MERCURY_PDFIUM_PREFIX and
#             MERCURY_PDFIUM_LIBRARY (the library that mercury-pdfium loads first)
#   prefix    print the unpacked directory (<cache>/<release>/<arch>)
#   library   print the path of lib/libpdfium.dylib inside it
#
# Several commands may be given; they run in the order written. The architecture is
# --arch, else $MERCURY_PDFIUM_ARCH, else this Mac's (arm64 or x64). The application
# bundle is arm64 only (bundle.sh); on an Intel development Mac the x64 build is what
# runs. `univ` (both) is pinned for later and is never used by the application.
#
# What it fetches: the pdfium-mac-<arch>.tgz asset of the bblanchon/pdfium-binaries
# release chromium/8165 (PDFium 160.0.8165.0), the build without V8 and without XFA.
# Never the pdfium-v8-* assets (about three times larger; they run form JavaScript).
# `unpack` refuses a tarball whose args.gn does not say pdf_enable_v8 = false and
# pdf_enable_xfa = false.
#
# Layout (the contract with mercury_pdfium::locate_library, which finds the library
# here when neither MERCURY_PDFIUM_LIBRARY nor <exe dir>/lib has one), the tarball
# unpacked as it is:
#   ~/Library/Caches/mercury-browser/pdfium/8165/<arch>/{lib/libpdfium.dylib,include/,
#       licenses/,LICENSE,VERSION,args.gn}   plus the hidden marker .mercury-unpacked-8165
#   ~/Library/Caches/mercury-browser/pdfium/8165/pdfium-mac-<arch>.tgz   the download
#
# Environment:
#   MERCURY_PDFIUM_ARCH       default for --arch
#   MERCURY_PDFIUM_CACHE      the cache root instead of ~/Library/Caches/mercury-browser/pdfium
#                             (CI jobs that must not share a cache, tests)
#   MERCURY_PDFIUM_URL_BASE   where the tarballs are downloaded from instead of the GitHub
#                             release (a mirror, or a file:// directory); the pins below
#                             are checked either way
#
# Nothing is installed system-wide. To bump PDFium, see docs/developer-guide/pdf.md.
# Usage: eval "$(scripts/macos/pdfium.sh env)"

set -euo pipefail

# ── The pin: one block, changed together ──────────────────────────────────
# pdfium-binaries tag chromium/<PDFIUM_RELEASE>; the digests are those of the release
# assets (GitHub's release API) and of lib/libpdfium.dylib inside each, both checked
# against real downloads on 19 Nov 2026.
PDFIUM_RELEASE="8165"
PDFIUM_VERSION="160.0.8165.0"
PDFIUM_ARM64_ASSET_SHA256="112e74d717f3e0310273b70f3c8780e8a61ff11b34834124465c8305c841d350"
PDFIUM_ARM64_LIBRARY_SHA256="d421e426bd487de02fa3bd1f2aeaf56947261f790e6116951e26f729ced1b018"
PDFIUM_X64_ASSET_SHA256="07285915b564e46a432a9909dbc1f15892885ca6489e3faeefc70dee77ed371a"
PDFIUM_X64_LIBRARY_SHA256="fcbfee9dc7f895289fbfb8f7d1f0d5d643f60fcb790708425daaa0cfa64f330d"
PDFIUM_UNIV_ASSET_SHA256="80d88c4bec1876e04448523b76d1a70ece00bbc33e15301419d4fb9d2f8cc895"
PDFIUM_UNIV_LIBRARY_SHA256="fdc694fe7e2681bf239c3462d036d4ff88c41b6f74c1533c25f36c33cfea4482"
# The licence files in the tarball's licenses/: one per component PDFium ships. A new or a
# missing one fails `check`, so that THIRD-PARTY-NOTICES.md and LICENSES/ are updated with
# the pin (docs/developer-guide/pdf.md).
PDFIUM_LICENSE_FILES="abseil.txt agg23.txt dragonbox-Apache2-LLVM.txt dragonbox-Boost.txt
fast_float.txt freetype.txt harfbuzz.txt highway.txt icu.txt lcms.txt libjpeg_turbo.ijg
libjpeg_turbo.md libopenjpeg.txt libpng.txt llvm-libc.txt pdfium.txt simdutf.txt zlib.txt"
# The install name the tarball's library carries; bundle.sh and the executable rely on it.
PDFIUM_INSTALL_NAME="@rpath/libpdfium.dylib"

PDFIUM_URL_BASE="${MERCURY_PDFIUM_URL_BASE:-https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/${PDFIUM_RELEASE}}"
cache_root="${MERCURY_PDFIUM_CACHE:-${HOME}/Library/Caches/mercury-browser/pdfium}"
release_dir="${cache_root}/${PDFIUM_RELEASE}"
unpacked_marker_name=".mercury-unpacked-${PDFIUM_RELEASE}"

say() {
    printf '%s\n' "$*" >&2
}

die() {
    say "error: $*"
    exit 1
}

require_macos() {
    [ "$(uname -s)" = "Darwin" ] || die "this script is for macOS only"
}

sha256_of() {
    /usr/bin/shasum -a 256 "$1" | cut -d ' ' -f 1
}

# ── The architecture and what is pinned for it ────────────────────────────
prefix="" archive="" library="" unpacked_marker=""
ARCH=""
ASSET_NAME=""
ASSET_SHA256=""
LIBRARY_SHA256=""
LIBRARY_ARCHS=""   # what `lipo -archs` prints for the library, in its own order
ARGS_CPU=""        # the target_cpu line args.gn must carry; empty = no such line (univ)

usage() {
    say "usage: $0 [--arch arm64|x64] fetch|unpack|check|env|prefix|library ..."
}

normalise_arch() {
    case "$1" in
        arm64 | aarch64) printf 'arm64' ;;
        x64 | x86_64 | amd64) printf 'x64' ;;
        univ | universal) printf 'univ' ;;
        *) return 1 ;;
    esac
}

select_arch() {
    local requested="${1:-}"
    if [ -z "$requested" ]; then requested="${MERCURY_PDFIUM_ARCH:-}"; fi
    if [ -z "$requested" ]; then requested="$(uname -m)"; fi
    ARCH="$(normalise_arch "$requested")" \
        || die "unknown architecture '${requested}'; use arm64 or x64 (univ is pinned but unused)"
    ASSET_NAME="pdfium-mac-${ARCH}.tgz"
    case "$ARCH" in
        arm64)
            ASSET_SHA256="$PDFIUM_ARM64_ASSET_SHA256"
            LIBRARY_SHA256="$PDFIUM_ARM64_LIBRARY_SHA256"
            LIBRARY_ARCHS="arm64"
            ARGS_CPU="arm64"
            ;;
        x64)
            ASSET_SHA256="$PDFIUM_X64_ASSET_SHA256"
            LIBRARY_SHA256="$PDFIUM_X64_LIBRARY_SHA256"
            LIBRARY_ARCHS="x86_64"
            ARGS_CPU="x64"
            ;;
        univ)
            ASSET_SHA256="$PDFIUM_UNIV_ASSET_SHA256"
            LIBRARY_SHA256="$PDFIUM_UNIV_LIBRARY_SHA256"
            LIBRARY_ARCHS="x86_64 arm64"
            ARGS_CPU=""
            ;;
    esac
    prefix="${release_dir}/${ARCH}"
    archive="${release_dir}/${ASSET_NAME}"
    library="${prefix}/lib/libpdfium.dylib"
    unpacked_marker="${prefix}/${unpacked_marker_name}"
}

# ── fetch ─────────────────────────────────────────────────────────────────
cmd_fetch() {
    require_macos
    mkdir -p "$release_dir"
    local actual
    if [ -f "$archive" ]; then
        actual="$(sha256_of "$archive")"
        if [ "$actual" = "$ASSET_SHA256" ]; then
            say "ok: ${ASSET_NAME} is already in ${release_dir} (sha256 verified)"
            return 0
        fi
        say "${archive} has sha256 ${actual}, not the pinned ${ASSET_SHA256}; deleting it and downloading again"
        rm -f -- "$archive"
    fi
    # A part file of its own per run, so that two builds fetching at once (the cache is
    # shared by every worktree) never write the same file.
    part="${archive}.part.$$"
    trap 'rm -f -- "$part"' EXIT
    say "downloading ${PDFIUM_URL_BASE}/${ASSET_NAME}"
    /usr/bin/curl --location --fail --silent --show-error --retry 5 --retry-all-errors \
        --proto '=https,file' --proto-redir '=https' --max-filesize 67108864 \
        --output "$part" "${PDFIUM_URL_BASE}/${ASSET_NAME}"
    actual="$(sha256_of "$part")"
    if [ "$actual" != "$ASSET_SHA256" ]; then
        rm -f -- "$part"
        die "${ASSET_NAME}: sha256 ${actual} does not match the pinned ${ASSET_SHA256}; the download was deleted"
    fi
    mv -f -- "$part" "$archive"
    trap - EXIT
    say "ok: ${ASSET_NAME} downloaded and verified ($(wc -c <"$archive" | tr -d ' ') bytes)"
}

# ── Verification of an unpacked directory ─────────────────────────────────
# Dies with a message unless the directory is the pinned build, unmodified. Executes
# nothing from it.
verify_unpacked() {
    local dir="$1" major minor build patch version actual archs install_name listing line dependency
    for tool in lipo otool; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool not found (install the Xcode command-line tools)"
    done
    [ -f "${dir}/${unpacked_marker_name}" ] || die "${dir} was not unpacked by this script (no ${unpacked_marker_name}); run: $0 unpack"

    [ -f "${dir}/VERSION" ] || die "${dir}/VERSION is missing"
    major="$(sed -n 's/^MAJOR=//p' "${dir}/VERSION")"
    minor="$(sed -n 's/^MINOR=//p' "${dir}/VERSION")"
    build="$(sed -n 's/^BUILD=//p' "${dir}/VERSION")"
    patch="$(sed -n 's/^PATCH=//p' "${dir}/VERSION")"
    version="${major}.${minor}.${build}.${patch}"
    [ "$version" = "$PDFIUM_VERSION" ] || die "${dir} is PDFium ${version}, the pin is ${PDFIUM_VERSION}"
    say "ok: PDFium ${version}"

    # No V8 and no XFA: the build that cannot run a PDF's JavaScript (decision OD-11).
    [ -f "${dir}/args.gn" ] || die "${dir}/args.gn is missing; the build variant cannot be checked"
    grep -Eq '^pdf_enable_v8 = false$' "${dir}/args.gn" || die "${dir}/args.gn does not say pdf_enable_v8 = false; this is the V8 build, which Mercury never uses"
    grep -Eq '^pdf_enable_xfa = false$' "${dir}/args.gn" || die "${dir}/args.gn does not say pdf_enable_xfa = false; this is the XFA build, which Mercury never uses"
    if [ -n "$ARGS_CPU" ]; then
        grep -Eq "^target_cpu = \"${ARGS_CPU}\"\$" "${dir}/args.gn" || die "${dir}/args.gn does not say target_cpu = \"${ARGS_CPU}\""
    fi
    say "ok: args.gn has pdf_enable_v8 = false and pdf_enable_xfa = false"

    local lib="${dir}/lib/libpdfium.dylib"
    if [ -L "$lib" ] || [ ! -f "$lib" ]; then die "${lib} is not a regular file"; fi
    actual="$(sha256_of "$lib")"
    [ "$actual" = "$LIBRARY_SHA256" ] || die "${lib} has sha256 ${actual}, not the pinned ${LIBRARY_SHA256}"
    say "ok: lib/libpdfium.dylib sha256 ${actual}"
    archs="$(lipo -archs "$lib")" || die "lipo cannot read ${lib}"
    [ "$archs" = "$LIBRARY_ARCHS" ] || die "${lib} is built for '${archs}', expected '${LIBRARY_ARCHS}'"
    install_name="$(otool -D "$lib" | sed -n '2p')"
    [ "$install_name" = "$PDFIUM_INSTALL_NAME" ] || die "${lib} has the install name '${install_name}', expected ${PDFIUM_INSTALL_NAME}"
    # Only the system's own libraries: nothing else would be found in a bundle.
    listing="$(otool -L "$lib")" || die "otool cannot read ${lib}"
    while IFS= read -r line; do
        case "$line" in
            $'\t'*) ;;
            *) continue ;;
        esac
        dependency="${line#$'\t'}"
        dependency="${dependency% (compatibility version*}"
        case "$dependency" in
            "$install_name" | /System/Library/* | /usr/lib/*) ;;
            *) die "${lib} loads ${dependency}; only system libraries may be needed" ;;
        esac
    done <<<"$listing"
    say "ok: ${archs}, install name ${install_name}, loads only system libraries"

    # Every component's licence text, and nothing else, as the notices list them.
    local expected found name
    # Splitting the list on white space is the point.
    # shellcheck disable=SC2086
    expected="$(printf '%s\n' $PDFIUM_LICENSE_FILES | LC_ALL=C sort)"
    [ -d "${dir}/licenses" ] || die "${dir}/licenses is missing"
    found="$(cd "${dir}/licenses" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)"
    if [ "$found" != "$expected" ]; then
        die "the licence files in ${dir}/licenses are not the ones THIRD-PARTY-NOTICES.md lists; update the notices, LICENSES/ and PDFIUM_LICENSE_FILES here. Found: $(tr '\n' ' ' <<<"$found")"
    fi
    for name in $PDFIUM_LICENSE_FILES; do
        [ -s "${dir}/licenses/${name}" ] || die "${dir}/licenses/${name} is empty"
    done
    [ -s "${dir}/LICENSE" ] || die "${dir}/LICENSE is missing"
    say "ok: licenses/ holds the $(wc -w <<<"$PDFIUM_LICENSE_FILES" | tr -d ' ') component licence files and LICENSE is there"
}

# ── unpack ────────────────────────────────────────────────────────────────
# Dies unless the archive holds only regular files and directories, and no path that
# is absolute or climbs out with "..".
validate_archive() {
    local listing names line name
    listing="$(/usr/bin/tar -tvzf "$archive")" || die "${archive} is not a readable tar.gz archive"
    names="$(/usr/bin/tar -tzf "$archive")" || die "${archive} is not a readable tar.gz archive"
    while IFS= read -r line; do
        case "${line:0:1}" in
            - | d) ;;
            *) die "${ASSET_NAME} holds something that is neither a file nor a directory: ${line}" ;;
        esac
    done <<<"$listing"
    while IFS= read -r name; do
        case "$name" in
            /* | .. | ../* | */.. | */../*) die "${ASSET_NAME} holds an unsafe path: ${name}" ;;
        esac
    done <<<"$names"
}

cmd_unpack() {
    require_macos
    [ -f "$archive" ] || die "${archive} is missing; run: make pdfium-macos (or $0 fetch)"
    local actual
    actual="$(sha256_of "$archive")"
    if [ "$actual" != "$ASSET_SHA256" ]; then
        rm -f -- "$archive"
        die "${ASSET_NAME} fails its checksum (${actual}, pinned ${ASSET_SHA256}); it was deleted, run fetch again"
    fi
    if [ -d "$prefix" ]; then
        if (verify_unpacked "$prefix" >/dev/null 2>&1); then
            say "ok: ${prefix} is already unpacked"
            return 0
        fi
        say "${prefix} is damaged or from another pin; unpacking it again"
    fi
    validate_archive
    mkdir -p "$release_dir"
    staging="$(mktemp -d "${prefix}.partial.XXXXXX")" || die "cannot create a staging directory beside ${prefix}"
    trap 'rm -rf -- "$staging"' EXIT
    /usr/bin/tar -xzf "$archive" -C "$staging" || die "cannot expand ${archive}"
    printf '%s\n' "$PDFIUM_VERSION" >"${staging}/${unpacked_marker_name}"
    chmod 755 "$staging"
    verify_unpacked "$staging"
    rm -rf -- "$prefix"
    mv -- "$staging" "$prefix"
    trap - EXIT
    say "ok: unpacked to ${prefix} ($(du -sh "$prefix" | cut -f 1))"
}

# ── The rest ──────────────────────────────────────────────────────────────
cmd_check() {
    require_macos
    [ -d "$prefix" ] || die "no unpacked PDFium in ${prefix}; run: make pdfium-macos (or $0 fetch, then $0 unpack)"
    verify_unpacked "$prefix"
    say "ok: PDFium ${PDFIUM_VERSION} (${ARCH}) from ${prefix}"
}

cmd_env() {
    require_macos
    [ -f "$unpacked_marker" ] || die "no unpacked PDFium in ${prefix}; run: make pdfium-macos (or $0 fetch, then $0 unpack)"
    printf 'export MERCURY_PDFIUM_PREFIX=%q\n' "$prefix"
    printf 'export MERCURY_PDFIUM_LIBRARY=%q\n' "$library"
}

# Run as a program; sourced (by collect-pdfium.sh and test-pdfium.sh) it only defines the
# pins and the functions above.
main() {
    local arch_argument="" commands=() command_name
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --arch)
                [ "$#" -ge 2 ] || { usage; exit 2; }
                arch_argument="$2"
                shift 2
                ;;
            --arch=*)
                arch_argument="${1#--arch=}"
                shift
                ;;
            fetch | unpack | check | env | prefix | library)
                commands+=("$1")
                shift
                ;;
            *)
                usage
                exit 2
                ;;
        esac
    done
    [ "${#commands[@]}" -gt 0 ] || { usage; exit 2; }

    select_arch "$arch_argument"
    for command_name in "${commands[@]}"; do
        case "$command_name" in
            fetch) cmd_fetch ;;
            unpack) cmd_unpack ;;
            check) cmd_check ;;
            env) cmd_env ;;
            prefix) printf '%s\n' "$prefix" ;;
            library) printf '%s\n' "$library" ;;
        esac
    done
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main "$@"
fi
