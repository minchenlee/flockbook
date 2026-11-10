#!/usr/bin/env bash
# The app's UI checks, one HerdrDesk run each: [ONLY=name,...] [SHARD=i/n] scripts/ui-checks.sh [offline|real] [out-dir]
#   offline (default): every check that needs no herdr server and opens no window (preview stores, offscreen windows).
#   real: the two checks with a real window (--test-fullscreen-real, --test-scroll-real). They take the screen and the
#         focus: CI runners only, or with the owner's leave. Needs FLOCKBOOK_VISIBLE_OK=1.
# Not here: the checks that drive a live pane or session (--test-approve, --snapshot-demo, ...): they need a herdr server.
# A check fails when it exits non-zero, runs past its timeout, prints FAIL, or does not print its PASS line.
# Known failures (KNOWN) still run: a failure there is a warning, not a failed run; a pass asks for its removal.
# A locked screen makes that check SKIP (also if it locks during the check); skipped checks never count as passed.
# Each check's output goes to <out-dir>/logs/<name>.log; its pictures to <out-dir>/<name>/ or the app's build/snapshots-*.
set -uo pipefail
cd "$(dirname "$0")/.."

MODE="${1:-offline}"
OUT="${2:-build/ui-checks}"
BIN="${BIN:-.build/debug/HerdrDesk}"
LIMIT="${UI_CHECK_TIMEOUT:-180}"   # seconds per check; a hung check is killed and fails, the others still run

# name | PASS line it must print ("-": exit 0 is the verdict) | arguments
OFFLINE=(
    "snapshot|-|--snapshot $OUT/snapshot"
    "snapshot-reading|-|--snapshot-reading $OUT/snapshot-reading"
    "snapshot-settings|snapshot-settings: PASS|--snapshot-settings $OUT/snapshot-settings"
    "snapshot-remote|snapshot-remote: PASS|--snapshot-remote $OUT/snapshot-remote"
    "snapshot-statusline|snapshot-statusline: PASS|--snapshot-statusline $OUT/snapshot-statusline"
    "hittest|hittest: overflow check PASS|--hittest"
    "chrome|chrome-test: PASS|--test-chrome"
    "table|table-test: PASS|--test-table"
    "bottom|bottom-test: PASS|--test-bottom"
    "closetab|closetab-test: PASS|--test-closetab"
    "menus|menus-test: PASS|--test-menus"
    "tabdrag|tabdrag-test: PASS|--test-tabdrag"
    "panemenu|panemenu-test: PASS|--test-panemenu"
    "composer|composer-test: PASS|--test-composer"
    "send-unconfirmed|unconfirmed-test: PASS|--test-send-unconfirmed"
    "sidebar|sidebar-test: PASS|--test-sidebar"
    "sidebar-layout|sidebar-layout: PASS|--test-sidebar-layout"
    "root-width|root-width: PASS|--test-root-width"
    "settings-migration|settings-migration: PASS|--test-settings-migration"
    "island-height|island-height: PASS|--test-island-height"
    "tab-overflow|tab-overflow: PASS|--test-tab-overflow"
    "tabkeys|tabkeys-test: PASS|--test-tabkeys"
    "settings-page|settings-page-test: PASS|--test-settings-page"
    "sendfail|sendfail-test: PASS|--test-sendfail"
    "slash|slash-test: PASS|--test-slash"
    "earlier-failure|earlier-failure: PASS|--test-earlier-failure"
    "loadstate|loadstate-test: PASS|--test-loadstate"
    "termfail|termfail-test: PASS|--test-termfail"
    "termtheme|termtheme-test: PASS|--test-termtheme"
    "glyph|glyph-test: PASS|--test-glyph"
    "nerdglyph|nerdglyph-test: PASS|--test-nerdglyph"
    "panebars|panebars-test: PASS|--test-panebars"
    "surfaces|surfaces-test: PASS|--test-surfaces"
    "search|search-test: PASS|--test-search"
    "textsize|textsize-test: PASS|--test-textsize"
    "scrolltop|scrolltop-test: PASS|--test-scrolltop"
    "shortcuts|shortcuts-test: PASS|--test-shortcuts"
    "question|question-test: PASS|--test-question"
    "permission|permission-test: PASS|--test-permission"
    "screencolumn|screencolumn-test: PASS|--test-screencolumn"
    "conversation-churn|churn-test: PASS|--test-conversation-churn"
    "busy-tab|busy-tab: PASS|--test-busy-tab"
    "legacy-look|legacy-look-test: PASS|--test-legacy-look"
    "pictures|pictures-test: PASS|--test-pictures"
    "dragroute|dragroute-test: PASS|--test-dragroute"
)
REAL=(
    "fullscreen-real|fs-real|--preview --test-fullscreen-real"
    "scroll-real|scroll-real|--preview --test-scroll-real"
)
# Checks that fail for a reason being looked into, e.g. " island-height glyph "; empty now.
KNOWN=" "
# Fail now and then on a loaded runner, for a reason not fixed yet: a failure here runs the check once more, and a pass
# then is a warning ("flaky passed on retry"). Only these: every other check fails on its first failure.
#   conversation-churn: autoload-fling-busy, every core spun on a macOS 15 runner: one anchor fix lands a frame late.
FLAKY=" conversation-churn "
case "$MODE" in
    offline) CHECKS=("${OFFLINE[@]}") ;;
    real)
        [ "${FLOCKBOOK_VISIBLE_OK:-}" = 1 ] || { echo "ui-checks: real opens a window: set FLOCKBOOK_VISIBLE_OK=1" >&2; exit 2; }
        CHECKS=("${REAL[@]}") ;;
    *) echo "usage: scripts/ui-checks.sh [offline|real] [out-dir]" >&2; exit 2 ;;
