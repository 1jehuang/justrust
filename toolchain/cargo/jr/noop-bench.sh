#!/usr/bin/env bash
# Time no-op cargo runs on justrust's pinned Desktop snapshot, alternating
# between cargo binaries. Usage: jr/noop-bench.sh N cargoA [cargoB ...]
# Wrap in `flock /tmp/justrust-bench.lock` (shared laptop).
# Env: NOOP_CMD (default "test -p jcode-desktop-ui --lib --no-run").
set -euo pipefail
n=$1; shift
tc=${JUSTRUST_HOME:-$HOME/.justrust}/toolchains/${TOOLCHAIN:-nightly-2026-10-08}
snap=$(ls -d "${JUSTRUST_HOME:-$HOME/.justrust}"/bench/work/desktop/*/ | head -1)
cd "$snap/jcode-desktop"
export CARGO_TARGET_DIR=$snap/target-${TOOLCHAIN:-nightly-2026-10-08}
export RUSTC=$tc/bin/rustc RUSTDOC=$tc/bin/rustdoc PATH=$tc/bin:$PATH
export CARGO_RESOLVER_FEATURE_UNIFICATION=workspace CARGO_UNSTABLE_FEATURE_UNIFICATION=true
export __CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS=nightly CARGO_TERM_VERBOSE=true
read -ra cmd <<< "${NOOP_CMD:-test -p jcode-desktop-ui --lib --no-run}"
for c in "$@"; do "$c" "${cmd[@]}" >/dev/null 2>&1; done  # warm
python3 - "$n" "${cmd[*]}" "$@" <<'PY'
import subprocess, sys, time, statistics
n, cmd, bins = int(sys.argv[1]), sys.argv[2].split(), sys.argv[3:]
t = {b: [] for b in bins}
for i in range(n):
    for b in (bins if i % 2 == 0 else bins[::-1]):
        s = time.monotonic()
        p = subprocess.run([b, *cmd], capture_output=True, text=True)
        t[b].append(time.monotonic() - s)
        dirty = p.stderr.count("Dirty ") + p.stderr.count("Compiling ")
        if p.returncode or dirty:
            sys.exit(f"{b}: exit {p.returncode}, {dirty} rebuilt\n{p.stderr[-2000:]}")
for b, xs in t.items():
    xs.sort()
    print(f"{statistics.median(xs):.3f}s median  min {xs[0]:.3f}  max {xs[-1]:.3f}  n={len(xs)}  {b}")
PY
