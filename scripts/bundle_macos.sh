#!/usr/bin/env bash
#
# Build a macOS .app bundle for file_transfer.
#
#   ./scripts/bundle_macos.sh              # host architecture only (fast)
#   ./scripts/bundle_macos.sh --universal  # arm64 + x86_64 fat binary
#
# Output: dist/file_transfer.app (ad-hoc signed so Gatekeeper runs it locally).
# Distributing to other machines additionally requires a Developer ID
# signature and notarization.

set -euo pipefail

APP_NAME="Stow"
BIN_NAME="file_transfer"
BUNDLE_ID="com.stow.app"
VERSION="0.1.0"

cd "$(dirname "$0")/.."

DIST="dist"
APP_DIR="$DIST/$APP_NAME.app"
UNIVERSAL=0
[[ "${1:-}" == "--universal" ]] && UNIVERSAL=1

mkdir -p "$DIST"

if [[ $UNIVERSAL -eq 1 ]]; then
  echo "==> Building universal (arm64 + x86_64)"
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  BIN="$DIST/$BIN_NAME"
  lipo -create -output "$BIN" \
    "target/aarch64-apple-darwin/release/$BIN_NAME" \
    "target/x86_64-apple-darwin/release/$BIN_NAME"
else
  echo "==> Building for host architecture ($(uname -m))"
  cargo build --release
  BIN="target/release/$BIN_NAME"
fi

echo "==> Assembling $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN" "$APP_DIR/Contents/MacOS/$BIN_NAME"
chmod +x "$APP_DIR/Contents/MacOS/$BIN_NAME"

ICON_KEYS=""
if [[ -f "assets/AppIcon.icns" ]]; then
  cp "assets/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
  ICON_KEYS="
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>"
fi

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleExecutable</key>
	<string>$BIN_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>$ICON_KEYS
</dict>
</plist>
PLIST

echo "==> Ad-hoc signing"
codesign --force --deep --sign - "$APP_DIR" >/dev/null 2>&1 || \
  echo "    (codesign skipped or failed; app still runs locally)"

echo "==> Done: $APP_DIR"
lipo -archs "$APP_DIR/Contents/MacOS/$BIN_NAME" 2>/dev/null | sed 's/^/    archs: /' || true
