#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Obtain the pinned libwebrtc prebuilts for the WebRTC spike (spikes/webrtc/) and
# the real WebRTC lane that follows it, the way gstreamer.sh obtains GStreamer:
# a tag, a size and a SHA-256 per archive, checked after every download and again
# before anything is linked. Nothing here builds libwebrtc and nothing is ever
# fetched without its hash being checked.
#
#   scripts/macos/webrtc.sh list                  the pin table: id, tag, size, sha256, URL
#   scripts/macos/webrtc.sh fetch   [id ...]      download, verify size and sha256, delete on mismatch
#   scripts/macos/webrtc.sh unpack  [id ...]      expand into the cache, write the manifest and the marker
#   scripts/macos/webrtc.sh check   [--deep] [id ...]
#                                                 re-hash the archive, check the marker, the key files
#                                                 and the manifest (--deep re-hashes every unpacked file)
#   scripts/macos/webrtc.sh path    <id>          print the unpacked directory (fails unless verified)
#   scripts/macos/webrtc.sh env     [id ...]      print export lines (fails, printing none, unless
#                                                 `check` passes):
#                                                   MERCURY_WEBRTC_DIR    the libwebrtc prebuilt (id libwebrtc)
#                                                   LK_CUSTOM_WEBRTC      the same directory (LiveKit's variable)
#                                                   MERCURY_WEBRTC_C_DIR  libwebrtc_c (id libwebrtc-c)
#   scripts/macos/webrtc.sh inspect [id ...]      print the static audit of the archives as Markdown
#
# Without an id, every id below is used. Usage:
#   env_lines="$(scripts/macos/webrtc.sh env)" && eval "$env_lines"
#
# The ids and what they are:
#   libwebrtc    shiguredo-webrtc-build/webrtc-build, webrtc.macos_arm64.tar.gz: Google's libwebrtc
#                (BSD-3-Clause, Apache-2.0 scripts) built without H.264 or H.265 codecs. Static
#                lib/libwebrtc.a, the headers in include/, the version files.
#   libwebrtc-c  shiguredo/webrtc-rs, libwebrtc_c-macos_arm64.tar.gz: the C API (webrtc_c) over
#                libwebrtc's classes and the generated Rust bindings that the crate
#                shiguredo_webrtc uses (Apache-2.0).
# Both are arm64 only: Shiguredo dropped macOS x86_64 in June 2022. On an Intel Mac this script
# fetches, verifies and inspects them, and `env` warns that the code can be compiled for
# aarch64-apple-darwin but not run.
#
# A pin is a tag, a byte count and a SHA-256 computed on the real download and equal to the digest
# GitHub shows for the release asset. If a download does not match, it is deleted and the script
# stops; the pin is never edited to fit. The pins below are changed only by a person who has
# downloaded the new archive, hashed it and compared the hash with the release page.
#
# Cache: ${MERCURY_WEBRTC_CACHE:-$HOME/Library/Caches/mercury-browser/webrtc}/<id>/<tag>/
#   <asset>.tar.gz        the verified archive
#   unpacked/             its contents (the archive's single top-level directory is stripped)
#   unpacked/.mercury-webrtc-verified   the marker: one line, the sha256 of the archive that was
#                         unpacked. Build scripts of the spike crates refuse a directory without it.
#   unpacked/.mercury-webrtc-manifest   one line per file: size, sha256, path (written by unpack,
#                         read by check; files whose names start with .mercury-webrtc- are not listed)
# MERCURY_WEBRTC_URL_BASE replaces the host part of the download URL (a file:// directory holding
# the assets, for the tests); the pins above are checked either way. MERCURY_WEBRTC_INSPECT_DIR
# is where `inspect` also writes its raw symbol lists (default: a temporary directory).

set -euo pipefail

PIN_IDS="libwebrtc libwebrtc-c"

