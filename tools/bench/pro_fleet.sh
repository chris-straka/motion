#!/usr/bin/env bash
# The pro correctness bench on a fleet machine: fetch the CC0 clips,
# build, run the planted bad-clip self-check, calibrate on the pros, and
# run every stage onto a rigged character and Quaternius' female mannequin
# (plus the held-out Mixamo set when present). Stage GLBs stay on the
# machine ($FORGE_BENCH/pro/<tag>); the tables go to $FLEET_ARTIFACTS.
#
#   fleet run --data motion-mixamo-heldout --alongside --cores 6 --snapshot -- \
#       bash tools/bench/pro_fleet.sh NAME RIG [RIG_JOB]
#
# NAME labels the target in the tables. RIG is the rigged GLB (copied,
# never edited); with RIG_JOB, RIG is the name of a file in that fleet
# job's artifacts (a rig newer than this machine's checkout of the game),
# fetched first.
set -euo pipefail
NAME=$1
RIG=$2
FB=${FORGE_BENCH:-$HOME/.cache/forge-bench}
OUT=${FLEET_ARTIFACTS:-bench/pro/out}
TAG=${TAG:-$(date +%F)}
WEIGHTFORGE=${WEIGHTFORGE:-/nonexistent} bash tools/bench/fetch.sh
(cd rust && cargo build --locked --release -j "${CORES:-6}" 2>&1 | tail -1)
# fleet may share a build folder between worktrees (CARGO_TARGET_DIR).
export MOTIONFORGE_BIN=${CARGO_TARGET_DIR:-$PWD/rust/target}/release/motionforge
mkdir -p "$FB/targets" "$OUT"
if [ -n "${3:-}" ]; then
  fleet artifact "$3" "$RIG" -o "$FB/targets/$NAME.glb"
  RIG=$FB/targets/$NAME.glb
fi
W=$FB/pro/$TAG
mkdir -p "$W"
LIBS=${LIBS:-ual1,ual2}
if [ -n "${FLEET_DATA_MOTION_MIXAMO_HELDOUT:-}" ] && [ -f "$FLEET_DATA_MOTION_MIXAMO_HELDOUT/mixamo_heldout.glb" ]; then
  case ",$LIBS," in
    *,mixamo,*) ;;
    *) LIBS="$LIBS,mixamo" ;;
  esac
elif [ -f "$FB/clips/mixamo/mixamo_heldout.glb" ]; then
  case ",$LIBS," in
    *,mixamo,*) ;;
    *) LIBS="$LIBS,mixamo" ;;
  esac
fi
nice -n 10 python3 tools/bench/pro_bench.py planted --out "$W/planted" | tee "$OUT/planted.txt"
nice -n 10 python3 tools/bench/pro_bench.py calibrate --libs "$LIBS" --out "$W/cal" > "$W.cal.log"
MF=--mannequin-f
[ "${MANNEQUIN_F:-1}" = 1 ] || MF=
nice -n 10 python3 tools/bench/pro_bench.py run --target "$NAME=$RIG" $MF \
  --libs "$LIBS" --jobs "${JOBS:-3}" --out "$W/run"
cp "$W/cal/calib.md" "$W/cal/calib.json" "$W/run/results.json" "$W/run/report.md" "$OUT/"
if [ -f "$W/planted/planted.json" ]; then
  cp "$W/planted/planted.json" "$OUT/"
fi
echo "stage GLBs: $W/run"