esac

# ONLY="question busy-tab" (spaces or commas): run just these. A name of the other mode is skipped; an unknown name exits 2.
if [ -n "${ONLY:-}" ]; then
    known_names=" "; for entry in "${OFFLINE[@]}" "${REAL[@]}"; do known_names+="${entry%%|*} "; done
    for want in ${ONLY//,/ }; do
        [[ "$known_names" == *" $want "* ]] || { echo "ui-checks: no check named '$want' (ONLY)" >&2; exit 2; }
    done
    picked=()
    for entry in "${CHECKS[@]}"; do
        [[ " ${ONLY//,/ } " == *" ${entry%%|*} "* ]] && picked+=("$entry")
    done
    [ ${#picked[@]} -gt 0 ] || { echo "ui-checks $MODE: nothing to run (ONLY=$ONLY)"; exit 0; }
    CHECKS=("${picked[@]}")
fi

# Seconds a check takes on a CI runner (macos-26 and macos-15 alike); a check not named here counts as 15.
estimate() {
    case "$1" in
        conversation-churn) echo 140 ;; tab-overflow) echo 113 ;; question) echo 47 ;; hittest) echo 45 ;;
        snapshot) echo 44 ;; settings-page) echo 44 ;; snapshot-statusline) echo 41 ;; snapshot-remote) echo 36 ;;
        snapshot-settings) echo 33 ;; chrome) echo 25 ;; root-width) echo 23 ;; screencolumn) echo 22 ;;
        shortcuts|tabdrag|menus|glyph|pictures) echo 16 ;; composer|termtheme|nerdglyph|scrolltop) echo 10 ;;
        dragroute) echo 4 ;;
        bottom|sidebar|search|busy-tab|textsize) echo 8 ;;
        snapshot-reading|panebars|tabkeys|loadstate|island-height|legacy-look|sendfail|table) echo 5 ;;
        closetab|termfail|surfaces|sidebar-layout|panemenu|settings-migration) echo 2 ;;
        *) echo 15 ;;
    esac
}

