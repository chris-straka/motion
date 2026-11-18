#!/bin/bash
# The tools the creature route calls, built at a known commit on the machine the job runs on
# (sourced by tools/creature.sh; never the machine's working checkouts, which other sessions
# use and may leave stale):
#   source tools/creature_deps.sh
#   -> MOTIONFORGE_BIN SKINTOKENS_BIN CREATURE_RIG_BIN CREATURE_BENCH_BIN WEIGHTS_BIN RFCHECK_BIN BLENDER,
#      CREATURE_DEP_DIRS (repo=checkout, space separated) and CREATURE_DEP_SHAS (repo@sha)
# motion is this checkout (the job's snapshot); skintokens, weightforge (meshweights) and rfcheck
# and charkit (the game step) are cloned from GitHub into $CREATURE_DEPS (default
# ~/.cache/creature-route) at origin/main, or SKINTOKENS_REF=..., WEIGHTFORGE_REF=..., RFCHECK_REF=...,
# CHARKIT_REF=....
# The machine's own settings (where its CUDA toolkit is) come from $CREATURE_ROUTE_ENV, default
# ~/.config/creature-route/env.sh, when that file exists. With CUDA_ROOT set, skintokens is built
# for the NVIDIA card; on macOS it uses Metal; otherwise the CPU.
_cr_here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
_cr_env=${CREATURE_ROUTE_ENV:-$HOME/.config/creature-route/env.sh}
# shellcheck disable=SC1090
[ -f "$_cr_env" ] && . "$_cr_env"
export PATH=$HOME/.cargo/bin:$HOME/.local/bin:$PATH
CREATURE_DEPS=${CREATURE_DEPS:-$HOME/.cache/creature-route}
mkdir -p "$CREATURE_DEPS"

_cr_dep() {  # _cr_dep GITHUB_NAME REF -> checkout path on stdout
  local dir=$CREATURE_DEPS/$1@$(echo "$2" | tr '/' '_')
  if [ ! -d "$dir/.git" ]; then
    git clone -q "https://github.com/chris-straka/$1.git" "$dir" >&2 || return 1
  fi
  git -C "$dir" fetch -q origin '+refs/heads/*:refs/remotes/origin/*' >&2 || return 1
  git -C "$dir" checkout -q --detach "$2" >&2 || return 1
  echo "$dir"
}

_cr_build() {  # _cr_build DIR [cargo args...]: release build into DIR's own target (fleet may point CARGO_TARGET_DIR elsewhere)
  local d=$1; shift
  (cd "$d" && CARGO_TARGET_DIR="$d/target" cargo build --release -q "$@" >&2)
}

_cr_fail() { echo "creature_deps: $*" >&2; return 1; }

SK_DIR=$(_cr_dep skintokens "${SKINTOKENS_REF:-origin/main}") || _cr_fail "no skintokens checkout"
WF_DIR=$(_cr_dep weightforge "${WEIGHTFORGE_REF:-origin/main}") || _cr_fail "no weightforge checkout"
RF_DIR=$(_cr_dep rfcheck "${RFCHECK_REF:-origin/main}") || _cr_fail "no rfcheck checkout"
CK_DIR=$(_cr_dep charkit "${CHARKIT_REF:-origin/main}") || _cr_fail "no charkit checkout"

_cr_build "$_cr_here/rust" --locked || _cr_fail "motionforge did not build"
if [ -n "${CUDA_ROOT:-}" ]; then
  _cr_build "$SK_DIR" -p skintokens --no-default-features --features cuda || _cr_fail "skintokens (cuda) did not build"
  export SKINTOKENS_DEVICE=${SKINTOKENS_DEVICE:-cuda}
elif [ "$(uname)" = Darwin ]; then
  _cr_build "$SK_DIR" -p skintokens || _cr_fail "skintokens (metal) did not build"
else
  _cr_build "$SK_DIR" -p skintokens --no-default-features || _cr_fail "skintokens (cpu) did not build"
fi
_cr_build "$SK_DIR" -p creature-rig -p creature-bench || _cr_fail "creature-rig did not build"
_cr_build "$WF_DIR/rust" || _cr_fail "weights did not build"
_cr_build "$RF_DIR" || _cr_fail "rfcheck did not build"

export MOTIONFORGE_BIN=$_cr_here/rust/target/release/motionforge
export SKINTOKENS_BIN=$SK_DIR/target/release/skintokens
export CREATURE_RIG_BIN=$SK_DIR/target/release/creature-rig
export CREATURE_BENCH_BIN=$SK_DIR/target/release/creature-bench
export WEIGHTS_BIN=$WF_DIR/rust/target/release/weights
export RFCHECK_BIN=$RF_DIR/target/release/rfcheck
export CREATURE_ROUTE=$SK_DIR/tools/creature_route.py
export TRIPO_TO_GLB=$_cr_here/tools/tripo_to_glb.py
if [ -z "${BLENDER:-}" ]; then
  if [ -x /Applications/Blender.app/Contents/MacOS/Blender ]; then
    export BLENDER=/Applications/Blender.app/Contents/MacOS/Blender
  else
    BLENDER=$(command -v blender || true); export BLENDER
  fi
fi
for _b in MOTIONFORGE_BIN SKINTOKENS_BIN CREATURE_RIG_BIN CREATURE_BENCH_BIN WEIGHTS_BIN RFCHECK_BIN BLENDER; do
  [ -x "${!_b}" ] || _cr_fail "$_b missing (${!_b})"
done
export CREATURE_DEP_DIRS="motion=$_cr_here skintokens=$SK_DIR meshweights=$WF_DIR rfcheck=$RF_DIR charkit=$CK_DIR"
CREATURE_DEP_SHAS=""
for _p in $CREATURE_DEP_DIRS; do
  CREATURE_DEP_SHAS="$CREATURE_DEP_SHAS ${_p%%=*}@$(git -C "${_p#*=}" rev-parse --short HEAD)"
done
export CREATURE_DEP_SHAS
echo "creature_deps:$CREATURE_DEP_SHAS (skintokens on ${SKINTOKENS_DEVICE:-its default device})" >&2
