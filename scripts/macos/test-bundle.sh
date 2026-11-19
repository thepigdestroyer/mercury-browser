#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Checks the parts of scripts/macos/bundle.sh that can be tested without a
# build or a signing identity: the identity checks, which entitlement file each
# identity gets and what the files grant, the notice files and the credits page
# every bundle carries, and the helpers in lib.sh that parse otool and codesign
# output.
#
#   scripts/macos/test-bundle.sh      (or: make test-bundle-macos)
#   MERCURY_TEST_BUNDLE="dist/Mercury Browser.app" scripts/macos/test-bundle.sh
#                                     also checks the notice files and the credits
#                                     page in a built bundle
#
# Prints one line per check and exits non-zero if any fails. Writes nothing
# except temporary files (the sample credits pages), which it removes.
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."
# shellcheck source=scripts/macos/lib.sh
. scripts/macos/lib.sh

RESOURCES="$PWD/apps/mercury-desktop/resources/macos"
STRICT="$RESOURCES/entitlements.plist"
ADHOC="$RESOURCES/entitlements-adhoc.plist"
JIT="com.apple.security.cs.allow-jit=<true/> com.apple.security.cs.allow-unsigned-executable-memory=<true/>"
NO_VALIDATION="com.apple.security.cs.disable-library-validation=<true/>"
DEVICES="com.apple.security.device.audio-input=<true/> com.apple.security.device.camera=<true/>"
LOCATION="com.apple.security.personal-information.location=<true/>"
LOCATION_WORDING="Websites you allow can ask for your location."
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

# Runs a command and prints "ok" or "fails" by its exit status.
status() {
    if "$@"; then printf 'ok'; else printf 'fails'; fi
}

# bundle.sh --print-entitlements with the given arguments, with
# MERCURY_SIGN_IDENTITY unset unless the caller sets it; prints the output and
# the exit status.
print_entitlements() {
    local output code=0
    output="$(scripts/macos/bundle.sh "$@" --print-entitlements 2>&1)" || code=$?
    printf '%s (exit %s)' "$output" "$code"
}

unset MERCURY_SIGN_IDENTITY

# ── Which entitlements each identity gets ─────────────────────────────────
check "ad-hoc identity uses the ad-hoc entitlements" "$ADHOC (exit 0)" "$(print_entitlements --identity -)"
check "default identity is ad hoc" "$ADHOC (exit 0)" "$(print_entitlements)"
check "a Developer ID uses the strict entitlements" "$STRICT (exit 0)" \
    "$(print_entitlements --identity 'Developer ID Application: Example (ABCDE12345)')"
check "an Apple Development identity uses the strict entitlements" "$STRICT (exit 0)" \
    "$(print_entitlements --identity 'Apple Development: someone@example.com (ABCDE12345)')"
check "MERCURY_SIGN_IDENTITY selects the strict entitlements" "$STRICT (exit 0)" \
    "$(MERCURY_SIGN_IDENTITY='Developer ID Application: Example (ABCDE12345)' print_entitlements)"
check "--identity overrides MERCURY_SIGN_IDENTITY" "$ADHOC (exit 0)" \
    "$(MERCURY_SIGN_IDENTITY='Developer ID Application: Example (ABCDE12345)' print_entitlements --identity -)"
check "an empty MERCURY_SIGN_IDENTITY means ad hoc" "$ADHOC (exit 0)" \
    "$(MERCURY_SIGN_IDENTITY='' print_entitlements)"

# ── Odd identities are refused, not signed with ───────────────────────────
check "an empty --identity is refused" \
    "bundle: error: the signing identity is empty; use - for ad hoc (exit 1)" \
    "$(print_entitlements --identity '')"
check "' - ' is refused" \
    "bundle: error: signing identity ' - ' has leading or trailing spaces (exit 1)" \
    "$(print_entitlements --identity ' - ')"
check "-x is refused" \
    "bundle: error: signing identity '-x' begins with '-'; use - for ad hoc, or a certificate name or SHA-1 hash (exit 1)" \
    "$(print_entitlements --identity -x)"
check "--identity without a value is refused" "bundle: error: --identity needs a value (exit 1)" \
    "$(code=0; output="$(scripts/macos/bundle.sh --identity 2>&1)" || code=$?; printf '%s (exit %s)' "$output" "$code")"

