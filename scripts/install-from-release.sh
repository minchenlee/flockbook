#!/bin/sh
# Flockbook for the Herdr app, in one line (published as the release asset install.sh):
#   curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --dry-run        (arguments go to `flockbook-modfeed install`)
# It downloads the helpers (flockbook-modfeed) and SHA256SUMS from the same release, stops if a sum
# does not match, then runs this machine's helper: `flockbook-modfeed install --helper <it> --remote <all>`.
# The layout, mod, themes and keys are built into the helper. Nothing runs before every sum matches.
# FLOCKBOOK_RELEASE_URL picks another release folder (a URL, or file:///path for a test).
set -eu

# All in one function, called on the last line: a download cut short runs nothing.
main() {
  base="${FLOCKBOOK_RELEASE_URL:-https://github.com/minchenlee/flockbook/releases/latest/download}"
  TARGETS="universal-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

  die() { echo "STOP: $*" >&2; exit 1; }

  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) target=universal-apple-darwin ;;
    Linux)
      case "$arch" in
        x86_64|amd64) target=x86_64-unknown-linux-musl ;;
        aarch64|arm64) target=aarch64-unknown-linux-musl ;;
        *) die "there is no helper for Linux on $arch yet." ;;
      esac ;;
    *) die "there is no helper for $os yet. Install from the zip on the release page instead." ;;
  esac

  command -v curl >/dev/null 2>&1 || die "this needs curl."
  if command -v sha256sum >/dev/null 2>&1; then
    sum() { sha256sum "$1" | cut -d' ' -f1; }
  elif command -v shasum >/dev/null 2>&1; then
    sum() { shasum -a 256 "$1" | cut -d' ' -f1; }
  else
    die "this needs sha256sum or shasum to check the download."
  fi

  tmp="$(mktemp -d "${TMPDIR:-/tmp}/flockbook-install.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 130' INT TERM

  echo "Downloading the Flockbook helpers from $base"
  curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || die "cannot download $base/SHA256SUMS"
  mkdir "$tmp/remote"
  for t in $TARGETS; do
    f="flockbook-modfeed-$t"
    want="$(awk -v f="$f" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS")"
    [ -n "$want" ] || die "SHA256SUMS has no line for $f"
    mkdir "$tmp/remote/$t"
    curl -fsSL "$base/$f" -o "$tmp/remote/$t/flockbook-modfeed" || die "cannot download $base/$f"
    have="$(sum "$tmp/remote/$t/flockbook-modfeed")"
    [ "$have" = "$want" ] || die "$f does not match SHA256SUMS (got $have, want $want). Nothing was installed."
    chmod 755 "$tmp/remote/$t/flockbook-modfeed"
    echo "$have  $t/flockbook-modfeed" >> "$tmp/remote/SHA256SUMS"
  done

  exe="$tmp/remote/$target/flockbook-modfeed"
  status=0
  "$exe" install --helper "$exe" --remote "$tmp/remote" "$@" </dev/null || status=$?
  exit "$status"
}

main "$@"
