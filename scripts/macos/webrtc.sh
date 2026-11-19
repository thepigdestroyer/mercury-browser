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

# The Markdown that `inspect` prints has backticks inside printf formats.
# shellcheck disable=SC2016
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
inspect_dir=""
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

# ── inspect: the static audit of an unpacked archive, as Markdown ───────────────────────────────
# Everything here reads files and runs nm, otool, lipo and vtool on them; nothing from an archive is
# executed, so it works on an Intel Mac and on the arm64 runner alike.

# md_row <cell> ...: one Markdown table row.
md_row() {
    local row="|" cell
    for cell in "$@"; do
        row="${row} ${cell} |"
    done
    printf '%s\n' "$row"
}

# md_header <cell> ...: a header row and its separator row.
md_header() {
    local separator="|" cell
    md_row "$@"
    for cell in "$@"; do
        separator="${separator} --- |"
    done
    printf '%s\n' "$separator"
}

# human <bytes>: 12345678 -> 11.8 MiB.
human() {
    awk -v b="$1" 'BEGIN {
        split("B KiB MiB GiB", unit, " "); i = 1
        while (b >= 1024 && i < 4) { b /= 1024; i++ }
        if (i == 1) printf "%d %s", b, unit[i]; else printf "%.1f %s", b, unit[i]
    }'
}

# sym_count <symbols.tsv> <def|undef|local|all> <awk regex>: how many symbols of that kind match.
# The table has the columns type, member and symbol (see build_symbol_table).
sym_count() {
    awk -F'\t' -v kind="$2" -v re="$3" '
        {
            ok = (kind == "all") || (kind == "undef" && $1 == "U") || (kind == "def" && $1 ~ /^[A-TV-Z]$/) ||
                 (kind == "local" && $1 ~ /^[a-z]$/)
            if (ok && $3 ~ re) n++
        }
        END { print n + 0 }' "$1"
}

# build_symbol_table <archive> <output>: "type<TAB>member<TAB>symbol" for every symbol of every member
# (nm -A: global, local and undefined).
build_symbol_table() {
    (cd "$(dirname -- "$1")" && nm -A "$(basename -- "$1")") | awk '
        { m = $1; sub(/^[^:]*:/, "", m); sub(/:$/, "", m)
          if ($2 == "U") print "U\t" m "\t" $3; else print $3 "\t" m "\t" $4 }' >"$2"
}

# One row per check: <label> <awk regex>, ... The counts are of global definitions, undefined
# references and local symbols, over all members; one pass over the table for all rows.
inspect_rows() {
    local table="$1" specs="${work_dir}/row-specs" label re
    shift
    : >"$specs"
    while [ "$#" -ge 2 ]; do
        printf '%s\t%s\n' "$1" "$2" >>"$specs"
        shift 2
    done
    awk -F'\t' '
        BEGIN { n = 0 }
        NR == FNR { label[n] = $1; re[n] = $2; n++; next }
        {
            kind = ""
            if ($1 == "U") kind = "undef"
            else if ($1 ~ /^[A-TV-Z]$/) kind = "def"
            else if ($1 ~ /^[a-z]$/) kind = "local"
            if (kind == "") next
            for (i = 0; i < n; i++) if ($3 ~ re[i]) count[i, kind]++
        }
        END { for (i = 0; i < n; i++) printf "%s\t%d\t%d\t%d\n", label[i], count[i, "def"], count[i, "undef"], count[i, "local"] }' \
        "$specs" "$table" | while IFS=$'\t' read -r label defined referenced local_count; do
        md_row "$label" "$defined" "$referenced" "$local_count"
    done
}