# ── What the entitlement files grant ──────────────────────────────────────
check "strict entitlements grant only the JIT, capture-device and location keys" "$JIT $DEVICES $LOCATION " \
    "$(plist_grants "$STRICT" | tr '\n' ' ')"
check "ad-hoc entitlements add only library validation" "$JIT $NO_VALIDATION $DEVICES $LOCATION " \
    "$(plist_grants "$ADHOC" | tr '\n' ' ')"

# ── Location usage strings ────────────────────────────────────────────────
# macOS shows one of these in its prompt; without them it refuses location access.
usage_string() { plutil -extract "$1" raw -o - -- "$RESOURCES/Info.plist.in" 2>/dev/null || echo missing; }
check "Info.plist explains location access" "$LOCATION_WORDING" "$(usage_string NSLocationUsageDescription)"
check "Info.plist explains when-in-use location access" "$LOCATION_WORDING" \
    "$(usage_string NSLocationWhenInUseUsageDescription)"

# ── Entitlements read back after signing ──────────────────────────────────
# The single-line form `codesign -d --entitlements - --xml` prints.
readback() {
    printf '<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>'
    local key
    for key in "$@"; do printf '<key>%s</key><true/>' "$key"; done
    printf '</dict></plist>\n'
}
ADHOC_READBACK="$(readback com.apple.security.cs.allow-jit com.apple.security.cs.allow-unsigned-executable-memory com.apple.security.cs.disable-library-validation com.apple.security.device.camera com.apple.security.device.audio-input com.apple.security.personal-information.location)"
STRICT_READBACK="$(readback com.apple.security.cs.allow-jit com.apple.security.cs.allow-unsigned-executable-memory com.apple.security.device.camera com.apple.security.device.audio-input com.apple.security.personal-information.location)"
check "read-back ad-hoc entitlements match the ad-hoc file" ok \
    "$(status entitlements_match "$ADHOC" <<<"$ADHOC_READBACK")"
check "read-back strict entitlements match the strict file" ok \
    "$(status entitlements_match "$STRICT" <<<"$STRICT_READBACK")"
check "ad-hoc entitlements on a strict build are a mismatch" fails \
    "$(status entitlements_match "$STRICT" <<<"$ADHOC_READBACK")"
check "strict entitlements on an ad-hoc build are a mismatch" fails \
    "$(status entitlements_match "$ADHOC" <<<"$STRICT_READBACK")"
check "no entitlements at all are a mismatch" fails "$(status entitlements_match "$ADHOC" <<<"")"
check "a signature without the location entitlement is a mismatch" fails \
    "$(status entitlements_match "$STRICT" <<<"$(readback com.apple.security.cs.allow-jit com.apple.security.cs.allow-unsigned-executable-memory)")"
check "a key granted as false is a mismatch" fails \
    "$(status entitlements_match "$STRICT" <<<"${STRICT_READBACK/<true\/><\/dict>/<false/></dict>}")"

# ── codesign -dv: runtime flag and team ───────────────────────────────────
check "runtime flag found (ad hoc)" ok \
    "$(status codesign_has_runtime <<<'CodeDirectory v=20500 size=342378 flags=0x10002(adhoc,runtime) hashes=10688+7 location=embedded')"
check "runtime flag found (Developer ID)" ok \
    "$(status codesign_has_runtime <<<'CodeDirectory v=20500 size=342378 flags=0x10000(runtime) hashes=10688+7 location=embedded')"
check "runtime flag missing" fails \
    "$(status codesign_has_runtime <<<'CodeDirectory v=20400 size=252 flags=0x2(adhoc) hashes=2+3 location=embedded')"
check "linker-signed is not runtime" fails \
    "$(status codesign_has_runtime <<<'CodeDirectory v=20400 size=252 flags=0x20002(adhoc,linker-signed) hashes=2+3 location=embedded')"
check "team of an ad-hoc signature" "not set" "$(codesign_team <<<$'Signature=adhoc\nTeamIdentifier=not set')"
check "team of a Developer ID signature" "ABCDE12345" \
    "$(codesign_team <<<$'Authority=Developer ID Application: Example (ABCDE12345)\nTeamIdentifier=ABCDE12345')"
check "ad hoc with libraries is fine" "" "$(team_problem - 3 'not set')"
check "a teamless identity with libraries is refused" \
    "identity 'Self Signed' has no team identifier, so the bundled libraries would fail library validation at launch; use a Developer ID or Apple Development certificate, or - for ad hoc" \
    "$(team_problem 'Self Signed' 3 'not set')"
