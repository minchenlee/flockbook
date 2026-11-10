#!/bin/sh
# Desktop stills of the Herdr mod on the herdr-core demo (scripts/demo-hc.sh seed), for "plan A" of the Threads assets.
# The person quits their own Herdr and opens the demo one themselves:
#   HC_SESSION=flockbook-demo HERDR_APP_HOME=<demo home>/.config/herdr-app herdr-app
# (install the mod there first: sh herdr-app/install-isolated.sh <demo home>/.config/herdr-app <flockbook-modfeed>).
# This script never opens a window, never moves the mouse and never types: it only talks to the window whose home is
# the demo's, with `herdr-app do` and `herdr-app shot`. Your own window (~/.config/herdr-app) is never reached.
# usage: scripts/demo/desktop-shots-hc.sh check | shot <name> [agent] | all
#   check   the demo window is open and shows the demo spaces (Orchard, Harbor, ...), and nothing else
#   shot    show <agent> (a demo agent name, e.g. orchard-api-dev), if given, then save <out>/desktop-a-<name>.png
#   all     chat (orchard-api-dev), question (orchard-promo-dev), approval (harbor-deploy-ops), done (meadow-card-dev)
# <demo home>: $FLOCKBOOK_DEMO_HOME, default the Shared folder beside your home, Shared/fbdemo.
# out-dir: $FLOCKBOOK_SHOTS (default build/desktop-shots-hc).
set -eu
dh=${FLOCKBOOK_DEMO_HOME:-$(dirname "$HOME")/Shared/fbdemo}
session=flockbook-demo
out=${FLOCKBOOK_SHOTS:-build/desktop-shots-hc}
real_home=$HOME
case "$dh" in "$real_home"|"$real_home"/*|"") echo "desktop-shots-hc: $dh is not a made-up home" >&2; exit 2 ;; esac
demo() {
    env -i HOME="$dh" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$real_home/.local/bin" LANG=en_US.UTF-8 \
        HC_SESSION="$session" HERDR_APP_HOME="$dh/.config/herdr-app" perl -e 'alarm 30; exec @ARGV' "$@"
}
check() {
    sock="$dh/.local/state/herdr-core/$session/$session.sock"
    [ -S "$sock" ] || { echo "desktop-shots-hc: no demo core at $sock (scripts/demo-hc.sh seed)" >&2; exit 1; }
    demo herdr-app status >/dev/null 2>&1 || { echo "desktop-shots-hc: no demo window (HERDR_APP_HOME=$dh/.config/herdr-app)" >&2; exit 1; }
    # Every space the window shows must be a demo project.
    demo herdr-app ls | python3 -c '
import json, sys
demo = {"Orchard", "Atlas", "Harbor", "Lighthouse", "Quarry", "Meadow", "Beacon", "Ledger", "Tidepool"}
d = json.load(sys.stdin)
spaces = d.get("spaces", d) if isinstance(d, dict) else d
names = [s.get("name") or s.get("label") or "" for s in spaces]
other = [n for n in names if n not in demo]
print("spaces:", ", ".join(names))
if not names or other:
    sys.exit("desktop-shots-hc: the window shows spaces that are not the demo: " + ", ".join(other or ["(none)"]))
'
}
address() {
    demo hc ls --json | python3 -c 'import json, sys
t = [t["address"] for t in json.load(sys.stdin)["terminals"] if t["name"] == sys.argv[1]]
print(t[0] if t else "")' "$1"
}
shot() {
    name=$1 agent=${2:-}
    if [ -n "$agent" ]; then
        a=$(address "$agent")
        [ -n "$a" ] || { echo "desktop-shots-hc: no demo agent $agent" >&2; exit 1; }
        demo herdr-app do agent.focus --terminal "$a" >/dev/null
        sleep 2
    fi
    mkdir -p "$out"
    dir=$(cd "$out" && pwd -P)
    demo herdr-app shot "$dir/desktop-a-$name.png"
}
case ${1:-} in
check) check ;;
shot) [ $# -ge 2 ] || { echo "usage: $0 shot <name> [agent]" >&2; exit 2; }; check >/dev/null; shot "$2" "${3:-}" ;;
all)
    check
    shot chat orchard-api-dev
    shot question orchard-promo-dev
    shot approval harbor-deploy-ops
    shot done meadow-card-dev
    ;;
*) sed -n 2,13p "$0" >&2; exit 2 ;;
esac
