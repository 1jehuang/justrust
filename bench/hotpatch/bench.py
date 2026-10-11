#!/usr/bin/env python3
"""Hot patch vs normal incremental test run, interleaved (FINDINGS 28).

    flock /tmp/justrust-bench.lock bench/hotpatch/bench.py desktop -n 5
    flock /tmp/justrust-bench.lock bench/hotpatch/bench.py toy -n 5

Each iteration makes a fresh one-line body edit (unique black_box number),
then runs `justrust test ...` either normally (full incremental relink) or
with JR_HOT=1 (jr-hot builds a patch image from the regenerated codegen
units instead of relinking; the runner preloads the loader), in alternating
blocks. Wall is what the agent waits for: edit to test result. Every run
must report the test result line, and the hot run must say it patched.

The workspaces are private copies (reflinks of the suite snapshots for
desktop) whose target dir was linked once through jr-hot. Files are restored
from a saved copy, never via git.
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
HOT = HERE / "target/release/jr-hot"
SCRATCH = Path(os.environ.get("JCODE_SCRATCH_DIR", "/tmp")) / "hot"

WORKLOADS = {
    # The reference loop: one line inside a #[test] fn (suite desktop-test-edit).
    "desktop": dict(
        cwd=SCRATCH / "desk/jcode-desktop",
        target=SCRATCH / "desk/target",
        file="crates/jcode-desktop-ui/src/fps_counter.rs",
        old="let mut counter = FpsCounter::default();\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        new="let mut counter = FpsCounter::default();\n        std::hint::black_box({n}u64);\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        cmd=["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"],
        env={
            "JUSTRUST_TOOLCHAIN": "nightly-2026-10-08",
            "JR_HOT_REAL_LINKER": str(SCRATCH / "desk/jcode-desktop/scripts/fast-linker"),
        },
    ),
    # Non-test body edit in the same crate (suite desktop-body-test).
    "desktop-body": dict(
        cwd=SCRATCH / "desk/jcode-desktop",
        target=SCRATCH / "desk/target",
        file="crates/jcode-desktop-ui/src/fps_counter.rs",
        old="            let current = snapshot();\n",
        new="            let current = snapshot();\n            std::hint::black_box({n}u64);\n",
        cmd=["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"],
        env={
            "JUSTRUST_TOOLCHAIN": "nightly-2026-10-08",
            "JR_HOT_REAL_LINKER": str(SCRATCH / "desk/jcode-desktop/scripts/fast-linker"),
        },
    ),
    # Small crate: a 30-line lib with 3 tests.
    "toy": dict(
        cwd=SCRATCH / "toy",
        target=SCRATCH / "toy/target",
        file="src/lib.rs",
        old="        a + b\n",
        new="        a + b + std::hint::black_box({n}u64) * 0\n",
        cmd=["test"],
        env={},
    ),
}


def run(w: dict, hot: bool, n: int) -> dict:
    path = w["cwd"] / w["file"]
    pristine = path.with_name(path.name + ".jrhot-pristine")
    if not pristine.exists():
        pristine.write_text(path.read_text())
    src = pristine.read_text()
    assert w["old"] in src, "edit anchor not found"
    path.write_text(src.replace(w["old"], w["new"].format(n=n), 1))
    env = dict(os.environ)
    env.update(w["env"])
    env.update(
        {
            "CARGO_TARGET_DIR": str(w["target"]),
            "JUSTRUST_REMOTE": "0",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER": str(HOT),
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER": f"{HOT} run",
        }
    )
    env.pop("JR_HOT", None)
    log = SCRATCH / "jrhot.log"
    log.write_text("")
    env["JR_HOT_LOG"] = str(log)
    if hot:
        env["JR_HOT"] = "1"
    t = time.monotonic()
    p = subprocess.run(
        ["justrust", *w["cmd"]], cwd=w["cwd"], env=env, capture_output=True, text=True
    )
    wall = time.monotonic() - t
    out = p.stdout + p.stderr + log.read_text()
    result = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed", out)
    patched = re.search(r"jr-hot: patched instead of linking: (.*?) in (\d+) ms", out)
    full = re.search(r"jr-hot: full link: (.*)", out)
    run_id = re.search(r"justrust show (\S+?)`", out)
    link = None
    if run_id:
        try:
            s = json.loads(
                (Path.home() / f".justrust/runs/{run_id.group(1)}/summary.json").read_text()
            )
            link = s.get("link")
        except Exception:
            pass
    return dict(
        mode="hot" if hot else "normal",
        n=n,
        wall=round(wall, 3),
        exit=p.returncode,
        result=result[0] if result else None,
        patched=patched.group(1) if patched else None,
        patch_ms=int(patched.group(2)) if patched else None,
        fallback=full.group(1).strip() if full else None,
        run_id=run_id.group(1) if run_id else None,
        link=link,
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("workload", choices=WORKLOADS)
    ap.add_argument("-n", type=int, default=3, help="measured runs per block (2 blocks per mode)")
    a = ap.parse_args()
    w = WORKLOADS[a.workload]
    base_n = int(time.time()) % 1_000_000 * 100
    samples = []
    path = w["cwd"] / w["file"]
    pristine = path.with_name(path.name + ".jrhot-pristine")
    try:
        # Warmup: one normal run so the base binary matches the source.
        samples.append(dict(run(w, False, base_n), warmup=True))
        # Blocks, not strict ABBA: a hot patch needs a base linked in hot
        # mode (--no-gc-sections), so the first hot run after a normal link
        # relinks (rebase) and is a warmup. N N N, H* H H H, N N N, H* H H H.
        order = []
        for _ in range(2):
            order += [(False, False)] * a.n
            order += [(True, True)] + [(True, False)] * a.n
        for i, (hot, warm) in enumerate(order):
            s = run(w, hot, base_n + 1 + i)
            if warm:
                s["warmup"] = True
            samples.append(s)
            print(json.dumps(s), file=sys.stderr)
    finally:
        if pristine.exists():
            path.write_text(pristine.read_text())
            pristine.unlink()
    out = HERE.parent / "results" / f"hotpatch-{a.workload}-{time.strftime('%Y%m%d-%H%M%S')}.json"
    summary = {}
    for mode in ("normal", "hot"):
        ms = [s for s in samples if s["mode"] == mode and not s.get("warmup")]
        ws = [s["wall"] for s in ms]
        summary[mode] = dict(
            median=statistics.median(ws), min=min(ws), max=max(ws), n=len(ws)
        )
        if mode == "hot":
            summary[mode]["patched"] = sum(1 for s in ms if s["patched"])
            pm = [s["patch_ms"] for s in ms if s["patch_ms"] is not None]
            if pm:
                summary[mode]["patch_ms_median"] = statistics.median(pm)
        summary[mode]["all_ok"] = all(s["result"] and s["exit"] == 0 for s in ms)
    out.write_text(
        json.dumps(dict(workload=a.workload, summary=summary, samples=samples), indent=1)
    )
    print(f"{a.workload}: normal {summary['normal']} hot {summary['hot']}")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
