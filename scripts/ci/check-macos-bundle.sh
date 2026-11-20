#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Adapted from Zervo, https://github.com/Goddv/Zervo, commit 86cb7126,
# .github/workflows/macos.yml lines 183-222 ("Check the bundle"), MPL-2.0.
#
# Check an assembled Mercury Browser.app: the things that are easy to get
# silently wrong and impossible to notice from a green build.
#
#   scripts/ci/check-macos-bundle.sh [--signature adhoc|developer-id]
#                                    [--require-media] <app>
#
#   --signature adhoc          require an ad-hoc signature (what CI builds)
#   --signature developer-id   require a Developer ID signature with a secure
#                              timestamp (a signed release)
#   (no --signature)           accept either and check the entitlements that
#                              go with the signature found
#   --require-media            require the GStreamer media libraries (see below)
#
# Checks, each printed as PASS or FAIL:
#   - Info.plist passes `plutil -lint` and names an executable that exists;
#   - every Mach-O file in the bundle is arm64 only;
#   - every Mach-O file records exactly one minimum macOS version; the
#     executable's equals LSMinimumSystemVersion, and no library's is newer
#     (a library built for an older macOS, such as GStreamer's 11.0, is fine);
#   - `codesign --verify --deep --strict` passes and the hardened runtime is on;
#   - the signature is the kind asked for, and the entitlements are exactly the
#     set scripts/macos/bundle.sh uses for it (the ad-hoc set for ad hoc);
#   - Contents/Resources/filter-lists matches its manifest and the executable
#     carries no filter-list text;
#   - Contents/Resources holds LICENSE, NOTICE, THIRD-PARTY-NOTICES.md (the IJG
#     credit and the offer of the LGPL sources are in them) and
#     LICENSES/LGPL-2.1.txt;
#   - Contents/Resources/credits/credits.html is the credits page (the licence
#     text of every crate built into the executable): it passes
#     credits_problem in scripts/macos/lib.sh, the rule bundle.sh applies too
#     (a real page, over 100000 bytes, crediting at least 500 crates, no script);
#   - scripts/macos/test-bundle.sh passes, including its checks of the notice
#     files (LICENSE, NOTICE, THIRD-PARTY-NOTICES.md) and the credits page in
#     this bundle.
#
# With --require-media, also:
#   - Contents/MacOS/lib/ holds libgstreamer-1.0.0.dylib and every plugin
#     library `scripts/macos/gstreamer-plugins.sh` lists, and none of those
#     `scripts/macos/gstreamer-plugins.sh --excluded` lists (so a stale one
#     cannot ride along). That script is the one definition the collector
#     uses too; it honours MERCURY_SERVO_PLUGIN_LISTS and
#     MERCURY_GSTREAMER_EXCLUDE_PLUGINS;
#   - the executable loads @rpath/libgstreamer-1.0.0.dylib, and its only rpath
#     is @executable_path/lib/ (an absolute rpath back to a build machine
#     fails);
#   - every library any Mach-O in the bundle loads is a system library or an
#     @rpath/@loader_path/@executable_path name whose file is in
#     Contents/MacOS/lib/: nothing points back at the machine it was built on;
#   - Contents/Resources/gstreamer-licenses/ holds the bundled libraries'
#     licence files;
#   - the PDF viewer's library: Contents/MacOS/lib/libpdfium.dylib is arm64
#     only with the install name @rpath/libpdfium.dylib, and
#     Contents/Resources/pdfium-licenses/ holds PDFium's LICENSE and every
#     component licence file scripts/macos/pdfium.sh lists.
#
# Runs every check even after a failure, then exits 1 if any failed. Reads the
# bundle; writes nothing except a temporary file it removes. macOS only.
set -euo pipefail

