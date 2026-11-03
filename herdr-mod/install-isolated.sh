#!/bin/sh
# Installs the Flockbook layout and mod into a test herdr-app home, never the real one.
# Usage: install-isolated.sh <herdr-app-home> [path/to/flockbook-modfeed]
#  - <herdr-app-home> must exist and must not be ~/.config/herdr-app. Run herdr-app with
#    HERDR_APP_HOME=<herdr-app-home> to use it.
#  - The sidecar binary, if given, is copied to <herdr-app-home>/flockbook/bin/flockbook-modfeed.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
[ $# -ge 1 ] || { echo "usage: $0 <herdr-app-home> [flockbook-modfeed]" >&2; exit 2; }
[ -d "$1" ] || { echo "STOP: $1 is not a folder" >&2; exit 1; }
dest="$(cd "$1" && pwd -P)"
real="$(cd "$HOME/.config/herdr-app" 2>/dev/null && pwd -P || true)"
case "$dest" in
  "$HOME/.config/herdr-app"|"$HOME/.config/herdr-app/"*) echo "STOP: $dest is the real herdr-app home" >&2; exit 1 ;;
esac
[ -n "$real" ] && [ "$dest" = "$real" ] && { echo "STOP: $dest is the real herdr-app home" >&2; exit 1; }
mkdir -p "$dest/layouts" "$dest/mods"
cp "$here/layouts/flockbook.json" "$dest/layouts/flockbook.json"
mkdir -p "$dest/themes"
cp "$here"/themes/*.json "$dest/themes/"
# Keys: our sections replace the same sections in the home's keys.json; other sections stay.
python3 - "$here/keys.json" "$dest/keys.json" <<'PY'
import json, os, sys
ours = json.load(open(sys.argv[1]))
home = json.load(open(sys.argv[2])) if os.path.exists(sys.argv[2]) else {}
home.update(ours)
home.pop("mod:flockbook", None)  # an older build put the keys here too; it stopped every key
open(sys.argv[2], "w").write(json.dumps(home, indent=2) + "\n")
PY
# Stage the mod beside the old one and swap it in with two renames, so a running window that
# reloads on file changes never sees a half-copied folder.
rm -rf "$dest/mods/.flockbook-new" "$dest/mods/.flockbook-old"
cp -R "$here/mods/flockbook" "$dest/mods/.flockbook-new"
[ -d "$dest/mods/flockbook" ] && mv "$dest/mods/flockbook" "$dest/mods/.flockbook-old"
mv "$dest/mods/.flockbook-new" "$dest/mods/flockbook"
rm -rf "$dest/mods/.flockbook-old"
if [ $# -ge 2 ]; then
  mkdir -p "$dest/flockbook/bin"
  cp "$2" "$dest/flockbook/bin/flockbook-modfeed"
  chmod +x "$dest/flockbook/bin/flockbook-modfeed"
fi
echo "installed into $dest"
