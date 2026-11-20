# Goal: motion-3 — Create `tools/gate/carbon.sh` (guarded Carbon/Neon gate for motion)

Create an executable bash script `tools/gate/carbon.sh` that serves as the reusable guarded gate for automated work in `~/Games/_blender/motion` (matching the pattern used in `skintokens/tools/gate/carbon.sh`).

## Scope

- Create/edit ONLY `tools/gate/carbon.sh` (and make it executable with `chmod +x tools/gate/carbon.sh`).
- Do NOT edit `tools/gate/planted.sh`, `gates/motion-3/`, `rust/`, `tests/`, `tools/bench/`, or docs.
- Must be 100% portable across Linux and macOS (POSIX ERE only in `grep -E`: never `\b` or `\w`; `LC_ALL=C` safe).

## Design

`tools/gate/carbon.sh` must support:
- `bash tools/gate/carbon.sh`: runs the scope check, then formatting, build (zero warnings), full Rust test suite (including CLI contract goldens), and Python stdlib test suites.
- `bash tools/gate/carbon.sh --scope-only`: runs ONLY the scope and test-count checks (used by `tools/gate/planted.sh` to verify every guarded path without rebuilding Rust).
- Prints `PASS  <msg>` or `FAIL  <msg>` for every check, ends with `GATE PASS` (exit 0) or `GATE FAIL (<N>)` (exit 1). Never silently skips a check (missing `cargo` or `python3` is a `FAIL`).

Checks inside `tools/gate/carbon.sh`:

1. **Base commit**:
   - `BASE=${GATE_BASE:-$(git log -1 --format=%H -- tools/gate)}` (fail if empty and fall back to `HEAD`).

2. **Guarded-path scope check**:
   - Collect all files touched by worker commits (`git log --format=%H --grep '^carbon [^ ]*: round' "$BASE..HEAD"` -> `git diff-tree --no-commit-id --name-only -r "$c"`) PLUS any uncommitted changes (`git status --porcelain | cut -c4-`, ignoring blank lines, sorted unique).
   - Fail if ANY touched file matches any guarded path:
     - `rust/core/tests/`
     - `rust/cli/tests/`
     - `tests/fixtures/`
     - `rust/core/src/readcam.rs`
     - `rust/core/src/motioncheck.rs`
     - `tools/bench/`
     - `tools/gate/`
   - Guarded regex (POSIX ERE):
     `^(rust/core/tests/|rust/cli/tests/|tests/fixtures/|rust/core/src/readcam\.rs$|rust/core/src/motioncheck\.rs$|tools/bench/|tools/gate/)`

3. **Test-count & ignore guard**:
   - Count `#[test]` in `rust/` both in the working tree (`grep -rc '#\[test\]' rust --include='*.rs'`) and in git (`git grep -c '#\[test\]' -- rust`) against `$BASE`, and count `#[ignore` in `rust/` against `$BASE`.
   - Fail if working-tree or committed `#[test]` count dropped below `$BASE`, OR if `#[ignore` count increased above `$BASE`.
   - If `$1` is `--scope-only`, exit here with `GATE PASS` (0) or `GATE FAIL ($FAILS)` (1).

4. **Rust format, build, full test suite, and CLI goldens** (offline `--locked`, zero warnings, `UPDATE_GOLDENS` explicitly unset so CLI contract goldens are verified and never overwritten):
   - Require `cargo` on PATH (else FAIL).
   - Use isolated `CARGO_TARGET_DIR` only if not already set by caller (or preserve caller's `CARGO_TARGET_DIR` so fleet/gate target caches work cleanly).
   - `(cd rust && cargo fmt --all --check)` -> PASS/FAIL.
   - `(cd rust && RUSTFLAGS="-D warnings" cargo build --locked --release --all-targets)` -> PASS/FAIL.
   - `(cd rust && env -u UPDATE_GOLDENS RUSTFLAGS="-D warnings" cargo test --locked --release --no-fail-fast)` -> verify exit 0, 0 `test result: FAILED`, and `> 0` passed (this runs all core tests, `detmath`, `skin_grip`, `motion_4_grip`, and CLI contract goldens in `rust/cli/tests/cli_contract.rs` + `adapter_contract.rs`).
   - Run Python stdlib test suites (`blender/tests/test_pure.py`, `python/tests/test_*.py`, `tools/tests/test_*.py`) -> PASS/FAIL.

## Evidence of Done

- `gates/motion-3/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- `bash tools/gate/planted.sh` prints `PLANTED PASS` (every guarded path fails, unguarded control passes).
- `bash tools/gate/carbon.sh` prints `GATE PASS` on the clean tree.
