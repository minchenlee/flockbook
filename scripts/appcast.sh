#!/usr/bin/env bash
# Write the Sparkle appcast for one release archive: scripts/appcast.sh <zip> <out-dir>
# The private EdDSA key comes from env SPARKLE_ED_PRIVATE_KEY (base64, as `generate_keys -x` exports it); it is
# only piped to Sparkle's tools, never written to disk. It must match SUPublicEDKey in APP's Info.plist.
# Env: DOWNLOAD_PREFIX (URL the zip is served under, ending in /), APP (default build/Flockbook.app),
# RELEASE_NOTES_URL and LINK (optional). Needs Sparkle's tools from `swift build` (.build/artifacts).
set -euo pipefail
cd "$(dirname "$0")/.."

ZIP="$1"; OUT="$2"
APP="${APP:-build/Flockbook.app}"
TOOLS=".build/artifacts/sparkle/Sparkle/bin"
: "${SPARKLE_ED_PRIVATE_KEY:?set SPARKLE_ED_PRIVATE_KEY}" "${DOWNLOAD_PREFIX:?set DOWNLOAD_PREFIX}"
[ -x "$TOOLS/generate_appcast" ] || { echo "missing $TOOLS (run swift build)"; exit 1; }
[ -f "$ZIP" ] || { echo "missing: $ZIP"; exit 1; }

# The key pair must match: an app signed for one key rejects updates signed with another.
APP_KEY="$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$APP/Contents/Info.plist" 2>/dev/null || true)"
[ -n "$APP_KEY" ] || { echo "no SUPublicEDKey in $APP (bundle with SPARKLE_PUBLIC_KEY)"; exit 1; }
OPENSSL="$(command -v /opt/homebrew/opt/openssl@3/bin/openssl || command -v openssl)"
RAW_LEN="$(printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | base64 -d | wc -c | tr -d ' ')"
case "$RAW_LEN" in
  32)   # seed only (current generate_keys): derive the public key
    KEY_PUB="$( { printf '302e020100300506032b657004220420' | xxd -r -p; printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | base64 -d; } \
      | "$OPENSSL" pkey -inform DER -pubout -outform DER | tail -c 32 | base64)" ;;
  96)   # older export: private key followed by the public key
    KEY_PUB="$(printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | base64 -d | tail -c 32 | base64)" ;;
  *) echo "SPARKLE_ED_PRIVATE_KEY: unexpected length ($RAW_LEN bytes)"; exit 1 ;;
esac
[ "$KEY_PUB" = "$APP_KEY" ] || { echo "SPARKLE_ED_PRIVATE_KEY does not match SUPublicEDKey in $APP"; exit 1; }
echo "appcast: key matches SUPublicEDKey"

# generate_appcast reads every archive in a folder: give it a folder with only this one.
WORK="$(mktemp -d)"
cp "$ZIP" "$WORK/"
ARGS=(--ed-key-file - --download-url-prefix "$DOWNLOAD_PREFIX" --maximum-deltas 0)
[ -n "${RELEASE_NOTES_URL:-}" ] && ARGS+=(--full-release-notes-url "$RELEASE_NOTES_URL")
[ -n "${LINK:-}" ] && ARGS+=(--link "$LINK")
printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | "$TOOLS/generate_appcast" "${ARGS[@]}" "$WORK"
mkdir -p "$OUT"
cp "$WORK/appcast.xml" "$OUT/appcast.xml"

# The feed must carry its own signature (SURequireSignedFeed) and the archive's.
grep -q 'sparkle:edSignature=' "$OUT/appcast.xml" || { echo "appcast: no archive signature"; exit 1; }
printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | "$TOOLS/sign_update" --ed-key-file - --verify "$OUT/appcast.xml" \
  || { echo "appcast: feed signature does not verify"; exit 1; }
# generate_appcast copies LSMinimumSystemVersion into the feed: a Mac below it never gets offered this update.
APP_MIN="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$APP/Contents/Info.plist")"
grep -q "<sparkle:minimumSystemVersion>$APP_MIN</sparkle:minimumSystemVersion>" "$OUT/appcast.xml" \
  || { echo "appcast: no sparkle:minimumSystemVersion $APP_MIN"; exit 1; }
echo "appcast: $OUT/appcast.xml"
grep -E '<sparkle:(version|shortVersionString|minimumSystemVersion)>|<enclosure' "$OUT/appcast.xml"
