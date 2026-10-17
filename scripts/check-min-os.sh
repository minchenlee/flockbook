#!/usr/bin/env bash
# Check that every place naming the lowest macOS the app runs on says the same version.
# Usage: scripts/check-min-os.sh <version> [<binary or .app> ...]
#   Sources: Package.swift platforms, the actool deployment target and LSMinimumSystemVersion in bundle.sh.
#   Each binary: its LC_BUILD_VERSION minos. Each .app: its Info.plist and main executable.
set -euo pipefail
cd "$(dirname "$0")/.."

WANT="${1:?usage: scripts/check-min-os.sh <version> [<binary or .app> ...]}"; shift
FAIL=0
check() { # check <what> <found>
  if [ "$2" = "$WANT" ]; then echo "min-os: $1: $2 ok"; else echo "min-os: $1: '$2' (want $WANT) FAIL"; FAIL=1; fi
}

check "Package.swift platforms" "$(sed -nE 's/.*platforms: \[\.macOS\("([0-9.]+)"\)\].*/\1/p' Package.swift)"
check "bundle.sh actool target" "$(sed -nE 's/.*--minimum-deployment-target ([0-9.]+).*/\1/p' scripts/bundle.sh)"
check "bundle.sh LSMinimumSystemVersion" \
  "$(sed -nE 's/.*<key>LSMinimumSystemVersion<\/key><string>([0-9.]+)<\/string>.*/\1/p' scripts/bundle.sh)"

# minos of a Mach-O, for its first architecture: "15.0", or empty.
minos() { xcrun vtool -show-build "$1" 2>/dev/null | awk '$1 == "minos" { print $2; exit }'; }

for P in "$@"; do
  if [ -d "$P" ]; then
    check "$P Info.plist" "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$P/Contents/Info.plist" 2>/dev/null || true)"
    EXE="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$P/Contents/Info.plist" 2>/dev/null || true)"
    check "$P executable minos" "$(minos "$P/Contents/MacOS/$EXE")"
  else
    check "$P minos" "$(minos "$P")"
  fi
done

if [ "$FAIL" = 0 ]; then echo "min-os: PASS"; else echo "min-os: FAIL"; exit 1; fi