check "a teamless identity without libraries is allowed" "" "$(team_problem 'Self Signed' 0 'not set')"
check "a Developer ID with libraries is fine" "" "$(team_problem 'Developer ID Application: Example (ABCDE12345)' 3 ABCDE12345)"

# ── What may sit in Contents/MacOS/lib ────────────────────────────────────
header() { # otool -hv output for one or more slices of the given file types
    local type
    for type in "$@"; do
        printf 'Mach header\n      magic  cputype cpusubtype  caps    filetype ncmds sizeofcmds      flags\n'
        printf 'MH_MAGIC_64    ARM64        ALL  0x00 %11s    15       1064   NOUNDEFS DYLDLINK TWOLEVEL\n' "$type"
    done
}
check "a library belongs in lib" "" "$(library_problem lib/libx.dylib <<<"$(header DYLIB)")"
check "a plugin bundle belongs in lib" "" "$(library_problem lib/x.so <<<"$(header BUNDLE)")"
check "a universal library belongs in lib" "" "$(library_problem lib/libx.dylib <<<"$(header DYLIB DYLIB)")"
check "an executable is refused" \
    "lib/scanner is an executable; only libraries belong in Contents/MacOS/lib (an executable there would be signed without entitlements)" \
    "$(library_problem lib/scanner <<<"$(header EXECUTE)")"
check "a universal file with an executable slice is refused" \
    "lib/odd is an executable; only libraries belong in Contents/MacOS/lib (an executable there would be signed without entitlements)" \
    "$(library_problem lib/odd <<<"$(header DYLIB EXECUTE)")"
check "an object file is refused" \
    "lib/x.o is a Mach-O OBJECT, not a library; only libraries belong in Contents/MacOS/lib" \
    "$(library_problem lib/x.o <<<"$(header OBJECT)")"
check "unreadable output is refused" "lib/x: not a Mach-O file otool can read" "$(library_problem lib/x <<<"")"

