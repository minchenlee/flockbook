#!/usr/bin/env bash
# dist/Flockbook-herdr-mod-<VERSION>.zip: the Herdr app layout, mod, themes, keys and install script
# from mod/, plus a universal flockbook-modfeed (the mod's helper) in bin/.
#   VERSION=0.7.0 scripts/package-mod.sh
# Signing: ad-hoc by default. With SIGN_ID set: Developer ID, hardened runtime, timestamp.
# Notarization: with NOTARIZE=1 and APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID, the zip goes to Apple's
# notary service. A bare executable cannot be stapled; Gatekeeper checks the ticket online.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${VERSION:?set VERSION, e.g. VERSION=0.7.0}"
ARCHS="${ARCHS:-arm64 x86_64}"
BUNDLE_ID="${BUNDLE_ID:-dev.mclee.flockbook}"
NAME="Flockbook-herdr-mod-$VERSION"
STAGE="build/mod-stage/$NAME"
ZIP="dist/$NAME.zip"

EXES=()
for a in $ARCHS; do
  swift build -c release --product flockbook-modfeed --arch "$a"
  EXES+=("$(swift build -c release --arch "$a" --show-bin-path)/flockbook-modfeed")
done

rm -rf "$STAGE" "$ZIP"
mkdir -p "$STAGE/bin" dist
lipo -create "${EXES[@]}" -output "$STAGE/bin/flockbook-modfeed"
lipo -info "$STAGE/bin/flockbook-modfeed"
# What a person installs; the tests, the docs folder and package.json stay in the repo.
cp -R mod/layouts mod/mods mod/themes "$STAGE/"
cp mod/keys.json mod/install.sh mod/install-isolated.sh mod/README.md mod/RECIPE.md mod/preview.png "$STAGE/"
cp LICENSE "$STAGE/"

if [ -n "${SIGN_ID:-}" ]; then
  codesign --force -s "$SIGN_ID" --options runtime --timestamp --identifier "$BUNDLE_ID.modfeed" "$STAGE/bin/flockbook-modfeed"
else
  codesign --force -s - --identifier "$BUNDLE_ID.modfeed" "$STAGE/bin/flockbook-modfeed"
fi
codesign -dvv "$STAGE/bin/flockbook-modfeed" 2>&1 | grep -E '^(Identifier|Authority|CodeDirectory)' || true
"$STAGE/bin/flockbook-modfeed" version | grep -q '"protocol"' || { echo "flockbook-modfeed does not run"; exit 1; }

ditto -c -k --norsrc --noextattr --keepParent "$STAGE" "$ZIP"
if [ "${NOTARIZE:-0}" = 1 ]; then
  [ -n "${SIGN_ID:-}" ] || { echo "NOTARIZE=1 needs SIGN_ID"; exit 1; }
  OUT="$(xcrun notarytool submit "$ZIP" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait 2>&1)"
  echo "$OUT"
  grep -q 'status: Accepted' <<<"$OUT" || { echo "notarization not accepted: $ZIP"; exit 1; }
fi
echo "packaged: $ZIP"
