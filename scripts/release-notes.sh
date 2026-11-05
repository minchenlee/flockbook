#!/usr/bin/env bash
# The release page's text from docs/release-notes/<tag>.md: scripts/release-notes.sh <tag> > NOTES.md
#   The file's What's New .. Bug Fixes, then Installation (update, download, signing, requirements, disclaimer)
#   and the Full Changelog link from the tag before.
#   Env: MAC_APP=1 when the release has the Mac app (else the mod only), NOTARIZED=1 for a Developer ID build, APPCAST=1 when the build ships an appcast (installed apps update),
#   GITHUB_REPOSITORY (owner/repo), PREV_TAG (default: the highest earlier vX.Y.Z tag on origin).
# Exits 3 when there is no docs/release-notes/<tag>.md: the release keeps GitHub's generated notes.
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${1:?usage: scripts/release-notes.sh <tag>}"
VERSION="${TAG#v}"
FILE="docs/release-notes/$TAG.md"
[ -f "$FILE" ] || { echo "release-notes: no $FILE" >&2; exit 3; }
REPO="${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY (owner/repo)}"

if [ -z "${PREV_TAG:-}" ]; then
  # Pre-release tags (v1.0.0-rc1) are not a "tag before" a release.
  PREV_TAG=$( { git ls-remote --tags --refs origin 'v*' | sed 's#.*refs/tags/##' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$'; echo "$TAG"; } \
    | sort -uV | grep -B1 -xF "$TAG" | grep -vxF "$TAG" || true)
fi

grep -v '^<!--' "$FILE"   # the template's comment line
echo
echo "### 📦 Installation"
echo
if [ "${MAC_APP:-}" = 1 ]; then
  if [ "${APPCAST:-}" = 1 ]; then
    echo "**Update:** Flockbook 0.3.0 or later offers this update by itself (or use **Flockbook > Check for Updates…**)."
    echo
  fi
  echo "**Download:** \`Flockbook-$VERSION.dmg\` (or \`.zip\`) below. Check it against \`SHA256SUMS\`."
  echo
  if [ "${NOTARIZED:-}" = 1 ]; then
    echo "Signed with a Developer ID and notarized by Apple."
  else
    echo "**Unsigned build.** It is not signed with a Developer ID and not notarized."
    echo "macOS blocks it on first open. To run it anyway: \`xattr -dr com.apple.quarantine /Applications/Flockbook.app\`."
  fi
  echo
  echo "Requires macOS 15 or later and a running Herdr. Universal (Apple silicon + Intel)."
  echo "Phone features need Tailscale or your local network."
  echo
fi
echo "**Flockbook for the Herdr app (preview, macOS and Linux):** \`curl -fsSL https://github.com/$REPO/releases/latest/download/install.sh | sh\`"
echo "(add \`-s -- --dry-run\` after \`sh\` to see the plan first). It checks the helper against \`SHA256SUMS\` before it runs anything."
echo "Or download \`Flockbook-herdr-mod-$VERSION.zip\`, unzip it and run \`./install.sh --dry-run\`, then \`./install.sh\`."
echo "Windows is not supported yet."
echo
if [ "${MAC_APP:-}" != 1 ]; then
  echo "**Flockbook for Mac:** no new build in this release. The last Mac build is on the [releases page](https://github.com/$REPO/releases)."
  echo
fi
echo "Unofficial: not affiliated with, endorsed by, or supported by Herdr, Inc."
if [ -n "$PREV_TAG" ]; then
  echo
  echo "---"
  echo
  echo "**Full Changelog**: https://github.com/$REPO/compare/$PREV_TAG...$TAG"
fi
