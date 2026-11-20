#!/bin/bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
# Put the PDFium library and its licence texts into an application bundle.
#
#   scripts/macos/collect-pdfium.sh <pdfium prefix> <destination lib dir> <resources dir>
#
# <pdfium prefix> is a directory unpacked by scripts/macos/pdfium.sh
# (~/Library/Caches/mercury-browser/pdfium/<release>/<arch>). The library goes to
# <destination lib dir>/libpdfium.dylib, which is Contents/MacOS/lib in a bundle: the
# directory the GStreamer libraries are in, already on the executable's rpath and
# signed inside out by bundle.sh. Its licence texts go to <resources dir>/pdfium-licenses/
# (Contents/Resources in a bundle): the LICENSE of the build and one file per component
# PDFium ships, flat, as the tarball names them.
#
# Before anything is written, the prefix is verified against the pins in pdfium.sh (the
# release, the build without V8 and XFA, the SHA-256 of the library, its architecture and
# install name, the licence files); a prefix that fails is refused and nothing changes.
# The library is copied unmodified. Its install name is already @rpath/libpdfium.dylib;
# if a future pin carried another one it would be set here with install_name_tool.
#
# Inside an application bundle (a destination ending in .app/Contents/MacOS/lib) the
# library must be arm64 only (the application is) and neither the library nor
# pdfium-licenses/ may exist yet: a bundle is collected once, into a fresh bundle. Anywhere
# else any pinned architecture is accepted and a library of ours is replaced.
#
# A build tree's lib directory (target/<profile>/lib), which scripts/macos/collect-gstreamer.sh
# owns and compares with a digest, is refused: the executable finds the library in the cache
# there (mercury_pdfium::locate_library). Nothing here runs the library.
set -euo pipefail
set -f

# shellcheck source=scripts/macos/pdfium.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/pdfium.sh"

collect_die() {
    printf 'collect-pdfium: error: %s\n' "$*" >&2
    exit 1
}
collect_say() {
    printf 'collect-pdfium: %s\n' "$*" >&2
}
# The pin functions' own messages: errors are ours, the "ok" lines stay quiet.
die() { collect_die "$@"; }
say() { :; }

collect_main() {
    [ "$#" -eq 3 ] || collect_die "usage: $0 <pdfium prefix> <destination lib dir> <resources dir>"
    local source_prefix="$1" lib_dir="${2%/}" resources_dir="${3%/}"
    [ -n "$lib_dir" ] && [ -n "$resources_dir" ] || collect_die "the destinations must not be empty"
    [ -d "$source_prefix" ] || collect_die "${source_prefix} is not a directory; run: make pdfium-macos"
    source_prefix="$(cd -- "$source_prefix" && pwd -P)"
    [ -f "${source_prefix}/lib/libpdfium.dylib" ] || collect_die "${source_prefix} is not an unpacked PDFium (no lib/libpdfium.dylib); run: make pdfium-macos"

    # Which build is it? By the library's own architectures; the pins then decide.
    local archs arch
    archs="$(lipo -archs "${source_prefix}/lib/libpdfium.dylib")" || collect_die "lipo cannot read ${source_prefix}/lib/libpdfium.dylib"
    case "$archs" in
        arm64) arch=arm64 ;;
        x86_64) arch=x64 ;;
        *) arch=univ ;;
    esac
    select_arch "$arch"
    # Every check of the pinned build, before the first write.
    verify_unpacked "$source_prefix"

    local bundle_mode=0
    case "$lib_dir" in
        *.app/Contents/MacOS/lib) bundle_mode=1 ;;
    esac
    if [ "$bundle_mode" = 1 ]; then
        [ "$archs" = "arm64" ] || collect_die "the PDFium library is built for '${archs}'; the application is arm64 only (fetch it with: scripts/macos/pdfium.sh --arch arm64 fetch unpack)"
        [ -d "$lib_dir" ] || collect_die "${lib_dir} does not exist; the media libraries are collected into it first"
        [ ! -e "${lib_dir}/libpdfium.dylib" ] || collect_die "${lib_dir}/libpdfium.dylib already exists; an application bundle's lib directory is collected once, into a fresh bundle"
        [ ! -e "${resources_dir}/pdfium-licenses" ] || collect_die "${resources_dir}/pdfium-licenses already exists; collect into a fresh bundle"
    else
        if [ -d "$lib_dir" ] && [ -n "$(find "${lib_dir}/" -maxdepth 1 -name '.gstreamer-*' -type f)" ]; then
            collect_die "${lib_dir} is a collected GStreamer directory (it has a .gstreamer-* marker); collect-gstreamer.sh would rebuild it and delete the library. The executable finds PDFium in the cache: ${release_dir}"
        fi
        if [ -e "${resources_dir}/pdfium-licenses" ] && [ ! -f "${resources_dir}/pdfium-licenses/.mercury-pdfium" ]; then
            collect_die "${resources_dir}/pdfium-licenses exists but was not made by this script (no .mercury-pdfium marker); refusing to replace it"
        fi
    fi

    # Staging files beside the destinations, so a failure leaves nothing that looks finished.
    mkdir -p -- "$lib_dir" "$resources_dir"
    STAGED_LIBRARY="${lib_dir}/.libpdfium.dylib.partial.$$"
    STAGED_LICENSES="${resources_dir}/.pdfium-licenses.partial.$$"
    trap 'rm -rf -- "$STAGED_LIBRARY" "$STAGED_LICENSES"' EXIT
    cp -- "${source_prefix}/lib/libpdfium.dylib" "$STAGED_LIBRARY"
    chmod 755 "$STAGED_LIBRARY"
    local install_name
    install_name="$(otool -D "$STAGED_LIBRARY" | sed -n '2p')"
    if [ "$install_name" != "$PDFIUM_INSTALL_NAME" ]; then
        install_name_tool -id "$PDFIUM_INSTALL_NAME" "$STAGED_LIBRARY" || collect_die "install_name_tool failed on the copy of the library"
    else
        [ "$(sha256_of "$STAGED_LIBRARY")" = "$LIBRARY_SHA256" ] || collect_die "the copy of the library differs from the pinned one"
    fi
    [ "$(otool -D "$STAGED_LIBRARY" | sed -n '2p')" = "$PDFIUM_INSTALL_NAME" ] || collect_die "the install name of the copy is not ${PDFIUM_INSTALL_NAME}"

    mkdir -p -- "$STAGED_LICENSES"
    cp -- "${source_prefix}/LICENSE" "${STAGED_LICENSES}/LICENSE"
    local name
    for name in $PDFIUM_LICENSE_FILES; do
        cp -- "${source_prefix}/licenses/${name}" "${STAGED_LICENSES}/${name}"
    done
    chmod 755 "$STAGED_LICENSES"
    if [ "$bundle_mode" = 0 ]; then
        printf '%s\n' "$PDFIUM_VERSION" >"${STAGED_LICENSES}/.mercury-pdfium"
        rm -rf -- "${resources_dir}/pdfium-licenses"
    fi

    mv -f -- "$STAGED_LIBRARY" "${lib_dir}/libpdfium.dylib"
    mv -- "$STAGED_LICENSES" "${resources_dir}/pdfium-licenses"
    trap - EXIT
    collect_say "ok: PDFium ${PDFIUM_VERSION} (${archs}) in ${lib_dir}/libpdfium.dylib, its licences in ${resources_dir}/pdfium-licenses"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    collect_main "$@"
fi