# shellcheck disable=SC2034  # read through ${!name} by pin()
{
    libwebrtc_repo="shiguredo-webrtc-build/webrtc-build"
    libwebrtc_tag="m150.7871.3.5"
    libwebrtc_asset="webrtc.macos_arm64.tar.gz"
    libwebrtc_size="318790999"
    libwebrtc_sha256="08d1ab62232519bb16958e75e0c5c64a65bdef6aeed4fd45da4028966e1028ca"
    libwebrtc_sidecar=""
    libwebrtc_key_files="lib/libwebrtc.a include VERSIONS NOTICE DEPS"

    libwebrtc_c_repo="shiguredo/webrtc-rs"
    libwebrtc_c_tag="0.157.0"
    libwebrtc_c_asset="libwebrtc_c-macos_arm64.tar.gz"
    libwebrtc_c_size="159249393"
    libwebrtc_c_sha256="7e05a006b6aaf506908de717353c94903208539ac662a2653f72fbc33e4b695a"
    libwebrtc_c_sidecar="libwebrtc_c-macos_arm64.tar.gz.sha256"
    libwebrtc_c_key_files="lib/libwebrtc_c.a bindings.rs include/webrtc_c.h LICENSE THIRD_PARTY_LICENSES.md"
}

MARKER_NAME=".mercury-webrtc-verified"
MANIFEST_NAME=".mercury-webrtc-manifest"

cache_root="${MERCURY_WEBRTC_CACHE:-${HOME}/Library/Caches/mercury-browser/webrtc}"

# Scratch space for the listings `check` compares; removed when the script ends.
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/mercury-webrtc.XXXXXX")"
trap 'rm -rf -- "$work_dir"' EXIT

say() {
    printf '%s\n' "$*" >&2
}

die() {
    say "error: $*"
    exit 1
}

# pin <id> <field>: the value of one field of an id's pin (aborts on an unknown id or field).
pin() {
    local name="${1//-/_}_$2"
    printf '%s' "${!name}"
}

valid_id() {
    local candidate
    for candidate in $PIN_IDS; do
        [ "$candidate" = "$1" ] && return 0
    done
    return 1
}

# ids_from <args...>: the ids to work on, one per line (all of them when no argument is given).
ids_from() {
    local id
    if [ "$#" -eq 0 ]; then
        for id in $PIN_IDS; do printf '%s\n' "$id"; done
        return 0
    fi
    for id in "$@"; do
        valid_id "$id" || die "unknown id '${id}' (known: ${PIN_IDS})"
        printf '%s\n' "$id"
    done
}

sha256_of() {
    shasum -a 256 "$1" | cut -d ' ' -f 1
}

size_of() {
    wc -c <"$1" | tr -d ' '
}

id_dir() {
    printf '%s/%s/%s' "$cache_root" "$1" "$(pin "$1" tag)"
}

archive_path() {
    printf '%s/%s' "$(id_dir "$1")" "$(pin "$1" asset)"
}

unpacked_path() {
    printf '%s/unpacked' "$(id_dir "$1")"
}

download_url() {
    local id="$1" base="${MERCURY_WEBRTC_URL_BASE:-}"
    if [ -z "$base" ]; then
        base="https://github.com/$(pin "$id" repo)/releases/download/$(pin "$id" tag)"
    fi
    printf '%s/%s' "${base%/}" "$2"
}

# download <url> <destination file>: curl to <destination>.part, resuming a partial file. https only
# (and no redirect to anything else) unless the URL is a file:// one given through
# MERCURY_WEBRTC_URL_BASE. Leaves the .part file for the caller to verify.
download() {
    local url="$1" dest="$2" protocols="=https"
    case "$url" in file://*) protocols="=file" ;; esac
    say "downloading ${url}"
    /usr/bin/curl --location --fail --silent --show-error --retry "${MERCURY_WEBRTC_CURL_RETRY:-5}" --retry-all-errors \
        --proto "$protocols" --proto-redir "$protocols" --continue-at - \
        --output "${dest}.part" "$url" \
        || die "the download of ${url} failed"
}

cmd_list() {
    local id ids
    ids="$(ids_from "$@")"
    while IFS= read -r id; do
        printf '%s %s %s %s %s\n' "$id" "$(pin "$id" tag)" "$(pin "$id" size)" "$(pin "$id" sha256)" \
            "$(download_url "$id" "$(pin "$id" asset)")"
    done <<<"$ids"
}

# fetch_sidecar <id>: the second source of truth some releases carry (a .sha256 file next to the
# archive). It must agree with the pin, or nothing proceeds.
fetch_sidecar() {
    local id="$1" name file expected line recorded
    name="$(pin "$id" sidecar)"
    [ -n "$name" ] || return 0
    file="$(id_dir "$id")/${name}"
    rm -f "$file" "${file}.part"
    download "$(download_url "$id" "$name")" "$file"
    mv -f "${file}.part" "$file"
    expected="$(pin "$id" sha256)"
    line="$(head -c 4096 "$file")"
    recorded="${line%%[[:space:]]*}"
    if [ "$recorded" != "$expected" ]; then
        die "${name} says '${recorded}', the pin says ${expected}; the release's own checksum disagrees with the pin, so nothing is used"
    fi
    say "ok: ${name} agrees with the pinned sha256"
}

