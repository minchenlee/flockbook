#!/bin/sh
# Installs the Flockbook layout, mod, themes, keys and helper into a herdr-app home.
# Usage: install.sh [--home <herdr-app home>] [--helper <flockbook-modfeed>] [--dry-run]
#  - The home is --home, else $HERDR_APP_HOME, else ~/.config/herdr-app.
#  - It never overwrites what you have: if a layout, mod or theme with the same name is there and
#    this script did not put it there (or you changed it since), it stops and writes nothing.
#  - keys.json is merged: your sections and keys stay; a key you already use keeps your command.
#  - --helper copies flockbook-modfeed to <home>/flockbook/bin/. Without it, a helper already there
#    stays; with none there, it says where to get one.
#  - After copying it runs `herdr-app check` on the mod. --dry-run prints the plan and writes nothing.
#  - What it installed (with hashes) is kept in <home>/flockbook/install.json, so a later install
#    can update its own files.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
home="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
helper=""
dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --home) [ $# -ge 2 ] || { echo "--home needs a folder" >&2; exit 2; }; home="$2"; shift 2 ;;
    --helper) [ $# -ge 2 ] || { echo "--helper needs a file" >&2; exit 2; }; helper="$2"; shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1 (see --help)" >&2; exit 2 ;;
  esac
done
[ -z "$helper" ] || [ -f "$helper" ] || { echo "STOP: no helper at $helper" >&2; exit 1; }
# /usr/bin/python3 without the Command Line Tools opens an install dialog; check first.
if [ "$(uname)" = Darwin ] && [ "$(command -v python3 || true)" = /usr/bin/python3 ] && ! xcode-select -p >/dev/null 2>&1; then
  echo "STOP: this script needs python3. Install the Command Line Tools (xcode-select --install) and run it again." >&2
  exit 1
fi
command -v python3 >/dev/null 2>&1 || { echo "STOP: this script needs python3." >&2; exit 1; }

python3 - "$here" "$home" "$helper" "$dry" <<'PY'
import hashlib, json, os, shutil, subprocess, sys

src, home, helper, dry = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] == "1"
RELEASES = "https://github.com/minchenlee/flockbook/releases"
marker_path = os.path.join(home, "flockbook", "install.json")

def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()

# Every file this installs, as a path relative to the home.
files = ["layouts/flockbook.json"]
files += sorted("themes/" + n for n in os.listdir(os.path.join(src, "themes")) if n.endswith(".json"))
for d, _, names in os.walk(os.path.join(src, "mods", "flockbook")):
    for n in names:
        if n != ".DS_Store":
            files.append(os.path.relpath(os.path.join(d, n), src))
files = sorted(set(files))

try:
    with open(marker_path) as f:
        mine = json.load(f).get("files", {})
except (OSError, ValueError):
    mine = {}

# A file is ours to replace when it is missing, already the same, or exactly what we put there.
clash = []
for rel in files:
    dest = os.path.join(home, rel)
    if not os.path.exists(dest):
        continue
    have = sha(dest) if os.path.isfile(dest) else None
    if have is None or (have != sha(os.path.join(src, rel)) and mine.get(rel) != have):
        clash.append(rel)
# Files in the person's mods/flockbook that we never put there also mean the folder is theirs.
mod_dest = os.path.join(home, "mods", "flockbook")
if os.path.isdir(mod_dest):
    for d, _, names in os.walk(mod_dest):
        for n in names:
            rel = os.path.relpath(os.path.join(d, n), home)
            # herdr-app writes jsconfig.json and *.d.ts at the mod's top level (see .gitignore).
            generated = d == mod_dest and (n == "jsconfig.json" or n.endswith(".d.ts"))
            if rel not in files and rel not in mine and n != ".DS_Store" and not generated:
                clash.append(rel)
elif os.path.exists(mod_dest):
    clash.append("mods/flockbook")
