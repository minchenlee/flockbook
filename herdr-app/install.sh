#!/bin/sh
# Installs Flockbook for the Herdr app from this folder (the release zip). It runs
# `flockbook-modfeed install` with this zip's helper (bin/) and the helpers for other machines (remote/).
# The layout, mod, themes and keys are built into the helper; python3 is not needed.
# Usage: ./install.sh [--home <herdr-app home>] [--dry-run]     (./install.sh --help: every option)
# Your arguments come last, so they win over the ones this script adds.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
exe="$here/bin/flockbook-modfeed"
[ -x "$exe" ] || exe="${FLOCKBOOK_MODFEED:-}"
if [ -z "$exe" ] || [ ! -x "$exe" ]; then
  echo "STOP: no helper at $here/bin/flockbook-modfeed." >&2
  echo "In the repo, build one: (cd xplat && cargo build --release -p flockbook-modfeed)," >&2
  echo "then run: xplat/target/release/flockbook-modfeed install --helper xplat/target/release/flockbook-modfeed" >&2
  exit 1
fi
set -- --helper "$exe" "$@"
if [ -f "$here/remote/SHA256SUMS" ]; then set -- --remote "$here/remote" "$@"; fi
exec "$exe" install "$@"
