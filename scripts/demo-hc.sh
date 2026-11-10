#!/bin/sh
# The Flockbook demo on herdr-core: fictional agents in an isolated core, with a made-up HOME, and the phone page for them.
# usage: scripts/demo-hc.sh seed | ping | serve [--user LOGIN] [--port N] [--hours H] [--helper PATH] | status | stop
#   seed    (re)build the demo agents (scripts/demo/hc_seed.py)
#   ping    rebuild only the three agents of the lock screen notices (scripts/demo/phone-frames.mjs): each turns
#           blocked again, so a subscribed phone gets three "Needs you" pushes. Reset with seed.
#   serve   the phone page on 127.0.0.1:PORT (8797) for the demo core only; it stops by itself after H hours (3).
#           LOGIN: the Tailscale account allowed (default: this machine's, read from `tailscale status --json`).
#           Reaching it from a phone needs `tailscale serve --bg --https=<port> <PORT>`, which this script never runs.
#   status  the demo core and its agents
#   stop    stop serve and the demo core
# FLOCKBOOK_DEMO_HOME picks the made-up HOME (default the Shared folder beside your home: Shared/fbdemo); the core is HC_SESSION=flockbook-demo.
# Every hc call runs with `env -i`, that HOME and that session, so nothing reaches your own core, logs or settings.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dh=${FLOCKBOOK_DEMO_HOME:-$(dirname "$HOME")/Shared/fbdemo}
session=flockbook-demo
real_home=$HOME
case "$dh" in "$real_home"|"$real_home"/.config*|"") echo "demo-hc: $dh is not a made-up home" >&2; exit 2 ;; esac
mkdir -p "$dh"
dh=$(cd "$dh" && pwd -P)
sock="$dh/.local/state/herdr-core/$session/$session.sock"
demo() {
    env -i HOME="$dh" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$real_home/.local/bin" TERM=xterm-256color LANG=en_US.UTF-8 \
        HC_SESSION="$session" "$@"
}
core_up() {
    demo hc server start --json >/dev/null 2>&1 || true
    [ -S "$sock" ] || { echo "demo-hc: no core socket at $sock" >&2; exit 1; }
}

cmd=${1:-}
[ $# -gt 0 ] && shift
case "$cmd" in
seed)
    bin="$dh/.flockbook-demo/bin"
    mkdir -p "$bin"
    cc -O2 -o "$bin/demo-agent" "$here/demo/stub.c"
    # Copies, not links: hc knows the agent by the process name, and a link runs under its target's name.
    for k in claude codex pi; do cp "$bin/demo-agent" "$bin/.$k.new" && mv -f "$bin/.$k.new" "$bin/$k"; done
    core_up
    PYTHONDONTWRITEBYTECODE=1 demo python3 -I "$here/demo/hc_seed.py" "$bin"
    ;;
ping)
    core_up
    PYTHONDONTWRITEBYTECODE=1 demo python3 -I "$here/demo/hc_seed.py" "$dh/.flockbook-demo/bin" \
        --only meadow-onboard-dev,harbor-deploy-ops,beacon-retry-dev
    ;;
serve)
    user="" port=8797 hours=3 helper="$here/../xplat/target/release/flockbook-modfeed"
    while [ $# -gt 0 ]; do
        case "$1" in
        --user) user=$2; shift 2 ;;
        --port) port=$2; shift 2 ;;
        --hours) hours=$2; shift 2 ;;
        --helper) helper=$2; shift 2 ;;
        *) echo "demo-hc: unknown option $1" >&2; exit 2 ;;
        esac
    done
    [ -x "$helper" ] || { echo "demo-hc: no helper at $helper (cd xplat && cargo build --release -p flockbook-modfeed)" >&2; exit 2; }
    if [ -z "$user" ]; then
        user=$(tailscale status --json | python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(d["User"][str(d["Self"]["UserID"])]["LoginName"])')
    fi
    core_up
    mkdir -p "$dh/.config/herdr-app"
    echo "demo-hc: serve on 127.0.0.1:$port for $session, stops after ${hours}h; log $dh/serve.log"
    env -i HOME="$dh" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$real_home/.local/bin" LANG=en_US.UTF-8 HC_SESSION="$session" \
        "$helper" serve --user "$user" --port "$port" \
        --cache "$dh/serve-cache.json" --home "$dh/.config/herdr-app" >>"$dh/serve.log" 2>&1 &
    pid=$!
    echo "$pid" > "$dh/serve.pid"
    # The time limit: stop it after H hours (env execs the helper, so $pid is the helper). The watch ends with it.
    end=$(( $(date +%s) + hours * 3600 ))
    ( while kill -0 "$pid" 2>/dev/null; do
        [ "$(date +%s)" -ge "$end" ] && { kill "$pid"; break; }
        sleep 20
      done ) &
    wait "$pid" || true
    echo "demo-hc: serve stopped"
    ;;
status)
    [ -S "$sock" ] || { echo "demo-hc: the demo core is not running ($sock)"; exit 0; }
    echo "core socket: $sock"
    demo hc ls --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
for t in d.get("terminals", []):
    print(t["address"], t.get("name"), t.get("status"), t.get("labels", {}).get("herdr.space"))'
    ;;
stop)
    if [ -f "$dh/serve.pid" ]; then
        pid=$(cat "$dh/serve.pid")
        if ps -p "$pid" -o command= 2>/dev/null | grep -q "flockbook-modfeed serve"; then kill "$pid"; echo "demo-hc: stopped serve ($pid)"; fi
        rm -f "$dh/serve.pid"
    fi
    if [ -S "$sock" ]; then demo hc server stop --json >/dev/null && echo "demo-hc: stopped the demo core"; fi
    ;;
*)
    sed -n '2,12p' "$0" >&2; exit 2 ;;
esac