if clash:
    print("STOP: these are already in %s and were not installed by this script (or you changed them):" % home, file=sys.stderr)
    for rel in sorted(set(clash)):
        print("  " + rel, file=sys.stderr)
    print("Nothing was written. Move or rename them, then run this again.", file=sys.stderr)
    sys.exit(1)

# Keys: add our sections and keys; a key the person already set to another command stays theirs.
with open(os.path.join(src, "keys.json")) as f:
    ours = json.load(f)
keys_path = os.path.join(home, "keys.json")
try:
    with open(keys_path) as f:
        keys = json.load(f)
except FileNotFoundError:
    keys = {}
except ValueError:
    print("STOP: %s is not valid JSON. Nothing was written." % keys_path, file=sys.stderr)
    sys.exit(1)
old_keys = json.loads(json.dumps(keys))
kept = []
for section, binds in ours.items():
    have = keys.setdefault(section, {})
    for key, cmd in binds.items():
        if key in have and have[key] != cmd:
            kept.append((section, key, have[key], cmd))
        else:
            have[key] = cmd

bin_dest = os.path.join(home, "flockbook", "bin", "flockbook-modfeed")
print(("Dry run: would install into " if dry else "Installing into ") + home)
for rel in files:
    if not rel.startswith("mods/"):
        print("  " + rel)
print("  mods/flockbook/... (%d files)" % sum(1 for r in files if r.startswith("mods/")))
print("  keys.json (%s)" % ("merged" if old_keys else "new"))
for section, key, theirs, cmd in kept:
    print("    kept your %s %s = %s (Flockbook uses it for %s)" % (section, key, theirs, cmd))
if helper:
    print("  flockbook/bin/flockbook-modfeed (from %s)" % helper)
if dry:
    print("Dry run: nothing was written; herdr-app check did not run.")
    sys.exit(0)

for rel in files:
    dest = os.path.join(home, rel)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    tmp = dest + ".flockbook-new"
    shutil.copyfile(os.path.join(src, rel), tmp)
    os.replace(tmp, dest)
# Files an earlier install put there that this version no longer has, if still unchanged.
for rel, h in mine.items():
    dest = os.path.join(home, rel)
    own = rel.startswith(("mods/flockbook/", "themes/flockbook")) and ".." not in rel.split("/")
    if own and rel not in files and os.path.isfile(dest) and sha(dest) == h:
        os.remove(dest)
if keys != old_keys:
    tmp = keys_path + ".flockbook-new"
    with open(tmp, "w") as f:
        f.write(json.dumps(keys, indent=2) + "\n")
    os.replace(tmp, keys_path)
if helper:
    os.makedirs(os.path.dirname(bin_dest), exist_ok=True)
    tmp = bin_dest + ".flockbook-new"
    shutil.copyfile(helper, tmp)
    os.chmod(tmp, 0o755)
    os.replace(tmp, bin_dest)
os.makedirs(os.path.dirname(marker_path), exist_ok=True)
with open(marker_path, "w") as f:
    json.dump({"files": {rel: sha(os.path.join(home, rel)) for rel in files}}, f, indent=2)
    f.write("\n")

if not os.path.isfile(bin_dest):
    print("The helper flockbook-modfeed is not installed; the chat stays empty without it.")
    print("Get it from %s (Flockbook-herdr-mod-<version>.zip, bin/flockbook-modfeed)" % RELEASES)
    print("or build it: swift build -c release --product flockbook-modfeed")
    print("then run this again with --helper <path>.")

sys.stdout.flush()
env = dict(os.environ, HERDR_APP_HOME=home)
try:
    r = subprocess.run(["herdr-app", "check", mod_dest], env=env)
except FileNotFoundError:
    print("Installed, but herdr-app is not on PATH, so herdr-app check did not run.")
    sys.exit(0)
if r.returncode != 0:
    print("herdr-app check found a problem (see above). The files are installed.", file=sys.stderr)
    sys.exit(1)
print("Installed. In herdr-app run: layout.switch {\"name\": \"flockbook\"}")
PY