# ── Large otool -l output through a pipe ──────────────────────────────────
# About 600 KB of load commands, far past the 64 KB pipe buffer, with
# LC_BUILD_VERSION near the top and the rpath at the very end. A parser that
# stops reading early breaks the pipe and, under pipefail, fails the pipeline.
BIG_LISTING="$(awk 'BEGIN {
    print "mercury-browser:"
    print "Load command 0"; print "      cmd LC_BUILD_VERSION"; print "  cmdsize 32"
    print " platform 1"; print "    minos 13.0"; print "      sdk 27.2"
    for (i = 1; i <= 6000; i++) {
        print "Load command " i; print "          cmd LC_LOAD_DYLIB"; print "      cmdsize 72"
        printf "         name @rpath/libgstexample%05d-1.0.0.dylib (offset 24)\n", i
    }
    print "Load command 6001"; print "          cmd LC_RPATH"; print "      cmdsize 40"
    print "         path @executable_path/lib/ (offset 12)"
}')"
check "the generated listing is larger than a pipe buffer" yes \
    "$([ "${#BIG_LISTING}" -gt 500000 ] && echo yes || echo "no (${#BIG_LISTING} bytes)")"
early_exit_reader() { awk '$1 == "minos" { print $2; exit }' >/dev/null; }
# Each pipes the big listing into a reader; they run under the script's pipefail.
big_into_early_exit_reader() { printf '%s\n' "$BIG_LISTING" | early_exit_reader; }
big_into_has_rpath() { printf '%s\n' "$BIG_LISTING" | listing_has_rpath "$1"; }
check "control: an early-exit reader fails on it under pipefail" fails \
    "$(status big_into_early_exit_reader 2>/dev/null)"
check "listing_minos reads it through a pipe" "13.0 ok" \
    "$(set -o pipefail; result="$(printf '%s\n' "$BIG_LISTING" | listing_minos)" && echo "$result ok" || echo "$result fails ($?)")"
check "listing_has_rpath finds the last rpath through a pipe" ok \
    "$(status big_into_has_rpath @executable_path/lib/)"
check "listing_has_rpath reports a missing rpath" fails \
    "$(status big_into_has_rpath @loader_path/)"
check "an LC_LOAD_DYLIB name is not an rpath" fails \
    "$(status big_into_has_rpath @rpath/libgstexample00001-1.0.0.dylib)"

# ── Version and deployment target ─────────────────────────────────────────
check "deployment target, plain form" 13.0 "$(config_deployment_target <<<'MACOSX_DEPLOYMENT_TARGET = "13.0"')"
check "deployment target, table form" 13.0 \
    "$(config_deployment_target <<<'MACOSX_DEPLOYMENT_TARGET = { value = "13.0", force = true }')"
check "deployment target from .cargo/config.toml is a version" yes \
    "$([[ "$(config_deployment_target <.cargo/config.toml)" =~ ^[0-9]+(\.[0-9]+)*$ ]] && echo yes || echo no)"
check "workspace version" 1.2.3 \
    "$(workspace_version <<<$'[package]\nversion = "9.9.9"\n[workspace.package]\nversion = "1.2.3"\nedition = "2024"')"

# ── Notice files in Contents/Resources ────────────────────────────────────
check "the bundle carries LICENSE, NOTICE and THIRD-PARTY-NOTICES.md" \
    "LICENSE NOTICE THIRD-PARTY-NOTICES.md " "$(bundled_notice_files | tr '\n' ' ')"
check "every notice file exists in the repository" "" "$(missing_notice_files "$PWD")"
check "NOTICE names Mercury's copyright holder" yes \
    "$(grep -q '^Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors\.$' NOTICE && echo yes || echo no)"
check "NOTICE carries the PolyForm Required Notice line" yes \
    "$(grep -q '^Required Notice: ' NOTICE && echo yes || echo no)"
check "both licence texts are in LICENSES/" yes \
    "$([ -f LICENSES/PolyForm-Noncommercial-1.0.0.md ] && [ -f LICENSES/MPL-2.0.txt ] && echo yes || echo no)"
# The pattern matches bundle.sh's literal text, so nothing in it is meant to expand.
# shellcheck disable=SC2016
check "bundle.sh copies LICENSES/ into Contents/Resources" yes \
    "$(grep -q '^ditto LICENSES "\$RES_DIR/LICENSES"$' scripts/macos/bundle.sh && echo yes || echo no)"
check "a directory without them is reported" "LICENSE NOTICE THIRD-PARTY-NOTICES.md " \
    "$(missing_notice_files "$PWD/scripts" | tr '\n' ' ')"
check "bundle.sh copies the notice files from that list" yes \
    "$(grep -q 'done < <(bundled_notice_files)' scripts/macos/bundle.sh && echo yes || echo no)"
# With a built bundle as MERCURY_TEST_BUNDLE, its Contents/Resources is checked too.
if [ -n "${MERCURY_TEST_BUNDLE:-}" ]; then
    check "the notice files are in $MERCURY_TEST_BUNDLE" "" \
        "$(missing_notice_files "$MERCURY_TEST_BUNDLE/Contents/Resources")"
    check "the bundled NOTICE is the repository's" yes \
        "$(cmp -s NOTICE "$MERCURY_TEST_BUNDLE/Contents/Resources/NOTICE" && echo yes || echo no)"
    check "the bundled LICENSE is the repository's" yes \
        "$(cmp -s LICENSE "$MERCURY_TEST_BUNDLE/Contents/Resources/LICENSE" && echo yes || echo no)"
    check "the licence texts are in the bundle's LICENSES/" yes \
        "$(cmp -s LICENSES/PolyForm-Noncommercial-1.0.0.md "$MERCURY_TEST_BUNDLE/Contents/Resources/LICENSES/PolyForm-Noncommercial-1.0.0.md" &&
            cmp -s LICENSES/MPL-2.0.txt "$MERCURY_TEST_BUNDLE/Contents/Resources/LICENSES/MPL-2.0.txt" && echo yes || echo no)"
    check "the credits page in $MERCURY_TEST_BUNDLE is usable" "" \
        "$(credits_problem "$MERCURY_TEST_BUNDLE/Contents/Resources/credits/credits.html")"
fi

# ── No bundle without its media libraries ─────────────────────────────────
# Each run stops before writing anything; the --out directory must not appear.
NO_OUT="$(mktemp -u -t mercury-test-bundle)"
bundle_status() {
    local output code=0
    output="$(scripts/macos/bundle.sh --out "$NO_OUT" "$@" 2>&1)" || code=$?
    printf '%s (exit %s)' "$output" "$code"
}
check "bundle.sh refuses to build without a GStreamer prefix" \
    "bundle: error: no GStreamer prefix: pass --gstreamer-prefix or set MERCURY_GSTREAMER_PREFIX (see scripts/macos/gstreamer.sh; make app-macos does this) (exit 1)" \
    "$(unset MERCURY_GSTREAMER_PREFIX; bundle_status)"