fetch_one() {
    local id="$1" dir archive expected_size expected_hash actual part_size
    dir="$(id_dir "$id")"
    archive="$(archive_path "$id")"
    expected_size="$(pin "$id" size)"
    expected_hash="$(pin "$id" sha256)"
    mkdir -p "$dir"
    if [ -f "$archive" ]; then
        if [ "$(size_of "$archive")" = "$expected_size" ] && [ "$(sha256_of "$archive")" = "$expected_hash" ]; then
            say "ok: ${id} (${archive##*/}) is already in ${dir} (size and sha256 verified)"
            fetch_sidecar "$id"
            return 0
        fi
        say "note: ${archive} does not match the pin; deleting it and downloading again"
        rm -f "$archive"
    fi
    # A partial download is resumed; one that is already too long can only be wrong.
    if [ -f "${archive}.part" ] && [ "$(size_of "${archive}.part")" -gt "$expected_size" ]; then
        rm -f "${archive}.part"
    fi
    part_size=0
    [ -f "${archive}.part" ] && part_size="$(size_of "${archive}.part")"
    if [ "$part_size" != "$expected_size" ]; then
        download "$(download_url "$id" "$(pin "$id" asset)")" "$archive"
    fi
    actual="$(size_of "${archive}.part")"
    if [ "$actual" != "$expected_size" ]; then
        rm -f "${archive}.part"
        die "${id}: the download has ${actual} bytes, the pin says ${expected_size} (truncated or the wrong file); it was deleted"
    fi
    actual="$(sha256_of "${archive}.part")"
    if [ "$actual" != "$expected_hash" ]; then
        rm -f "${archive}.part"
        die "${id}: sha256 ${actual} does not match the pinned ${expected_hash}; the download was deleted"
    fi
    mv -f "${archive}.part" "$archive"
    say "ok: ${id} downloaded and verified (${expected_size} bytes, sha256 ${expected_hash})"
    fetch_sidecar "$id"
}

cmd_fetch() {
    local id ids
    # Assigned first: a bad id stops the script here (a failure inside <(...) would not).
    ids="$(ids_from "$@")"
    while IFS= read -r id; do
        fetch_one "$id"
    done <<<"$ids"
}

# verify_archive <id>: the archive is present and has the pinned size and sha256.
verify_archive() {
    local id="$1" archive
    archive="$(archive_path "$id")"
    [ -f "$archive" ] || die "${id}: ${archive} is missing; run: $0 fetch ${id}"
    [ "$(size_of "$archive")" = "$(pin "$id" size)" ] \
        || die "${id}: ${archive} is not $(pin "$id" size) bytes; delete it and run: $0 fetch ${id}"
    [ "$(sha256_of "$archive")" = "$(pin "$id" sha256)" ] \
        || die "${id}: ${archive} fails its sha256 check; delete it and run: $0 fetch ${id}"
}

# The member names of the archive must stay inside the directory it is expanded into.
refuse_unsafe_members() {
    local archive="$1" listing
    listing="$(tar -tzf "$archive")" || die "cannot list ${archive}"
    if grep -E '^/|(^|/)\.\.(/|$)' <<<"$listing" >/dev/null; then
        die "${archive} has a member with an absolute path or a '..' component; refusing to unpack it"
    fi
}

# The listings below run in the unpacked directory (they print paths relative to it, sorted bytewise)
# and use a handful of processes whatever the file count: the libwebrtc tree has 38,000 files, and
# one process per file made `check` take minutes.
tree_files() {
    find . -type f ! -name '.mercury-webrtc-*' -print0 | LC_ALL=C sort -z
}

# "<size> <path>" per regular file.
tree_sizes() {
    if [ "$(uname -s)" = "Darwin" ]; then
        tree_files | xargs -0 -r stat -f '%z %N'
    else
        tree_files | xargs -0 -r stat -c '%s %n'
    fi
}

# "<sha256> <path>" per regular file (shasum, with one space after the hash).
tree_hashes() {
    tree_files | xargs -0 -r shasum -a 256 | sed 's/^\([0-9a-f]\{64\}\)  /\1 /'
}

