#!/usr/bin/env bash
# dist/Flockbook-herdr-mod-<VERSION>.zip: the Herdr app layout, mod, themes, keys and install script
# from herdr-app/, the Rust flockbook-modfeed (the mod's helper) for this Mac in bin/, and the helper
# for every platform in remote/<target>/ with remote/SHA256SUMS. Also the one-line install's assets:
# dist/install.sh and dist/flockbook-modfeed-<target>.
#   VERSION=0.7.0 HELPERS=<folder> scripts/package-mod.sh
# HELPERS holds <target>/flockbook-modfeed for universal-apple-darwin, x86_64-unknown-linux-musl and
# aarch64-unknown-linux-musl (the helpers workflow builds them). Without HELPERS it builds the macOS
# one here (cargo, both Apple targets) and ships only that.
# Signing (the macOS helper): ad-hoc by default. With SIGN_ID set: Developer ID, hardened runtime,
# timestamp. Notarization: with NOTARIZE=1 and APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID, the zip goes
# to Apple's notary service. A bare executable cannot be stapled; Gatekeeper checks the ticket online.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${VERSION:?set VERSION, e.g. VERSION=0.7.0}"
BUNDLE_ID="${BUNDLE_ID:-dev.mclee.flockbook}"
NAME="Flockbook-herdr-mod-$VERSION"
STAGE="build/mod-stage/$NAME"
ZIP="dist/$NAME.zip"
MAC=universal-apple-darwin

if [ -z "${HELPERS:-}" ]; then
  HELPERS="build/mod-helpers"
  rm -rf "$HELPERS"; mkdir -p "$HELPERS/$MAC"
  EXES=()
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    (cd xplat && CARGO_PROFILE_RELEASE_STRIP=true cargo build --release --locked -p flockbook-modfeed --target "$t")
    EXES+=("xplat/target/$t/release/flockbook-modfeed")
  done
  lipo -create "${EXES[@]}" -output "$HELPERS/$MAC/flockbook-modfeed"
fi
[ -f "$HELPERS/$MAC/flockbook-modfeed" ] || { echo "no $HELPERS/$MAC/flockbook-modfeed"; exit 1; }

rm -rf "$STAGE" "$ZIP"
mkdir -p "$STAGE/bin" "$STAGE/remote" dist
for d in "$HELPERS"/*/; do
  t="$(basename "$d")"
  [ -f "$d/flockbook-modfeed" ] || continue
  mkdir -p "$STAGE/remote/$t"
  cp "$d/flockbook-modfeed" "$STAGE/remote/$t/flockbook-modfeed"
  chmod 755 "$STAGE/remote/$t/flockbook-modfeed"
done
lipo -info "$STAGE/remote/$MAC/flockbook-modfeed"

# Sign the macOS helper before any sum is taken.
if [ -n "${SIGN_ID:-}" ]; then
  codesign --force -s "$SIGN_ID" --options runtime --timestamp --identifier "$BUNDLE_ID.modfeed" "$STAGE/remote/$MAC/flockbook-modfeed"
else
  codesign --force -s - --identifier "$BUNDLE_ID.modfeed" "$STAGE/remote/$MAC/flockbook-modfeed"
fi
codesign -dvv "$STAGE/remote/$MAC/flockbook-modfeed" 2>&1 | grep -E '^(Identifier|Authority|CodeDirectory)' || true
cp "$STAGE/remote/$MAC/flockbook-modfeed" "$STAGE/bin/flockbook-modfeed"
"$STAGE/bin/flockbook-modfeed" version | grep -q '"protocol"' || { echo "flockbook-modfeed does not run"; exit 1; }
(cd "$STAGE/remote" && for t in */; do shasum -a 256 "${t%/}/flockbook-modfeed"; done > SHA256SUMS && cat SHA256SUMS)

# What a person installs; the tests, the docs folder and package.json stay in the repo.
cp -R herdr-app/layouts herdr-app/mods herdr-app/themes "$STAGE/"
cp herdr-app/keys.json herdr-app/install.sh herdr-app/install-isolated.sh herdr-app/README.md herdr-app/RECIPE.md herdr-app/preview.png "$STAGE/"
cp LICENSE "$STAGE/"

# The one-line install's assets: the same (signed) helpers, by target, and its script.
for d in "$STAGE/remote"/*/; do
  t="$(basename "$d")"
  cp "$d/flockbook-modfeed" "dist/flockbook-modfeed-$t"
done
cp scripts/install-from-release.sh dist/install.sh

ditto -c -k --norsrc --noextattr --keepParent "$STAGE" "$ZIP"
if [ "${NOTARIZE:-0}" = 1 ]; then
  [ -n "${SIGN_ID:-}" ] || { echo "NOTARIZE=1 needs SIGN_ID"; exit 1; }
  OUT="$(xcrun notarytool submit "$ZIP" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait 2>&1)"
  echo "$OUT"
  grep -q 'status: Accepted' <<<"$OUT" || { echo "notarization not accepted: $ZIP"; exit 1; }
fi
echo "packaged: $ZIP"