# The component each archive member belongs to, by the symbols it defines and, failing that, by its
# name. One line per member: "<component><TAB><member>".
classify_members() {
    awk -F'\t' '
        $1 == "U" { next }
        {
            m = $2; s = $3; seen[m] = 1
            if (s ~ /^_(vpx_|vp8_|vp9_|vpx|vp8cx|vp8dx)/) k = "libvpx"
            else if (s ~ /^_dav1d_/) k = "dav1d"
            else if (s ~ /^_(aom_|av1_|aomx_|od_|cdef_|av1)/) k = "libaom"
            else if (s ~ /^_(opus_|silk_|celt_|ec_|WebRtcOpus|mapping|clt_|downmix|opus)/) k = "opus"
            else if (s ~ /^_(srtp_|crypto_kernel|aes_icm|aes_gcm|stat_test|rdbx|null_cipher|datatypes)/) k = "libsrtp"
            else if (s ~ /^_(chromium_|jsimd_|jpeg_)/) k = "libjpeg-turbo"
            else if (s ~ /^_Cr_z_/) k = "zlib (Chromium, Cr_z_ prefixed)"
            else if (s ~ /^__ZN4bssl|^_(SSL_|EVP_|X509_|BN_|CRYPTO_|OPENSSL_|ERR_|RSA_|EC_|ECDSA_|ECDH_|AES_|SHA|HMAC|CBS_|CBB_|BIO_|ASN1_|OBJ_|d2i_|i2d_|DH_|DSA_|RAND_|ED25519|X25519|CTR_|GCM|bssl|ssl_|x509|PEM_|CONF_|NCONF|BUF_|PKCS|sk_|MD5|RC4|chacha|fiat_|KYBER|MLKEM|MLDSA|SLHDSA|TRUST_TOKEN|SPAKE|XWING|poly1305|aes_|bn_|ecp_|ec_|rsa_|evp_|des_|DES_)/) k = "BoringSSL"
            else if (s ~ /^__ZN6libyuv|^_(I4[0-9]|NV|ARGB|Scale|ConvertTo|ConvertFrom|Rotate|Mirror|Android|RAW|RGB|UYVY|YUY|H4|J4|I2|I0|I3|P2|Interpolate|Blend|Sobel|Compare|Hash|Sum|Convert)/) k = "libyuv"
            else if (s ~ /^__ZN6dcsctp|^__ZN6webrtc|^__ZN3rtc|^__ZN8cricket|^__ZN7rnnoise|^_WebRtc|^_OBJC_(CLASS|METACLASS|IVAR)_[$]_RTC|^_RTC|^_rtc_|^_webrtc_|^_kRTC/) k = "webrtc"
            else if (s ~ /^_(PaUtil|Pa_|pffft|ooura|rdft|cdft|makewt|G722)/) k = "webrtc third_party DSP"
            else if (s ~ /google8protobuf/) k = "protobuf"
            else if (s ~ /perfetto/) k = "perfetto"
            else if (s ~ /^__ZN4absl|^_Absl/) k = "abseil"
            else if (s ~ /^___cxa_|^__Znwm$|^__Znam$|^___dynamic_cast$|^___gxx_personality_v0$|^__ZSt9terminatev$|^__ZNSt4__Cr(6locale|8ios_base|4cout|4cerr|5ctype|7collate|8messages|9basic_ios|13basic_istream|13basic_ostream|12system_error|19__shared_weak_count)/) k = "libc++ and libc++abi"
            else k = "none"
            c[m, k]++
        }
        END {
            # A member that defines webrtc symbols is a webrtc member even when it also defines
            # protobuf, abseil or std:: template instantiations, so webrtc comes before those.
            n = split("libvpx|dav1d|libaom|opus|libsrtp|libjpeg-turbo|zlib (Chromium, Cr_z_ prefixed)|BoringSSL|libyuv|webrtc|webrtc third_party DSP|protobuf|perfetto|abseil|libc++ and libc++abi", order, "|")
            for (m in seen) {
                cls = "none"
                for (i = 1; i <= n; i++) if (c[m, order[i]] > 0) { cls = order[i]; break }
                if (cls == "none") {
                    # Members without a recognisable symbol: arch-guarded stubs of BoringSSL and libaom
                    # assembly, protozero generated files, libyuv and opus leftovers; sorted by name.
                    if (m ~ /[.]pbzero[.]o$|^trace_|^tracing/) cls = "perfetto"
                    else if (m ~ /-(apple|linux|win|freebsd|openbsd|netbsd)[.]o$|^cpu_(aarch64|arm|intel|ppc)|^(siphash|engine|getentropy|err_data|mlkem|xwing|ssl_|tasn_|t_x509|v3_|f_string|buf|rc4|input)/) cls = "BoringSSL"
                    else if (m ~ /^(row_|scale_|compare_|rotate_|convert_)/) cls = "libyuv"
                    else if (m ~ /^(vp9_|vp8_)/) cls = "libvpx"
                    else if (m ~ /^(cwrs|odintrin|cdef|mathops|pitch)/) cls = "opus or libaom (unclear)"
                    else if (m ~ /^(trees|inflate|gz|crc32|deflate|adler)/) cls = "zlib (Chromium, Cr_z_ prefixed)"
                    else if (m ~ /^j[a-z0-9]+[.]o$/) cls = "libjpeg-turbo"
                    else cls = "unclassified (no global symbol)"
                }
                print cls "\t" m
            }
        }' "$1"
}

