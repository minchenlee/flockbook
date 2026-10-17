#!/usr/bin/env bash
# Build release and wrap it into build/Flockbook.app.
# Signing: ad-hoc by default. Set SIGN_ID to a Developer ID identity for a
# hardened-runtime release build, e.g.
#   SIGN_ID="Developer ID Application: <Your Name> (<TEAMID>)" scripts/bundle.sh
# Then notarize with scripts/notarize.sh.
# Optional env: VERSION (CFBundleShortVersionString, default 0.1.0), BUILD_NUMBER (default 1),
# ARCHS (e.g. "arm64 x86_64" for a universal binary; default native only), SCRATCH_PATH (swift --scratch-path),
# APP (output bundle, default build/Flockbook.app), BUNDLE_ID (default dev.mclee.flockbook; test bundles use their own,
# so they never touch the installed app's settings or caches).
# Auto-update (Sparkle): SPARKLE_PUBLIC_KEY is the EdDSA public key (base64) that update archives are signed with;
# without it the app never checks for updates. FEED_URL overrides the appcast URL.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${VERSION:-0.1.0}"
BUILD_NUMBER="${BUILD_NUMBER:-1}"
SP=(); [ -n "${SCRATCH_PATH:-}" ] && SP=(--scratch-path "$SCRATCH_PATH")
mkdir -p build
if [ -z "${ARCHS:-}" ]; then
  swift build -c release --product HerdrDesk ${SP[@]+"${SP[@]}"}
  swift build -c release --product flockbook-notify ${SP[@]+"${SP[@]}"}
  BIN="$(swift build -c release --show-bin-path ${SP[@]+"${SP[@]}"})"
  EXE="$BIN/HerdrDesk"
  NOTIFY="$BIN/flockbook-notify"
else
  EXES=(); NOTIFYS=()
  for a in $ARCHS; do
    swift build -c release --product HerdrDesk --arch "$a" ${SP[@]+"${SP[@]}"}
    swift build -c release --product flockbook-notify --arch "$a" ${SP[@]+"${SP[@]}"}
    BIN="$(swift build -c release --arch "$a" --show-bin-path ${SP[@]+"${SP[@]}"})"
    EXES+=("$BIN/HerdrDesk"); NOTIFYS+=("$BIN/flockbook-notify")
  done
  EXE="build/HerdrDesk-universal"
  lipo -create "${EXES[@]}" -output "$EXE"
  lipo -info "$EXE"
  NOTIFY="build/flockbook-notify-universal"
  lipo -create "${NOTIFYS[@]}" -output "$NOTIFY"
  lipo -info "$NOTIFY"
fi

APP="${APP:-build/Flockbook.app}"
BUNDLE_ID="${BUNDLE_ID:-dev.mclee.flockbook}"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$EXE" "$APP/Contents/MacOS/HerdrDesk"
# The agents' command (Settings > Remote access shows its path; nothing changes the owner's PATH).
cp "$NOTIFY" "$APP/Contents/MacOS/flockbook-notify"
# Sparkle.framework (universal), found through the rpath @executable_path/../Frameworks (Package.swift).
ditto "$BIN/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"

# Copy SwiftPM resource bundles next to the executable's resources, if any.
for b in "$BIN"/*.bundle; do [ -e "$b" ] && cp -R "$b" "$APP/Contents/Resources/"; done

# App icon: only when the designer's 1024px master exists.
ICON_SRC="docs/design/icon/AppIcon-1024.png"
ICON_KEY=""
if [ -f "$ICON_SRC" ]; then
  SET="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$SET"
  for sz in 16 32 128 256 512; do
    sips -z $sz $sz "$ICON_SRC" --out "$SET/icon_${sz}x${sz}.png" >/dev/null
    sips -z $((sz*2)) $((sz*2)) "$ICON_SRC" --out "$SET/icon_${sz}x${sz}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"
  ICON_KEY="  <key>CFBundleIconFile</key><string>AppIcon</string>"
  echo "icon: AppIcon.icns from $ICON_SRC"
else
  echo "icon: $ICON_SRC not found, skipped"
fi

# Light/dark app icon: compile the Icon Composer document into Assets.car.
# actool ships only with Xcode (CI has it); without it the .icns above is the only icon.
ICON_DOC="docs/design/icon/AppIcon.icon"
if [ -d "$ICON_DOC" ] && xcrun --find actool >/dev/null 2>&1; then
  xcrun actool "$ICON_DOC" --compile "$APP/Contents/Resources" --app-icon AppIcon \
    --platform macosx --target-device mac --minimum-deployment-target 15.0 \
    --output-partial-info-plist "$(mktemp -d)/icon.plist" --output-format human-readable-text --errors --warnings
  ICON_KEY="$ICON_KEY
  <key>CFBundleIconName</key><string>AppIcon</string>"
  echo "icon: Assets.car from $ICON_DOC; appearances:"
  xcrun assetutil --info "$APP/Contents/Resources/Assets.car" | grep -o '"Appearance" : "[^"]*"' | sort | uniq -c
else
  echo "icon: actool not available (needs Xcode), no light/dark Assets.car"
fi

FEED_URL="${FEED_URL:-https://flockbook.mclee.dev/appcast.xml}"
UPDATE_KEYS="  <key>SUFeedURL</key><string>$FEED_URL</string>"
if [ -n "${SPARKLE_PUBLIC_KEY:-}" ]; then
  # Daily check, no first-launch permission prompt; the user still confirms every install.
  # The feed and the archive must carry EdDSA signatures from the matching private key.
  UPDATE_KEYS="$UPDATE_KEYS
  <key>SUPublicEDKey</key><string>$SPARKLE_PUBLIC_KEY</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUScheduledCheckInterval</key><integer>86400</integer>
  <key>SURequireSignedFeed</key><true/>
  <key>SUVerifyUpdateBeforeExtraction</key><true/>"
  echo "updates: on, feed $FEED_URL"
else
  echo "updates: off (no SPARKLE_PUBLIC_KEY)"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>Flockbook</string>
  <key>CFBundleDisplayName</key><string>Flockbook</string>
  <key>NSHumanReadableCopyright</key><string>Unofficial companion for Herdr. Not affiliated with, endorsed by, or supported by Herdr, Inc.</string>
  <key>CFBundleExecutable</key><string>HerdrDesk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
$ICON_KEY
$UPDATE_KEYS
</dict>
</plist>
PLIST

# Sign inside-out (not --deep): Sparkle's helpers first, then the framework, then the app.
if [ -n "${SIGN_ID:-}" ]; then
  SIGN=(-s "$SIGN_ID" --options runtime --timestamp)
else
  SIGN=(-s -)
fi
FW="$APP/Contents/Frameworks/Sparkle.framework/Versions/B"
codesign --force "${SIGN[@]}" "$FW/XPCServices/Installer.xpc"
codesign --force "${SIGN[@]}" --preserve-metadata=entitlements "$FW/XPCServices/Downloader.xpc"
codesign --force "${SIGN[@]}" "$FW/Autoupdate"
codesign --force "${SIGN[@]}" "$FW/Updater.app"
codesign --force "${SIGN[@]}" "$APP/Contents/Frameworks/Sparkle.framework"
codesign --force "${SIGN[@]}" --identifier "$BUNDLE_ID.notify" "$APP/Contents/MacOS/flockbook-notify"
codesign --force "${SIGN[@]}" "$APP"
echo "signed: ${SIGN_ID:-ad-hoc}${SIGN_ID:+ (hardened runtime)}"
codesign --verify --deep --strict "$APP"
codesign --verify --strict "$APP/Contents/MacOS/flockbook-notify"
echo "built: $APP"