usage() {
    sed -n '/^# Check an assembled/,/^# bundle; writes/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

die() {
    printf 'check-macos-bundle: error: %s\n' "$*" >&2
    exit 2
}

WANT_SIGNATURE=""
REQUIRE_MEDIA=0
APP=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --signature)
            WANT_SIGNATURE="${2:?--signature needs adhoc or developer-id}"
            shift 2
            ;;
        --require-media) REQUIRE_MEDIA=1; shift ;;
        -h | --help) usage; exit 0 ;;
        -*) die "unknown option: $1 (try --help)" ;;
        *)
            [ -z "$APP" ] || die "more than one bundle given"
            APP="$1"
            shift
            ;;
    esac
done
case "$WANT_SIGNATURE" in
    "" | adhoc | developer-id) ;;
    *) die "--signature must be adhoc or developer-id, not '$WANT_SIGNATURE'" ;;
esac
[ -n "$APP" ] || die "no bundle given (try --help)"
[ "$(uname -s)" = "Darwin" ] || die "this checks a macOS application bundle and must run on macOS"
[ -d "$APP/Contents" ] || die "$APP is not an application bundle (no Contents directory)"
for tool in codesign file lipo otool plutil; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool not found (install the Xcode command-line tools)"
done

REPO="$(cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/macos/lib.sh
. "$REPO/scripts/macos/lib.sh"
for script in scripts/macos/bundle.sh scripts/macos/test-bundle.sh; do
    [ -x "$REPO/$script" ] || die "$script is missing from $REPO; this check uses it"
done
failures=0

pass() {
    printf 'PASS %s\n' "$*"
}

fail() {
    printf 'FAIL %s\n' "$*"
    failures=$((failures + 1))
}

# 0 when dotted version $1 <= $2 (numeric, up to three components).
version_le() {
    local a1 a2 a3 b1 b2 b3
    IFS=. read -r a1 a2 a3 <<<"$1"
    IFS=. read -r b1 b2 b3 <<<"$2"
    a1=${a1:-0} a2=${a2:-0} a3=${a3:-0} b1=${b1:-0} b2=${b2:-0} b3=${b3:-0}
    [ "$a1" -lt "$b1" ] && return 0
    [ "$a1" -gt "$b1" ] && return 1
    [ "$a2" -lt "$b2" ] && return 0
    [ "$a2" -gt "$b2" ] && return 1
    [ "$a3" -le "$b3" ]
}

# Every minimum-macOS record in a Mach-O file, one per line: LC_BUILD_VERSION's
# `minos`, or LC_VERSION_MIN_MACOSX's `version` for files built by older tools.
minimum_versions() {
    otool -l "$1" | awk '
        $1 == "cmd" { cmd = $2 }
        cmd == "LC_BUILD_VERSION" && $1 == "minos" { print $2 }
        cmd == "LC_VERSION_MIN_MACOSX" && $1 == "version" { print $2 }'
}

# The entitlements in a plist file as sorted `key=value` lines (values are
# `true`, `false` or `other`), so two files can be compared exactly.
entitlement_lines() {
    plutil -convert xml1 -o - "$1" | awk '
        /<key>/ { sub(/.*<key>/, ""); sub(/<\/key>.*/, ""); key = $0; next }
        key != "" && /<true\/>/ { print key "=true"; key = ""; next }
        key != "" && /<false\/>/ { print key "=false"; key = ""; next }
        key != "" && NF { print key "=other"; key = "" }' | sort
}

TMP="$(mktemp)"
trap 'rm -f -- "$TMP"' EXIT

printf 'checking %s\n' "$APP"

# ── Info.plist and the executable ─────────────────────────────────────────
PLIST="$APP/Contents/Info.plist"
EXECUTABLE=""
MIN_MACOS=""
if [ -f "$PLIST" ] && plutil -lint "$PLIST" >/dev/null; then
    pass "Info.plist passes plutil -lint"
    EXECUTABLE="$(plutil -extract CFBundleExecutable raw -o - "$PLIST" 2>/dev/null || true)"
    MIN_MACOS="$(plutil -extract LSMinimumSystemVersion raw -o - "$PLIST" 2>/dev/null || true)"
else
    fail "Info.plist is missing or does not pass plutil -lint"
