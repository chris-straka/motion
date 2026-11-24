#!/bin/bash
# One Tripo creature to a game-ready monster, headless, every monster gate:
#   tools/creature.sh NAME MODEL OUT
# NAME is its name in the game (snake case), MODEL its Tripo export (.fbx, or a .glb), OUT a folder.
# Builds the route's tools at their origin/main on this machine (tools/creature_deps.sh), then runs
# tools/creature_run.py: the route (tools/tripo_rig.py --kind creature --class monster), the gate
# checks (tools/creature_gates.py), a second run, the planted inputs, the films and a blind judge,
# and charkit's game step. Writes OUT/gates.json. Exit 0 only when every gate passes; 1 when one
# fails; 2 when it could not start. Run it through fleet: fleet run --snapshot --gpu --cores 8 -- ...
set -uo pipefail
[ $# -eq 3 ] || { echo "usage: tools/creature.sh NAME MODEL OUT" >&2; exit 2; }
name=$1
model=$(cd "$(dirname "$2")" 2>/dev/null && pwd)/$(basename "$2")
out=$3; mkdir -p "$out" && out=$(cd "$out" && pwd)
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck disable=SC1091
source "$here/tools/creature_deps.sh" || { echo "creature.sh: the route's tools did not build" >&2; exit 2; }
exec python3 "$here/tools/creature_run.py" "$name" "$model" "$out"
