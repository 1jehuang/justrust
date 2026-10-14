#!/usr/bin/env bash
# Capture -Zself-profile of the reference loop's rustc unit, for
# bench/critpath.py. Runs on the build host (or locally), inside the pinned
# Desktop bench snapshot that bench/suite.py prepares.
#
#   bench/critpath-capture.sh [THREADS...]     default: 1 16
#
# For each frontend thread count: warm the target with the normal reference
# edit, then one profiled one-line body edit. The profiler is added only to
# the jcode_desktop_ui rustc invocation by a wrapper placed in front of the
# Desktop rustc wrapper (cargo --config), so no source or config file changes.
# -Zself-profile is not part of the incremental hash, so the profiled run
# reuses the warm cache. Profiles land in bench/results/critpath/<host>-<time>/t<N>/.
set -euo pipefail
cd "$(dirname "$0")/.."
threads=("$@"); [[ ${#threads[@]} -gt 0 ]] || threads=(1 16)
TOOLCHAIN=${TOOLCHAIN:-nightly-2026-10-08}
out=bench/results/critpath/$(hostname -s)-$(date +%Y%m%d-%H%M%S)
mkdir -p "$out"
python3 - "$out" "$TOOLCHAIN" "${threads[@]}" <<'EOF'
import os, subprocess, sys, dataclasses
from pathlib import Path
sys.path.insert(0, "bench")
import suite
out, tc, threads = Path(sys.argv[1]).resolve(), sys.argv[2], [int(t) for t in sys.argv[3:]]
suite.TOOLCHAIN = tc
s = suite.BY_NAME["desktop-test-edit"]
root = suite.prepare(s.snapshot)
target = root / f"target-critpath-{tc}"
# Desktop sets build.rustc-wrapper = scripts/rustc-parallel-frontend. We put
# our wrapper in that slot for every run (so warm and profiled runs share one
# compiler command line and incremental cache); it adds the profiler only for
# jcode_desktop_ui and only when CRITPATH_PROFILE_DIR is set, then runs the
# Desktop wrapper unchanged.
ws = root / suite.SNAPSHOTS[s.snapshot]["cwd"]
desk = ws / "scripts" / "rustc-parallel-frontend"
wrap = out / "prof-wrapper.sh"
wrap.write_text(f"""#!/bin/sh
rustc="$1"; shift
if [ "${{CARGO_CRATE_NAME:-}}" = jcode_desktop_ui ] && [ -n "${{CRITPATH_PROFILE_DIR:-}}" ]; then
  set -- "$@" -Zself-profile="$CRITPATH_PROFILE_DIR" -Zself-profile-events=default,args,query-blocked
fi
exec {desk} "$rustc" "$@"
""")
wrap.chmod(0o755)
for t in threads:
    d = out / f"t{t}"; d.mkdir(parents=True, exist_ok=True)
    base = {**s.env, "JCODE_FRONTEND_THREADS": str(t)}
    for i, profiled in enumerate([False, False, True]):
        env = {**base}
        cfg = ["--config", f'env.JCODE_FRONTEND_THREADS.value="{t}"', "--config", "env.JCODE_FRONTEND_THREADS.force=true",
               "--config", f'build.rustc-wrapper="{wrap}"']
        if profiled:
            env["CRITPATH_PROFILE_DIR"] = str(d)
        s2 = dataclasses.replace(s, env=env, cmd=[*s.cmd[:1], *cfg, *s.cmd[1:]])
        try:
            suite.apply_edit(root, s.edit, 7000 + t * 10 + i)
            r = suite.run_once(s2, root, target)
        finally:
            suite.restore(root, s.edit)
        print(f"threads={t} run {i} {'profiled' if profiled else 'warm'}: wall {r['wall']:.2f}s id {r['id']}", flush=True)
    prof = sorted(d.glob("jcode_desktop_ui-*.mm_profdata"))
    print(f"threads={t}: {len(prof)} profile(s) in {d}", flush=True)
EOF
echo "profiles: $out"