fi
BIN="$APP/Contents/MacOS/$EXECUTABLE"
if [ -n "$EXECUTABLE" ] && [ -f "$BIN" ]; then
    pass "CFBundleExecutable '$EXECUTABLE' exists"
else
    fail "CFBundleExecutable '$EXECUTABLE' is not a file in Contents/MacOS"
    BIN=""
fi
if [ -n "$MIN_MACOS" ]; then
    pass "LSMinimumSystemVersion is $MIN_MACOS"
else
    fail "Info.plist has no LSMinimumSystemVersion"
fi

# ── Every Mach-O: architecture and minimum macOS ──────────────────────────
machos=0
MACHO_FILES=()
while IFS= read -r -d '' path; do
    case "$(file -b "$path")" in
        Mach-O*) ;;
        *) continue ;;
    esac
    machos=$((machos + 1))
    MACHO_FILES+=("$path")
    name="${path#"$APP"/}"
    archs="$(lipo -archs "$path" 2>/dev/null || true)"
    if [ "$archs" = "arm64" ]; then
        pass "$name is arm64 only"
    else
        fail "$name is built for '$archs', not arm64 only"
    fi
    minimum_versions "$path" >"$TMP"
    count="$(wc -l <"$TMP" | tr -d ' ')"
    minos="$(head -n 1 "$TMP")"
    if [ "$count" != 1 ]; then
        fail "$name records $count minimum macOS versions ($(tr '\n' ' ' <"$TMP")), not exactly one"
    elif [ -z "$MIN_MACOS" ]; then
        fail "$name targets macOS $minos, but there is no LSMinimumSystemVersion to compare with"
    elif [ "$path" = "$BIN" ]; then
        if [ "$minos" = "$MIN_MACOS" ]; then
            pass "$name targets macOS $minos, as LSMinimumSystemVersion says"
        else
            fail "$name targets macOS $minos but LSMinimumSystemVersion is $MIN_MACOS"
        fi
    elif version_le "$minos" "$MIN_MACOS"; then
        pass "$name targets macOS $minos, not newer than $MIN_MACOS"
    else
        fail "$name needs macOS $minos, newer than LSMinimumSystemVersion $MIN_MACOS"
    fi
done < <(find "$APP/Contents" -type f -print0)
if [ "$machos" -gt 0 ]; then
    pass "found $machos Mach-O file(s)"
else
    fail "no Mach-O files in the bundle"
fi

# ── Signature ─────────────────────────────────────────────────────────────
if codesign --verify --deep --strict --verbose=2 "$APP" >"$TMP" 2>&1; then
    pass "codesign --verify --deep --strict"
else
    fail "codesign --verify --deep --strict: $(tr '\n' ' ' <"$TMP")"
fi

codesign -dv --verbose=4 "$APP" >"$TMP" 2>&1 || true
if grep -q '^Signature=adhoc' "$TMP"; then
    SIGNATURE="adhoc"
    IDENTITY="-"
elif grep -q '^Authority=Developer ID Application: ' "$TMP"; then
    SIGNATURE="developer-id"
    IDENTITY="$(sed -n '/^Authority=Developer ID Application: /{s/^Authority=//;p;q;}' "$TMP")"
else
    SIGNATURE="other"
    IDENTITY=""
fi
if [ -z "$WANT_SIGNATURE" ] && [ "$SIGNATURE" != other ]; then
    pass "signature is $SIGNATURE"
elif [ "$SIGNATURE" = "$WANT_SIGNATURE" ]; then
    pass "signature is $SIGNATURE, as required"
else
    fail "signature is $SIGNATURE, expected ${WANT_SIGNATURE:-adhoc or developer-id}"
fi
if grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' "$TMP"; then
    pass "hardened runtime is on"
else
    fail "hardened runtime is off: $(grep '^CodeDirectory' "$TMP" || echo 'no CodeDirectory line')"
fi
if [ "$SIGNATURE" = developer-id ]; then
    if grep -q '^Timestamp=' "$TMP" && ! grep -q '^TeamIdentifier=not set' "$TMP"; then
        pass "Developer ID signature has a secure timestamp and a team identifier"
    else
        fail "Developer ID signature lacks a secure timestamp or a team identifier"
    fi
