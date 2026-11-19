#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Checks the PDFium supply scripts: scripts/macos/pdfium.sh (fetch, unpack, check, env), the
# collector scripts/macos/collect-pdfium.sh and the PDFium lines of bundle.sh, without a build.
#
#   scripts/macos/test-pdfium.sh            offline: a tiny fake tarball, served from a file://
#                                           directory (MERCURY_PDFIUM_URL_BASE), in a temporary
#                                           directory
#   scripts/macos/test-pdfium.sh --online   also the real release: fetch, unpack and check for x64;
#                                           arm64 and the universal build fetched and checked
#                                           statically (lipo, otool; never executed, this may be
#                                           an Intel Mac); the licence texts in LICENSES/ against
#                                           the tarball's; the collector on the real arm64 build
#
# The offline checks build the fake library with `cc` (an empty C file from standard input, so no
# source is kept); without a compiler the checks that need a Mach-O file are skipped and say so.
# The pins in pdfium.sh are changed only for the in-process checks of the good path, in a subshell:
# the checks of the script as it is, run against a fake tarball, must refuse it.
#
# Prints one line per check and exits non-zero if any fails. Writes only to a temporary
# directory, which it removes.
#
# Listings with ls (file names this script made, all plain), the unquoted licence-name list (white
# space separated on purpose) and the subshell-local exports are deliberate throughout.
# shellcheck disable=SC2010,SC2012,SC2016,SC2030,SC2031,SC2086
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."
REPO="$PWD"

ONLINE=0
case "${1:-}" in
    "") ;;
    --online) ONLINE=1 ;;
    *)
        printf 'usage: %s [--online]\n' "$0" >&2
        exit 2
        ;;
esac

WORK="$(mktemp -d "${TMPDIR:-/tmp}/mercury-test-pdfium.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT
failures=0

check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s: expected [%s], got [%s]\n' "$name" "$expected" "$actual"
        failures=$((failures + 1))
    fi
}

skip() {
    printf 'SKIP %s\n' "$*"
}

contains_text() { # haystack needle
    case "$1" in *"$2"*) echo yes ;; *) echo no ;; esac
}

sha() {
    shasum -a 256 "$1" | cut -d ' ' -f 1
}

# The values the script holds, read in a subshell (it defines die, say and others).
# shellcheck source=scripts/macos/pdfium.sh
pin() { ( . "$REPO/scripts/macos/pdfium.sh" && eval "printf '%s' \"\$$1\"" ); }

LICENSE_NAMES="$(pin PDFIUM_LICENSE_FILES)"

# Where each component's licence text is kept in LICENSES/ (copied from the tarball, not
# retyped). The tarball's own file names are the first column.
licence_copy() {
    case "$1" in
        LICENSE) echo MIT-pdfium-binaries.txt ;;
        abseil.txt) echo Apache-2.0-abseil.txt ;;
        agg23.txt) echo LicenseRef-agg23.txt ;;
        dragonbox-Apache2-LLVM.txt) echo Apache-2.0-LLVM-exception-dragonbox.txt ;;
        dragonbox-Boost.txt) echo BSL-1.0-dragonbox.txt ;;
        fast_float.txt) echo MIT-fast_float.txt ;;
        freetype.txt) echo FTL-freetype.txt ;;
        harfbuzz.txt) echo MIT-harfbuzz.txt ;;
        highway.txt) echo Apache-2.0-highway.txt ;;
        icu.txt) echo Unicode-3.0-icu.txt ;;
        lcms.txt) echo MIT-lcms.txt ;;
        libjpeg_turbo.ijg) echo IJG-libjpeg-turbo.txt ;;
        libjpeg_turbo.md) echo BSD-3-Clause-libjpeg-turbo.md ;;
        libopenjpeg.txt) echo BSD-2-Clause-openjpeg.txt ;;
        libpng.txt) echo libpng-2.0-libpng.txt ;;
        llvm-libc.txt) echo Apache-2.0-LLVM-exception-llvm-libc.txt ;;
        pdfium.txt) echo BSD-3-Clause-pdfium.txt ;;
        simdutf.txt) echo MIT-simdutf.txt ;;
        zlib.txt) echo Zlib-zlib.txt ;;
        *) echo "" ;;
    esac
}

# ── Running the scripts ───────────────────────────────────────────────────────────────────
# The output and the exit status of a command, as one line.
outcome() {
    local output code=0
    output="$("$@" 2>&1)" || code=$?
    printf '%s (exit %s)' "$output" "$code"
}

