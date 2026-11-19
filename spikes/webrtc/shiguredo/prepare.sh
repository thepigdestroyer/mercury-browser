#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Creates vendor/shiguredo_webrtc: the crate `shiguredo_webrtc` 0.157.0 from the cargo registry plus
# patches/shiguredo_webrtc-0.157.0.patch (sp-rtc spike, route S). Third-party source is not committed;
# vendor/ is git-ignored and Cargo.toml's [patch.crates-io] points at it, so a forgotten prepare.sh
# stops cargo before the unpatched build script (which downloads libwebrtc_c with curl) can run.
#
#   ./prepare.sh           fetch (cargo), verify the .crate checksum, unpack, drop *.py, patch
#   ./prepare.sh --check   exit 0 when vendor/ was made from this patch, 1 when it must be redone
#
# The Python files in the crate package (webrtc/run.py, canary.py) are never run and are deleted
# from vendor/ so that no build feature can reach them.
set -eu

VERSION="0.157.0"
# sha256 of shiguredo_webrtc-0.157.0.crate: the "cksum" of crates.io's sparse index entry
# (https://index.crates.io/sh/ig/shiguredo_webrtc), read on 19 Nov 2026.
CRATE_SHA256="ba235b1bb2a1239cc5efb99d52670b1713629ad80731c804f5578306df90f9e5"

here="$(cd "$(dirname "$0")" && pwd)"
vendor="${here}/vendor/shiguredo_webrtc"
patch_file="${here}/patches/shiguredo_webrtc-${VERSION}.patch"
marker="${vendor}/.mercury-prepared"

say() { printf '%s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        sha256sum "$1" | cut -d ' ' -f 1
    fi
}

state="$(sha256_of "$patch_file"):${CRATE_SHA256}"

if [ "${1:-}" = "--check" ]; then
    [ -f "$marker" ] && [ "$(cat "$marker")" = "$state" ]
    exit $?
fi
[ "$#" -eq 0 ] || die "usage: ./prepare.sh [--check]"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/mercury-shiguredo-prepare.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT INT TERM HUP

# 1. Let cargo download the .crate (registry index and static.crates.io, like any dependency). The
#    scratch project is a separate manifest, so our [patch] is not in effect, and
#    nothing is built: `cargo fetch` runs no build script.
mkdir -p "${scratch}/fetch/src"
cat >"${scratch}/fetch/Cargo.toml" <<MANIFEST
[package]
name = "fetch"
version = "0.0.0"
edition = "2024"
[dependencies]
shiguredo_webrtc = "=${VERSION}"
MANIFEST
: >"${scratch}/fetch/src/lib.rs"
# Run from here so that rustup finds the repository's rust-toolchain.toml; --manifest-path names the scratch project.
(cd "$here" && cargo fetch --quiet --manifest-path "${scratch}/fetch/Cargo.toml")

# 2. Find the downloaded file and check it against the pinned checksum (a registry mirror or a
#    changed index cannot slip another crate in).
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
crate_file=""
for candidate in "${cargo_home}"/registry/cache/*/shiguredo_webrtc-"${VERSION}".crate; do
    [ -f "$candidate" ] || continue
    if [ "$(sha256_of "$candidate")" = "$CRATE_SHA256" ]; then
        crate_file="$candidate"
        break
    fi
done
[ -n "$crate_file" ] || die "no shiguredo_webrtc-${VERSION}.crate with sha256 ${CRATE_SHA256} in ${cargo_home}/registry/cache"
say "ok: ${crate_file} (sha256 verified)"

# 3. Unpack, remove every Python file, apply the patch.
rm -rf "${here}/vendor"
mkdir -p "${here}/vendor"
tar -xzf "$crate_file" -C "${scratch}"
mv "${scratch}/shiguredo_webrtc-${VERSION}" "$vendor"
find "$vendor" -name '*.py' -delete
patch -p1 -s -d "$vendor" -i "$patch_file" || die "patches/shiguredo_webrtc-${VERSION}.patch does not apply"
find "$vendor" -name '*.orig' -delete
printf '%s\n' "$state" >"$marker"
say "ok: vendor/shiguredo_webrtc is shiguredo_webrtc ${VERSION} plus $(basename "$patch_file")"