# notice_components <file> <heading marker>: one line per component of a NOTICE-style file (headings
# such as "# name" with the marker "#", each followed by the licence text):
# "<name><TAB><lines><TAB><licence words found>".
notice_components() {
    awk -v marker="$2" '
        function flush() {
            if (name == "") return
            k = ""
            if (txt ~ /Apache License/ && txt ~ /LLVM Exception/) k = k "Apache-2.0 WITH LLVM-exception; "
            else if (txt ~ /Apache License/) k = k "Apache-2.0; "
            if (txt ~ /Permission is hereby granted, free of charge/) k = k "MIT; "
            if (txt ~ /Redistribution and use in source and binary forms/ && txt ~ /Neither the name/) k = k "BSD-3-Clause; "
            else if (txt ~ /Redistribution and use in source and binary forms/) k = k "BSD-2-Clause; "
            if (txt ~ /with or without fee/) k = k "ISC; "
            if (txt ~ /Altered source versions/) k = k "zlib; "
            if (txt ~ /Independent JPEG Group/) k = k "IJG; "
            if (txt ~ /SSLeay|OpenSSL/) k = k "OpenSSL/SSLeay (advertising clause); "
            if (txt ~ /public domain/ || txt ~ /Public Domain/) k = k "public-domain dedication; "
            if (txt ~ /Mozilla Public License/) k = k "MPL (REVIEW); "
            if (txt ~ /GNU (Lesser |Library )?General Public License|GPLv[23]|LGPL/) k = k "mentions GPL/LGPL (REVIEW: see the lines below); "
            if (k == "") k = "custom permissive text (read it)"
            printf "%s\t%d\t%s\n", name, lines, k
        }
        index($0, marker " ") == 1 { flush(); name = substr($0, length(marker) + 2); lines = 0; txt = "" }
        { lines++; txt = txt "\n" $0 }
        END { flush() }' "$1"
}

# inspect_licences <unpacked dir> <id>
inspect_licences() {
    local dir="$1" id="$2" file components marker
    printf '\n### Licence and notice files\n\n'
    printf 'Files in the archive whose name says licence, notice, patents or copying (outside include/):\n\n'
    (cd "$dir" && find . -path ./include -prune -o -type f \( -iname 'LICENSE*' -o -iname 'NOTICE*' -o -iname 'PATENTS*' -o -iname 'COPYING*' -o -iname 'THIRD_PARTY*' \) -print | LC_ALL=C sort) |
        while IFS= read -r file; do
            printf -- '- `%s` (%s bytes)\n' "${file#./}" "$(size_of "${dir}/${file}")"
        done
    printf '\nPATENTS files anywhere (including include/): %s\n' \
        "$(cd "$dir" && find . -iname 'PATENTS*' -type f | grep -c . || true)"
    for file in NOTICE THIRD_PARTY_LICENSES.md; do
        [ -f "${dir}/${file}" ] || continue
        # NOTICE has "# component" headings; THIRD_PARTY_LICENSES.md has "# title" and "## component".
        marker="#"
        [ "$file" = "NOTICE" ] || marker="##"
        components="$(notice_components "${dir}/${file}" "$marker")"
        if [ -z "$components" ]; then
            continue
        fi
        printf '\nComponents named in `%s`:\n\n' "$file"
        md_header "Component" "Lines of text" "Licence found in the text"
        while IFS=$'\t' read -r name lines licences; do
            md_row "$name" "$lines" "$licences"
        done <<<"$components"
        if grep -q -i -E 'GNU|GPL' "${dir}/${file}"; then
            printf '\nLines that mention GPL or LGPL in `%s` (these are the only ones; each is a mention, not a licence):\n\n' "$file"
            grep -n -i -E 'GNU|GPL' "${dir}/${file}" | cut -c1-140 | sed 's/^/    /' || true
        else
            printf '\nNo line of `%s` mentions GPL, LGPL or GNU.\n' "$file"
        fi
    done
    if [ "$id" = "libwebrtc-c" ]; then
        printf '\n`LICENSE` is %s.\n' "$(head -n 3 "${dir}/LICENSE" | tr -s ' \n' ' ' | sed 's/^ //; s/ $//')"
    fi
}