# run_pdfium <arch> <asset sha> <library sha> <cache> <url base> <arguments of pdfium.sh...>
# (The names here avoid the globals pdfium.sh sets when it is sourced: library, prefix, archive...)
# runs pdfium.sh's own main in a subshell with the pins of <arch> replaced (the fake build is not
# the release). Prints its output and "(exit N)".
run_pdfium() {
    local pin_arch="$1" pin_asset="$2" pin_library="$3" cache_dir="$4" url_base="$5" output code=0
    shift 5
    output="$(
        exec 2>&1
        export MERCURY_PDFIUM_CACHE="$cache_dir" MERCURY_PDFIUM_URL_BASE="$url_base"
        # shellcheck source=scripts/macos/pdfium.sh
        . "$REPO/scripts/macos/pdfium.sh"
        # (An if chain: bash 3.2 mis-reads a case inside $( ) .)
        if [ "$pin_arch" = arm64 ]; then
            PDFIUM_ARM64_ASSET_SHA256="$pin_asset"
            PDFIUM_ARM64_LIBRARY_SHA256="$pin_library"
        else
            PDFIUM_X64_ASSET_SHA256="$pin_asset"
            PDFIUM_X64_LIBRARY_SHA256="$pin_library"
        fi
        main --arch "$pin_arch" "$@"
    )" || code=$?
    printf '%s (exit %s)' "$output" "$code"
}

# run_collect <arch of the pins to replace> <library sha> <collect_main arguments...>
run_collect() {
    local pin_arch="$1" pin_library="$2" output code=0
    shift 2
    output="$(
        exec 2>&1
        # shellcheck source=scripts/macos/collect-pdfium.sh
        . "$REPO/scripts/macos/collect-pdfium.sh"
        if [ "$pin_arch" = arm64 ]; then
            PDFIUM_ARM64_LIBRARY_SHA256="$pin_library"
        else
            PDFIUM_X64_LIBRARY_SHA256="$pin_library"
        fi
        collect_main "$@"
    )" || code=$?
    printf '%s (exit %s)' "$output" "$code"
}

# ── The fake build ────────────────────────────────────────────────────────────────────────
HAVE_CC=0
if command -v cc >/dev/null 2>&1 && printf '' | cc -x c - -w -dynamiclib -arch "$(uname -m)" \
    -install_name @rpath/libprobe.dylib -o "$WORK/libprobe.dylib" 2>/dev/null; then
    HAVE_CC=1
fi

# build_dylib <out> <arm64|x64> [install name] [extra cc arguments...]: an empty library
build_dylib() {
    local out="$1" arch="$2" install="${3:-@rpath/libpdfium.dylib}" cc_arch
    shift 2
    [ "$#" -eq 0 ] || shift
    case "$arch" in
        arm64) cc_arch=arm64 ;;
        *) cc_arch=x86_64 ;;
    esac
    printf '' | cc -x c - -w -dynamiclib -arch "$cc_arch" -mmacosx-version-min=13.0 \
        -install_name "$install" -o "$out" "$@"
}

# build_tree <dir> <arm64|x64>: what a PDFium tarball holds, much smaller.
build_tree() {
    local dir="$1" arch="$2" name
    local cpu="$arch"
    rm -rf -- "$dir"
    mkdir -p "$dir/lib" "$dir/include" "$dir/licenses"
    printf 'MAJOR=160\nMINOR=0\nBUILD=8165\nPATCH=0\n' >"$dir/VERSION"
    {
        printf 'clang_use_chrome_plugins = false\nis_component_build = false\nis_debug = false\n'
        printf 'pdf_enable_v8 = false\npdf_enable_xfa = false\npdf_is_standalone = true\n'
        printf 'pdf_use_partition_alloc = false\ntarget_cpu = "%s"\ntarget_os = "mac"\n' "$cpu"
        printf 'treat_warnings_as_errors = false\n'
    } >"$dir/args.gn"
    printf 'The builds are MIT; the components are in licenses/.\n' >"$dir/LICENSE"
    printf '/* fake header */\n' >"$dir/include/fpdfview.h"
    # shellcheck disable=SC2086
    for name in $LICENSE_NAMES; do
        printf 'fake licence text of %s\n' "$name" >"$dir/licenses/$name"
    done
    build_dylib "$dir/lib/libpdfium.dylib" "$arch"
}

pack() { # <tree> <out.tgz>: every top-level entry of the tree
    (cd "$1" && tar -czf "$2" -- *)
}

# ── 1. What the script pins ───────────────────────────────────────────────────────────────
check "the pinned release is 8165 (PDFium 160.0.8165.0)" "8165 160.0.8165.0" \
    "$(pin PDFIUM_RELEASE) $(pin PDFIUM_VERSION)"
for name in PDFIUM_ARM64_ASSET_SHA256 PDFIUM_ARM64_LIBRARY_SHA256 PDFIUM_X64_ASSET_SHA256 \
    PDFIUM_X64_LIBRARY_SHA256 PDFIUM_UNIV_ASSET_SHA256 PDFIUM_UNIV_LIBRARY_SHA256; do
    case "$(pin "$name")" in
        *[!0-9a-f]* | "") shape=bad ;;
        *) shape="$([ "$(pin "$name" | wc -c | tr -d ' ')" = 64 ] && echo ok || echo bad)" ;;
    esac
    check "$name is a SHA-256" ok "$shape"
done
check "the pins are distinct" 6 \
    "$(for name in PDFIUM_ARM64_ASSET_SHA256 PDFIUM_ARM64_LIBRARY_SHA256 PDFIUM_X64_ASSET_SHA256 \
        PDFIUM_X64_LIBRARY_SHA256 PDFIUM_UNIV_ASSET_SHA256 PDFIUM_UNIV_LIBRARY_SHA256; do pin "$name"; echo; done |
        sed '/^$/d' | sort -u | wc -l | tr -d ' ')"