# "link - <path><TAB><target>" per symbolic link.
tree_links() {
    local link
    find . -type l -print0 | LC_ALL=C sort -z | while IFS= read -r -d '' link; do
        printf 'link - %s\t%s\n' "$link" "$(readlink "$link")"
    done
}

# write_manifest <directory>: "<size> <sha256> <path>" for every regular file below the directory,
# then a "link - <path><TAB><target>" line for every symbolic link.
write_manifest() {
    local root="$1" tmp
    tmp="${work_dir}/manifest.$$"
    mkdir -p "$tmp"
    (
        cd "$root"
        tree_hashes >"${tmp}/hashes"
        tree_sizes | cut -d ' ' -f 1 >"${tmp}/sizes"
        paste -d ' ' "${tmp}/sizes" "${tmp}/hashes" >"${tmp}/manifest"
        tree_links >>"${tmp}/manifest"
    )
    mv -f "${tmp}/manifest" "${root}/${MANIFEST_NAME}"
    rm -rf "$tmp"
}

# first_difference <expected listing> <actual listing>: a listing line is "<key> <path>"; prints
# "missing <path>", "changed <path>" or "added <path>" for the first path that differs.
first_difference() {
    awk '
        NR == FNR { want[substr($0, length($1) + 2)] = $1; next }
        { have[substr($0, length($1) + 2)] = $1 }
        END {
            for (path in want) {
                if (!(path in have)) { print "missing " path; exit }
                if (have[path] != want[path]) { print "changed " path; exit }
            }
            for (path in have) if (!(path in want)) { print "added " path; exit }
        }' "$1" "$2"
}

unpack_one() {
    local id="$1" dir archive target staging entries count only
    dir="$(id_dir "$id")"
    archive="$(archive_path "$id")"
    target="$(unpacked_path "$id")"
    verify_archive "$id"
    if [ -f "${target}/${MARKER_NAME}" ] && [ "$(head -n 1 "${target}/${MARKER_NAME}")" = "$(pin "$id" sha256)" ]; then
        say "ok: ${id} is already unpacked in ${target}"
        return 0
    fi
    refuse_unsafe_members "$archive"
    staging="${dir}/unpacked.partial"
    rm -rf "$staging" "$target"
    mkdir -p "$staging"
    say "expanding ${archive##*/}"
    tar -xzf "$archive" -C "$staging" || { rm -rf "$staging"; die "${id}: expanding ${archive} failed"; }
    # An archive with one top-level directory (webrtc/) is used from inside it.
    entries="$(find "$staging" -mindepth 1 -maxdepth 1 -print)"
    count="$(grep -c . <<<"$entries" || true)"
    if [ "$count" = "1" ] && [ -d "$entries" ] && [ ! -L "$entries" ]; then
        only="$entries"
        mv "$only" "${staging}.root"
        rm -rf "$staging"
        mv "${staging}.root" "$target"
    else
        mv "$staging" "$target"
    fi
    rm -f "${target}/${MARKER_NAME}" "${target}/${MANIFEST_NAME}"
    write_manifest "$target"
    printf '%s\n' "$(pin "$id" sha256)" >"${target}/${MARKER_NAME}"
    say "ok: ${id} unpacked to ${target} ($(du -sh "$target" | cut -f 1))"
    check_one "$id" ""
}

cmd_unpack() {
    local id ids
    ids="$(ids_from "$@")"
    while IFS= read -r id; do
        unpack_one "$id"
    done <<<"$ids"
}

