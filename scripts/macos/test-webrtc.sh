#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Checks scripts/macos/webrtc.sh, the pinned and fail-closed way to obtain the libwebrtc
# prebuilts, without a network and without the real archives:
#
#   scripts/macos/test-webrtc.sh
#
# Fake archives are served from a file:// directory through MERCURY_WEBRTC_URL_BASE, into a
# cache directory of their own, under a copy of webrtc.sh whose size and sha256 pins are the fake
# archives' (only those lines are replaced; the test checks this). Every network proxy variable
# points at a closed port, so a download that went to a real host would fail the run.
# Prints one line per check and exits non-zero if any fails. Writes only to a temporary
# directory, which it removes.
# Markdown backticks sit inside single-quoted strings below.
# shellcheck disable=SC2016
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."
REAL_SCRIPT="$PWD/scripts/macos/webrtc.sh"
TMP="$(mktemp -d -t mercury-test-webrtc)"
trap 'rm -rf -- "$TMP"' EXIT
failures=0

# No test may reach a real host: every proxy variable names a port nothing listens on.
export http_proxy="http://127.0.0.1:9" https_proxy="http://127.0.0.1:9" all_proxy="http://127.0.0.1:9"
export HTTP_PROXY="$http_proxy" HTTPS_PROXY="$https_proxy" ALL_PROXY="$all_proxy"
export MERCURY_WEBRTC_CURL_RETRY=0
unset MERCURY_WEBRTC_CACHE MERCURY_WEBRTC_URL_BASE LK_CUSTOM_WEBRTC MERCURY_WEBRTC_DIR MERCURY_WEBRTC_C_DIR

check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s: expected [%s], got [%s]\n' "$name" "$expected" "$actual"
        failures=$((failures + 1))
    fi
}

# yes or no: does standard error of the last `run` contain the text?
stderr_has() {
    if grep -F -- "$1" "$TMP/stderr" >/dev/null; then printf 'yes'; else printf 'no'; fi
}

# Runs a command; prints "<exit status>|<standard output>" and keeps standard error in $TMP/stderr.
run() {
    local output code=0
    output="$("$@" 2>"$TMP/stderr")" || code=$?
    printf '%s|%s' "$code" "$output"
}

exit_code() {
    local result
    result="$(run "$@")"
    printf '%s' "${result%%|*}"
}

exists() {
    if [ -e "$1" ]; then printf 'yes'; else printf 'no'; fi
}

size_of() {
    wc -c <"$1" | tr -d ' '
}

# ── The real script's pins are the ones the lane brief names (a tripwire against edits) ──────
real_list="$(run "$REAL_SCRIPT" list)"
check "list works" "0" "${real_list%%|*}"
check "libwebrtc is Shiguredo m150.7871.3.5, 318790999 bytes, the digest GitHub shows" \
    "libwebrtc m150.7871.3.5 318790999 08d1ab62232519bb16958e75e0c5c64a65bdef6aeed4fd45da4028966e1028ca https://github.com/shiguredo-webrtc-build/webrtc-build/releases/download/m150.7871.3.5/webrtc.macos_arm64.tar.gz" \
    "$("$REAL_SCRIPT" list libwebrtc)"
check "libwebrtc-c is shiguredo/webrtc-rs 0.157.0, 159249393 bytes, the digest GitHub shows" \
    "libwebrtc-c 0.157.0 159249393 7e05a006b6aaf506908de717353c94903208539ac662a2653f72fbc33e4b695a https://github.com/shiguredo/webrtc-rs/releases/download/0.157.0/libwebrtc_c-macos_arm64.tar.gz" \
    "$("$REAL_SCRIPT" list libwebrtc-c)"
check "the only hosts the script names are github.com release downloads" "" \
    "$(grep -oE 'https?://[^/" ]+' "$REAL_SCRIPT" | grep -v '^https://github.com$' | sort -u | tr '\n' ' ' | sed 's/ $//')"
for sub in list fetch unpack check env path; do
    check "an unknown id is refused by ${sub}" "1" "$(exit_code "$REAL_SCRIPT" "$sub" bogus)"
done
check "a known id followed by an unknown one is refused before anything is fetched" "1" \
    "$(MERCURY_WEBRTC_CACHE="$TMP/never" exit_code "$REAL_SCRIPT" fetch libwebrtc bogus)"