check "the default download is the GitHub release chromium/8165 of pdfium-binaries" yes \
    "$(contains_text "$(env -u MERCURY_PDFIUM_URL_BASE bash -c '. scripts/macos/pdfium.sh && printf %s "$PDFIUM_URL_BASE"')" \
        'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8165')"
check "the script never names a pdfium-v8 asset outside comments" 0 \
    "$(grep -v '^ *#' scripts/macos/pdfium.sh | grep -c 'pdfium-v8' || true)"
check "18 component licence files are pinned" 18 "$(printf '%s\n' $LICENSE_NAMES | wc -l | tr -d ' ')"
check "every pinned licence file has a copy name in LICENSES/" "" \
    "$(for name in $LICENSE_NAMES LICENSE; do [ -n "$(licence_copy "$name")" ] || printf '%s ' "$name"; done)"
check "the scripts are executable" "" \
    "$(for script in pdfium.sh collect-pdfium.sh test-pdfium.sh; do [ -x "scripts/macos/$script" ] || printf '%s ' "$script"; done)"

# ── 2. The command line ───────────────────────────────────────────────────────────────────
usage_text="usage: scripts/macos/pdfium.sh [--arch arm64|x64] fetch|unpack|check|env|prefix|library ..."
check "no command prints the usage and exits 2" "$usage_text (exit 2)" "$(outcome scripts/macos/pdfium.sh)"
check "an unknown command prints the usage and exits 2" "$usage_text (exit 2)" \
    "$(outcome scripts/macos/pdfium.sh bogus)"
check "--arch without a value prints the usage and exits 2" "$usage_text (exit 2)" \
    "$(outcome scripts/macos/pdfium.sh --arch)"
check "an unknown architecture is refused" \
    "error: unknown architecture 'riscv'; use arm64 or x64 (univ is pinned but unused) (exit 1)" \
    "$(outcome scripts/macos/pdfium.sh --arch riscv prefix)"

# ── 3. Where things go (the contract with mercury_pdfium::locate_library) ─────────────────
CACHE="$WORK/cache-paths"
check "prefix is <cache>/8165/<arch>" "$CACHE/8165/arm64" \
    "$(MERCURY_PDFIUM_CACHE="$CACHE" scripts/macos/pdfium.sh --arch arm64 prefix)"
check "library is <prefix>/lib/libpdfium.dylib" "$CACHE/8165/x64/lib/libpdfium.dylib" \
    "$(MERCURY_PDFIUM_CACHE="$CACHE" scripts/macos/pdfium.sh --arch x64 library)"
check "MERCURY_PDFIUM_ARCH picks the architecture" "$CACHE/8165/x64" \
    "$(MERCURY_PDFIUM_CACHE="$CACHE" MERCURY_PDFIUM_ARCH=x64 scripts/macos/pdfium.sh prefix)"
check "--arch beats MERCURY_PDFIUM_ARCH" "$CACHE/8165/arm64" \
    "$(MERCURY_PDFIUM_CACHE="$CACHE" MERCURY_PDFIUM_ARCH=x64 scripts/macos/pdfium.sh --arch arm64 prefix)"
case "$(uname -m)" in
    arm64) host=arm64 ;;
    *) host=x64 ;;
esac
check "the architecture defaults to this Mac's" "$CACHE/8165/$host" \
    "$(env -u MERCURY_PDFIUM_ARCH MERCURY_PDFIUM_CACHE="$CACHE" scripts/macos/pdfium.sh prefix)"
check "the default cache is ~/Library/Caches/mercury-browser/pdfium" \
    "$HOME/Library/Caches/mercury-browser/pdfium/8165/arm64" \
    "$(env -u MERCURY_PDFIUM_CACHE scripts/macos/pdfium.sh --arch arm64 prefix)"
check "env refuses when nothing is unpacked" yes \
    "$(contains_text "$(outcome env MERCURY_PDFIUM_CACHE="$CACHE" scripts/macos/pdfium.sh --arch arm64 env)" 'no unpacked PDFium')"
check "check refuses when nothing is unpacked" yes \
    "$(contains_text "$(outcome env MERCURY_PDFIUM_CACHE="$CACHE" scripts/macos/pdfium.sh --arch arm64 check)" 'no unpacked PDFium')"
check "nothing was written by those" no "$([ -e "$CACHE" ] && echo yes || echo no)"

# ── 4. The pins are enforced (the script as it is, against a fake tarball) ────────────────
SERVE="$WORK/serve"
mkdir -p "$SERVE"
if [ "$HAVE_CC" = 1 ]; then
    build_tree "$WORK/tree-arm64" arm64
    pack "$WORK/tree-arm64" "$SERVE/pdfium-mac-arm64.tgz"
else
    printf 'not a real tarball\n' >"$SERVE/pdfium-mac-arm64.tgz"
fi
ENFORCED="$WORK/cache-enforced"
result="$(outcome env MERCURY_PDFIUM_CACHE="$ENFORCED" MERCURY_PDFIUM_URL_BASE="file://$SERVE" \
    scripts/macos/pdfium.sh --arch arm64 fetch)"
