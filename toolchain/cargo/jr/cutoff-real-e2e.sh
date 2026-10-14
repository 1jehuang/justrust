#!/usr/bin/env bash
# End-to-end: jr-cargo cross-crate early cutoff with the real jr-rustc
# (-Zjr-iface-hash), no fake wrapper. Real body edits in `up`, then a full
# build: `down` (and `mid`) must be cut off, `top` relinked, and the binary
# must run the new body. Interface edits must rebuild dependents. Tests run.
#
# Usage: jr/cutoff-real-e2e.sh <jr-cargo> <jr-rustc>
set -euo pipefail
CARGO_BIN=${1:?jr-cargo}
export RUSTC=${2:?jr-rustc}
WS=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/jr-cutoff-real.XXXXXX")
trap '[ -n "${KEEP:-}" ] || rm -rf "$WS"' EXIT
export CARGO_TARGET_DIR=$WS/target JUSTRUST_CARGO_CUTOFF_LOG=$WS/cutoff.log
export RUSTFLAGS="-Zjr-iface-hash" RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=1
fail() { echo "FAIL: $*" >&2; cat "$WS/out.txt" >&2 || true; exit 1; }
pass() { echo "ok: $*"; }
cd "$WS"
mkdir -p up/src mid/src down/src top/src
cat >Cargo.toml <<'EOF'
[workspace]
members = ["up", "mid", "down", "top"]
resolver = "2"
EOF
for c in up mid down top; do
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n[dependencies]\n' $c >$c/Cargo.toml
done
echo 'up = { path = "../up" }' >>mid/Cargo.toml
echo 'up = { path = "../up" }' >>down/Cargo.toml
echo 'mid = { path = "../mid" }' >>down/Cargo.toml
echo 'down = { path = "../down" }' >>top/Cargo.toml
cat >up/src/lib.rs <<'EOF'
pub struct P { pub v: u32 }
pub fn val() -> u32 {
    1
}
#[inline]
pub fn inl() -> u32 { 10 }
pub fn gen<T: Into<u64>>(t: T) -> u64 { t.into() * 100 }
pub fn loc() -> u32 { line!() }
#[cfg(test)]
mod tests { #[test] fn t() { assert!(super::val() > 0); } }
EOF
cat >mid/src/lib.rs <<'EOF'
pub fn mid() -> u32 { up::val() + 1 }
// A glob re-export: `top` sees up's items through mid's module children, whose
// spans point into up's file (incremental ICE before jr-rustc's incr_hash).
pub mod reexp { pub use up::*; }
EOF
cat >down/src/lib.rs <<'EOF'
pub fn total() -> u64 {
    (up::val() + mid::mid() + up::inl()) as u64 + up::gen(1u8)
}
pub fn p() -> up::P { up::P { v: up::val() } }
#[cfg(test)]
mod tests { #[test] fn t() { assert_eq!(super::p().v, up::val()); } }
EOF
cat >top/src/main.rs <<'EOF'
fn main() { println!("{} {}", down::total(), down::p().v); }
#[allow(dead_code)]
fn uses_reexport() -> u32 { mid::reexp::loc() + mid::reexp::inl() }
EOF
echo 'mid = { path = "../mid" }' >>top/Cargo.toml

compiled() { { grep -o 'Compiling [a-z]*' "$WS/out.txt" || true; } | awk '{print $2}' | sort | tr '\n' ' '; }
C() { : >"$WS/cutoff.log"; "$CARGO_BIN" "$@" >"$WS/out.txt" 2>&1 || fail "cargo $*"; }
expect() { local got; got=$(compiled); [ "$got" = "$1" ] || fail "$2: compiled '$got', expected '$1'"; pass "$2 (compiled: ${1:-nothing})"; }
run() { local out; out=$("$WS/target/debug/top"); [ "$out" = "$1" ] || fail "top printed '$out', expected '$1'"; pass "top runs: $out"; }

"$CARGO_BIN" -V; "$RUSTC" -V
C build; expect "down mid top up " "cold"; run "113 1"
C build; expect "" "no-op"

# body edit: val 1 -> 2 (adds a line too: spans move)
sed -i 's|^    1$|    // two\n    2|' up/src/lib.rs
C build; expect "top up " "body edit => mid, down cut off"
grep -q "skip down" cutoff.log && grep -q "skip mid" cutoff.log || fail "cutoff log: $(cat cutoff.log)"
run "115 2"   # val 2 + mid 3 + 10 + 100
C build; expect "" "no-op after cutoff"

# second body edit in a row (record must chain)
sed -i 's/^    2$/    3/' up/src/lib.rs
C build; expect "top up " "second body edit => cut off again"; run "117 3"

# inline body: dependents must rebuild
sed -i 's/{ 10 }/{ 20 }/' up/src/lib.rs
C build; expect "down mid top up " "inline body edit => rebuild"; run "127 3"

# generic body
sed -i 's/t.into() \* 100/t.into() * 1000/' up/src/lib.rs
C build; expect "down mid top up " "generic body edit => rebuild"; run "1027 3"

# signature
sed -i 's/pub struct P { pub v: u32 }/pub struct P { pub v: u32, pub w: u8 }/' up/src/lib.rs
sed -i 's/up::P { v: up::val() }/up::P { v: up::val(), w: 0 }/' down/src/lib.rs
C build; expect "down mid top up " "struct change => rebuild"; run "1027 3"

# tests: build test harnesses, then a body edit, then tests again
C test --workspace --lib --bins --tests; pass "tests pass"
sed -i 's/^    3$/    4/' up/src/lib.rs
C test --workspace --lib --bins --tests; grep -q "skip down" cutoff.log || fail "test: down lib not cut off: $(cat cutoff.log)"
pass "test after body edit: $(compiled | tr -s ' ')(down lib cut off)"
C build; run "1029 4"

# check mode
C check; sed -i 's/^    4$/    5/' up/src/lib.rs
C check; grep -q "skip down" cutoff.log || fail "check: no cutoff: $(cat cutoff.log)"; pass "check cutoff"

# off switch
sed -i 's/^    5$/    6/' up/src/lib.rs
JUSTRUST_CARGO_CUTOFF=0 C build; expect "down mid top up " "JUSTRUST_CARGO_CUTOFF=0 => stock"; run "1033 6"
echo "all real cutoff e2e checks passed"
