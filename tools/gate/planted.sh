#!/usr/bin/env bash
# Proof that tools/gate/carbon.sh can fail (motion-3): in a throwaway clone,
# one worker commit ("carbon planted: round 1") per guarded path must turn its
# scope check red, and a commit to an unguarded file must leave it green.
# Builds nothing.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
if [ ! -f tools/gate/carbon.sh ]; then
  echo "FAIL  tools/gate/carbon.sh missing"
  echo "PLANTED FAIL (1)"
  exit 1
fi
fails=0
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
git clone -q --no-hardlinks . "$T/r" 2>/dev/null || { echo "FAIL  clone"; exit 1; }
mkdir -p "$T/r/tools/gate" && cp tools/gate/carbon.sh "$T/r/tools/gate/carbon.sh"
cd "$T/r" || exit 2
git add -A && git -c user.name=t -c user.email=t@t commit -q -m "gate under test" --allow-empty

case_() { # name, command, want 0|1
  git reset -q --hard HEAD && git clean -qfd
  base=$(git rev-parse HEAD)
  eval "$2"
  git add -A && git -c user.name=t -c user.email=t@t commit -q -m "carbon planted: round 1"
  GATE_BASE=$base bash tools/gate/carbon.sh --scope-only > out.txt 2>&1
  got=$?
  if [ "$got" = "$3" ]; then
    echo "PASS  $1 (exit $got)"
  else
    echo "FAIL  $1 (exit $got, want $3)"
    sed 's/^/      /' out.txt
    fails=$((fails + 1))
  fi
  git reset -q --hard "$base"
}

# Positive control: unguarded core source file passes scope check
case_ "control: unguarded core source" 'echo "// planted" >> rust/core/src/clean.rs' 0

# Every guarded path from docs/plans/motion.md (motion-3)
case_ "rust/core/tests/" 'echo "// planted" >> rust/core/tests/attack.rs' 1
case_ "rust/cli/tests/" 'echo "// planted" >> rust/cli/tests/cli_contract.rs' 1
case_ "tests/fixtures/" 'echo "" >> tests/fixtures/hero_map.json' 1
case_ "rust/core/src/readcam.rs" 'echo "// planted" >> rust/core/src/readcam.rs' 1
case_ "rust/core/src/motioncheck.rs" 'echo "// planted" >> rust/core/src/motioncheck.rs' 1
case_ "tools/bench/" 'echo "# planted" >> tools/bench/grip_gate.sh' 1
case_ "tools/gate/" 'echo "# planted" >> tools/gate/carbon.sh' 1

# Uncommitted dirty edit to a guarded path must also fail
git reset -q --hard HEAD && git clean -qfd
base=$(git rev-parse HEAD)
echo "// dirty" >> rust/core/src/readcam.rs
GATE_BASE=$base bash tools/gate/carbon.sh --scope-only > out.txt 2>&1
got=$?
if [ "$got" = "1" ]; then
  echo "PASS  uncommitted edit to readcam.rs (exit 1)"
else
  echo "FAIL  uncommitted edit to readcam.rs (exit $got, want 1)"
  sed 's/^/      /' out.txt
  fails=$((fails + 1))
fi
git reset -q --hard "$base"

# Removed #[test] in an unguarded file must fail
case_ "a #[test] removed" 'python3 -c "
import glob
for f in sorted(glob.glob(\"rust/core/src/*.rs\")):
    s = open(f).read()
    if \"#[test]\" in s:
        open(f, \"w\").write(s.replace(\"#[test]\", \"\", 1)); break"' 1

# Added #[ignore] in an unguarded file must fail
case_ "an #[ignore] added" 'python3 -c "
import glob
for f in sorted(glob.glob(\"rust/core/src/*.rs\")):
    s = open(f).read()
    if \"#[test]\" in s:
        open(f, \"w\").write(s.replace(\"#[test]\", \"#[test]\n#[ignore]\", 1)); break"' 1

if [ "$fails" -eq 0 ]; then
  echo "PLANTED PASS"
  exit 0
fi
echo "PLANTED FAIL ($fails)"
exit 1
