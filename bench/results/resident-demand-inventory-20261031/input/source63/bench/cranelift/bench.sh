#!/bin/bash
# Usage: bench.sh <variant> <step>
#   variant: llvm | clif (all crates cranelift) | cliflocal (local crates only)
#   step: cold | edit | full | upstream | test   (FILTER=<name> for test, CARGO_CMD=justrust to record)
# Setup (see FINDINGS.md "Cranelift backend"): a rustup nightly with
# rustc-codegen-cranelift-preview under $CLIF_SCRATCH/{rustup,cargo}, and
# rsync copies of ~/jcode and ~/jcode-desktop (without target dirs) under
# $CLIF_SCRATCH/ws. Never point this at the shared Desktop checkout.
set -u
S=${CLIF_SCRATCH:-$HOME/.jcode/scratch/cranelift}
WS=$S/ws/jcode-desktop
variant=$1; step=$2
export CARGO_TARGET_DIR=$S/target-$variant
export RUSTUP_HOME=$S/rustup
export PATH=$S/cargo/bin:$PATH
unset RUSTFLAGS
case $variant in
  llvm) ;;
  clif) export RUSTFLAGS="-Zcodegen-backend=cranelift" ;;
  cliflocal) export CARGO_BUILD_RUSTC_WRAPPER=$(dirname "$(readlink -f "$0")")/clif-local-wrapper ;;
esac
cd $WS
log=$S/logs/$variant-$step-$(date +%H%M%S).log
mkdir -p $S/logs
others=$(pgrep -c rustc)
f=crates/jcode-desktop-ui/src/fps_counter.rs
u=$S/ws/jcode/crates/jcode-sdk/src/lib.rs
case $step in
  edit)
    cp $f $f.bak
    sed -i "s/fn fps_counter_excludes_idle_time_and_limits_snapshot_work() {/&\n    let _probe = $RANDOM;/" $f ;;
  full)
    # Scratch target only: drop desktop-ui incremental cache so the whole crate regenerates.
    rm -rf "$CARGO_TARGET_DIR"/debug/incremental/jcode_desktop_ui-*
    touch crates/jcode-desktop-ui/src/lib.rs ;;
  upstream)
    cp $u $u.bak
    printf '\npub fn __clif_probe_%s() -> u32 { %s }\n' $RANDOM $RANDOM >> $u ;;
esac
start=$(date +%s.%N)
if [ "$step" = test ]; then
  ${CARGO_CMD:-cargo} test -p jcode-desktop-ui --lib -- ${FILTER:-} > $log 2>&1; rc=$?
elif [ "$step" = edit ]; then
  ${CARGO_CMD:-cargo} test -p jcode-desktop-ui --lib -- fps_counter > $log 2>&1; rc=$?
else
  ${CARGO_CMD:-cargo} test -p jcode-desktop-ui --lib --no-run > $log 2>&1; rc=$?
fi
end=$(date +%s.%N)
case $step in
  edit) mv $f.bak $f ;;
  upstream) mv $u.bak $u ;;
esac
echo "$variant $step rc=$rc wall=$(awk "BEGIN{printf \"%.1f\", $end - $start}") others_rustc_at_start=$others log=$log"
