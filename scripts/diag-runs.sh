#!/usr/bin/env bash
# Throwaway: diag-runs.sh <debug HerdrDesk> <release HerdrDesk> <os label>
set -uo pipefail
cd "$(dirname "$0")/.."
DBG="$1"; REL="$2"; OS="$3"; OUT="build/diag-$OS"; mkdir -p "$OUT"
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV
for cfg in debug release; do
  B="$DBG"; [ "$cfg" = release ] && B="$REL"
  for i in 1 2 3; do
    "$B" --test-conversation-churn > "$OUT/churn-$cfg-$i.log" 2>&1
    echo "== $OS $cfg churn $i: $(grep -E 'churn-test: (PASS|FAIL)' "$OUT/churn-$cfg-$i.log")"
    grep -E 'longest main-thread stall|first view' "$OUT/churn-$cfg-$i.log" | sed 's/^/   /'
  done
  for look in normal legacy; do
    A=(--test-scrolltop); [ "$look" = legacy ] && A+=(--legacy-look)
    "$B" "${A[@]}" > "$OUT/scrolltop-$cfg-$look.log" 2>&1
    echo "== $OS $cfg scrolltop $look"; grep -v NSQuitAlways "$OUT/scrolltop-$cfg-$look.log" | sed 's/^/   /'
    mkdir -p "$OUT/pics-$cfg-$look"; cp build/snapshots-content/scrolltop-*.png "$OUT/pics-$cfg-$look/" 2>/dev/null || true
  done
done