check "and nothing was created in the cache for it" "no" "$(exists "$TMP/never")"
check "no command prints the usage and exits 2" "2" "$(exit_code "$REAL_SCRIPT")"

# ── Fake archives ───────────────────────────────────────────────────────────────────────────
# libwebrtc: a top-level directory webrtc/ with the key files, a header tree, a symbolic link.
# libwebrtc-c: no top-level directory, plus the .sha256 file the release carries.
SERVED="$TMP/served"
mkdir -p "$SERVED" "$TMP/src/webrtc/lib" "$TMP/src/webrtc/include/api" "$TMP/src/webrtc/Frameworks/W.xcframework/A" "$TMP/src/c/lib"
head -c 20000 /dev/zero | tr '\0' 'a' >"$TMP/src/webrtc/lib/libwebrtc.a"
printf '#pragma once\n' >"$TMP/src/webrtc/include/api/peer_connection_interface.h"
printf 'name with spaces\n' >"$TMP/src/webrtc/include/api/a file.h"
printf 'WEBRTC_SRC_COMMIT=fake\n' >"$TMP/src/webrtc/VERSIONS"
printf 'notice\n' >"$TMP/src/webrtc/NOTICE"
printf 'deps\n' >"$TMP/src/webrtc/DEPS"
printf 'framework\n' >"$TMP/src/webrtc/Frameworks/W.xcframework/A/W"
ln -s A "$TMP/src/webrtc/Frameworks/W.xcframework/Current"
head -c 9000 /dev/zero | tr '\0' 'c' >"$TMP/src/c/lib/libwebrtc_c.a"
printf '// bindings\n' >"$TMP/src/c/bindings.rs"
mkdir -p "$TMP/src/c/include"
printf '#pragma once\n' >"$TMP/src/c/include/webrtc_c.h"
printf 'licence\n' >"$TMP/src/c/LICENSE"
printf 'third party\n' >"$TMP/src/c/THIRD_PARTY_LICENSES.md"
tar -czf "$SERVED/webrtc.macos_arm64.tar.gz" -C "$TMP/src" webrtc
tar -czf "$SERVED/libwebrtc_c-macos_arm64.tar.gz" -C "$TMP/src/c" .
GOOD_SHA="$(shasum -a 256 "$SERVED/webrtc.macos_arm64.tar.gz" | cut -d ' ' -f 1)"
GOOD_SIZE="$(size_of "$SERVED/webrtc.macos_arm64.tar.gz")"
GOODC_SHA="$(shasum -a 256 "$SERVED/libwebrtc_c-macos_arm64.tar.gz" | cut -d ' ' -f 1)"
GOODC_SIZE="$(size_of "$SERVED/libwebrtc_c-macos_arm64.tar.gz")"
printf '%s  libwebrtc_c-macos_arm64.tar.gz\n' "$GOODC_SHA" >"$SERVED/libwebrtc_c-macos_arm64.tar.gz.sha256"
cp "$SERVED/webrtc.macos_arm64.tar.gz" "$TMP/good-webrtc.tar.gz"
cp "$SERVED/libwebrtc_c-macos_arm64.tar.gz" "$TMP/good-webrtc-c.tar.gz"
cp "$SERVED/libwebrtc_c-macos_arm64.tar.gz.sha256" "$TMP/good-webrtc-c.sha256"

# A copy of webrtc.sh with the size and sha256 pins of the fake archives; the key files stay.
SCRIPT="$TMP/webrtc.sh"
sed -e "s/^\( *libwebrtc_size=\).*/\1\"${GOOD_SIZE}\"/" \
    -e "s/^\( *libwebrtc_sha256=\).*/\1\"${GOOD_SHA}\"/" \
    -e "s/^\( *libwebrtc_c_size=\).*/\1\"${GOODC_SIZE}\"/" \
    -e "s/^\( *libwebrtc_c_sha256=\).*/\1\"${GOODC_SHA}\"/" \
    "$REAL_SCRIPT" >"$SCRIPT"
chmod +x "$SCRIPT"
check "the test copy differs from the real script in exactly the four pin lines" "4" \
    "$(diff "$REAL_SCRIPT" "$SCRIPT" | grep -c '^>' || true)"
# Key files of the fake archives are the real key files, so the real checks run unchanged.
check "libwebrtc's key files are the static library, the headers and the notice files" \
    "lib/libwebrtc.a include VERSIONS NOTICE DEPS" "$(sed -n 's/^ *libwebrtc_key_files="\(.*\)"$/\1/p' "$REAL_SCRIPT")"

