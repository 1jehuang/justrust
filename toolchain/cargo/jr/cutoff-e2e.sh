#!/usr/bin/env bash
# End-to-end test of cross-crate early cutoff (src/compiler/fingerprint/jr_cutoff.rs).
#
# Scratch workspace up -> down -> top (bin). A fake RUSTC_WRAPPER plays the
# part of jr-rustc: it runs the real rustc and writes `<rmeta>.jriface` as soon
# as rustc reports the rmeta artifact (before cargo sees the notification,
# like jr-rustc must). The interface hash of crate X is the content of
# $WS/iface-X (default all zeros), so a test can simulate "body-only edit"
# (keep the file) or "interface change" (change it). The wrapper logs every
# real compile to $LOG.
#
# With stock rustc a dependent cut off after a *real* body edit fails to load
# (E0460, the dependency's crate hash changed); jr-rustc accepts it. So the
# body-edit case is checked up to `down` (`build -p down`), and the full
# down -> top relink is checked with identical-source rebuilds (touch), where
# stock rustc's crate hash is unchanged too.
#
# Usage: jr/cutoff-e2e.sh [path/to/jr-cargo]   (default target/release/cargo)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
CARGO_BIN=${1:-$here/target/release/cargo}
TC=${JR_TOOLCHAIN:-nightly-2026-10-08}
export RUSTC=${RUSTC:-${JUSTRUST_HOME:-$HOME/.justrust}/toolchains/$TC/bin/rustc}
WS=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/jr-cutoff-e2e.XXXXXX")
LOG=$WS/rustc.log
trap 'rm -rf "$WS"' EXIT
fail() { echo "FAIL: $*" >&2; echo "--- rustc log:" >&2; cat "$LOG" >&2 || true; exit 1; }
pass() { echo "ok: $*"; }

mkdir -p "$WS"/{up,down,top}/src
cat >"$WS/Cargo.toml" <<'EOF'
[workspace]
members = ["up", "down", "top"]
resolver = "2"
EOF
for c in up down top; do
  cat >"$WS/$c/Cargo.toml" <<EOF
[package]
name = "$c"
version = "0.1.0"
edition = "2021"
[dependencies]
EOF
done
echo 'up = { path = "../up" }' >>"$WS/down/Cargo.toml"
echo 'down = { path = "../down" }' >>"$WS/top/Cargo.toml"
cat >"$WS/up/src/lib.rs" <<'EOF'
pub fn val() -> u32 { 1 }
EOF
cat >"$WS/down/src/lib.rs" <<'EOF'
pub fn twice() -> u32 { up::val() * 2 }
EOF
cat >"$WS/top/src/main.rs" <<'EOF'
fn main() { println!("top={}", down::twice()); }
EOF

cat >"$WS/wrapper.sh" <<'EOF'
#!/usr/bin/env bash
# fake jr-rustc: real rustc, plus a .jriface sidecar per emitted rmeta.
rustc=$1; shift
name= ; outdir= ; extra=
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[i]}" in
    --crate-name) name=${args[i+1]} ;;
    --out-dir) outdir=${args[i+1]} ;;
    -C) case "${args[i+1]}" in extra-filename=*) extra=${args[i+1]#extra-filename=} ;; esac ;;
  esac
done
if [ -z "$name" ] || [ "$name" = ___ ]; then exec "$rustc" "$@"; fi
echo "$name $*" >>"$JR_E2E_LOG"
if [ "${JR_E2E_SIDECARS:-1}" = 0 ]; then exec "$rustc" "$@"; fi
iface=$(cat "$JR_E2E_WS/iface-$name" 2>/dev/null || printf '0%.0s' {1..32})
rmeta="$outdir/lib$name$extra.rmeta"
set -o pipefail
# Write the sidecar before forwarding rustc's rmeta artifact notification.
{ "$rustc" "$@" 2>&1 1>&3 3>&- | while IFS= read -r line; do
    case "$line" in
      *'"artifact":'*'.rmeta"'*) printf '%s\n' "$iface" >"$rmeta.jriface" ;;
    esac
    printf '%s\n' "$line" >&2
  done; } 3>&1
