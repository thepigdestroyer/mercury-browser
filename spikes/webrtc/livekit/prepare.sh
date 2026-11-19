#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Makes vendor/webrtc-sys and vendor/libwebrtc: the crates.io packages webrtc-sys 0.3.54 and
# libwebrtc 0.4.1 (LiveKit, Apache-2.0), each checked against its pinned SHA-256, with
# patches/<crate>-<version>.patch applied and overlay/<crate>/ copied in. Third-party source is never
# committed (spike decision D7); vendor/ is git-ignored and Cargo.toml's [patch.crates-io] points at
# it, so a forgotten run of this script makes cargo stop instead of building the unpatched crates
# (whose build script would download LiveKit's prebuilt).
# Run from anywhere; needs cargo, patch and shasum. Safe to run again (vendor/ is rebuilt).
set -eu

# crate version sha256 (the .crate file as crates.io lists it)
CRATES="
webrtc-sys 0.3.54 7b35efe157521adc1ccc875e0687d5434c422758f44b1306be961447d7e5660c
libwebrtc 0.4.1 814e93897c8689013dac8e4babc08957fdc04823748903f4be2409d5d3b8d9a2
"

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"
scratch="$here/target/prepare"

say() { printf '%s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }

# A throwaway manifest that asks only for the pinned crates, so that cargo fetches them into the
# registry (and checks them against the index) without needing vendor/.
rm -rf -- "$scratch"
mkdir -p "$scratch/src"
{
    printf '[package]\nname = "prepare"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n[dependencies]\n'
    printf '%s\n' "$CRATES" | while read -r name version _sum; do
        if [ -n "$name" ]; then printf '%s = "=%s"\n' "$name" "$version"; fi
    done
} > "$scratch/Cargo.toml"
: > "$scratch/src/lib.rs"
(cd "$scratch" && cargo fetch --quiet)

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rm -rf -- "$here/vendor"
mkdir -p "$here/vendor"

printf '%s\n' "$CRATES" | while read -r name version sum; do
    [ -n "$name" ] || continue
    lock_sum="$(awk -v n="$name" '$1 == "name" && $3 == "\"" n "\"" { found = 1 } found && $1 == "checksum" { gsub("\"", "", $3); print $3; exit }' "$scratch/Cargo.lock")"
    [ "$lock_sum" = "$sum" ] || die "${name} ${version}: the registry says sha256 '${lock_sum}', this script pins '${sum}'; nothing was copied"

    src=""
    for candidate in "$cargo_home"/registry/src/*/"${name}-${version}"; do
        [ -d "$candidate" ] && src="$candidate"
    done
    [ -n "$src" ] || die "${name}-${version} is not in ${cargo_home}/registry/src after cargo fetch"

    cp -R "$src" "$here/vendor/${name}"
    rm -f "$here/vendor/${name}/.cargo-ok"
    chmod -R u+w "$here/vendor/${name}"
    patch -p1 -d "$here/vendor/${name}" --quiet < "$here/patches/${name}-${version}.patch"
    if [ -d "$here/overlay/${name}" ]; then
        cp -R "$here/overlay/${name}/." "$here/vendor/${name}/"
    fi
    say "ok: vendor/${name} is ${name} ${version} (sha256 ${sum}) with the Mercury patch and overlay"
done
rm -rf -- "$scratch"

# The libc++ headers the glue is compiled against (fetch-libcxx.sh verifies them against pinned hashes);
# build.rs reads the directory from vendor/webrtc-sys/mercury-libcxx-dir unless MERCURY_LIBCXX_DIR is set.
"$here/fetch-libcxx.sh" fetch
"$here/fetch-libcxx.sh" env | sed 's/^export MERCURY_LIBCXX_DIR=//' > "$here/vendor/webrtc-sys/mercury-libcxx-dir"
say "ok: libc++ headers in $(cat "$here/vendor/webrtc-sys/mercury-libcxx-dir")"

# Code for other platforms that the macOS build never compiles, and the NVIDIA Video Codec SDK sample
# code (its licence is NVIDIA's EULA, not a permissive one), are not kept in the patched copy.
rm -rf -- "$here/vendor/webrtc-sys/src/nvidia"
