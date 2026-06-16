#!/usr/bin/env bash
# dev.sh — build / test / install the local sluuz dev build without publishing.
#
#   ./dev.sh demo            build + show the new commands (trace, repos, …)
#   ./dev.sh run <args>      run the dev binary, e.g. ./dev.sh run trace --graph
#   ./dev.sh install         swap your installed `slu` with this dev build
#   ./dev.sh restore         go back to the published crates.io version
#   ./dev.sh build           just compile
#
# `install` and `restore` both touch ~/.cargo/bin/slu only — never crates.io.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$ROOT/Cargo.toml"
BIN="$ROOT/target/debug/slu"

build() { cargo build --quiet --manifest-path "$MANIFEST"; }

case "${1:-help}" in
  build)
    build
    echo "built: $BIN"
    ;;

  run)
    shift
    build
    # run the dev binary in your CURRENT directory (so it sees your repo)
    "$BIN" "$@"
    ;;

  demo)
    build
    echo "==================== slu trace ===================="
    "$BIN" trace -n 8
    echo
    echo "================ slu trace --graph ================"
    "$BIN" trace --graph -n 6
    echo
    echo "==================== slu repos ===================="
    "$BIN" repos
    echo
    echo "============ slu log (real git, untouched) ========"
    "$BIN" log --oneline -3
    ;;

  install)
    echo ">> installing dev build as your 'slu' (replaces local binary, not crates.io)"
    cargo install --path "$ROOT" --force
    echo ">> done. run 'slu trace' anywhere. './dev.sh restore' puts the published version back."
    ;;

  restore)
    echo ">> reinstalling the published sluuz from crates.io"
    cargo install sluuz --force
    echo ">> restored."
    ;;

  *)
    # print the comment header (skip shebang, stop at first non-comment line)
    awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "${BASH_SOURCE[0]}"
    ;;
esac