check "a tarball that is not the pinned one is refused by fetch" yes \
    "$(contains_text "$result" 'does not match the pinned')"
check "the refused fetch exits 1" yes "$(contains_text "$result" '(exit 1)')"
check "the refused download is deleted, part file included" "" \
    "$(find "$ENFORCED" -type f 2>/dev/null | tr '\n' ' ')"
mkdir -p "$ENFORCED/8165"
cp "$SERVE/pdfium-mac-arm64.tgz" "$ENFORCED/8165/pdfium-mac-arm64.tgz"
result="$(outcome env MERCURY_PDFIUM_CACHE="$ENFORCED" scripts/macos/pdfium.sh --arch arm64 unpack)"
check "unpack refuses a tarball that fails its checksum" yes "$(contains_text "$result" 'fails its checksum')"
check "and deletes it" no "$([ -e "$ENFORCED/8165/pdfium-mac-arm64.tgz" ] && echo yes || echo no)"
check "and unpacks nothing" no "$([ -e "$ENFORCED/8165/arm64" ] && echo yes || echo no)"
result="$(outcome env MERCURY_PDFIUM_CACHE="$ENFORCED" scripts/macos/pdfium.sh --arch arm64 unpack)"
check "unpack without a download says to fetch" yes "$(contains_text "$result" 'is missing; run: make pdfium-macos')"

# ── 5. The good path, with the pins of the fake build ─────────────────────────────────────
if [ "$HAVE_CC" = 1 ]; then
    ASSET_SHA="$(sha "$SERVE/pdfium-mac-arm64.tgz")"
    LIBRARY_SHA="$(sha "$WORK/tree-arm64/lib/libpdfium.dylib")"
    GOOD="$WORK/cache-good"
    result="$(run_pdfium arm64 "$ASSET_SHA" "$LIBRARY_SHA" "$GOOD" "file://$SERVE" fetch unpack check)"
    check "fetch, unpack and check pass on the fake build" yes "$(contains_text "$result" '(exit 0)')"
    check "the download is kept, verified" "$ASSET_SHA" "$(sha "$GOOD/8165/pdfium-mac-arm64.tgz")"
    check "the tarball is unpacked as it is" "$(cd "$WORK/tree-arm64" && find . -type f | LC_ALL=C sort)" \
        "$(cd "$GOOD/8165/arm64" && find . -type f ! -name '.mercury-unpacked-*' | LC_ALL=C sort)"
    check "the library is byte for byte the one in the tarball" "$LIBRARY_SHA" "$(sha "$GOOD/8165/arm64/lib/libpdfium.dylib")"
    check "a marker names the release" "160.0.8165.0" "$(cat "$GOOD/8165/arm64/.mercury-unpacked-8165")"
    check "the directory is readable by everyone" "755" "$(stat -f %Lp "$GOOD/8165/arm64")"
    check "no staging directory is left" "arm64 pdfium-mac-arm64.tgz" "$(ls "$GOOD/8165" | tr '\n' ' ' | sed 's/ $//')"
    check "a second unpack changes nothing" yes \
        "$(contains_text "$(run_pdfium arm64 "$ASSET_SHA" "$LIBRARY_SHA" "$GOOD" "file://$SERVE" unpack)" 'is already unpacked')"
    check "a second fetch finds the download" yes \
        "$(contains_text "$(run_pdfium arm64 "$ASSET_SHA" "$LIBRARY_SHA" "$GOOD" "file://$SERVE" fetch)" 'is already in')"
    env_text="$(MERCURY_PDFIUM_CACHE="$GOOD" bash -c '
        . scripts/macos/pdfium.sh
        PDFIUM_ARM64_ASSET_SHA256='"$ASSET_SHA"' PDFIUM_ARM64_LIBRARY_SHA256='"$LIBRARY_SHA"'
        eval "$(main --arch arm64 env)"
        printf "%s %s" "$MERCURY_PDFIUM_PREFIX" "$MERCURY_PDFIUM_LIBRARY"')"
    check "env exports the prefix and the library" \
        "$GOOD/8165/arm64 $GOOD/8165/arm64/lib/libpdfium.dylib" "$env_text"

    # A damaged unpacked library: check fails, unpack repairs it.
    printf 'x' >>"$GOOD/8165/arm64/lib/libpdfium.dylib"
    check "check notices a library that was changed" yes \
        "$(contains_text "$(run_pdfium arm64 "$ASSET_SHA" "$LIBRARY_SHA" "$GOOD" "file://$SERVE" check)" 'not the pinned')"
    check "unpack repairs it" yes \
        "$(contains_text "$(run_pdfium arm64 "$ASSET_SHA" "$LIBRARY_SHA" "$GOOD" "file://$SERVE" unpack check)" 'damaged or from another pin')"
    check "the repaired library is the pinned one" "$LIBRARY_SHA" "$(sha "$GOOD/8165/arm64/lib/libpdfium.dylib")"
else
    skip "the good path (no C compiler to make a fake library)"
fi

# ── 6. What unpack refuses, one defect at a time ──────────────────────────────────────────
LIBRARY_SHA_OVERRIDE=""