export MERCURY_WEBRTC_URL_BASE="file://${SERVED}"
CACHE="$TMP/cache"
export MERCURY_WEBRTC_CACHE="$CACHE"
LDIR="$CACHE/libwebrtc/m150.7871.3.5"
CDIR="$CACHE/libwebrtc-c/0.157.0"
LARCHIVE="$LDIR/webrtc.macos_arm64.tar.gz"
CARCHIVE="$CDIR/libwebrtc_c-macos_arm64.tar.gz"
LUNPACKED="$LDIR/unpacked"
CUNPACKED="$CDIR/unpacked"

reset() {
    rm -rf "$CACHE"
    cp "$TMP/good-webrtc.tar.gz" "$SERVED/webrtc.macos_arm64.tar.gz"
    cp "$TMP/good-webrtc-c.tar.gz" "$SERVED/libwebrtc_c-macos_arm64.tar.gz"
    cp "$TMP/good-webrtc-c.sha256" "$SERVED/libwebrtc_c-macos_arm64.tar.gz.sha256"
}

check "list prints the pins and the download URL under the URL base" \
    "libwebrtc m150.7871.3.5 ${GOOD_SIZE} ${GOOD_SHA} file://${SERVED}/webrtc.macos_arm64.tar.gz" \
    "$("$SCRIPT" list libwebrtc)"

# ── fetch ───────────────────────────────────────────────────────────────────────────────────
reset
result="$(run "$SCRIPT" fetch)"
check "fetch of both ids succeeds" "0" "${result%%|*}"
check "the libwebrtc archive is in the cache, whole" "$GOOD_SIZE" "$(size_of "$LARCHIVE")"
check "the libwebrtc-c archive is in the cache, whole" "$GOODC_SIZE" "$(size_of "$CARCHIVE")"
check "fetch says the .sha256 of libwebrtc-c agrees with the pin" "yes" "$(stderr_has "agrees with the pinned sha256")"
check "no .part file is left" "no" "$(exists "$LARCHIVE.part")"
result="$(run "$SCRIPT" fetch)"
check "a second fetch finds the archives already verified" "0|yes" "${result%%|*}|$(stderr_has "already in")"

# A wrong archive of the right size (one byte of the middle changed).
reset
cp "$TMP/good-webrtc.tar.gz" "$SERVED/webrtc.macos_arm64.tar.gz"
printf 'X' | dd of="$SERVED/webrtc.macos_arm64.tar.gz" bs=1 seek=200 conv=notrunc 2>/dev/null
check "the wrong-hash archive has the right size (the test is about the hash)" "$GOOD_SIZE" "$(size_of "$SERVED/webrtc.macos_arm64.tar.gz")"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "a wrong sha256 is refused" "1" "${result%%|*}"
check "the refusal names the pin and says the download was deleted" "yes" "$(stderr_has "does not match the pinned ${GOOD_SHA}")"
check "the wrong download is deleted (no archive)" "no" "$(exists "$LARCHIVE")"
check "the wrong download is deleted (no .part)" "no" "$(exists "$LARCHIVE.part")"
check "nothing is unpacked after a refusal" "no" "$(exists "$LUNPACKED")"

# A truncated download.
reset
head -c $((GOOD_SIZE - 10)) "$TMP/good-webrtc.tar.gz" >"$SERVED/webrtc.macos_arm64.tar.gz"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "a truncated download is refused" "1" "${result%%|*}"
check "the refusal names the byte count" "yes" "$(stderr_has "truncated")"
check "a truncated download is deleted" "no|no" "$(exists "$LARCHIVE")|$(exists "$LARCHIVE.part")"

# An interrupted download is resumed (the first half is already there).
reset
mkdir -p "$LDIR"
head -c $((GOOD_SIZE / 2)) "$TMP/good-webrtc.tar.gz" >"$LARCHIVE.part"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "a half-finished download is completed" "0|$GOOD_SIZE" "${result%%|*}|$(size_of "$LARCHIVE")"
# A .part longer than the pin can only be wrong: it is dropped and the download starts again.
reset
mkdir -p "$LDIR"
head -c $((GOOD_SIZE + 100)) /dev/zero >"$LARCHIVE.part"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "an over-long .part file is discarded and the archive downloaded again" "0|$GOOD_SIZE" "${result%%|*}|$(size_of "$LARCHIVE")"
# A cached archive that does not match is replaced, not trusted.
reset
mkdir -p "$LDIR"
head -c "$GOOD_SIZE" /dev/zero >"$LARCHIVE"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "a cached archive with the wrong content is downloaded again" "0|yes" "${result%%|*}|$(stderr_has "does not match the pin; deleting it")"
check "and then has the pinned sha256" "$GOOD_SHA" "$(shasum -a 256 "$LARCHIVE" | cut -d ' ' -f 1)"