# SHARD=2/3: run the second of three groups. Longest check first, into the group with the fewest seconds so far
# (the lower group on a tie): the same groups on every runner. Applied after ONLY.
if [ -n "${SHARD:-}" ]; then
    [[ "$SHARD" =~ ^([0-9]+)/([0-9]+)$ ]] && [ "${BASH_REMATCH[1]}" -ge 1 ] && [ "${BASH_REMATCH[1]}" -le "${BASH_REMATCH[2]}" ] \
        || { echo "ui-checks: SHARD must be i/n with 1 <= i <= n, not '$SHARD'" >&2; exit 2; }
    shard=${BASH_REMATCH[1]}; shards=${BASH_REMATCH[2]}
    load=(); for ((g = 1; g <= shards; g++)); do load[g]=0; done
    order=$(for ((k = 0; k < ${#CHECKS[@]}; k++)); do echo "$(estimate "${CHECKS[k]%%|*}") $k"; done | sort -k1,1nr -k2,2n)
    mine=()
    while read -r secs k; do
        best=1; for ((g = 2; g <= shards; g++)); do [ "${load[g]}" -lt "${load[best]}" ] && best=$g; done
        load[best]=$(( load[best] + secs ))
        [ "$best" = "$shard" ] && mine+=("$k")
    done <<< "$order"
    picked=(); for k in $(printf '%s\n' ${mine[@]+"${mine[@]}"} | sort -n); do picked+=("${CHECKS[k]}"); done
    names=(); for entry in ${picked[@]+"${picked[@]}"}; do names+=("${entry%%|*}"); done
    echo "ui-checks: shard $SHARD (~${load[shard]}s): ${names[*]:-none}"
    [ ${#picked[@]} -gt 0 ] || { echo "ui-checks $MODE: nothing to run (SHARD=$SHARD)"; exit 0; }
    CHECKS=("${picked[@]}")
fi

screen_locked() { ioreg -n Root -d1 | grep -q '"CGSSessionScreenIsLocked"=Yes'; }
[ -x "$BIN" ] || { echo "ui-checks: $BIN not found: run swift build first" >&2; exit 2; }
mkdir -p "$OUT/logs"
# A herdr pane's variables would point the app at the owner's session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV


# Runs "$@" with a time limit (macOS has no timeout(1)): TERM at the limit, KILL 5 s later. Exit 124 on a timeout.
run_limited() {
    local limit=$1; shift
    "$@" & local pid=$!
    ( sleep "$limit"; kill -TERM "$pid" 2>/dev/null && { sleep 5; kill -KILL "$pid" 2>/dev/null; }; ) & local guard=$!
    wait "$pid"; local code=$?
    if kill -0 "$guard" 2>/dev/null; then kill "$guard" 2>/dev/null; wait "$guard" 2>/dev/null; else code=124; fi
    return "$code"
}

# One run of the current check: sets secs and verdict.
run_once() {
    start=$(date +%s)
    run_limited "$LIMIT" "$BIN" "${args[@]}" > "$log" 2>&1; code=$?
    secs=$(( $(date +%s) - start ))
    verdict=ok
    if screen_locked; then verdict="SKIP (screen locked during check)"
    elif [ "$code" = 124 ]; then verdict="timeout after ${LIMIT}s"
    elif [ "$code" != 0 ]; then verdict="exit $code"
    elif grep -q 'FAIL' "$log"; then verdict="FAIL line"
    elif [ "$pass" != - ] && ! grep -qF "$pass" "$log"; then verdict="no '$pass' line"
    fi
}

failed=(); known_failed=(); flaky_passed=(); skipped=()
summary="$OUT/summary.txt"; : > "$summary"
for entry in "${CHECKS[@]}"; do
    IFS='|' read -r name pass argline <<< "$entry"
    read -r -a args <<< "$argline"
    log="$OUT/logs/$name.log"
    if screen_locked; then
        skipped+=("$name")
        printf '%-20s %4ss  SKIP (screen locked)\n' "$name" 0 | tee -a "$summary"
        echo "ui-checks: $name not run: screen locked" > "$log"
        continue
    fi
    [ "$MODE" = real ] && export FS_REAL_DIR="$OUT/$name"
    run_once
    if [[ "$verdict" != SKIP* ]] && [ "$verdict" != ok ] && [[ "$FLAKY" == *" $name "* ]]; then
        first="$verdict"; mv "$log" "$OUT/logs/$name.first.log"
        printf '%-20s %4ss  %s\n' "$name" "$secs" "$first; once more (FLAKY)" | tee -a "$summary"
        grep 'FAIL' "$OUT/logs/$name.first.log" | sed "s/^/$name failed, first run: /"
        run_once
        if [ "$verdict" = ok ]; then
            flaky_passed+=("$name"); echo "::warning title=ui-checks::$name failed ($first), passed on retry: flaky"
        fi
    fi
    line=$(printf '%-20s %4ss  %s' "$name" "$secs" "$verdict")
    echo "$line" | tee -a "$summary"
    if [[ "$verdict" == SKIP* ]]; then skipped+=("$name"); continue; fi
    grep 'WARN (CI runner' "$log" | sed "s/^/::warning title=ui-checks::$name: /"   # timing misses a CI runner does not count
    grep '^tab-overflow: WARN selected tab ' "$log" | sed "s/^/::warning title=ui-checks::$name: /"
    known=no; [[ "$KNOWN" == *" $name "* ]] && known=yes
    if [ "$verdict" != ok ]; then
        if [ "$known" = yes ]; then
            known_failed+=("$name"); echo "::warning title=ui-checks::$name fails ($verdict): known failure, not counted"
        else
            failed+=("$name")
        fi
        grep 'FAIL' "$log" | sed "s/^/$name failed: /"   # every FAIL line, also those above the tail below
        echo "::group::$name log"; tail -40 "$log"; echo "::endgroup::"
    elif [ "$known" = yes ]; then
        echo "::notice title=ui-checks::$name passes now: remove it from KNOWN in scripts/ui-checks.sh"
    fi
done
echo "ui-checks $MODE: $(( ${#CHECKS[@]} - ${#failed[@]} - ${#known_failed[@]} - ${#skipped[@]} ))/${#CHECKS[@]} passed${failed:+; failed: ${failed[*]}}${known_failed:+; known failures: ${known_failed[*]}}${flaky_passed:+; flaky passed on retry: ${flaky_passed[*]}}${skipped:+; skipped: ${#skipped[@]} (${skipped[*]})}" | tee -a "$summary"
[ ${#failed[@]} = 0 ]