# neg_case <name> <mutation function> <part of the message> [arm64|x64 for the tree]
neg_case() {
    local name="$1" mutate="$2" expect="$3" arch="${4:-arm64}"
    local tree="$WORK/neg-tree" cache="$WORK/neg-cache" serve="$WORK/neg-serve" asset library result
    rm -rf -- "$cache" "$serve"
    mkdir -p "$serve"
    LIBRARY_SHA_OVERRIDE=""
    build_tree "$tree" "$arch"
    "$mutate" "$tree"
    pack "$tree" "$serve/pdfium-mac-$arch.tgz"
    asset="$(sha "$serve/pdfium-mac-$arch.tgz")"
    library="${LIBRARY_SHA_OVERRIDE:-$(sha "$tree/lib/libpdfium.dylib")}"
    result="$(run_pdfium "$arch" "$asset" "$library" "$cache" "file://$serve" fetch unpack)"
    check "$name is refused" "yes yes" "$(contains_text "$result" "$expect") $(contains_text "$result" '(exit 1)')"
    check "$name leaves nothing unpacked" "" \
        "$(ls "$cache/8165" 2>/dev/null | grep -v '\.tgz$' | tr '\n' ' ')"
}

if [ "$HAVE_CC" = 1 ]; then
    m_v8() { sed -i '' 's/^pdf_enable_v8 = false$/pdf_enable_v8 = true/' "$1/args.gn"; }
    m_xfa() { sed -i '' 's/^pdf_enable_xfa = false$/pdf_enable_xfa = true/' "$1/args.gn"; }
    m_no_args() { rm -f "$1/args.gn"; }
    m_cpu() { sed -i '' 's/^target_cpu = "arm64"$/target_cpu = "x64"/' "$1/args.gn"; }
    m_version() { printf 'MAJOR=161\nMINOR=0\nBUILD=8200\nPATCH=0\n' >"$1/VERSION"; }
    m_no_licence() { rm -f "$1/licenses/zlib.txt"; }
    m_extra_licence() { printf 'new component\n' >"$1/licenses/newlib.txt"; }
    m_empty_licence() { : >"$1/licenses/icu.txt"; }
    m_no_top_licence() { rm -f "$1/LICENSE"; }
    m_wrong_library() { LIBRARY_SHA_OVERRIDE="$(printf 'nothing' | shasum -a 256 | cut -d ' ' -f 1)"; }
    m_wrong_arch() { build_dylib "$1/lib/libpdfium.dylib" x64; }
    m_install_name() { build_dylib "$1/lib/libpdfium.dylib" arm64 /opt/pdfium/libpdfium.dylib; }
    m_foreign_dependency() {
        build_dylib "$WORK/libother.dylib" arm64 /opt/other/libother.dylib
        build_dylib "$1/lib/libpdfium.dylib" arm64 @rpath/libpdfium.dylib "-Wl,-needed_library,$WORK/libother.dylib"
    }
    m_symlink_library() {
        mv "$1/lib/libpdfium.dylib" "$1/lib/real.dylib"
        ln -s real.dylib "$1/lib/libpdfium.dylib"
    }
    m_symlink_entry() { ln -s /etc/passwd "$1/link"; }

    neg_case "a build with V8" m_v8 'pdf_enable_v8 = false'
    neg_case "a build with XFA" m_xfa 'pdf_enable_xfa = false'
    neg_case "a build without args.gn" m_no_args 'args.gn is missing'
    neg_case "an x64 build under the arm64 pin" m_cpu 'target_cpu = "arm64"'
    neg_case "another release" m_version 'is PDFium 161.0.8200.0, the pin is 160.0.8165.0'
    neg_case "a missing component licence" m_no_licence 'are not the ones THIRD-PARTY-NOTICES.md lists'
    neg_case "a new component licence" m_extra_licence 'are not the ones THIRD-PARTY-NOTICES.md lists'
    neg_case "an empty component licence" m_empty_licence 'icu.txt is empty'
    neg_case "a missing LICENSE" m_no_top_licence 'LICENSE is missing'
    neg_case "a library with another digest" m_wrong_library 'not the pinned'
    neg_case "a library for the wrong architecture" m_wrong_arch "is built for 'x86_64', expected 'arm64'"
    neg_case "a library with another install name" m_install_name 'has the install name'
    neg_case "a library that loads a non-system library" m_foreign_dependency 'only system libraries may be needed'
    neg_case "a library that is a symbolic link" m_symlink_library 'neither a file nor a directory'
    neg_case "an archive with a symbolic link" m_symlink_entry 'neither a file nor a directory'

    # An archive path that climbs out of the directory (made with bsdtar's -s).
    tree="$WORK/neg-tree"
    build_tree "$tree" arm64
    cache="$WORK/neg-cache2"
    serve="$WORK/neg-serve2"
    mkdir -p "$serve"
    printf 'evil\n' >"$tree/evil.txt"
    tar -czf "$serve/pdfium-mac-arm64.tgz" -C "$tree" -s '|^evil.txt$|../evil.txt|' evil.txt \
        LICENSE VERSION args.gn include lib licenses 2>/dev/null || true
    if tar -tzf "$serve/pdfium-mac-arm64.tgz" 2>/dev/null | grep -q '^\.\./evil.txt$'; then
        result="$(run_pdfium arm64 "$(sha "$serve/pdfium-mac-arm64.tgz")" "$(sha "$tree/lib/libpdfium.dylib")" \
            "$cache" "file://$serve" fetch unpack)"
        check "an archive path that climbs out is refused" "yes yes" \
            "$(contains_text "$result" 'holds an unsafe path') $(contains_text "$result" '(exit 1)')"
        check "and nothing was written beside the cache" no "$([ -e "$cache/evil.txt" ] || [ -e "$cache/8165/evil.txt" ] && echo yes || echo no)"
    else
        skip "an archive path that climbs out (this tar cannot make one)"
    fi