# A release whose own .sha256 file disagrees with the pin.
reset
printf '%s  libwebrtc_c-macos_arm64.tar.gz\n' "$GOOD_SHA" >"$SERVED/libwebrtc_c-macos_arm64.tar.gz.sha256"
result="$(run "$SCRIPT" fetch libwebrtc-c)"
check "a .sha256 file that disagrees with the pin stops the fetch" "1|yes" \
    "${result%%|*}|$(stderr_has "the release's own checksum disagrees with the pin")"

# A missing archive.
reset
rm -f "$SERVED/webrtc.macos_arm64.tar.gz"
result="$(run "$SCRIPT" fetch libwebrtc)"
check "an archive the server does not have fails the fetch and leaves nothing" "1|no|no" \
    "${result%%|*}|$(exists "$LARCHIVE")|$(exists "$LUNPACKED")"

# ── unpack and check ────────────────────────────────────────────────────────────────────────
reset
"$SCRIPT" fetch 2>/dev/null
result="$(run "$SCRIPT" check libwebrtc)"
check "check before unpack fails and says to unpack" "1|yes" "${result%%|*}|$(stderr_has "unpack libwebrtc")"
result="$(run "$SCRIPT" unpack)"
check "unpack of both ids succeeds" "0" "${result%%|*}"
check "the archive's top-level directory is stripped (libwebrtc)" "yes" "$(exists "$LUNPACKED/lib/libwebrtc.a")"
check "the headers and the version file are there" "yes|yes" \
    "$(exists "$LUNPACKED/include/api/peer_connection_interface.h")|$(exists "$LUNPACKED/VERSIONS")"
check "libwebrtc-c unpacks without a top-level directory" "yes|yes" \
    "$(exists "$CUNPACKED/lib/libwebrtc_c.a")|$(exists "$CUNPACKED/bindings.rs")"
check "the marker holds the sha256 of the archive" "$GOOD_SHA" "$(cat "$LUNPACKED/.mercury-webrtc-verified")"
check "the marker of libwebrtc-c holds its own sha256" "$GOODC_SHA" "$(cat "$CUNPACKED/.mercury-webrtc-verified")"
check "the manifest lists every file and link (7 files, 1 link)" "7|1" \
    "$(grep -vc '^link ' "$LUNPACKED/.mercury-webrtc-manifest")|$(grep -c '^link ' "$LUNPACKED/.mercury-webrtc-manifest")"
result="$(run "$SCRIPT" unpack libwebrtc)"
check "a second unpack finds it already unpacked" "0|yes" "${result%%|*}|$(stderr_has "already unpacked")"
result="$(run "$SCRIPT" check)"
check "check passes on a clean cache" "0" "${result%%|*}"
result="$(run "$SCRIPT" check --deep)"
check "check --deep passes on a clean cache" "0|yes" "${result%%|*}|$(stderr_has "every file re-hashed")"

# Everything that can go wrong after unpacking is caught.
corrupt_and_check() {
    local name="$1" what="$2" deep="${3:-}"
    local result
    result="$(run "$SCRIPT" check ${deep:+"$deep"} libwebrtc)"
    check "$name" "1|yes" "${result%%|*}|$(stderr_has "$what")"
    if ! grep -F -- "$what" "$TMP/stderr" >/dev/null; then
        printf '     (standard error was: %s)\n' "$(tail -n 1 "$TMP/stderr")"
    fi
}
printf 'x' >>"$LARCHIVE"
corrupt_and_check "check fails when the archive has changed (size)" "is not ${GOOD_SIZE} bytes"
head -c "$GOOD_SIZE" "$LARCHIVE" >"$TMP/restored" && mv "$TMP/restored" "$LARCHIVE"
check "check passes again once the archive is back" "0" "$(exit_code "$SCRIPT" check libwebrtc)"
cp "$TMP/good-webrtc.tar.gz" "$LARCHIVE"
printf 'X' | dd of="$LARCHIVE" bs=1 seek=200 conv=notrunc 2>/dev/null
corrupt_and_check "check fails when the archive changed without changing size" "fails its sha256 check"
cp "$TMP/good-webrtc.tar.gz" "$LARCHIVE"

