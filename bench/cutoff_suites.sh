#!/usr/bin/env bash
# Full-suite equivalence for cross-crate cutoff: after a body edit deep in the
# graph (jcode-core), run every Desktop workspace test with the cutoff on and
# off (separate target dirs) and compare the per-test results.
# Usage: bench/cutoff_suites.sh <toolchain> [justrust] [desktop|jcode]
# jcode: run the Jcode workspace's tests from the same snapshot (same edit).
set -euo pipefail
TC=${1:?toolchain}
J=${2:-justrust}
ROOT=$HOME/.justrust/bench/work/desktop/jcode@abff94eeb-jcode-desktop@7d38fb7
WHICH=${3:-desktop}
D=$ROOT/jcode-desktop
[ "$WHICH" = jcode ] && D=$ROOT/jcode
F=$ROOT/jcode/crates/jcode-core/src/id.rs
OUT=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/cutoff-suites.XXXXXX")
cp "$F" "$OUT/id.rs.bak"
trap 'cp "$OUT/id.rs.bak" "$F"' EXIT
exec 9>"$ROOT/../.bench.lock"; flock 9
# The local crate cache would restore cut-off crates' dependents by content and hide
# the cutoff; the edit is unique per run anyway.
export JUSTRUST_TOOLCHAIN=$TC JUSTRUST_REMOTE=0 JUSTRUST_BUILDD=0 JUSTRUST_PREBUILT=off JUSTRUST_LOCALCACHE=0
edit() { # a unique body edit
  python3 - "$F" <<'EOF'
import sys, time
p = sys.argv[1]
s = open(p).read()
a = "pub fn session_icon(name: &str) -> &'static str {\n"
assert s.count(a) == 1
open(p, "w").write(s.replace(a, a + f"    std::hint::black_box({time.time_ns()}u64);\n", 1))
EOF
}
run() { # name, extra env
  local name=$1; shift
  # Warm build with one body edit, then a second body edit and the measured suite.
  # (Going from the pristine source to the first edit can change the interface:
  # a small fn is cross-crate inlinable, its MIR is exported.)
  cp "$OUT/id.rs.bak" "$F"; edit
  (cd "$D" && env "$@" CARGO_TARGET_DIR=$ROOT/target-suites-$WHICH-$name "$J" test --workspace --no-fail-fast --no-run >/dev/null 2>&1) || true
  cp "$OUT/id.rs.bak" "$F"; edit
  (cd "$D" && env "$@" CARGO_TARGET_DIR=$ROOT/target-suites-$WHICH-$name JUSTRUST_CARGO_CUTOFF_LOG=$OUT/cutoff-$name.log \
    "$J" test --workspace --no-fail-fast >"$OUT/$name.out" 2>&1) || true
  "$J" log >"$OUT/$name.log" 2>&1 || true
  grep -aE '^test .* \.\.\. ' "$OUT/$name.log" >"$OUT/$name.tests" || true
  grep -a 'test result' "$OUT/$name.log" >"$OUT/$name.results" || true
  # Drop timings; known timing-sensitive tests (they fail now and then with the
  # cutoff on or off, and pass on rerun) are reported separately.
  flaky='the_hint_chip_uses_only_spare_tab_space|bundled_github_applet_renders_valid_documents'
  sed -E 's/ <[0-9.]+s>//' "$OUT/$name.tests" | grep -vE "$flaky" | sort >"$OUT/$name.sorted"
  grep -aE "$flaky" "$OUT/$name.tests" | sed "s/^/  flaky ($name): /" || true
  echo "$name: $(wc -l <"$OUT/$name.sorted") tests, $(grep -c ' ok$' "$OUT/$name.sorted" || true) ok, $(grep -c 'FAILED' "$OUT/$name.sorted" || true) failed, cut off: $(sort -u "$OUT/cutoff-$name.log" 2>/dev/null | wc -l) units"
}
run on
run off JUSTRUST_CUTOFF=0 JUSTRUST_CARGO_CUTOFF=0
if diff -q "$OUT/on.sorted" "$OUT/off.sorted" >/dev/null; then
  echo "identical per-test results ($OUT)"
else
  echo "DIFFERENT per-test results:"; diff "$OUT/on.sorted" "$OUT/off.sorted" | head -40; exit 1
fi