else
    skip "the refusals of defective builds (no C compiler)"
fi

# ── 7. The collector ──────────────────────────────────────────────────────────────────────
if [ "$HAVE_CC" = 1 ]; then
    tree="$WORK/tree-arm64"
    LIBRARY_SHA="$(sha "$tree/lib/libpdfium.dylib")"
    PREFIX="$WORK/cache-good/8165/arm64"
    APP="$WORK/Mercury.app/Contents"
    mkdir -p "$APP/MacOS/lib" "$APP/Resources"
    : >"$APP/MacOS/lib/libother.dylib"
    result="$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$APP/MacOS/lib" "$APP/Resources")"
    check "the collector fills a bundle" yes "$(contains_text "$result" '(exit 0)')"
    check "the library is copied unmodified" "$LIBRARY_SHA" "$(sha "$APP/MacOS/lib/libpdfium.dylib")"
    check "the library is executable by its owner and no one else may write it" "755" "$(stat -f %Lp "$APP/MacOS/lib/libpdfium.dylib")"
    check "its install name is @rpath/libpdfium.dylib" "@rpath/libpdfium.dylib" "$(otool -D "$APP/MacOS/lib/libpdfium.dylib" | sed -n 2p)"
    check "the licences are LICENSE and the component files, flat" \
        "$(printf '%s\n' LICENSE $LICENSE_NAMES | LC_ALL=C sort | tr '\n' ' ')" \
        "$(ls "$APP/Resources/pdfium-licenses" | LC_ALL=C sort | tr '\n' ' ')"
    check "the licence texts are the tarball's, byte for byte" "" \
        "$(for name in $LICENSE_NAMES; do cmp -s "$PREFIX/licenses/$name" "$APP/Resources/pdfium-licenses/$name" || printf '%s ' "$name"; done)"
    check "LICENSE is the build's" yes "$(cmp -s "$PREFIX/LICENSE" "$APP/Resources/pdfium-licenses/LICENSE" && echo yes || echo no)"
    check "no marker or staging file is in the bundle" "libother.dylib libpdfium.dylib" \
        "$(ls -A "$APP/MacOS/lib" | tr '\n' ' ' | sed 's/ $//')"
    check "nothing but the licences is in the Resources" "pdfium-licenses" "$(ls -A "$APP/Resources" | tr '\n' ' ' | sed 's/ $//')"

    check "a bundle's lib directory is collected once" yes \
        "$(contains_text "$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$APP/MacOS/lib" "$APP/Resources")" 'already exists')"

    # An x64 build never goes into a bundle.
    build_tree "$WORK/tree-x64" x64
    pack "$WORK/tree-x64" "$SERVE/pdfium-mac-x64.tgz"
    X64_ASSET="$(sha "$SERVE/pdfium-mac-x64.tgz")"
    X64_LIBRARY="$(sha "$WORK/tree-x64/lib/libpdfium.dylib")"
    run_pdfium x64 "$X64_ASSET" "$X64_LIBRARY" "$WORK/cache-good" "file://$SERVE" fetch unpack >/dev/null
    BAD="$WORK/Bad.app/Contents"
    mkdir -p "$BAD/MacOS/lib" "$BAD/Resources"
    result="$(run_collect x64 "$X64_LIBRARY" "$WORK/cache-good/8165/x64" "$BAD/MacOS/lib" "$BAD/Resources")"
    check "an x64 library is refused for a bundle" yes "$(contains_text "$result" 'the application is arm64 only')"
    check "and nothing was written for it" "" "$(find "$BAD/MacOS/lib" "$BAD/Resources" -mindepth 1 | tr '\n' ' ')"

    # A prefix whose library was changed is refused before anything is written.
    TAMPERED="$WORK/tampered"
    cp -R "$PREFIX" "$TAMPERED"
    printf 'x' >>"$TAMPERED/lib/libpdfium.dylib"
    mkdir -p "$BAD/MacOS/lib"
    result="$(run_collect arm64 "$LIBRARY_SHA" "$TAMPERED" "$BAD/MacOS/lib" "$BAD/Resources")"
    check "a prefix whose library was changed is refused" yes "$(contains_text "$result" 'not the pinned')"
    check "and nothing was written for that either" "" "$(find "$BAD/MacOS/lib" "$BAD/Resources" -mindepth 1 | tr '\n' ' ')"

    # A directory that this script's unpack did not make.
    HAND="$WORK/by-hand"
    cp -R "$PREFIX" "$HAND"
    rm -f "$HAND"/.mercury-unpacked-*
    result="$(run_collect arm64 "$LIBRARY_SHA" "$HAND" "$BAD/MacOS/lib" "$BAD/Resources")"
    check "a prefix that was not unpacked by pdfium.sh is refused" yes "$(contains_text "$result" 'was not unpacked by this script')"
    check "a prefix that is not there is refused" yes \
        "$(contains_text "$(run_collect arm64 "$LIBRARY_SHA" "$WORK/nowhere" "$BAD/MacOS/lib" "$BAD/Resources")" 'is not a directory')"

    # Anywhere else than a bundle.
    LOOSE="$WORK/loose"
    result="$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$LOOSE/lib" "$LOOSE/res")"
    check "a loose directory is filled too" yes "$(contains_text "$result" '(exit 0)')"
    check "and again, replacing what it made" yes \
        "$(contains_text "$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$LOOSE/lib" "$LOOSE/res")" '(exit 0)')"
    check "its licence folder carries a marker" yes "$([ -f "$LOOSE/res/pdfium-licenses/.mercury-pdfium" ] && echo yes || echo no)"
    mkdir -p "$LOOSE/foreign/lib" "$LOOSE/foreign/res/pdfium-licenses"
    check "a licence folder it did not make is never replaced" yes \
        "$(contains_text "$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$LOOSE/foreign/lib" "$LOOSE/foreign/res")" 'refusing to replace it')"
    mkdir -p "$LOOSE/gst"
    : >"$LOOSE/gst/.gstreamer-1.28.9"
    check "a collected GStreamer directory is refused (it would delete the library)" yes \
        "$(contains_text "$(run_collect arm64 "$LIBRARY_SHA" "$PREFIX" "$LOOSE/gst" "$LOOSE/gst-res")" 'collect-gstreamer.sh would rebuild it')"
    check "the arm64 pin is the one the bundle needs: an x64 library in a loose directory is fine" yes \
        "$(contains_text "$(run_collect x64 "$X64_LIBRARY" "$WORK/cache-good/8165/x64" "$LOOSE/x64lib" "$LOOSE/x64res")" '(exit 0)')"