printf 'more' >>"$LUNPACKED/lib/libwebrtc.a"
corrupt_and_check "check fails when a library has grown" "lib/libwebrtc.a has changed size"
head -c 20000 /dev/zero | tr '\0' 'a' >"$LUNPACKED/lib/libwebrtc.a"
check "check passes with the library restored" "0" "$(exit_code "$SCRIPT" check libwebrtc)"
head -c 20000 /dev/zero | tr '\0' 'b' >"$LUNPACKED/lib/libwebrtc.a"
check "the default check does not read file contents (a same-size change passes)" "0" "$(exit_code "$SCRIPT" check libwebrtc)"
corrupt_and_check "check --deep catches a same-size change" "lib/libwebrtc.a has changed since it was unpacked (sha256)" "--deep"
head -c 20000 /dev/zero | tr '\0' 'a' >"$LUNPACKED/lib/libwebrtc.a"

printf 'extra\n' >"$LUNPACKED/include/added.h"
corrupt_and_check "check fails when a file was added" "changed after unpacking"
rm -f "$LUNPACKED/include/added.h"
rm -f "$LUNPACKED/VERSIONS"
corrupt_and_check "check fails when a file was removed" "VERSIONS is missing"
printf 'WEBRTC_SRC_COMMIT=fake\n' >"$LUNPACKED/VERSIONS"
mv "$LUNPACKED/include/api/a file.h" "$TMP/a file.h.away"
corrupt_and_check "check fails when a header with a space in its name was removed" "include/api/a file.h is missing from"
mv "$TMP/a file.h.away" "$LUNPACKED/include/api/a file.h"
rm -f "$LUNPACKED/Frameworks/W.xcframework/Current"
ln -s B "$LUNPACKED/Frameworks/W.xcframework/Current"
corrupt_and_check "check fails when a symbolic link points elsewhere" "the symbolic links in"
rm -f "$LUNPACKED/Frameworks/W.xcframework/Current"
ln -s A "$LUNPACKED/Frameworks/W.xcframework/Current"
check "check passes with the link restored" "0" "$(exit_code "$SCRIPT" check libwebrtc)"

mv "$LUNPACKED/lib/libwebrtc.a" "$TMP/libwebrtc.a.away"
corrupt_and_check "check fails when a key file is gone" "lib/libwebrtc.a is missing"
mv "$TMP/libwebrtc.a.away" "$LUNPACKED/lib/libwebrtc.a"
mv "$LUNPACKED/.mercury-webrtc-verified" "$TMP/marker.away"
corrupt_and_check "check fails without the marker" "has no .mercury-webrtc-verified"
printf '%s\n' "0000000000000000000000000000000000000000000000000000000000000000" >"$LUNPACKED/.mercury-webrtc-verified"
corrupt_and_check "check fails when the marker records another hash" "the marker in"
mv "$TMP/marker.away" "$LUNPACKED/.mercury-webrtc-verified"
mv "$LUNPACKED/.mercury-webrtc-manifest" "$TMP/manifest.away"
corrupt_and_check "check fails without the manifest" "mercury-webrtc-manifest is missing"
mv "$TMP/manifest.away" "$LUNPACKED/.mercury-webrtc-manifest"
check "check passes with everything restored" "0" "$(exit_code "$SCRIPT" check --deep)"

# A tree that someone unpacked by hand has no marker and is not accepted.
mkdir -p "$TMP/handmade/lib"
cp "$TMP/src/webrtc/lib/libwebrtc.a" "$TMP/handmade/lib/"
rm -rf "$LUNPACKED"
mv "$TMP/handmade" "$LUNPACKED"
corrupt_and_check "a hand-unpacked directory without the marker is refused" "has no .mercury-webrtc-verified"
"$SCRIPT" unpack libwebrtc 2>/dev/null
check "unpack replaces it with a verified one" "0" "$(exit_code "$SCRIPT" check libwebrtc)"

