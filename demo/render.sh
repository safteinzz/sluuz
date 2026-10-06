#!/usr/bin/env bash
# Render every README asset in a container: stage the world once, run each tape
# against it, tear it down. Staging once is deliberate - it keeps the commit
# hashes and the relative dates identical across every clip, so the same commit
# seen in `search`, `iscan` and `ilog` reads as one story instead of three.
# That is why this rig runs every tape in one container, where the others give
# each tape its own. A machine needs podman or docker and nothing else: no vhs,
# no git, no font, and the same frames on every machine that runs it.
#
#   ./render.sh              everything
#   ./render.sh history      one tape, against a freshly staged world
#
# VHS wants the machine to itself: two tapes sharing the stage would fight over
# the same working trees, so they run strictly one at a time.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

ENGINE="$(command -v podman || command -v docker || true)"
[ -n "$ENGINE" ] || { echo "render.sh needs podman or docker" >&2; exit 1; }
IMAGE=localhost/sluuz-render

if [ $# -gt 0 ]; then
  TAPES=("$1")
else
  # plain goes first and stash, tags and branches last: those are the only
  # tapes that change the world they run in, because they drop a stash and
  # delete a tag and a branch, and the stills should show it untouched.
  TAPES=(plain sleuth repos history status stash tags branches)
fi

(cd .. && cargo build --release)
# The Dockerfile is the whole build context: nothing in this folder is copied in.
"$ENGINE" build -q -t "$IMAGE" - < Dockerfile > /dev/null

# Run as this user, not root: the images it writes stay yours, and the staged
# shell's prompt ends in `$` as it does on a machine rendering without a
# container, rather than root's `#`.
if [ "$(basename "$ENGINE")" = docker ]; then
  USER_ARGS=(--user "$(id -u):$(id -g)" -e HOME=/tmp)
else
  USER_ARGS=(--userns=keep-id -e HOME=/tmp)
fi

# No network: every remote is a bare repo inside the stage, so nothing in a
# take has a reason to leave the machine.
"$ENGINE" run --rm --network none "${USER_ARGS[@]}" \
  -v "$(cd .. && pwd):/work/sluuz:Z" -w /work/sluuz/demo \
  --entrypoint bash "$IMAGE" -c '
    ./stage.sh up > /dev/null || exit
    for t in "$@"; do
      echo "── $t.tape"
      vhs "$t.tape" > /dev/null || { s=$?; break; }
    done
    ./stage.sh down > /dev/null
    exit "${s:-0}"
  ' render "${TAPES[@]}"
echo "done - see ../readme-assets/"