else
    skip "the collector (no C compiler to make a fake library)"
fi

# ── 8. bundle.sh stops before building without PDFium ─────────────────────────────────────
GST="$WORK/fake-gstreamer"
mkdir -p "$GST/lib"
: >"$GST/lib/libgstreamer-1.0.0.dylib"
OUT="$WORK/bundle-out"
unset MERCURY_SIGN_IDENTITY
bundle_status() {
    outcome scripts/macos/bundle.sh --out "$OUT" --gstreamer-prefix "$GST" "$@"
}
check "bundle.sh refuses a PDFium prefix that does not exist" yes \
    "$(contains_text "$(bundle_status --pdfium-prefix "$WORK/no-pdfium")" 'is not an unpacked arm64 PDFium')"
check "bundle.sh refuses a directory that is not PDFium" yes \
    "$(contains_text "$(bundle_status --pdfium-prefix "$REPO/scripts")" 'is not an unpacked arm64 PDFium')"
check "without --pdfium-prefix it looks in the cache and says how to fetch it" yes \
    "$(contains_text "$(MERCURY_PDFIUM_CACHE="$WORK/empty-cache" bundle_status)" 'run make pdfium-macos')"
check "--pdfium-prefix needs a value" "bundle: error: --pdfium-prefix needs a value (exit 1)" \
    "$(outcome scripts/macos/bundle.sh --pdfium-prefix)"
check "the refusals wrote nothing" no "$([ -e "$OUT" ] && echo yes || echo no)"
check "bundle.sh collects PDFium with collect-pdfium.sh" yes \
    "$(grep -q 'bash "\$PDFIUM_COLLECTOR" "\$PDFIUM_PREFIX" "\$BIN_DIR/lib" "\$RES_DIR"' scripts/macos/bundle.sh && echo yes || echo no)"
check "bundle.sh asks for the arm64 build by default" yes \
    "$(grep -q 'scripts/macos/pdfium.sh --arch arm64 prefix' scripts/macos/bundle.sh && echo yes || echo no)"
check "bundle.sh collects PDFium after the media libraries and before signing" yes \
    "$(awk '/^bash "\$COLLECTOR"/ && !g {g=NR} /^bash "\$PDFIUM_COLLECTOR"/ && !p {p=NR} /^say "signing with identity/ && !s {s=NR} END {print (g && p > g && s > p) ? "yes" : "no"}' scripts/macos/bundle.sh)"

# ── 9. The wiring ─────────────────────────────────────────────────────────────────────────
check "the Makefile has a pdfium-macos target" yes "$(grep -q '^pdfium-macos:' Makefile && echo yes || echo no)"
check "and a help line for it" yes "$(grep -q "'pdfium-macos'" Makefile && echo yes || echo no)"
check "app-macos and package-macos need it, for arm64" yes \
    "$(grep -q '^app-macos package-macos: pdfium-macos$' Makefile && grep -q '^app-macos package-macos: PDFIUM_ARCH = arm64$' Makefile && echo yes || echo no)"