# Captured first and matched as a string: no pipe into an early-exiting reader.
contains_text() { case "$1" in *"$2"*) echo yes ;; *) echo no ;; esac; }
check "an empty MERCURY_GSTREAMER_PREFIX is refused too" yes \
    "$(contains_text "$(MERCURY_GSTREAMER_PREFIX='' bundle_status)" 'no GStreamer prefix')"
check "a prefix that does not exist is refused" yes \
    "$(contains_text "$(bundle_status --gstreamer-prefix "$NO_OUT-prefix")" 'does not exist (exit 1)')"
check "a directory that is not a GStreamer prefix is refused" yes \
    "$(contains_text "$(bundle_status --gstreamer-prefix "$PWD/scripts")" 'is not a GStreamer prefix')"
check "the refusals wrote nothing" no "$([ -e "$NO_OUT" ] && echo yes || echo no)"

# ── Filter lists (blocklists) ─────────────────────────────────────────────
check "the repository's filter lists match their manifest" 5 \
    "$(filter_list_files crates/talaria-servo/resources/adblock 2>/dev/null | wc -l | tr -d ' ')"
check "a directory without a manifest is refused" fails \
    "$(status filter_list_files "$NO_OUT-lists" 2>/dev/null)"

# ── The credits page ──────────────────────────────────────────────────────
# credits_problem is the one rule for the page (crates/mercury-credits writes it): bundle.sh
# applies it before it builds anything, check-macos-bundle.sh on the finished bundle. The
# sample pages are written here; no real page is needed.
CREDITS_TMP="$(mktemp -d "${TMPDIR:-/tmp}/mercury-test-credits.XXXXXX")"
trap 'rm -rf -- "$CREDITS_TMP"' EXIT
GENERATOR_TAG='<meta name="generator" content="mercury-credits 0.2.0">'
COUNT_TAG='<meta name="credits-crates" content="882">'
# credits_page <generator tag> <crate-count tag> <lines of licence text>
credits_page() {
    printf '<!doctype html>\n<html><head><meta charset="utf-8">\n%s\n%s\n<title>Mercury Browser: third-party notices</title></head>\n<body><pre>\n' "$1" "$2"
    awk -v lines="$3" 'BEGIN { for (i = 0; i < lines; i++) print "Permission is hereby granted, free of charge, to any person obtaining a copy of this software" }'
    printf '</pre></body></html>\n'
}
# The reason credits_problem gives for a sample page made from the arguments of credits_page.
sample_problem() {
    credits_page "$@" >"$CREDITS_TMP/sample.html"
    credits_problem "$CREDITS_TMP/sample.html"
}
credits_page "$GENERATOR_TAG" "$COUNT_TAG" 2000 >"$CREDITS_TMP/good.html"
check "the good sample page is over the size floor" yes \
    "$([ "$(wc -c <"$CREDITS_TMP/good.html")" -gt 100000 ] && echo yes || echo no)"
check "a credits page of the expected shape is accepted" "" "$(credits_problem "$CREDITS_TMP/good.html")"
check "a missing credits page is refused" yes \
    "$(contains_text "$(credits_problem "$CREDITS_TMP/none.html")" 'none.html is missing or empty')"
: >"$CREDITS_TMP/empty.html"
check "an empty credits page is refused" yes \
    "$(contains_text "$(credits_problem "$CREDITS_TMP/empty.html")" 'empty.html is missing or empty')"
check "a directory is not a credits page" yes \
    "$(contains_text "$(credits_problem "$CREDITS_TMP")" 'is missing or empty')"
check "a page under 100000 bytes is refused" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" "$COUNT_TAG" 10)" 'sample.html is only ')"
check "a page from another generator is refused" yes \
    "$(contains_text "$(sample_problem '<meta name="generator" content="cargo-about 0.6">' "$COUNT_TAG" 2000)" 'was not written by mercury-credits')"
check "generator text that is only escaped inside the page is refused" yes \
    "$(contains_text "$(sample_problem '&lt;meta name="generator" content="mercury-credits 0.2.0"&gt;' "$COUNT_TAG" 2000)" 'was not written by mercury-credits')"
