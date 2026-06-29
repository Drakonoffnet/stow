#!/usr/bin/env bash
#
# Rasterize assets/icon.svg into assets/AppIcon.icns using macOS-native tools
# (qlmanage for SVG -> PNG, sips for resizing, iconutil for the .icns).
#
# Run after editing assets/icon.svg; bundle_macos.sh then picks up the .icns.

set -euo pipefail
cd "$(dirname "$0")/.."

SVG="assets/icon.svg"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Rendering $SVG to 1024px PNG"
qlmanage -t -s 1024 -o "$WORK" "$SVG" >/dev/null 2>&1
MASTER="$WORK/icon.svg.png"
[[ -f "$MASTER" ]] || { echo "qlmanage failed to render the SVG"; exit 1; }
# Normalize to an exact 1024x1024 canvas.
sips -z 1024 1024 "$MASTER" --out "$WORK/master.png" >/dev/null
MASTER="$WORK/master.png"

echo "==> Building iconset"
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  s2=$((size * 2))
  sips -z $size  $size  "$MASTER" --out "$SET/icon_${size}x${size}.png"      >/dev/null
  sips -z $s2    $s2    "$MASTER" --out "$SET/icon_${size}x${size}@2x.png"   >/dev/null
done

echo "==> Packing AppIcon.icns"
iconutil -c icns "$SET" -o "assets/AppIcon.icns"
echo "==> Done: assets/AppIcon.icns"