check "CI fetches PDFium before it assembles the bundle" yes \
    "$(awk '/make pdfium-macos/ && !f {f=NR} /make app-macos/ && !a {a=NR} END {print (f && a > f) ? "yes" : "no"}' .github/workflows/ci.yml)"
check "the release build job fetches PDFium too" yes "$(grep -c 'make pdfium-macos' .github/workflows/release.yml | awk '{print ($1 >= 1) ? "yes" : "no"}')"
check "the signing job downloads PDFium afresh into its own directory" yes \
    "$(grep -q 'MERCURY_PDFIUM_CACHE: \${{ runner.temp }}/pdfium' .github/workflows/release.yml && echo yes || echo no)"
check "every component licence has a copy in LICENSES/" "" \
    "$(for name in LICENSE $LICENSE_NAMES; do [ -s "LICENSES/$(licence_copy "$name")" ] || printf '%s ' "$name"; done)"
check "THIRD-PARTY-NOTICES.md names PDFium and says there is no V8 and no XFA" yes \
    "$(grep -q 'PDFium' THIRD-PARTY-NOTICES.md && grep -qi 'no V8' THIRD-PARTY-NOTICES.md && grep -q 'XFA' THIRD-PARTY-NOTICES.md && echo yes || echo no)"
check "every component named in the notices has its licence file named too" "" \
    "$(for name in $LICENSE_NAMES LICENSE; do grep -q "LICENSES/$(licence_copy "$name")" THIRD-PARTY-NOTICES.md || printf '%s ' "$name"; done)"

# ── 10. The real release (--online) ───────────────────────────────────────────────────────
if [ "$ONLINE" = 1 ]; then
    REAL="$WORK/real-cache"
    export MERCURY_PDFIUM_CACHE="$REAL"
    unset MERCURY_PDFIUM_URL_BASE
    for arch in x64 arm64 univ; do
        result="$(outcome scripts/macos/pdfium.sh --arch "$arch" fetch unpack check)"
        check "the real $arch release is fetched, unpacked and checked" yes "$(contains_text "$result" '(exit 0)')"
    done
    check "the real x64 library is x86_64" "x86_64" "$(lipo -archs "$REAL/8165/x64/lib/libpdfium.dylib")"
    check "the real arm64 library is arm64 only (read, never run)" "arm64" "$(lipo -archs "$REAL/8165/arm64/lib/libpdfium.dylib")"
    check "its install name is @rpath/libpdfium.dylib" "@rpath/libpdfium.dylib" "$(otool -D "$REAL/8165/arm64/lib/libpdfium.dylib" | sed -n 2p)"
    check "the universal library has both" "x86_64 arm64" "$(lipo -archs "$REAL/8165/univ/lib/libpdfium.dylib")"
    check "the real arm64 and x64 licence texts are the same" yes \
        "$(diff -r "$REAL/8165/arm64/licenses" "$REAL/8165/x64/licenses" >/dev/null && echo yes || echo no)"
    check "LICENSES/ has the tarball's texts, byte for byte" "" \
        "$(for name in $LICENSE_NAMES; do cmp -s "$REAL/8165/arm64/licenses/$name" "LICENSES/$(licence_copy "$name")" || printf '%s ' "$name"; done; cmp -s "$REAL/8165/arm64/LICENSE" "LICENSES/$(licence_copy LICENSE)" || printf 'LICENSE ')"
    # The collector on the real arm64 build, with the real pins.
    APP="$WORK/Real.app/Contents"
    mkdir -p "$APP/MacOS/lib" "$APP/Resources"
    result="$(outcome scripts/macos/collect-pdfium.sh "$REAL/8165/arm64" "$APP/MacOS/lib" "$APP/Resources")"
    check "the collector accepts the real arm64 build for a bundle" yes "$(contains_text "$result" '(exit 0)')"
    check "and copies the pinned library" "$(pin PDFIUM_ARM64_LIBRARY_SHA256)" "$(sha "$APP/MacOS/lib/libpdfium.dylib")"
    BAD="$WORK/RealBad.app/Contents"
    mkdir -p "$BAD/MacOS/lib" "$BAD/Resources"
    result="$(outcome scripts/macos/collect-pdfium.sh "$REAL/8165/x64" "$BAD/MacOS/lib" "$BAD/Resources")"
    check "and refuses the real x64 build for a bundle" yes "$(contains_text "$result" 'the application is arm64 only')"
    # The deployment target the bundle check allows (LSMinimumSystemVersion).
    minos="$(otool -l "$REAL/8165/arm64/lib/libpdfium.dylib" | awk '$1 == "cmd" { cmd = $2 } cmd == "LC_BUILD_VERSION" && $1 == "minos" { print $2 }')"
    want="$(. scripts/macos/lib.sh && config_deployment_target <.cargo/config.toml)"
    check "the real library needs no newer macOS than the application's minimum ($minos, $want)" yes \
        "$(awk -v minos="$minos" -v want="$want" 'BEGIN { print (minos + 0 <= want + 0) ? "yes" : "no" }')"
fi

if [ "$failures" -gt 0 ]; then
    printf '%d check(s) failed\n' "$failures"
    exit 1
fi
printf 'all checks passed\n'