# An unpack refuses an archive that is not the pinned one.
cp "$TMP/good-webrtc.tar.gz" "$LARCHIVE"
printf 'X' | dd of="$LARCHIVE" bs=1 seek=200 conv=notrunc 2>/dev/null
rm -rf "$LUNPACKED"
result="$(run "$SCRIPT" unpack libwebrtc)"
check "unpack re-hashes the archive and refuses a changed one" "1|no" "${result%%|*}|$(exists "$LUNPACKED")"
cp "$TMP/good-webrtc.tar.gz" "$LARCHIVE"

# An archive with an absolute member path is refused, whatever its hash: a second copy of the
# script whose pin is that archive's.
reset
mkdir -p "$TMP/evil"
printf 'evil\n' >"$TMP/evil/passwd"
tar -czPf "$SERVED/webrtc.macos_arm64.tar.gz" "$TMP/evil/passwd"
EVIL_SHA="$(shasum -a 256 "$SERVED/webrtc.macos_arm64.tar.gz" | cut -d ' ' -f 1)"
EVIL_SIZE="$(size_of "$SERVED/webrtc.macos_arm64.tar.gz")"
sed -e "s/^\( *libwebrtc_size=\).*/\1\"${EVIL_SIZE}\"/" -e "s/^\( *libwebrtc_sha256=\).*/\1\"${EVIL_SHA}\"/" \
    "$REAL_SCRIPT" >"$TMP/webrtc-evil.sh"
chmod +x "$TMP/webrtc-evil.sh"
"$TMP/webrtc-evil.sh" fetch libwebrtc 2>/dev/null
result="$(run "$TMP/webrtc-evil.sh" unpack libwebrtc)"
check "an archive with an absolute path is refused" "1|yes|no" \
    "${result%%|*}|$(stderr_has "absolute path")|$(exists "$LUNPACKED")"

# ── path and env ────────────────────────────────────────────────────────────────────────────
reset
check "path of an id that was never fetched fails" "1" "$(exit_code "$SCRIPT" path libwebrtc)"
"$SCRIPT" fetch 2>/dev/null
check "path of a fetched but unpacked id fails" "1" "$(exit_code "$SCRIPT" path libwebrtc)"
"$SCRIPT" unpack 2>/dev/null
check "path prints the unpacked directory" "0|$LUNPACKED" "$(exit_code "$SCRIPT" path libwebrtc)|$("$SCRIPT" path libwebrtc 2>/dev/null)"
check "path wants exactly one id" "1" "$(exit_code "$SCRIPT" path)"

result="$(run "$SCRIPT" env)"
check "env succeeds on a verified cache" "0" "${result%%|*}"
env_lines="${result#*|}"
check "env exports MERCURY_WEBRTC_DIR, LK_CUSTOM_WEBRTC (the same directory) and MERCURY_WEBRTC_C_DIR" \
    "export MERCURY_WEBRTC_DIR=$LUNPACKED|export LK_CUSTOM_WEBRTC=$LUNPACKED|export MERCURY_WEBRTC_C_DIR=$CUNPACKED" \
    "$(sed -n 1p <<<"$env_lines")|$(sed -n 2p <<<"$env_lines")|$(sed -n 3p <<<"$env_lines")"
check "env prints nothing else" "3" "$(grep -c . <<<"$env_lines")"
check "env evaluates to existing directories" "yes|yes|yes" \
    "$(eval "$env_lines"; exists "$MERCURY_WEBRTC_DIR/lib/libwebrtc.a")|$(eval "$env_lines"; exists "$LK_CUSTOM_WEBRTC/include")|$(eval "$env_lines"; exists "$MERCURY_WEBRTC_C_DIR/lib/libwebrtc_c.a")"
result="$(run "$SCRIPT" env libwebrtc-c)"
check "env of one id prints only that id's variable" "0|export MERCURY_WEBRTC_C_DIR=$CUNPACKED" "${result%%|*}|${result#*|}"
if [ "$(uname -m)" = "arm64" ]; then
    check "env does not warn on an arm64 host" "no" "$(stderr_has "arm64 only")"
else
    run "$SCRIPT" env >/dev/null
    check "env warns on an x86_64 host that the code cannot run there" "yes" "$(stderr_has "arm64 only")"
fi

