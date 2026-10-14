#!/usr/bin/env bash
# Tests JUSTRUST_RUSTC_TEST_FILTER (test-filter-aware harness).
# Usage: jr/test-filter-test.sh [rustc]
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
R=${1:-$here/build/jr/dev/bin/rustc}
W=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/jr-tf.XXXXXX")
fails=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
cat >"$W/t.rs" <<'EOF'
fn only_for_alpha() -> u32 { 1 }
fn shared() -> u32 { 2 }
pub fn lib_fn() -> u32 { shared() }
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn alpha_one() { assert_eq!(only_for_alpha() + shared(), 3); }
    #[test] fn alpha_two() { assert_eq!(shared(), 2); }
    #[test] fn beta() { assert_eq!(lib_fn(), 2); }
    #[test] #[should_panic] fn gamma_panics() { panic!("x") }
    #[test] #[ignore] fn ignored_alpha() {}
    mod nested { #[test] fn alpha_deep() {} }
}
EOF
build() { # out, filter(or -), extra
  local out=$1 f=$2; shift 2
  if [ "$f" = - ]; then env -u JUSTRUST_RUSTC_TEST_FILTER "$R" --edition 2021 --test -o "$W/$out" "$W/t.rs" "$@" 2>"$W/$out.err"
  else JUSTRUST_RUSTC_TEST_FILTER="$f" "$R" --edition 2021 --test -o "$W/$out" "$W/t.rs" "$@" 2>"$W/$out.err"; fi
}
list() { "$W/$1" --list 2>/dev/null | grep ': test$' | sed 's/: test$//' | sort | tr '\n' ' '; }

build all -; [ ! -s "$W/all.err" ] && ok "no warnings without filter" || bad "warnings: $(cat "$W/all.err")"
[ "$(list all)" = "tests::alpha_one tests::alpha_two tests::beta tests::gamma_panics tests::ignored_alpha tests::nested::alpha_deep " ] && ok "unfiltered lists all" || bad "unfiltered: $(list all)"

build a alpha; [ ! -s "$W/a.err" ] && ok "no warnings with filter (no dead_code from dropped tests)" || bad "filter warnings: $(cat "$W/a.err")"
[ "$(list a)" = "tests::alpha_one tests::alpha_two tests::ignored_alpha tests::nested::alpha_deep " ] && ok "filter alpha: $(list a)" || bad "filter alpha: $(list a)"
out=$("$W/a" alpha 2>&1); echo "$out" | grep -q "3 passed; 0 failed; 1 ignored" && ok "runs filtered tests" || bad "run: $out"

build ab "$(printf 'alpha_one\nbeta')"
[ "$(list ab)" = "tests::alpha_one tests::beta " ] && ok "two filters" || bad "two filters: $(list ab)"

build ex "$(printf -- '--exact\ntests::beta')"
[ "$(list ex)" = "tests::beta " ] && ok "exact" || bad "exact: $(list ex)"

build none nomatch
[ -z "$(list none)" ] && ok "no match: empty harness" || bad "no match: $(list none)"

# dead_code must still fire for code no test (selected or not) uses
cat >>"$W/t.rs" <<'EOF'
fn really_dead() {}
EOF
build dead alpha; grep -q "really_dead" "$W/dead.err" && ok "real dead code still warned" || bad "dead_code lost: $(cat "$W/dead.err")"
# type errors in unselected tests are still reported
cat >>"$W/t.rs" <<'EOF'
#[test] fn beta_broken() { let _x: u32 = "no"; }
EOF
build err alpha && bad "type error in unselected test accepted" || { grep -q "E0308" "$W/err.err" && ok "type error in unselected test reported" || bad "err: $(cat "$W/err.err")"; }

# codegen: unselected tests are not compiled to code
sed -i '/really_dead\|beta_broken/d' "$W/t.rs"
build sym1 - -C opt-level=0; build sym2 alpha_one -C opt-level=0
n1=$(nm "$W/sym1" | grep -c "gamma_panics\|beta" || true); n2=$(nm "$W/sym2" | grep -c "gamma_panics\|beta" || true)
[ "$n2" = 0 ] && [ "$n1" -gt 0 ] && ok "unselected tests not codegened ($n1 -> $n2 symbols)" || bad "symbols $n1 -> $n2"

# env dependency recorded in dep-info
JUSTRUST_RUSTC_TEST_FILTER=alpha "$R" --edition 2021 --test --emit=dep-info,link -o "$W/dep" "$W/t.rs" 2>/dev/null
grep -q "# env-dep:JUSTRUST_RUSTC_TEST_FILTER=alpha" "$W/dep.d" && ok "env-dep recorded" || bad "dep-info: $(cat "$W/dep.d")"

[ $fails = 0 ] && { rm -rf "$W"; echo "all passed"; } || { echo "$fails failed (kept $W)"; exit 1; }