check "a page without a crate count is refused" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" '' 2000)" 'no usable credits-crates tag')"
check "a crate count that is not a number is refused" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="many">' 2000)" 'no usable credits-crates tag')"
check "a crate count of twenty digits is refused" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="12345678901234567890">' 2000)" 'no usable credits-crates tag')"
check "499 crates are too few" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="499">' 2000)" 'credits only 499 crates')"
check "500 crates are enough" "" \
    "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="500">' 2000)"
check "a count with a leading zero is read as decimal (0882)" "" \
    "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="0882">' 2000)"
check "a count with a leading zero is read as decimal (0499)" yes \
    "$(contains_text "$(sample_problem "$GENERATOR_TAG" '<meta name="credits-crates" content="0499">' 2000)" 'credits only 499 crates')"
{ credits_page "$GENERATOR_TAG" "$COUNT_TAG" 2000; printf '<SCRIPT>alert(1)</SCRIPT>\n'; } >"$CREDITS_TMP/script.html"
check "a page with a script is refused" yes \
    "$(contains_text "$(credits_problem "$CREDITS_TMP/script.html")" 'contains a script tag')"

# bundle.sh against a made-up GStreamer prefix (it only looks for the one library) and the
# sample pages. Each run stops at or before its executable check, so nothing is written.
mkdir -p "$CREDITS_TMP/gstreamer/lib" "$CREDITS_TMP/good" "$CREDITS_TMP/small"
: >"$CREDITS_TMP/gstreamer/lib/libgstreamer-1.0.0.dylib"
cp "$CREDITS_TMP/good.html" "$CREDITS_TMP/good/credits.html"
credits_page "$GENERATOR_TAG" "$COUNT_TAG" 10 >"$CREDITS_TMP/small/credits.html"
credits_bundle_status() { (unset MERCURY_CREDITS_DIR; bundle_status --gstreamer-prefix "$CREDITS_TMP/gstreamer" "$@"); }
check "bundle.sh refuses to build without a credits page" yes \
    "$(contains_text "$(credits_bundle_status)" 'no credits page: pass --credits or set MERCURY_CREDITS_DIR')"
check "an empty MERCURY_CREDITS_DIR is refused too" yes \
    "$(contains_text "$(MERCURY_CREDITS_DIR='' bundle_status --gstreamer-prefix "$CREDITS_TMP/gstreamer")" 'no credits page')"
check "a credits directory without credits.html is refused" yes \
    "$(contains_text "$(credits_bundle_status --credits "$CREDITS_TMP/gstreamer")" 'credits.html is missing or empty')"
check "a damaged credits page is refused" yes \
    "$(contains_text "$(credits_bundle_status --credits "$CREDITS_TMP/small")" 'credits.html is only ')"
check "a good credits page passes; the next check is the executable" yes \
    "$(contains_text "$(credits_bundle_status --credits "$CREDITS_TMP/good" --binary "$CREDITS_TMP/no-such-executable")" 'no executable at')"
check "MERCURY_CREDITS_DIR works like --credits" yes \
    "$(contains_text "$(MERCURY_CREDITS_DIR="$CREDITS_TMP/good" bundle_status --gstreamer-prefix "$CREDITS_TMP/gstreamer" --binary "$CREDITS_TMP/no-such-executable")" 'no executable at')"
check "the credits refusals wrote nothing" no "$([ -e "$NO_OUT" ] && echo yes || echo no)"
# The patterns are bundle.sh's literal text, so nothing in them is meant to expand.
# shellcheck disable=SC2016
check "bundle.sh copies the credits page into Contents/Resources/credits" yes \
    "$(grep -q -F 'cp "$CREDITS_DIR/credits.html" "$RES_DIR/credits/credits.html"' scripts/macos/bundle.sh && echo yes || echo no)"
# shellcheck disable=SC2016
check "bundle.sh checks the page it is given before it builds anything" yes \
    "$(grep -q -F 'problem="$(credits_problem "$CREDITS_DIR/credits.html")"' scripts/macos/bundle.sh && echo yes || echo no)"
# The signing job has no build tools by design (release.yml), so this script takes the page as a file.
check "bundle.sh runs neither cargo nor make" no \
    "$(grep -Eq '^[[:space:]]*(cargo|make|rustc|rustup|"\$\{?(CARGO|MAKE)\}?")[[:space:]]' scripts/macos/bundle.sh && echo yes || echo no)"

[ "$failures" -eq 0 ] || { printf '%d check(s) failed\n' "$failures"; exit 1; }
printf 'all checks passed\n'