# inspect_one <id>
inspect_one() {
    local id="$1" dir archive lib tag table out first_members cls
    tag="$(pin "$id" tag)"
    dir="$(unpacked_path "$id")"
    archive="$(archive_path "$id")"
    case "$id" in
        libwebrtc) lib="${dir}/lib/libwebrtc.a" ;;
        libwebrtc-c) lib="${dir}/lib/libwebrtc_c.a" ;;
    esac
    out="${inspect_dir}/${id}"
    mkdir -p "$out"
    table="${out}/symbols.tsv"

    printf '\n## %s %s\n\n' "$id" "$tag"
    printf -- '- Asset: `%s` from `%s`\n' "$(pin "$id" asset)" "$(pin "$id" repo)"
    printf -- '- Tarball: %s bytes (%s), sha256 `%s`, verified by `check`\n' "$(pin "$id" size)" "$(human "$(pin "$id" size)")" "$(pin "$id" sha256)"
    printf -- '- Unpacked: %s KiB in %s files and %s symbolic links\n' \
        "$(du -sk "$dir" | cut -f 1)" "$(grep -vc '^link ' "${dir}/${MANIFEST_NAME}" || true)" "$(grep -c '^link ' "${dir}/${MANIFEST_NAME}" || true)"
    printf -- '- Static library: `%s`, %s bytes (%s)\n' "${lib#"${dir}"/}" "$(size_of "$lib")" "$(human "$(size_of "$lib")")"
    printf -- '- Architecture: `%s`\n' "$(lipo -info "$lib" 2>&1)"
    if [ -f "$(archive_path "$id").sha256" ] || [ -n "$(pin "$id" sidecar)" ]; then
        printf -- '- Release checksum file `%s`: `%s`; the local sha256 is `%s`: %s\n' "$(pin "$id" sidecar)" \
            "$(head -c 200 "$(id_dir "$id")/$(pin "$id" sidecar)" 2>/dev/null | tr -d '\n')" "$(sha256_of "$archive")" \
            "$([ "$(head -c 64 "$(id_dir "$id")/$(pin "$id" sidecar)" 2>/dev/null)" = "$(sha256_of "$archive")" ] && echo equal || echo DIFFERENT)"
    fi

    printf '\n### Tree (top level of the unpacked archive)\n\n'
    md_header "Entry" "Size" "Files"
    (cd "$dir" && for entry in * ; do
        [ -e "$entry" ] || continue
        md_row "\`${entry}\`" "$(human $(($(du -sk "$entry" | cut -f 1) * 1024)))" "$(find "$entry" -type f | grep -c . || true)"
    done)
    if [ -d "${dir}/include" ]; then
        printf '\n`include/` holds %s files; the largest groups:\n\n' "$(find "${dir}/include" -type f | grep -c . || true)"
        md_header "Directory" "Files"
        (cd "${dir}/include" && for entry in * third_party/*; do
            if [ -d "$entry" ]; then printf '%s\t%s\n' "$(find "$entry" -type f | grep -c . || true)" "$entry"; fi
        done | LC_ALL=C sort -rn | head -12 | while IFS=$'\t' read -r n entry; do md_row "\`include/${entry}\`" "$n"; done)
    fi

    inspect_licences "$dir" "$id"

    say "inspect: reading the symbols of ${lib##*/} (${id})"
    build_symbol_table "$lib" "$table"
    printf '\n### Build target\n\n'
    printf 'Members of the archive: %s (%s distinct names).\n\n' "$(ar t "$lib" | grep -vc '^__.SYMDEF' || true)" \
        "$(ar t "$lib" | grep -v '^__.SYMDEF' | LC_ALL=C sort -u | grep -c . || true)"
    md_header "minos and sdk of the members (otool -l)" "Members"
    otool -l "$lib" | awk '
        /^ +minos / { minos = $2 }
        /^ +sdk /   { sdk = $2; count["minos " minos ", sdk " sdk]++ }
        END { for (k in count) print count[k] "\t" k }' | LC_ALL=C sort -rn |
        while IFS=$'\t' read -r n what; do md_row "$what" "$n"; done

    printf '\n### Members by component\n\n'
    printf 'By the symbols each member defines (and, for members without a recognisable symbol, by name): a guide, not a proof. Full list: `%s`.\n\n' "${out}/members.tsv"
    classify_members "$table" | LC_ALL=C sort >"${out}/members.tsv"
    md_header "Component" "Members" "First members"
    cut -f 1 "${out}/members.tsv" | LC_ALL=C sort | uniq -c | LC_ALL=C sort -rn | while read -r n cls_name; do
        cls="${cls_name}"
        first_members="$(awk -F'\t' -v c="$cls" '$1 == c { printf "%s ", $2; if (++k == 5) exit }' "${out}/members.tsv")"
        md_row "$cls" "$n" "\`${first_members% }\`"
    done

    printf '\n### Absence checks (no software H.264, H.265, FFmpeg)\n\n'
    printf 'Counts over every symbol of every member: global definitions, undefined references, local symbols.\n\n'
    md_header "Pattern (awk regex on the symbol)" "Defined" "Referenced" "Local"
    inspect_rows "$table" \
        'Wels (OpenH264)' 'Wels' \
        'openh264 or OpenH264' '[Oo]pen[Hh]264' \
        'x264' 'x264' \
        'x265' 'x265' \
        'libde265 (de265)' 'de265' \
        'ff_ (FFmpeg internals)' '^_ff_' \
        'avcodec_' '^_avcodec_' \
        'av_ (FFmpeg)' '^_av_' \
        'ffmpeg in any case' '[Ff][Ff][Mm][Pp][Ee][Gg]' \
        'iLBC (WebRtcIlbc)' 'WebRtcIlbc'

    printf '\n### Strings in the library (strings -a, case-insensitive)\n\n'
    local strings_hits token
    strings_hits="$(strings -a "$lib" | LC_ALL=C grep -i -o -E 'GNU General Public|GNU Lesser|LGPL|GPLv[23]|FFmpeg|libavcodec|OpenH264|x264|libde265|Cisco Systems' | tr '[:upper:]' '[:lower:]' | LC_ALL=C sort | uniq -c || true)"
    md_header "String" "Occurrences"
    for token in 'gnu general public' 'gnu lesser' 'lgpl' 'gplv2' 'gplv3' 'ffmpeg' 'libavcodec' 'openh264' 'x264' 'libde265' 'cisco systems'; do
        md_row "\`${token}\`" "$(awk -v t="$token" 'index($0, " " t) { print $1; found = 1 } END { if (!found) print 0 }' <<<"$strings_hits")"
    done

    printf '\n### Presence checks\n\n'
    md_header "Pattern (awk regex on the symbol)" "Defined" "Referenced" "Local"
    inspect_rows "$table" \
        'ObjC RTCDefaultVideoEncoderFactory' '^_OBJC_CLASS_[$]_RTCDefaultVideoEncoderFactory$' \
        'ObjC RTCDefaultVideoDecoderFactory' '^_OBJC_CLASS_[$]_RTCDefaultVideoDecoderFactory$' \
        'ObjC RTCVideoEncoderH264 (VideoToolbox)' '^_OBJC_CLASS_[$]_RTCVideoEncoderH264$' \
        'ObjC RTCVideoDecoderH264 (VideoToolbox)' '^_OBJC_CLASS_[$]_RTCVideoDecoderH264$' \
        'ObjC RTCVideoEncoderFactoryH264' '^_OBJC_CLASS_[$]_RTCVideoEncoderFactoryH264$' \
        'ObjC RTCVideoEncoderH265 (VideoToolbox)' '^_OBJC_CLASS_[$]_RTCVideoEncoderH265$' \
        'ObjC RTCVideoDecoderH265 (VideoToolbox)' '^_OBJC_CLASS_[$]_RTCVideoDecoderH265$' \
        'VTCompressionSessionCreate (the OS encoder)' '^_VTCompressionSessionCreate$' \
        'VTDecompressionSessionCreate (the OS decoder)' '^_VTDecompressionSessionCreate$' \
        'webrtc::H264 and H265 RTP packetizers and parsers' 'RtpPacketizerH26[45]|H26[45]BitstreamParser' \
        'libvpx VP8 encoder (vpx_codec_vp8_cx_algo)' '^_vpx_codec_vp8_cx_algo$' \
        'libvpx VP8 decoder (vpx_codec_vp8_dx_algo)' '^_vpx_codec_vp8_dx_algo$' \
        'libvpx VP9 encoder (vpx_codec_vp9_cx_algo)' '^_vpx_codec_vp9_cx_algo$' \
        'libvpx VP9 decoder (vpx_codec_vp9_dx_algo)' '^_vpx_codec_vp9_dx_algo$' \
        'libvpx, all vpx_codec_* functions' '^_vpx_codec_' \
        'libaom AV1 encoder (aom_codec_av1_cx_algo)' '^_aom_codec_av1_cx_algo$' \
        'libaom AV1 decoder (aom_codec_av1_dx_algo)' '^_aom_codec_av1_dx_algo$' \
        'dav1d (dav1d_open)' '^_dav1d_open$' \
        'dav1d, all dav1d_* functions' '^_dav1d_' \
        'Opus (opus_encoder_create)' '^_opus_encoder_create$' \
        'Opus (opus_decoder_create)' '^_opus_decoder_create$' \
        'iSAC (WebRtcIsac_)' '^_WebRtcIsac_' \
        'G.711 and G.722 (WebRtcG7xx)' '^_WebRtcG7' \
        'BoringSSL, unprefixed SSL_new' '^_SSL_new$' \
        'BoringSSL, unprefixed EVP_ functions' '^_EVP_' \
        'BoringSSL, any boringssl or BORINGSSL prefixed name' '^_(boringssl|BORINGSSL|bssl)_' \
        'dcsctp (namespace dcsctp)' '__ZN6dcsctp' \
        'libsrtp (srtp_ functions)' '^_srtp_' \
        'libyuv (namespace libyuv)' '__ZN6libyuv' \
        'rnnoise weights (namespace rnnoise)' '__ZN7rnnoise' \
        'AEC3 (EchoCanceller3)' 'EchoCanceller3' \
        'AecDump' 'AecDump' \
        'AudioProcessingBuilder' 'AudioProcessingBuilder' \
        'AudioDeviceMac (the CoreAudio device layer)' 'AudioDeviceMac' \
        'AudioDeviceModule (any)' 'AudioDeviceModule' \
        'Desktop capture (DesktopCapturer)' 'DesktopCapturer' \
        'Desktop capture (ScreenCapturer)' 'ScreenCapturer' \
        'ScreenCaptureKit (SCStream)' '^_OBJC_CLASS_[$]_SCStream$' \
        'RTCCameraVideoCapturer (AVFoundation camera)' '^_OBJC_CLASS_[$]_RTCCameraVideoCapturer$' \
        'webrtc::test:: (test support code)' '__ZN6webrtc4test' \
        'perfetto' 'perfetto' \
        'protobuf (google::protobuf)' 'google8protobuf' \
        'libjpeg-turbo (chromium_jpeg_)' '^_chromium_jpeg_' \
        'zlib (Cr_z_ prefix)' '^_Cr_z_' \
        'zlib unprefixed (inflate, deflate, crc32)' '^_(inflate|deflate|crc32|adler32)$'

    printf '\n### C++ runtime inside the archive (libc++ in std::__Cr, libc++abi)\n\n'
    md_header "Symbol (awk regex)" "Defined" "Referenced" "Local"
    inspect_rows "$table" \
        '___cxa_throw' '^___cxa_throw$' \
        '___cxa_allocate_exception' '^___cxa_allocate_exception$' \
        '___gxx_personality_v0' '^___gxx_personality_v0$' \
        '___cxa_begin_catch' '^___cxa_begin_catch$' \
        'operator new (_Znwm)' '^__Znwm$' \
        'operator new[] (_Znam)' '^__Znam$' \
        'operator delete (_ZdlPv)' '^__ZdlPv$' \
        'std::terminate' '^__ZSt9terminatev$' \
        '___dynamic_cast' '^___dynamic_cast$' \
        'std::type_info vtable' '^__ZTVSt9type_info$' \
        'std::__Cr:: symbols (any)' '^__ZNSt4__Cr' \
        'std::__Cr::locale' '^__ZNSt4__Cr6locale' \
        'std::__Cr::cout' '^__ZNSt4__Cr4cout' \
        'std::__1:: (the system libc++ namespace)' '^__ZNSt3__1' \
        '__Unwind_ (the unwinder)' '^___?Unwind_'
    printf '\nMembers that define the C++ runtime (any of ___cxa_throw, operator new, ___gxx_personality_v0, ___dynamic_cast, std::terminate, std::__Cr::locale, std::__Cr::cout): `%s`\n' \
        "$(awk -F'\t' '$1 ~ /^[A-TV-Z]$/ && $3 ~ /^(___cxa_throw|__Znwm|___gxx_personality_v0|___dynamic_cast|__ZSt9terminatev|__ZNSt4__Cr6localeC1Ev|__ZNSt4__Cr4cout.*)$/ { m[$2] = 1 } END { for (k in m) print k }' "$table" | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')"

    if [ -d "${dir}/include/third_party/libc++" ]; then
        printf '\n### Headers and the C++ library configuration\n\n'
        printf -- '- `__config_site` anywhere under include/: %s\n' \
            "$(find "${dir}/include" -name '__config_site*' | grep -c . || true)"
        printf -- '- libc++ headers under `third_party/libc++/src/include`: %s `.h` files and **%s** files without an extension. `<string>`: %s, `__config`: %s. libc++ keeps its public headers without an extension and the archive keeps only `.h`, `.hpp` and `.inc`, so C++ built against these headers needs libc++ from somewhere else.\n' \
            "$(find "${dir}/include/third_party/libc++/src/include" -type f -name '*.h' 2>/dev/null | grep -c . || true)" \
            "$(find "${dir}/include/third_party/libc++/src/include" -type f ! -name '*.*' 2>/dev/null | grep -c . || true)" \
            "$([ -f "${dir}/include/third_party/libc++/src/include/string" ] && echo present || echo absent)" \
            "$([ -f "${dir}/include/third_party/libc++/src/include/__config" ] && echo present || echo absent)"
        printf -- '- libc++abi headers (`cxxabi.h`): %s; libunwind headers (`libunwind.h`): %s\n' \
            "$([ -f "${dir}/include/third_party/libc++abi/src/include/cxxabi.h" ] && echo present || echo absent)" \
            "$([ -f "${dir}/include/third_party/libunwind/src/include/libunwind.h" ] && echo present || echo absent)"
        printf -- '- `include/` carries the headers of the whole source tree, including third-party trees that are not linked. Directories under `third_party/` named for H.264, H.265 or FFmpeg sources: %s\n' \
            "$(for name in ffmpeg openh264 x264 libde265; do if [ -d "${dir}/include/third_party/${name}" ]; then printf '%s ' "$name"; fi; done)"
        printf '  (headers only: the absence checks above show that none of their code is in the library).\n'
    fi

    if [ -d "${dir}/Frameworks" ]; then
        printf '\n### Frameworks\n\n'
        find "${dir}/Frameworks" -type f -perm -u+x ! -name '*.sh' | LC_ALL=C sort | while IFS= read -r file; do
            case "$(file -b "$file")" in
                *Mach-O*)
                    printf -- '- `%s`: %s bytes, sha256 `%.12s`, %s; minos %s\n' "${file#"${dir}"/}" "$(size_of "$file")" "$(sha256_of "$file")" \
                        "$(lipo -info "$file" 2>&1 | sed 's/^.*is architecture: //')" "$(vtool -show-build "$file" 2>/dev/null | awk '/minos/ { print $2; exit }')"
                    printf '  - dynamic libraries it loads: %s\n' "$(otool -L "$file" | sed -n '2,$p' | awk '{print $1}' | sed 's#.*/##' | tr '\n' ' ')"
                    printf '  - exported symbols: %s, of them Objective-C classes `RTC*`: %s\n' "$(nm -gU "$file" 2>/dev/null | grep -c . || true)" \
                        "$(nm -gU "$file" 2>/dev/null | grep -c '_OBJC_CLASS_[$]_RTC' || true)"
                    ;;
            esac
        done
        printf -- '- Equal digests above mean the archive stores copies of one binary where a framework has symbolic links.\n'
        printf -- '- `dSYMs` (debug information, not needed to link): %s\n' \
            "$(human $(($(find "${dir}/Frameworks" -name '*.dSYM' -prune -exec du -sk {} + | awk '{ k += $1 } END { print k + 0 }') * 1024)))"
        printf '  The framework is a separate, dynamically linked build; the spike links the static library and not this.\n'
    fi

    if [ "$id" = "libwebrtc-c" ]; then
        printf '\n### The C API\n\n'
        printf -- '- `bindings.rs`: %s bytes, %s `pub fn webrtc_` declarations, %s `pub struct` types, generator line: %s\n' \
            "$(size_of "${dir}/bindings.rs")" "$(grep -c 'pub fn webrtc_' "${dir}/bindings.rs" || true)" "$(grep -c 'pub struct' "${dir}/bindings.rs" || true)" \
            "$(head -n 1 "${dir}/bindings.rs" | tr -d '/*')"
        printf -- '- exported wrapper functions in `%s` (`_webrtc_` prefix): %s\n' "${lib##*/}" "$(sym_count "$table" def '^_webrtc_')"
        printf -- '- headers in `include/webrtc_c`: %s files\n' "$(find "${dir}/include" -type f | grep -c . || true)"
        printf -- '- the archive is self-contained: it holds libwebrtc, Chromium'"'"'s libc++ and libc++abi, and the wrapper; the libwebrtc archive is not needed with it.\n'
    fi

    inspect_c_symbols "$id" "$table" "$out"
}

