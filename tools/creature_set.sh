#!/bin/bash
# Run tools/creature.sh over one of the fixed sets in tools/creature_sets.json:
#   tools/creature_set.sh dev|held_out MAP OUTROOT [id ...]
# MAP is the machine's own list of where each file is (`id <tab> path <tab> game name`), kept off
# the repo. A file whose sha256 differs from the committed list is refused, so the sets can't be
# swapped. Each creature goes to OUTROOT/<game name>/ (gates.json there); a line per creature, and
# OUTROOT/summary.json. The dev set runs with CREATURE_SET=dev (no game step for benchmark animals).
set -uo pipefail
[ $# -ge 3 ] || { echo "usage: tools/creature_set.sh dev|held_out MAP OUTROOT [id ...]" >&2; exit 2; }
set_name=$1; map=$2; root=$3; shift 3
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$root"
[ "$set_name" = dev ] && export CREATURE_SET=dev
python3 - "$here/tools/creature_sets.json" "$set_name" "$map" "$@" > "$root/plan.tsv" <<'PY' || exit 2
import hashlib, json, sys
sets = json.load(open(sys.argv[1]))
key = {"dev": "dev", "held_out": "held_out"}[sys.argv[2]]
where = {}
for ln in open(sys.argv[3]):
    if ln.strip() and not ln.startswith("#"):
        parts = ln.rstrip("\n").split("\t")
        where[parts[0]] = (parts[1], parts[2] if len(parts) > 2 else parts[0].replace(" ", "_").replace("-", "_"))
only = set(sys.argv[4:])
bad = False
for item in sets[key]:
    if only and item["id"] not in only:
        continue
    if item["id"] not in where:
        print(f"{item['id']}: not in the map", file=sys.stderr); bad = True; continue
    path, game = where[item["id"]]
    h = hashlib.sha256(open(path, "rb").read()).hexdigest()
    if h != item["sha256"]:
        print(f"{item['id']}: sha256 {h[:12]} is not the fixed file's {item['sha256'][:12]}", file=sys.stderr); bad = True; continue
    print(f"{item['id']}\t{game}\t{path}")
sys.exit(1 if bad else 0)
PY
while IFS=$'\t' read -r id game path; do
  t0=$(date +%s)
  bash "$here/tools/creature.sh" "$game" "$path" "$root/$game" > "$root/$game.log" 2>&1
  code=$?
  echo "$id ($game): exit $code in $(( $(date +%s) - t0 )) s: $(tail -1 "$root/$game.log")"
done < "$root/plan.tsv"
# fleet keeps 200 files and no folders: each creature's evidence goes in one tar
if [ -n "${FLEET_ARTIFACTS:-}" ]; then
  while IFS=$'\t' read -r id game path; do
    (cd "$root" && tar -cf "$FLEET_ARTIFACTS/$game.tar" --exclude='*/run1/*' --exclude='*/run2/*' --exclude='*/frames/*' \
       --exclude='*/planted/*' --exclude='*.glb' "$game/gates.json" "$game.log" $(cd "$root" && ls -d "$game"/gate6 "$game"/gate9 2>/dev/null) \
       && tar -rf "$FLEET_ARTIFACTS/$game.tar" "$game/$game.glb" "$game/run1/route.log" "$game/run1/report.json" 2>/dev/null)
  done < "$root/plan.tsv"
fi
python3 - "$root" "$root/plan.tsv" > "$root/summary.json" <<'PY'
import json, os, sys
root, rows = sys.argv[1], [ln.rstrip("\n").split("\t") for ln in open(sys.argv[2]) if ln.strip()]
out = {}
for id_, game, _ in rows:
    try:
        g = json.load(open(os.path.join(root, game, "gates.json")))
        out[id_] = {"pass": g.get("pass"), "failed": g.get("failed"), "not_run": g.get("not_run"),
                    "gates": {k: v.get("pass") for k, v in g["gates"].items()}}
    except (OSError, ValueError, KeyError) as e:
        out[id_] = {"pass": False, "error": str(e)}
print(json.dumps(out, indent=1))
PY
