#!/bin/bash
# One graphics-card command as its own short fleet job, waited for, so the creature route's own
# job never holds the card (fleet gives a card job the whole card for its whole length; the
# route uses it for under a minute per model run):
#   card_job.sh REAL_BINARY ARGS...
# Its output and exit code come back as if it had run here. Every path it is given must be on
# this machine's disk (the card job runs on the machine with the card, which is this one: run the
# route's own job with --linux and without --gpu, or the card job waits for the card it holds).
# It asks for one core and goes before waiting jobs (--priority 5): it is a short child of a job
# that is already running and holding its own cores.
# CREATURE_CARD_LOG, when set, gets a line per card job: seconds waited, seconds run, command.
set -uo pipefail
real=$1; shift
base=${CREATURE_DEPS:-$HOME/.cache/creature-route}
mkdir -p "$base"
d=$(mktemp -d "$base/card.XXXXXX")
envs=()
for v in CUDA_ROOT LD_LIBRARY_PATH SKINTOKENS_DEVICE SKINTOKENS_HOME SKINTOKENS_CKPT SKINTOKENS_TRANSFER SKINTOKENS_DTYPE; do
  [ -n "${!v:-}" ] && envs+=("$v=${!v}")
done
# the card job runs this script: the model's own command, its output and exit code to files here
{
  echo '#!/bin/bash'
  printf 'cd %q || exit 2\n' "$PWD"
  printf 'date +%%s > %q\n' "$d/start"
  for e in "${envs[@]}"; do printf 'export %q\n' "$e"; done
  printf '%q' "$real"; printf ' %q' "$@"; printf ' > %q 2> %q\n' "$d/out" "$d/err"
  printf 'echo $? > %q\n' "$d/code"
} > "$d/run.sh"
t0=$(date +%s)
fleet run --gpu --cores 1 --priority 5 --alongside --repo Games/_blender/skintokens \
  -t "creature route: $(basename "$real") $1 on the card" -w -- bash "$d/run.sh" > "$d/fleet.log" 2>&1
t1=$(date +%s)
cat "$d/out" 2>/dev/null
cat "$d/err" >&2 2>/dev/null
if [ -f "$d/code" ]; then
  code=$(cat "$d/code")
else
  echo "card_job: the card job did not run: $(tail -3 "$d/fleet.log")" >&2
  code=2
fi
if [ -n "${CREATURE_CARD_LOG:-}" ] && [ -f "$d/start" ]; then
  s=$(cat "$d/start")
  printf '%s\t%s\t%s %s\n' $((s - t0)) $((t1 - s)) "$(basename "$real")" "$1" >> "$CREATURE_CARD_LOG"
fi
rm -rf "$d"
exit "$code"
