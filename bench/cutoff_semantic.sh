#!/usr/bin/env bash
# Semantic correctness of cross-crate early cutoff on the Desktop snapshot:
# a temporary UI test asserts on what an upstream function returns. Change
# only that function's body upstream (dependents cut off), rebuild, and the
# test must see the new value, then the old value again. Restores every file.
#
# Usage: bench/cutoff_semantic.sh <toolchain> [justrust]
set -euo pipefail
TC=${1:?toolchain}
J=${2:-justrust}
ROOT=$HOME/.justrust/bench/work/desktop/jcode@abff94eeb-jcode-desktop@7d38fb7
D=$ROOT/jcode-desktop
UP=$D/crates/jcode-desktop-ui-core/src/render_stats.rs
UI=$D/crates/jcode-desktop-ui/src/lib.rs
TMP=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/cutoff-sem.XXXXXX")
cp "$UP" "$TMP/up.bak"; cp "$UI" "$TMP/ui.bak"
restore() { cp "$TMP/up.bak" "$UP"; cp "$TMP/ui.bak" "$UI"; }
trap restore EXIT
export CARGO_TARGET_DIR=$ROOT/target-$TC JUSTRUST_TOOLCHAIN=$TC JUSTRUST_REMOTE=0 JUSTRUST_BUILDD=0 \
  JUSTRUST_PREBUILT=off JUSTRUST_CARGO_CUTOFF_LOG=$TMP/cutoff.log
cat >>"$UI" <<'EOF'

#[cfg(test)]
mod jr_cutoff_semantic {
    #[test]
    fn jr_cutoff_semantic_probe() {
        let marker = jcode_desktop_ui_core::render_stats::snapshot()
            .iter()
            .find(|(n, _)| n.starts_with("jr-marker-"))
            .map(|(n, _)| *n)
            .unwrap_or("none");
        assert_eq!(marker, std::env::var("JR_EXPECT").unwrap());
    }
}
EOF
anchor='            .map(|table| table.views.clone())'
grep -qF "$anchor" "$UP" || { echo "anchor missing"; exit 1; }
set_marker() { # $1 = marker or empty
  cp "$TMP/up.bak" "$UP"
  if [ -n "$1" ]; then
    python3 - "$UP" "$anchor" "$1" <<'EOF'
import sys
p, anchor, m = sys.argv[1:]
s = open(p).read()
s = s.replace(anchor, anchor.replace("table.views.clone()", f'{{ let mut v = table.views.clone(); v.push(("{m}", ViewStat::default())); v }}'), 1)
open(p, "w").write(s)
EOF
  fi
}
run() { # expect
  : >"$TMP/cutoff.log"
  (cd "$D" && JR_EXPECT=$1 "$J" test -p jcode-desktop-ui --lib -- jr_cutoff_semantic_probe >"$TMP/out" 2>&1) \
    || { cat "$TMP/out"; echo "FAIL: expected $1"; exit 1; }
  "$J" log --grep "1 passed" >"$TMP/log" 2>&1; grep -q "1 passed" "$TMP/log" || { cat "$TMP/out"; echo "FAIL: test did not run"; exit 1; }
  echo "ok: sees '$1' (cut off: $(sort -u "$TMP/cutoff.log" | awk '{print $2}' | tr '\n' ' '))"
}
set_marker ""; run none
set_marker jr-marker-a; run jr-marker-a
set_marker jr-marker-b; run jr-marker-b
set_marker ""; run none
echo "cutoff semantic check passed"
