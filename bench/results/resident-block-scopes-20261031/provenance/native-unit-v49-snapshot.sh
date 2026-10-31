#!/bin/bash
set -euo pipefail
ROOT=/home/jeremy/.jcode/scratch/blaze-native-engine-20261030
TC=/home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu
UNIT=$ROOT/candidate-unit-v49-snapshot
cd /home/jeremy/justrust
sha256sum -c "$ROOT/block-scopes-v49/candidate.sha256"
python3 "$ROOT/prepare-unit-v49-snapshot.py"
trap 'code=$?; printf "%s\n" "$code" > "$UNIT/orchestration-exit"' EXIT
export PATH="$TC/bin:$PATH" JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_PROCMACRO_CACHE=0 JUSTRUST_DEPCACHE=0 JUSTRUST_LOCALCACHE=0 JUSTRUST_PREBUILT=off JUSTRUST_BUILD_SCRIPT_CACHE=0
export JUSTRUST_HOME="$UNIT/justrust-home" JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc" CARGO_TARGET_DIR="$ROOT/candidate-build-v2/target"
unset RA_JIT_GIT_SHA
run_gate() {
 local name=$1
 shift
 set +e
 "$@" > "$UNIT/$name.log" 2>&1
 local code=$?
 set -e
 printf '%s\n' "$code" > "$UNIT/$name.exit"
 if [ "$code" != 0 ]; then exit "$code"; fi
}
run_gate block-scopes-native justrust test --manifest-path "$UNIT/vendor/rust-analyzer/Cargo.toml" --release --offline --locked -p hir-def --lib -j 6 -- resident_block_scopes --nocapture
run_gate full-hir-def-native justrust test --manifest-path "$UNIT/vendor/rust-analyzer/Cargo.toml" --release --offline --locked -p hir-def --lib -j 6
python3 - <<'PY'
import hashlib,json
from pathlib import Path
r=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-unit-v49-snapshot')
before=json.loads((r/'source-copy-before.json').read_text())
def hashes(p):
    return {str(x.relative_to(p)):hashlib.sha256(x.read_bytes()).hexdigest() for x in sorted(p.rglob('*')) if x.is_file()}
original=hashes(Path('/home/jeremy/justrust/toolchain/resident/vendor/rust-analyzer'))
selected=hashes(r/'vendor/rust-analyzer')
assert original==before['original'] and selected==before['selected']
(r/'source-copy-after.json').write_text(json.dumps({'original':original,'selected':selected,'unchanged':True},indent=2)+'\n')
PY
cd /home/jeremy/justrust
sha256sum -c "$ROOT/block-scopes-v49/candidate.sha256" > "$UNIT/frozen-source-after.log"
printf '0\n' > "$UNIT/exit"
bash "$ROOT/build-test-candidate-v49.sh"
