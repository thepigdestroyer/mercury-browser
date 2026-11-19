#!/bin/sh
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# t12 of the sp-rtc spike, route S: the patched build script of shiguredo_webrtc refuses to run
# unless MERCURY_WEBRTC_C_DIR names a verified directory, and never calls curl or cmake.
#
# It runs the build script's own executable (a host program, so this works on an Intel Mac too) in
# nine scenarios with a fake `curl` and a fake `cmake` first on PATH, and prints one line
#   RESULT t12 pass|fail <detail>
# Build the crate once first (cargo build --tests ...): that leaves the executable under target/.
set -u

here="$(cd "$(dirname "$0")" && pwd)"
vendor="${here}/vendor/shiguredo_webrtc"

fail() {
    printf 'RESULT t12 fail %s\n' "$*"
    exit 1
}

# Older cargo: build/shiguredo_webrtc-<hash>/build-script-build; cargo 1.100: build/shiguredo_webrtc/<hash>/out/build_script_build.
exe="$(find "${here}/target" \( -path '*/build/shiguredo_webrtc-*/build-script-build' -o -path '*/build/shiguredo_webrtc/*/out/build_script_build' \) -type f 2>/dev/null | head -n 1)"
[ -n "$exe" ] || fail "no build script executable under target/; build the crate first"
[ -d "$vendor" ] || fail "no vendor/shiguredo_webrtc; run ./prepare.sh"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/mercury-t12.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT INT TERM HUP
mkdir -p "${tmp}/bin" "${tmp}/out"
for tool in curl cmake; do
    cat >"${tmp}/bin/${tool}" <<FAKE
#!/bin/sh
echo "${tool} \$*" >>"${tmp}/tools-called"
exit 1
FAKE
    chmod +x "${tmp}/bin/${tool}"
done

hash64="$(printf '%064d' 0 | tr 0 a)"
verified="${tmp}/verified"
mkdir -p "${verified}/lib"
: >"${verified}/lib/libwebrtc_c.a"
: >"${verified}/bindings.rs"
printf '%s\n' "$hash64" >"${verified}/.mercury-webrtc-verified"

no_marker="${tmp}/no-marker"
mkdir -p "${no_marker}/lib"
: >"${no_marker}/lib/libwebrtc_c.a"
: >"${no_marker}/bindings.rs"

short_marker="${tmp}/short-marker"
mkdir -p "${short_marker}/lib"
: >"${short_marker}/lib/libwebrtc_c.a"
: >"${short_marker}/bindings.rs"
printf 'deadbeef\n' >"${short_marker}/.mercury-webrtc-verified"

no_lib="${tmp}/no-lib"
mkdir -p "$no_lib"
: >"${no_lib}/bindings.rs"
printf '%s\n' "$hash64" >"${no_lib}/.mercury-webrtc-verified"

# run_case <name> <expected: refuse|ok> <message the refusal must contain> [VAR=value ...]
cases=0
run_case() {
    name="$1"
    expected="$2"
    message="$3"
    shift 3
    rm -f "${tmp}/out/bindings.rs"
    (
        cd "$vendor" &&
            env -i PATH="${tmp}/bin:/usr/bin:/bin" HOME="$tmp" \
                CARGO_MANIFEST_DIR="$vendor" CARGO_PKG_VERSION=0.157.0 OUT_DIR="${tmp}/out" \
                TARGET=aarch64-apple-darwin HOST=x86_64-apple-darwin \
                CARGO_CFG_TARGET_OS=macos CARGO_CFG_TARGET_ARCH=aarch64 \
                "$@" "$exe"
    ) >"${tmp}/stdout" 2>"${tmp}/stderr"
    status=$?
    cases=$((cases + 1))
    if [ "$expected" = refuse ]; then
        [ "$status" -ne 0 ] || fail "${name}: the build script succeeded, it must refuse"
        grep -q "$message" "${tmp}/stderr" || fail "${name}: refused without the message '${message}': $(head -c 300 "${tmp}/stderr")"
        grep -q 'cargo:rustc-link-lib=static=webrtc_c' "${tmp}/stdout" && fail "${name}: it printed link directives while refusing"
    else
        [ "$status" -eq 0 ] || fail "${name}: the build script failed with $status: $(head -c 300 "${tmp}/stderr")"
        grep -q 'cargo:rustc-link-lib=static=webrtc_c' "${tmp}/stdout" || fail "${name}: no link directive"
        grep -q "cargo:rustc-link-search=native=${verified}/lib" "${tmp}/stdout" || fail "${name}: it does not link from the pinned directory"
        [ -f "${tmp}/out/bindings.rs" ] || fail "${name}: bindings.rs was not copied"
    fi
}

run_case "dir not set" refuse "MERCURY_WEBRTC_C_DIR is not set"
run_case "dir missing" refuse "not a verified directory" MERCURY_WEBRTC_C_DIR="${tmp}/does-not-exist"
run_case "no marker" refuse "not a verified directory" MERCURY_WEBRTC_C_DIR="$no_marker"
run_case "marker is not a sha256" refuse "not a verified directory" MERCURY_WEBRTC_C_DIR="$short_marker"
run_case "no library" refuse "has no lib/libwebrtc_c.a" MERCURY_WEBRTC_C_DIR="$no_lib"
run_case "x86_64 target" refuse "has no pinned prebuilt" MERCURY_WEBRTC_C_DIR="$verified" CARGO_CFG_TARGET_ARCH=x86_64
run_case "source-build feature" refuse "build from source" MERCURY_WEBRTC_C_DIR="$verified" CARGO_FEATURE_SOURCE_BUILD=1
run_case "WEBRTC_BUILD_ROOT" refuse "build from source" MERCURY_WEBRTC_C_DIR="$verified" WEBRTC_BUILD_ROOT="${tmp}/root"
run_case "verified directory" ok "" MERCURY_WEBRTC_C_DIR="$verified"

if [ -e "${tmp}/tools-called" ]; then
    fail "the build script called a fake tool: $(cat "${tmp}/tools-called")"
fi
printf 'RESULT t12 pass %s scenarios of the patched build script: 8 refused (unset, missing, no marker, marker not a sha256, no library, x86_64, source-build, WEBRTC_BUILD_ROOT), the verified directory linked from itself; fake curl and cmake never called\n' "$cases"
