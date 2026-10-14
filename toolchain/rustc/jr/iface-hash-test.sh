#!/usr/bin/env bash
# Tests -Zjr-iface-hash: which edits keep the interface hash (`<rmeta>.jriface`),
# and that a dependent built against an old build of `up` loads (and links,
# and runs) against a new build with the same interface.
#
# Usage: jr/iface-hash-test.sh [path/to/rustc]   (default build/jr/dev/bin/rustc)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
RUSTC=${1:-$here/build/jr/dev/bin/rustc}
W=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/jr-iface.XXXXXX")
trap 'rm -rf "$W"' EXIT
fails=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }

base='pub struct S { pub a: u32 }
pub fn f(x: u32) -> u32 {
    let _ = 1;
    x + 2
}
#[inline]
pub fn g(x: u32) -> u32 { x * 2 }
pub fn h<T: Clone>(t: &T) -> T { t.clone() }
#[macro_export]
macro_rules! m { () => { 5 } }
pub const C: u32 = 3;
pub fn after() -> u32 { 7 }
pub fn panics() -> u32 { if std::env::var("X").is_ok() { panic!("boom") } 1 }
'

build_up() { # $1 = source, $2 = outdir
  mkdir -p "$2"
  printf '%s' "$1" >"$2/up.rs"
  "$RUSTC" --edition 2021 --crate-type rlib --crate-name up --emit=metadata,link \
    -Zjr-iface-hash -C incremental="$2/incr" --out-dir "$2" "$2/up.rs" \
    --error-format=json --json=artifacts 2>"$2/stderr" || { cat "$2/stderr" >&2; return 1; }
  # the sidecar must exist by the time the rmeta artifact is reported
  grep -q '"artifact":.*libup.rmeta' "$2/stderr" || { echo "no rmeta notification" >&2; return 1; }
  cat "$2/libup.rmeta.jriface"
}

iface0=$(build_up "$base" "$W/v0")
[[ $iface0 =~ ^[0-9a-f]{32}$ ]] && ok "sidecar format ($iface0)" || bad "sidecar format: '$iface0'"
again=$(build_up "$base" "$W/v0b")
[ "$again" = "$iface0" ] && ok "deterministic" || bad "same source, different iface"

check() { # name, expect(same|diff), sed expression
  local src
  src=$(printf '%s' "$base" | sed "$3")
  [ "$src" != "$base" ] || { bad "$1: sed did not change the source"; return; }
  local h
  h=$(build_up "$src" "$W/$1") || { bad "$1: build failed"; return; }
  if [ "$2" = same ] && [ "$h" = "$iface0" ]; then ok "$1 keeps iface"
  elif [ "$2" = diff ] && [ "$h" != "$iface0" ]; then ok "$1 changes iface"
  else bad "$1: expected $2 (base $iface0, got $h)"; fi
}

# body-only edits: same interface
check body same 's/x + 2/x + 3/'
check body-lines same 's/let _ = 1;/let _ = 1;\n    let _ = 2;\n    let _ = 3;/'
check comment-above same 's/^pub struct S/\/\/ a comment\n\/\/ another\npub struct S/'
check panic-msg same 's/"boom"/"bang bang"/'
# interface edits
check signature diff 's/pub fn f(x: u32) -> u32/pub fn f(x: u32) -> u64/;s/x + 2/x as u64 + 2/'
check inline-body diff 's/x \* 2/x * 3/'
check generic-body diff 's/t.clone()/T::clone(t)/'
check macro diff 's/() => { 5 }/() => { 6 }/'
check const diff 's/C: u32 = 3/C: u32 = 4/'
check new-item diff 's/^pub fn after/pub fn added() {}\npub fn after/'
check struct-field diff 's/pub a: u32/pub a: u64/'

# Cross-version load: `down` is built against v0, then loaded against `body`
# (same iface, different crate hash) as a dependency of `top`.
D=$W/link
mkdir -p "$D"
cat >"$D/down.rs" <<'EOF'
pub fn twice() -> u32 { up::f(1) * 2 + up::g(1) + up::m!() + up::C + up::h(&1u32) }
pub fn up_panics() -> u32 { up::panics() }
EOF
cat >"$D/top.rs" <<'EOF'
fn main() { println!("{} {}", down::twice(), down::up_panics()); }
EOF
"$RUSTC" --edition 2021 --crate-type rlib --crate-name down -Zjr-iface-hash \
  --emit=metadata,link --out-dir "$D" -L "$W/v0" --extern up="$W/v0/libup.rlib" "$D/down.rs"
# swap in the body-edited up (f returns x + 3)
"$RUSTC" --edition 2021 --crate-type bin --crate-name top -L "$D" -L "$W/body" \
  --extern down="$D/libdown.rlib" --extern up="$W/body/libup.rlib" -o "$D/top" "$D/top.rs" \
  2>"$D/top.err" && ok "dependent built against old up links with new up" \
  || { bad "link against body-edited up: $(cat "$D/top.err")"; }
if [ -x "$D/top" ]; then
  out=$("$D/top")
  # f(1) = 4 (new body), g(1) = 2, m!() = 5, C = 3, h = 1  => 8+2+5+3+1 = 19
  [ "$out" = "19 1" ] && ok "runs new body ($out)" || bad "unexpected output '$out' (want '19 1')"
  msg=$(X=1 "$D/top" 2>&1 || true)
  echo "$msg" | grep -q "bang bang\|boom" && ok "panic location from swapped up: $(echo "$msg" | grep -o 'up.rs:[0-9:]*' | head -1)" || bad "panic output: $msg"
fi
# interface change must still be rejected
[ -f "$W/signature/libup.rlib" ] || bad "signature variant did not build"
"$RUSTC" --edition 2021 --crate-type bin --crate-name top -L "$D" -L "$W/signature" \
  --extern down="$D/libdown.rlib" --extern up="$W/signature/libup.rlib" -o "$D/top2" "$D/top.rs" \
  2>"$D/top2.err" && bad "signature change accepted" \
  || { grep -q "E0460\|E0463\|incompatible\|found possibly newer" "$D/top2.err" && ok "signature change rejected" || bad "unexpected error: $(head -5 "$D/top2.err")"; }

# Without the flag: no sidecar, stock behavior (crate hash mismatch rejected).
mkdir -p "$W/noflag"; printf '%s' "$base" >"$W/noflag/up.rs"
"$RUSTC" --edition 2021 --crate-type rlib --crate-name up --emit=metadata,link --out-dir "$W/noflag" "$W/noflag/up.rs"
[ ! -e "$W/noflag/libup.rmeta.jriface" ] && ok "no sidecar without the flag" || bad "sidecar without flag"

[ $fails = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