printf 'x' >>"$LUNPACKED/lib/libwebrtc.a"
result="$(run "$SCRIPT" env)"
check "env refuses an unverified directory and prints nothing" "1|" "${result%%|*}|${result#*|}"
check "env says why" "yes" "$(stderr_has "lib/libwebrtc.a has changed size")"
head -c 20000 /dev/zero | tr '\0' 'a' >"$LUNPACKED/lib/libwebrtc.a"
rm -f "$LUNPACKED/.mercury-webrtc-verified"
result="$(run "$SCRIPT" env libwebrtc)"
check "env refuses a directory without the marker" "1|" "${result%%|*}|${result#*|}"
rm -rf "$CACHE"
result="$(run "$SCRIPT" env)"
check "env refuses when nothing was fetched" "1|" "${result%%|*}|${result#*|}"

# ── inspect, on small real static libraries built with the host's compiler ───────────────────
if command -v cc >/dev/null 2>&1 && command -v ar >/dev/null 2>&1 && command -v nm >/dev/null 2>&1 && command -v otool >/dev/null 2>&1; then
    ISRC="$TMP/isrc"
    mkdir -p "$ISRC/webrtc/lib" "$ISRC/webrtc/include/api" "$ISRC/c/lib" "$ISRC/c/include" "$TMP/iserved" "$TMP/iobj"
    cat >"$TMP/iobj/webrtc.c" <<'CEOF'
int vpx_codec_vp8_cx_algo = 1;
int Cr_z_inflate(void) { return 2; }
int Cr_z_deflate(void) { return 3; }
int SSL_new(void) { return 4; }
int inflate_unprefixed_is_not_here = 5;
CEOF
    cat >"$TMP/iobj/c.c" <<'CEOF'
int webrtc_PeerConnectionFactory_Create(void) { return 1; }
CEOF
    cc -c -o "$TMP/iobj/webrtc.o" "$TMP/iobj/webrtc.c"
    cc -c -o "$TMP/iobj/c.o" "$TMP/iobj/c.c"
    ar rcs "$ISRC/webrtc/lib/libwebrtc.a" "$TMP/iobj/webrtc.o"
    ar rcs "$ISRC/c/lib/libwebrtc_c.a" "$TMP/iobj/c.o"
    printf '#pragma once\n' >"$ISRC/webrtc/include/api/x.h"
    printf 'WEBRTC_SRC_COMMIT=fake\n' >"$ISRC/webrtc/VERSIONS"
    printf 'MACOS_DEPLOYMENT_TARGET=14\n' >"$ISRC/webrtc/DEPS"
    cat >"$ISRC/webrtc/NOTICE" <<'NEOF'
# webrtc
```
Redistribution and use in source and binary forms, with or without modification, are permitted.
Neither the name of Google nor the names of its contributors may be used to endorse.
```

