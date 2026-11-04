#!/bin/sh
# Installs Flockbook for the Herdr app from this folder (the release zip). It runs
# `flockbook-modfeed install` with this machine's helper and the helpers for other machines (remote/).
# The helper: bin/ on macOS (the signed one); remote/<target>/ on Linux, checked against remote/SHA256SUMS.
# The layout, mod, themes and keys are built into the helper; python3 is not needed.
# Usage: ./install.sh [--home <herdr-app home>] [--dry-run]     (./install.sh --help: every option)
# Your arguments come last, so they win over the ones this script adds.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
die() { echo "STOP: $*" >&2; exit 1; }

os="$(uname -s)"
case "$os" in
  Darwin) rel=bin/flockbook-modfeed ;;
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) rel=remote/x86_64-unknown-linux-musl/flockbook-modfeed ;;
      aarch64|arm64) rel=remote/aarch64-unknown-linux-musl/flockbook-modfeed ;;
      *) die "there is no helper for Linux on $(uname -m) yet." ;;
    esac ;;
  MINGW* | MSYS* | CYGWIN*) die "Windows is not supported yet. Flockbook for the Herdr app installs on macOS and Linux." ;;
  *) die "$os is not supported yet. Flockbook for the Herdr app installs on macOS and Linux." ;;
esac

exe="$here/$rel"
if [ -x "$exe" ]; then
  # A helper from remote/ runs only if its sum matches (bin/ is the same file as remote/universal-apple-darwin/).
  case "$rel" in
    remote/*)
      [ -f "$here/remote/SHA256SUMS" ] || die "no remote/SHA256SUMS to check $rel against."
      if command -v sha256sum >/dev/null 2>&1; then have="$(sha256sum "$exe" | cut -d' ' -f1)"
      elif command -v shasum >/dev/null 2>&1; then have="$(shasum -a 256 "$exe" | cut -d' ' -f1)"
      else die "this needs sha256sum or shasum to check $rel."; fi
      want="$(awk -v f="${rel#remote/}" '$2 == f || $2 == "*" f { print $1 }' "$here/remote/SHA256SUMS")"
      [ -n "$want" ] && [ "$have" = "$want" ] || die "$rel does not match remote/SHA256SUMS. Nothing was installed."
      ;;
  esac
else
  exe="${FLOCKBOOK_MODFEED:-}"
fi
if [ -z "$exe" ] || [ ! -x "$exe" ]; then
  echo "STOP: no helper at $here/$rel." >&2
  echo "In the repo, build one: (cd xplat && cargo build --release -p flockbook-modfeed)," >&2
  echo "then run: xplat/target/release/flockbook-modfeed install --helper xplat/target/release/flockbook-modfeed" >&2
  exit 1
fi
set -- --helper "$exe" "$@"
if [ -f "$here/remote/SHA256SUMS" ]; then set -- --remote "$here/remote" "$@"; fi
exec "$exe" install "$@"