fi

# ── Entitlements: exactly the set bundle.sh chooses for this signature ────
if [ -n "$IDENTITY" ]; then
    EXPECTED="$("$REPO/scripts/macos/bundle.sh" --identity "$IDENTITY" --print-entitlements)"
    if codesign -d --entitlements - --xml "$APP" >"$TMP" 2>/dev/null && [ -s "$TMP" ] &&
        plutil -lint "$TMP" >/dev/null; then
        actual="$(entitlement_lines "$TMP")"
        expected="$(entitlement_lines "$EXPECTED")"
        if [ "$actual" = "$expected" ]; then
            pass "entitlements are exactly ${EXPECTED#"$REPO"/}: $(printf '%s' "$actual" | tr '\n' ' ')"
        else
            fail "entitlements [$(printf '%s' "$actual" | tr '\n' ' ')] differ from ${EXPECTED#"$REPO"/} [$(printf '%s' "$expected" | tr '\n' ' ')]"
        fi
    else
        fail "the bundle carries no readable entitlements"
    fi
else
    fail "entitlements not checked: the signature is neither ad hoc nor Developer ID"
fi

# ── Media libraries (--require-media) ─────────────────────────────────────
PLUGIN_LIST_SCRIPT="$REPO/scripts/macos/gstreamer-plugins.sh"

# The libraries a Mach-O file loads, without its own install name.
loaded_libraries() {
    local own
    own="$(otool -D "$1" | sed -n '2p')"
    otool -L "$1" | sed -n 's/^[[:space:]]\{1,\}\([^ ]*\) (compatibility.*/\1/p' |
        awk -v own="$own" '$0 != own' | sort -u
}

if [ "$REQUIRE_MEDIA" = 1 ]; then
    LIB_DIR="$APP/Contents/MacOS/lib"
    if [ -f "$LIB_DIR/libgstreamer-1.0.0.dylib" ]; then
        pass "Contents/MacOS/lib/libgstreamer-1.0.0.dylib is present"
    else
        fail "Contents/MacOS/lib/libgstreamer-1.0.0.dylib is missing: the bundle has no media libraries"
    fi

    # One plugin library file name per line, from the shared definition.
    if [ ! -x "$PLUGIN_LIST_SCRIPT" ]; then
        fail "scripts/macos/gstreamer-plugins.sh is missing: cannot tell which plugins belong in the bundle"
    elif EFFECTIVE="$("$PLUGIN_LIST_SCRIPT")" && EXCLUDED="$("$PLUGIN_LIST_SCRIPT" --excluded)" &&
        [ -n "$EFFECTIVE" ]; then
        listed=0
        missing=""
        for library in $EFFECTIVE; do
            listed=$((listed + 1))
            [ -f "$LIB_DIR/$library" ] || missing="$missing $library"
        done
        excluded_names="$(printf '%s\n' "$EXCLUDED" | sed '/^$/d' | tr '\n' ' ' | sed 's/ $//')"
        if [ -z "$missing" ]; then
            pass "all $listed plugins listed by scripts/macos/gstreamer-plugins.sh are in Contents/MacOS/lib (excluded: ${excluded_names:-none})"
        else
            fail "plugins missing from Contents/MacOS/lib:$missing"
        fi
        present=""
        for library in $EXCLUDED; do
            [ ! -e "$LIB_DIR/$library" ] || present="$present $library"
        done
        if [ -z "$present" ]; then
            pass "no excluded plugin is in Contents/MacOS/lib (excluded: ${excluded_names:-none})"
        else
            fail "excluded plugins are in Contents/MacOS/lib:$present"
        fi
    else
        fail "scripts/macos/gstreamer-plugins.sh could not list the plugins (see its message above)"
    fi

    if [ -n "$BIN" ]; then
        if loaded_libraries "$BIN" >"$TMP" && grep -qx '@rpath/libgstreamer-1.0.0.dylib' "$TMP"; then
            pass "$EXECUTABLE loads @rpath/libgstreamer-1.0.0.dylib"
        else
            fail "$EXECUTABLE does not load @rpath/libgstreamer-1.0.0.dylib"
        fi
        # Exactly one rpath, and that one: any other (an absolute path to the
        # build machine's prefix, say) would let the app load from outside.
        otool -l "$BIN" | awk '$1 == "cmd" { cmd = $2 } cmd == "LC_RPATH" && $1 == "path" { print $2 }' >"$TMP"
        rpaths="$(tr '\n' ' ' <"$TMP" | sed 's/ $//')"
        case "$rpaths" in
            "@executable_path/lib/" | "@executable_path/lib")
                pass "$EXECUTABLE's only rpath is @executable_path/lib/" ;;
            *)
                fail "$EXECUTABLE must have exactly the rpath @executable_path/lib/, has: ${rpaths:-none}" ;;
        esac
    fi

    outside=0
    for path in ${MACHO_FILES[@]+"${MACHO_FILES[@]}"}; do
        loaded_libraries "$path" >"$TMP"
        while IFS= read -r library; do
            case "$library" in
                /System/Library/* | /usr/lib/*) ;;
                @rpath/* | @loader_path/* | @executable_path/*)
                    if [ ! -f "$LIB_DIR/${library##*/}" ]; then
                        fail "${path#"$APP"/} loads $library, which is not in Contents/MacOS/lib"
                        outside=$((outside + 1))
                    fi
                    ;;
                *)
                    fail "${path#"$APP"/} loads $library from outside the bundle"
                    outside=$((outside + 1))
                    ;;
            esac
        done <"$TMP"
    done
    if [ "$outside" = 0 ]; then
        pass "every Mach-O file loads only system libraries and libraries in Contents/MacOS/lib"
    fi
