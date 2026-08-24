#!/usr/bin/env bash
# Render every README asset: stage the world once, run each tape against it,
# tear it down. Staging once is deliberate - it keeps the commit hashes and the
# relative dates identical across every clip, so the same commit seen in
# `search`, `iscan` and `ilog` reads as one story instead of three.
#
#   ./render.sh              everything
#   ./render.sh history      one tape, against a freshly staged world
#
# VHS wants the machine to itself: two tapes sharing the stage would fight over
# the same working trees, so they run strictly one at a time.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

command -v vhs > /dev/null || { echo "vhs is not on PATH - install it from https://github.com/charmbracelet/vhs" >&2; exit 1; }

run() { echo "── $1.tape"; vhs "$1.tape" > /dev/null; }

./stage.sh up > /dev/null

if [ $# -gt 0 ]; then
  run "$1"
else
  # branches goes last: it is the only tape that changes the world it runs in,
  # because it actually deletes a branch.
  run sleuth
  run repos
  run history
  run status
  run branches
fi

./stage.sh down > /dev/null
echo "done - see ../readme-assets/"
