#!/usr/bin/env python3
"""How much each rustc pass parallelizes: the same warm edit loop at
several -Zthreads values, per-pass wall from -Ztime-passes, bucketed like
bench/pass_ledger.py.

  flock /tmp/justrust-bench.lock bench/threads_sweep.py [-n 5] [--threads 1,8,16]
        [--toolchain jr-ac20de5e7921] [--scenario desktop-test-edit,jcode-base-check]

-Zthreads is not part of rustc's incremental hash, so all thread counts
share one warm target dir per scenario and alternate in rotating order.
Threads are set through Desktop's rustc wrapper (JCODE_FRONTEND_THREADS),
which threads local crates only, as in normal use.
Result: bench/results/threads-<time>.json and a table on stdout.
"""
import argparse, collections, dataclasses, json, statistics, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402
import pass_ledger  # noqa: E402

EXTRA = {
    "jcode-base-check": suite.Scenario(
        "jcode-base-check", "desktop", ["check", "-p", "jcode-base"],
        suite.Edit("jcode/crates/jcode-base/src/todo.rs",
                   "pub fn todo_status_is_completed(status: &str) -> bool {\n",
                   "pub fn todo_status_is_completed(status: &str) -> bool {\n    std::hint::black_box({n}u64);\n"),
        "body edit in jcode-base (65k lines), check it"),
    "jcode-base-build": suite.Scenario(
        "jcode-base-build", "desktop", ["build", "-p", "jcode-base", "--lib"],
        suite.Edit("jcode/crates/jcode-base/src/todo.rs",
                   "pub fn todo_status_is_completed(status: &str) -> bool {\n",
                   "pub fn todo_status_is_completed(status: &str) -> bool {\n    std::hint::black_box({n}u64);\n"),
        "body edit in jcode-base, build its rlib (metadata + codegen)"),
}


def unit_buckets(rid, crate):
    out = None
    for line in (suite.JR_HOME / "runs" / rid / "units.jsonl").read_text().splitlines():
        u = json.loads(line)
        if u.get("crate_name") == crate and u.get("passes"):
            b = pass_ledger.buckets(u["passes"], u["wall"])
            b["unit wall"] = u["wall"]
            b["unit cpu"] = (u.get("user") or 0) + (u.get("sys") or 0)
            out = b
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--threads", default="1,8,16")
    ap.add_argument("--toolchain", default="jr-ac20de5e7921")
    ap.add_argument("--scenario", default="desktop-test-edit,jcode-base-check")
    ap.add_argument("--env", default="", help="extra K=V,K=V for every run")
    a = ap.parse_args()
    suite.TOOLCHAIN = a.toolchain
    suite.bench_lock()
    threads = [int(t) for t in a.threads.split(",")]
    extra_env = dict(kv.split("=", 1) for kv in a.env.split(",") if kv)
    result = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": suite.env_info(),
              "threads": threads, "extra_env": extra_env, "scenarios": {}}
    base_n = int(time.time()) % 100000 * 100
    for sname in a.scenario.split(","):
        s = EXTRA.get(sname) or suite.BY_NAME[sname]
        crate = "jcode_base" if "jcode-base" in sname else "jcode_desktop_ui"
        root = suite.prepare(s.snapshot)
        target = root / f"target-threads-{a.toolchain}"
        samples = collections.defaultdict(list)
        k = 0
        for i in range(a.warmup + a.n):
            order = threads[i % len(threads):] + threads[:i % len(threads)]
            for t in order:
                s2 = dataclasses.replace(s, env={**s.env, **extra_env, "JCODE_FRONTEND_THREADS": str(t)})
                k += 1
                try:
                    suite.apply_edit(root, s.edit, base_n + k)
                    r = suite.run_once(s2, root, target)
                finally:
                    suite.restore(root, s.edit)
                b = unit_buckets(r["id"], crate)
                print(f"  {sname} threads={t} {i + 1}/{a.warmup + a.n}: wall {r['wall']:.2f}s "
                      f"unit {b and b['unit wall']:.2f}s id {r['id']}{' noisy' if r['noisy'] else ''}", flush=True)
                if i >= a.warmup and b:
                    samples[t].append({"id": r["id"], "wall": r["wall"], "noisy": r["noisy"], "buckets": dict(b)})
        result["scenarios"][sname] = {str(t): v for t, v in samples.items()}
        # table
        keys = collections.Counter()
        for t in threads:
            for x in samples[t]:
                for kk, v in x["buckets"].items():
                    keys[kk] += v
        print(f"\n{sname} ({crate}), median seconds, n={a.n}")
        print(f"{'bucket':52} " + " ".join(f"{'t=' + str(t):>8}" for t in threads))
        for kk, _ in keys.most_common():
            meds = [statistics.median([x["buckets"].get(kk, 0) for x in samples[t]]) for t in threads]
            print(f"{kk:52} " + " ".join(f"{m:8.3f}" for m in meds))
        meds = [statistics.median([x["wall"] for x in samples[t]]) for t in threads]
        print(f"{'agent wall':52} " + " ".join(f"{m:8.3f}" for m in meds))
    out = suite.ROOT / "bench" / "results" / f"threads-{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.write_text(json.dumps(result, indent=1) + "\n")
    print(f"\nwrote {out}")


if __name__ == "__main__":
    main()