fi

# ── PDF library (--require-media) ─────────────────────────────────────────
# pdf-viewer: PDFium beside the media libraries, with its licence texts. The file list is
# the one definition in scripts/macos/pdfium.sh (read in a subshell: it defines die and say).
if [ "$REQUIRE_MEDIA" = 1 ]; then
    PDFIUM_LIBRARY="$APP/Contents/MacOS/lib/libpdfium.dylib"
    if [ -f "$PDFIUM_LIBRARY" ]; then
        pass "Contents/MacOS/lib/libpdfium.dylib is present"
        archs="$(lipo -archs "$PDFIUM_LIBRARY" 2>/dev/null || true)"
        if [ "$archs" = "arm64" ]; then
            pass "libpdfium.dylib is arm64 only"
        else
            fail "libpdfium.dylib is built for '$archs', not arm64 only"
        fi
        install_name="$(otool -D "$PDFIUM_LIBRARY" 2>/dev/null | sed -n '2p')"
        if [ "$install_name" = "@rpath/libpdfium.dylib" ]; then
            pass "libpdfium.dylib has the install name @rpath/libpdfium.dylib"
        else
            fail "libpdfium.dylib has the install name '$install_name', not @rpath/libpdfium.dylib"
        fi
    else
        fail "Contents/MacOS/lib/libpdfium.dylib is missing: the bundle has no PDF viewer library"
    fi
    # shellcheck source=scripts/macos/pdfium.sh disable=SC2086
    PDFIUM_LICENSE_NAMES="$(. "$REPO/scripts/macos/pdfium.sh" && printf '%s\n' $PDFIUM_LICENSE_FILES)" || PDFIUM_LICENSE_NAMES=""
    PDFIUM_LICENSES="$APP/Contents/Resources/pdfium-licenses"
    missing=""
    for licence_file in LICENSE $PDFIUM_LICENSE_NAMES; do
        [ -s "$PDFIUM_LICENSES/$licence_file" ] || missing="$missing $licence_file"
    done
    if [ -z "$PDFIUM_LICENSE_NAMES" ]; then
        fail "scripts/macos/pdfium.sh could not list PDFium's licence files"
    elif [ -z "$missing" ]; then
        pass "Contents/Resources/pdfium-licenses holds PDFium's LICENSE and its $(printf '%s\n' "$PDFIUM_LICENSE_NAMES" | wc -l | tr -d ' ') component licence files"
    else
        fail "Contents/Resources/pdfium-licenses lacks:$missing"
    fi