# check_one <id> <--deep or empty>
check_one() {
    local id="$1" deep="$2" target key marker manifest expected actual difference count
    target="$(unpacked_path "$id")"
    manifest="${target}/${MANIFEST_NAME}"
    verify_archive "$id"
    [ -d "$target" ] || die "${id}: ${target} does not exist; run: $0 unpack ${id}"
    [ -f "${target}/${MARKER_NAME}" ] || die "${id}: ${target} has no ${MARKER_NAME}; it was not unpacked by this script (run: $0 unpack ${id})"
    marker="$(head -n 1 "${target}/${MARKER_NAME}")"
    [ "$marker" = "$(pin "$id" sha256)" ] \
        || die "${id}: the marker in ${target} records ${marker}, not the pinned $(pin "$id" sha256)"
    for key in $(pin "$id" key_files); do
        [ -e "${target}/${key}" ] || die "${id}: ${target}/${key} is missing"
    done
    [ -f "$manifest" ] || die "${id}: ${manifest} is missing; run: $0 unpack ${id} again"

    # Sizes: the files that are there are the files that were unpacked, byte count for byte count.
    expected="${work_dir}/expected-sizes"
    actual="${work_dir}/actual-sizes"
    sed -n 's/^\([0-9][0-9]*\) [0-9a-f]\{64\} \(.*\)$/\1 \2/p' "$manifest" >"$expected"
    (cd "$target" && tree_sizes) >"$actual"
    if ! cmp -s "$expected" "$actual"; then
        difference="$(first_difference "$expected" "$actual")"
        case "$difference" in
            "missing "*) die "${id}: ${difference#missing } is missing from ${target}" ;;
            "changed "*) die "${id}: ${difference#changed } has changed size since it was unpacked" ;;
            *) die "${id}: ${target} was changed after unpacking (${difference})" ;;
        esac
    fi
    count="$(grep -c . "$expected" || true)"
    # Symbolic links point where they did.
    expected="${work_dir}/expected-links"
    actual="${work_dir}/actual-links"
    sed -n '/^link - /p' "$manifest" >"$expected"
    (cd "$target" && tree_links) >"$actual"
    if ! cmp -s "$expected" "$actual"; then
        difference="$(diff "$expected" "$actual" | sed -n '/^[<>]/{p;q;}' || true)"
        die "${id}: the symbolic links in ${target} are not what was unpacked (${difference})"
    fi
    if [ -n "$deep" ]; then
        expected="${work_dir}/expected-hashes"
        actual="${work_dir}/actual-hashes"
        sed -n 's/^[0-9][0-9]* \([0-9a-f]\{64\}\) \(.*\)$/\1 \2/p' "$manifest" >"$expected"
        (cd "$target" && tree_hashes) >"$actual"
        if ! cmp -s "$expected" "$actual"; then
            difference="$(first_difference "$expected" "$actual")"
            die "${id}: ${difference#* } has changed since it was unpacked (sha256)"
        fi
    fi
    say "ok: ${id} $(pin "$id" tag) verified (archive sha256, marker, key files, ${count} files${deep:+, every file re-hashed})"
}

cmd_check() {
    local deep="" id ids
    if [ "${1:-}" = "--deep" ]; then
        deep="--deep"
        shift
    fi
    ids="$(ids_from "$@")"
    while IFS= read -r id; do
        check_one "$id" "$deep"
    done <<<"$ids"
}

cmd_path() {
    [ "$#" -eq 1 ] || die "usage: $0 path <id>"
    valid_id "$1" || die "unknown id '$1' (known: ${PIN_IDS})"
    local log
    log="$(check_one "$1" "" 2>&1)" || {
        say "$log"
        exit 1
    }
    unpacked_path "$1"
    printf '\n'
}

cmd_env() {
    local ids id lines="" target
    ids="$(ids_from "$@")"
    while IFS= read -r id; do
        check_one "$id" "" >&2
        target="$(unpacked_path "$id")"
        case "$id" in
            libwebrtc)
                lines="${lines}$(printf 'export MERCURY_WEBRTC_DIR=%q\nexport LK_CUSTOM_WEBRTC=%q\n' "$target" "$target")"$'\n' ;;
            libwebrtc-c)
                lines="${lines}$(printf 'export MERCURY_WEBRTC_C_DIR=%q\n' "$target")"$'\n' ;;
        esac
    done <<<"$ids"
    if [ "$(uname -m)" != "arm64" ]; then
        say "warning: this host is $(uname -m); the archives are arm64 only. Code can be compiled for"
        say "aarch64-apple-darwin (cargo ... --target aarch64-apple-darwin) but it cannot be run here."
    fi
    printf '%s' "$lines"
}

cmd_inspect() {
    die "inspect is not written yet"
}

command="${1:-}"
[ "$#" -gt 0 ] && shift
case "$command" in
    list) cmd_list "$@" ;;
    fetch) cmd_fetch "$@" ;;
    unpack) cmd_unpack "$@" ;;
    check) cmd_check "$@" ;;
    path) cmd_path "$@" ;;
    env) cmd_env "$@" ;;
    inspect) cmd_inspect "$@" ;;
    *)
        say "usage: $0 list|fetch|unpack|check|path|env|inspect [id ...]"
        say "ids: ${PIN_IDS}"
        exit 2
        ;;
esac