# g711
```
Despite my general liking of the GPL, I place this code in the public domain.
```
NEOF
    printf '// bindings\npub fn webrtc_PeerConnectionFactory_Create();\n' >"$ISRC/c/bindings.rs"
    printf '#pragma once\n' >"$ISRC/c/include/webrtc_c.h"
    printf 'Apache License\nVersion 2.0, January 2004\n' >"$ISRC/c/LICENSE"
    printf '# Third Party Licenses\n\n## WebRTC\n\nRedistribution and use in source and binary forms. Neither the name of Google.\n' >"$ISRC/c/THIRD_PARTY_LICENSES.md"
    tar -czf "$TMP/iserved/webrtc.macos_arm64.tar.gz" -C "$ISRC" webrtc
    tar -czf "$TMP/iserved/libwebrtc_c-macos_arm64.tar.gz" -C "$ISRC/c" .
    ISHA="$(shasum -a 256 "$TMP/iserved/webrtc.macos_arm64.tar.gz" | cut -d ' ' -f 1)"
    ISIZE="$(size_of "$TMP/iserved/webrtc.macos_arm64.tar.gz")"
    ICSHA="$(shasum -a 256 "$TMP/iserved/libwebrtc_c-macos_arm64.tar.gz" | cut -d ' ' -f 1)"
    ICSIZE="$(size_of "$TMP/iserved/libwebrtc_c-macos_arm64.tar.gz")"
    printf '%s  libwebrtc_c-macos_arm64.tar.gz\n' "$ICSHA" >"$TMP/iserved/libwebrtc_c-macos_arm64.tar.gz.sha256"
    sed -e "s/^\( *libwebrtc_size=\).*/\1\"${ISIZE}\"/" -e "s/^\( *libwebrtc_sha256=\).*/\1\"${ISHA}\"/" \
        -e "s/^\( *libwebrtc_c_size=\).*/\1\"${ICSIZE}\"/" -e "s/^\( *libwebrtc_c_sha256=\).*/\1\"${ICSHA}\"/" \
        "$REAL_SCRIPT" >"$TMP/webrtc-inspect.sh"
    chmod +x "$TMP/webrtc-inspect.sh"
    export MERCURY_WEBRTC_URL_BASE="file://$TMP/iserved" MERCURY_WEBRTC_CACHE="$TMP/icache" MERCURY_WEBRTC_INSPECT_DIR="$TMP/inspect"
    "$TMP/webrtc-inspect.sh" fetch 2>/dev/null
    "$TMP/webrtc-inspect.sh" unpack 2>/dev/null
    result="$(HOME="$TMP/nohome" run "$TMP/webrtc-inspect.sh" inspect)"
    report="${result#*|}"
    has() {
        if grep -F -- "$1" <<<"$report" >/dev/null; then printf 'yes'; else printf 'no'; fi
    }
    check "inspect succeeds on both archives" "0" "${result%%|*}"
    check "inspect prints a Markdown report with a section per archive" "yes|yes|yes" \
        "$(has '# libwebrtc prebuilts: static audit')|$(has '## libwebrtc m150.7871.3.5')|$(has '## libwebrtc-c 0.157.0')"
    check "inspect finds the VP8 encoder the fake library defines" "yes" "$(has '| libvpx VP8 encoder (vpx_codec_vp8_cx_algo) | 1 | 0 | 0 |')"
    check "inspect counts the prefixed zlib symbols, and no unprefixed ones" "yes|yes" \
        "$(has '| zlib (Cr_z_ prefix) | 2 | 0 | 0 |')|$(has '| zlib unprefixed (inflate, deflate, crc32) | 0 | 0 | 0 |')"
    check "inspect counts the unprefixed SSL_new" "yes" "$(has '| BoringSSL, unprefixed SSL_new | 1 | 0 | 0 |')"
    check "inspect reports zero for every absence check (Wels, x264, ffmpeg)" "yes|yes|yes" \
        "$(has '| Wels (OpenH264) | 0 | 0 | 0 |')|$(has '| x264 | 0 | 0 | 0 |')|$(has '| ffmpeg in any case | 0 | 0 | 0 |')"
    check "inspect names the licences it finds in NOTICE" "yes|yes" \
        "$(has '| BSD-3-Clause; ')|$(has 'public-domain dedication')"
    check "inspect lists the lines that mention GPL" "yes" "$(has 'Despite my general liking of the GPL')"
    check "inspect searches the library's strings for GPL and FFmpeg and finds none" "yes|yes" \
        "$(has '| `gnu general public` | 0 |')|$(has '| `ffmpeg` | 0 |')"
    check "inspect reads the .sha256 of libwebrtc-c and finds it equal" "yes" "$(has 'the local sha256 is `'"$ICSHA"'`: equal')"
    check "inspect counts the exported wrapper functions" "yes" "$(has '(`_webrtc_` prefix): 1')"
    check "inspect finds the Apache licence of libwebrtc-c" "yes" "$(has '`LICENSE` is Apache License Version 2.0')"
    check "inspect writes its raw symbol lists to the directory it was given" "yes|yes" \
        "$(exists "$TMP/inspect/libwebrtc/c-symbols.txt")|$(exists "$TMP/inspect/libwebrtc-c/members.tsv")"
    check "inspect's C symbol list holds the fake symbols" "1" "$(grep -c -x '_Cr_z_inflate' "$TMP/inspect/libwebrtc/c-symbols.txt")"
    printf 'x' >>"$TMP/icache/libwebrtc/m150.7871.3.5/unpacked/lib/libwebrtc.a"
    result="$(run "$TMP/webrtc-inspect.sh" inspect libwebrtc)"
    check "inspect refuses an archive that no longer verifies and prints no report" "1|" "${result%%|*}|${result#*|}"
    unset MERCURY_WEBRTC_INSPECT_DIR
else
    printf 'SKIP inspect (needs cc, ar, nm and otool)\n'
fi

# ── Nothing reached the network ─────────────────────────────────────────────────────────────
check "no curl call in the script goes anywhere but the pinned URL" "0" \
    "$(grep -c 'curl.*http://' "$REAL_SCRIPT" || true)"

if [ "$failures" -ne 0 ]; then
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
printf 'all checks passed\n'
