#!/usr/bin/env bash
# Builds Drift.app and a drag-to-Applications .dmg for macOS, using only the
# tools that ship with macOS and Xcode's command line tools.
#
#   scripts/package-macos.sh               app + dmg in target/release/macos/
#   scripts/package-macos.sh --no-dmg      app only
#   scripts/package-macos.sh --universal   one app for Apple silicon and Intel Macs
#                                          (first: rustup target add x86_64-apple-darwin aarch64-apple-darwin)
#   scripts/package-macos.sh --no-build    package the existing release binary
#
# The app is signed ad hoc, which is enough to run it on this Mac. To give it to
# other people, sign it with a Developer ID and notarize it instead.
set -euo pipefail

cd "$(dirname "$0")/.."
[[ "$(uname)" == Darwin ]] || { echo "This script only runs on macOS." >&2; exit 1; }

NAME=Drift
BIN=drift
ID=io.github.rk-kabilan.drift
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
OUT=target/release/macos
APP="$OUT/$NAME.app"

DMG_WANTED=1 UNIVERSAL=0 BUILD=1
for arg in "$@"; do
  case "$arg" in
    --no-dmg) DMG_WANTED=0 ;;
    --universal) UNIVERSAL=1 ;;
    --no-build) BUILD=0 ;;
    *) echo "Unknown option: $arg" >&2; exit 1 ;;
  esac
done

EXE="target/release/$BIN"
if [[ $UNIVERSAL == 1 ]]; then
  if [[ $BUILD == 1 ]]; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
  fi
  mkdir -p "$OUT"
  EXE="$OUT/$BIN-universal"
  lipo -create -output "$EXE" \
    "target/aarch64-apple-darwin/release/$BIN" "target/x86_64-apple-darwin/release/$BIN"
elif [[ $BUILD == 1 ]]; then
  cargo build --release
fi
[[ -x "$EXE" ]] || { echo "$EXE is missing; build it first." >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$EXE" "$APP/Contents/MacOS/$BIN"
if [[ $UNIVERSAL == 1 ]]; then rm -f "$EXE"; fi
cp assets/app-icon/macos/drift.icns "$APP/Contents/Resources/$BIN.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleExecutable</key><string>$BIN</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleIconFile</key><string>$BIN</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP"
echo "Built $APP"

if [[ $DMG_WANTED == 0 ]]; then
  exit 0
fi

DMG="$OUT/$NAME-$VERSION.dmg"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
echo "Built $DMG"