status=$?
# rustc may emit no artifact notification (no --json): write it afterwards.
if [ $status = 0 ] && [ -f "$rmeta" ] && [ ! -f "$rmeta.jriface" ]; then
  printf '%s\n' "$iface" >"$rmeta.jriface"
fi
exit $status
EOF
chmod +x "$WS/wrapper.sh"
export RUSTC_WRAPPER=$WS/wrapper.sh JR_E2E_LOG=$LOG JR_E2E_WS=$WS
export CARGO_TARGET_DIR=$WS/target JUSTRUST_CARGO_CUTOFF_LOG=$WS/cutoff.log
cd "$WS"

C() { : >"$LOG"; : >"$WS/cutoff.log"; "$CARGO_BIN" "$@" >"$WS/out.txt" 2>&1 || { cat "$WS/out.txt" >&2; fail "cargo $*"; }; }
compiled() { cut -d' ' -f1 "$LOG" | sort -u | tr '\n' ' '; }
expect() { local got; got=$(compiled); [ "$got" = "$1" ] || fail "$2: compiled '$got', expected '$1'"; pass "$2 (compiled: ${1:-nothing})"; }
run_top() { local out; out=$("$WS/target/debug/top"); [ "$out" = "top=$1" ] || fail "top printed '$out', expected top=$1"; }
bump() { sleep 0.05; touch "$@"; }

"$CARGO_BIN" -V
# 1. cold build, then no-op
C build; expect "down top up " "cold build"; run_top 2
C build; expect "" "no-op build"

# 2. identical-source rebuild of up: down cut off, top relinked and runs
bump up/src/lib.rs
C build; expect "top up " "touch up => down cut off"
grep -q "skip down" "$WS/cutoff.log" || fail "cutoff log has no 'skip down'"
run_top 2
C build; expect "" "no-op after cutoff (outputs mtimes bumped)"

# 3. body-only edit in up (interface hash unchanged): down cut off
sed -i 's/{ 1 }/{ 1 + 0 }/' up/src/lib.rs
C build -p down; expect "up " "body edit in up => down cut off (build -p down)"
C build -p down; expect "" "no-op build -p down"

# 4. interface change in up: down rebuilt, top relinked and runs
echo 11111111111111111111111111111111 >"$WS/iface-up"
sed -i 's/{ 1 + 0 }/{ 3 }/' up/src/lib.rs
C build; expect "down top up " "iface change in up => down rebuilt"
run_top 6

# 5. cargo check units
C check; expect "down top up " "cold check"
bump up/src/lib.rs
C check; expect "top up " "check: touch up => down cut off"
C check; expect "" "check no-op after cutoff"

# 6. own source change in down is never cut off
bump up/src/lib.rs down/src/lib.rs
C build; expect "down top up " "touch up+down => down rebuilt"

# 7. JUSTRUST_CARGO_CUTOFF=0: stock behaviour
bump up/src/lib.rs
JUSTRUST_CARGO_CUTOFF=0 C build; expect "down top up " "JUSTRUST_CARGO_CUTOFF=0 => down rebuilt"
bump up/src/lib.rs
C build; expect "down top up " "first build after a disabled run records again"
bump up/src/lib.rs
C build; expect "top up " "then cuts off again"

# 8. no sidecars (stock rustc): stock behaviour
export JR_E2E_SIDECARS=0
bump up/src/lib.rs
C build; expect "down top up " "no sidecar => down rebuilt"
bump up/src/lib.rs
C build; expect "down top up " "no sidecar, again => down rebuilt"
run_top 6
unset JR_E2E_SIDECARS

# 9. deleted target dir
rm -rf "$WS/target"
C build; expect "down top up " "deleted target dir => full build"
run_top 6
bump up/src/lib.rs
C build; expect "top up " "after cold rebuild, cutoff works"
run_top 6

echo "all cutoff e2e checks passed"
