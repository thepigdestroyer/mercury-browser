#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Adapted from Zervo, https://github.com/Goddv/Zervo, commit 86cb7126,
# scripts/make-icns.sh, MPL-2.0. Changed: the iconset is rendered from the
# project's SVG app icon by the workspace tool mercury-iconset (resvg) instead
# of scaled from a committed PNG, and it is kept in the repository.
#
# Regenerate the application icon from its SVG masters:
#
#   scripts/macos/make-icns.sh
#
# Reads apps/mercury-desktop/resources/macos/AppIcon.svg (and AppIcon-small.svg
# for the 16 and 32 px sizes) and writes AppIcon.iconset/ (ten PNGs) and
# AppIcon.icns next to them. Both are committed, so packaging never needs to
# run this; run it again only after the icon changes. Needs cargo and macOS's
# iconutil and sips; nothing else.
set -euo pipefail

cd "$(dirname -- "${BASH_SOURCE[0]}")/../.."

die() {
    printf 'make-icns: %s\n' "$*" >&2
    exit 1
}

for tool in cargo iconutil sips; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool not found"
done

RESOURCES="apps/mercury-desktop/resources/macos"
SOURCE="$RESOURCES/AppIcon.svg"
SMALL="$RESOURCES/AppIcon-small.svg"
ICONSET="$RESOURCES/AppIcon.iconset"
OUTPUT="$RESOURCES/AppIcon.icns"

[ -f "$SOURCE" ] || die "missing $SOURCE"
[ -f "$SMALL" ] || die "missing $SMALL"

rm -rf -- "$ICONSET"

# Each size is drawn from the vector artwork, not scaled down from the largest
# one; mercury-iconset writes the file names iconutil expects.
cargo run --release --quiet -p mercury-iconset -- "$SOURCE" "$SMALL" "$ICONSET"

# A wrong size would still make a valid .icns, so check every file.
for spec in "16:icon_16x16" "32:icon_16x16@2x" "32:icon_32x32" "64:icon_32x32@2x" \
            "128:icon_128x128" "256:icon_128x128@2x" "256:icon_256x256" \
            "512:icon_256x256@2x" "512:icon_512x512" "1024:icon_512x512@2x"; do
    size="${spec%%:*}"
    name="${spec#*:}"
    [ -f "$ICONSET/$name.png" ] || die "$name.png was not written"
    width="$(sips -g pixelWidth "$ICONSET/$name.png" | awk '/pixelWidth/ { print $2 }')"
    height="$(sips -g pixelHeight "$ICONSET/$name.png" | awk '/pixelHeight/ { print $2 }')"
    [ "$width" = "$size" ] && [ "$height" = "$size" ] ||
        die "$name.png is ${width}x${height}, expected ${size}x${size}"
done

iconutil -c icns "$ICONSET" -o "$OUTPUT"
printf 'make-icns: wrote %s and %s\n' "$ICONSET" "$OUTPUT"