fi

# ── Filter lists (blocklists) ─────────────────────────────────────────────
# Data files beside the program, not compiled into it.
LISTS="$APP/Contents/Resources/filter-lists"
if filter_list_files "$LISTS" >"$TMP" 2>&1; then
    pass "Contents/Resources/filter-lists matches its manifest"
else
    fail "Contents/Resources/filter-lists: $(tr '\n' ' ' <"$TMP")"
fi
if [ -n "$EXECUTABLE" ] && [ -f "$APP/Contents/MacOS/$EXECUTABLE" ]; then
    # Rule fragments a compiled-in engine carried (the lists' own text is never compiled in).
    embedded=""
    for fragment in "advanced-ads-" "scorecardresearch" "Brave Firstparty specific filters"; do
        if LC_ALL=C grep -q -a -F -- "$fragment" "$APP/Contents/MacOS/$EXECUTABLE"; then
            embedded="$embedded '$fragment'"
        fi
    done
    if [ -z "$embedded" ]; then
        pass "$EXECUTABLE contains no filter-list text"
    else
        fail "$EXECUTABLE contains filter-list text:$embedded"
    fi
fi

# ── Licence files ─────────────────────────────────────────────────────────
# They must stay with the application when it is dragged out of the disk image: the
# licence terms and Required Notice, the notices that carry the IJG credit and the offer of
# the LGPL sources, and the LGPL text the bundled libraries are replaceable under.
RESOURCES="$APP/Contents/Resources"
missing_licence_files=""
for licence_file in LICENSE NOTICE THIRD-PARTY-NOTICES.md LICENSES/LGPL-2.1.txt; do
    [ -s "$RESOURCES/$licence_file" ] || missing_licence_files="$missing_licence_files $licence_file"
done
if [ -z "$missing_licence_files" ]; then
    pass "Contents/Resources holds LICENSE, NOTICE, THIRD-PARTY-NOTICES.md and LICENSES/LGPL-2.1.txt"
else
    fail "Contents/Resources lacks:$missing_licence_files"
fi
# The licence text, copyright line and notices of every crate in the executable, written by
# `mercury-credits generate` at package time; the About window opens it from here. The rule is
# credits_problem in lib.sh (shared with bundle.sh), which prints why the file is not usable.
CREDITS_PAGE="$RESOURCES/credits/credits.html"
credits_reason="$(credits_problem "$CREDITS_PAGE")"
if [ -z "$credits_reason" ]; then
    pass "Contents/Resources/credits/credits.html is the credits page ($(wc -c <"$CREDITS_PAGE" | tr -d ' ') bytes)"
else
    fail "Contents/Resources/credits/credits.html: $credits_reason"
fi
if [ "$REQUIRE_MEDIA" = 1 ]; then
    if [ -d "$RESOURCES/gstreamer-licenses" ] && [ -n "$(ls -A "$RESOURCES/gstreamer-licenses")" ]; then
        pass "Contents/Resources/gstreamer-licenses holds the bundled libraries' licence files"
    else
        fail "Contents/Resources/gstreamer-licenses is missing or empty: the bundled libraries' licence files are not in the application"
    fi
fi

# ── The bundle script's own checks ────────────────────────────────────────
# Given the bundle, it also checks the notice files in Contents/Resources.
if MERCURY_TEST_BUNDLE="$(cd "$APP" && pwd)" "$REPO/scripts/macos/test-bundle.sh" >"$TMP" 2>&1; then
    pass "scripts/macos/test-bundle.sh"
else
    fail "scripts/macos/test-bundle.sh:"
    sed 's/^/    /' "$TMP"
fi

if [ "$failures" -gt 0 ]; then
    printf '%d check(s) failed for %s\n' "$failures" "$APP"
    exit 1
fi
printf 'all checks passed for %s\n' "$APP"