# inspect_c_symbols <id> <symbol table> <output directory>
inspect_c_symbols() {
    local id="$1" table="$2" out="$3" gst_lib family re
    awk -F'\t' '$1 ~ /^[A-TV-Z]$/ && $3 !~ /^__Z/ && $3 !~ /^_OBJC_|^__OBJC|^_[$]s/ { print $3 }' "$table" | LC_ALL=C sort -u >"${out}/c-symbols.txt"
    sed -E 's/^_//; s/^([A-Za-z0-9]+)_.*/\1/; s/^(.{1,16}).*/\1/' "${out}/c-symbols.txt" | LC_ALL=C sort | uniq -c | LC_ALL=C sort -rn >"${out}/c-symbol-prefixes.txt"
    printf '\n### C symbols (collision candidates for static linking)\n\n'
    printf 'Global defined symbols that are neither C++ (`_Z`) nor Objective-C: **%s** distinct names; by prefix in `%s`, the names in `%s`. The largest families:\n\n' \
        "$(grep -c . "${out}/c-symbols.txt" || true)" "${out}/c-symbol-prefixes.txt" "${out}/c-symbols.txt"
    md_header "Prefix" "Symbols"
    head -n 14 "${out}/c-symbol-prefixes.txt" | while read -r n prefix; do md_row "\`${prefix}\`" "$n"; done
    printf '\nNames that other static libraries in the Mercury link also define (exact, case-sensitive):\n\n'
    md_header "Library" "Pattern" "Defined here"
    while IFS='|' read -r family re; do
        md_row "$family" "\`${re//|/\\|}\`" "$(grep -c -E "$re" "${out}/c-symbols.txt" || true)"
    done <<'EOF'
zlib (libz-sys, system libz)|^_(inflate|deflate|crc32|adler32|compress|uncompress|zlibVersion|gz[a-z]+)$
SQLite (libsqlite3-sys bundled)|^_sqlite3_
aws-lc (aws-lc-sys, prefixed aws_lc_)|^_(aws_lc_|AWS_LC_)
harfbuzz|^_hb_
FreeType|^_FT_
libpng|^_png_
expat|^_XML_
ICU|^_(u_|ucnv_|uprv_)
OpenSSL or BoringSSL names (SSL_, EVP_, X509_)|^_(SSL|EVP|X509)_
libjpeg unprefixed (jpeg_)|^_jpeg_
EOF
    printf '\nRenamed on purpose by Chromium: zlib as `Cr_z_*` (%s names), libjpeg-turbo as `chromium_*` (%s names).\n' \
        "$(grep -c '^_Cr_z_' "${out}/c-symbols.txt" || true)" "$(grep -c '^_chromium_' "${out}/c-symbols.txt" || true)"

    gst_lib=""
    if [ -n "${MERCURY_GSTREAMER_PREFIX:-}" ] && [ -d "${MERCURY_GSTREAMER_PREFIX}/lib" ]; then
        gst_lib="${MERCURY_GSTREAMER_PREFIX}/lib"
    else
        for family in "${HOME}"/Library/Caches/mercury-browser/gstreamer/*/GStreamer.framework/Versions/1.0/lib; do
            [ -d "$family" ] && gst_lib="$family"
        done
    fi
    if [ -n "$gst_lib" ]; then
        local dylib exports="${inspect_dir}/gstreamer-exports.txt"
        if [ ! -f "$exports" ]; then
            for dylib in "$gst_lib"/*.dylib; do
                nm -gU "$dylib" 2>/dev/null | awk 'NF == 3 && $2 ~ /^[A-Z]$/ { print $3 }'
            done | LC_ALL=C sort -u >"$exports"
        fi
        printf '\nThe same names exported by the dynamic libraries of the GStreamer framework in use (%s dylibs, %s exported names): **%s** in common. Static symbols in the executable and the same names inside a dylib do not clash under macOS two-level namespaces, but the executable must not call the framework'"'"'s copy by accident; the largest overlapping families:\n\n' \
            "$(find "$gst_lib" -maxdepth 1 -name '*.dylib' | grep -c . || true)" "$(grep -c . "$exports" || true)" \
            "$(LC_ALL=C comm -12 "${out}/c-symbols.txt" "$exports" | grep -c . || true)"
        md_header "Prefix" "Names in common"
        LC_ALL=C comm -12 "${out}/c-symbols.txt" "$exports" | sed -E 's/^_//; s/^([A-Za-z0-9]+)_.*/\1/; s/^(.{1,16}).*/\1/' |
            LC_ALL=C sort | uniq -c | LC_ALL=C sort -rn | head -n 10 | while read -r n prefix; do md_row "\`${prefix}\`" "$n"; done
    else
        printf '\n(GStreamer is not unpacked here, so the overlap with its dylibs was not computed.)\n'
    fi
    say "inspect: ${id}: raw lists are in ${out}"
}

cmd_inspect() {
    local ids id
    ids="$(ids_from "$@")"
    inspect_dir="${MERCURY_WEBRTC_INSPECT_DIR:-${work_dir}/inspect}"
    mkdir -p "$inspect_dir"
    # A for loop, not `while read`: the tools run inside would read the loop's standard input.
    for id in $ids; do
        check_one "$id" "" >&2
    done
    printf '# libwebrtc prebuilts: static audit\n\n'
    printf 'Host: %s, macOS %s.\n' "$(uname -sm)" "$(sw_vers -productVersion 2>/dev/null || echo unknown)"
    for id in $ids; do
        inspect_one "$id"
    done
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
