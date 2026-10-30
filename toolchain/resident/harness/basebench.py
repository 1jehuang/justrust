#!/usr/bin/env python3
"""basebench.py: ra-jit compiling every crate of a workspace, against rustc doing the same.

usage: basebench.py WORKSPACE PACKAGE [--runs N] [--out FILE] [--keep WORK]

The registry crates are built once and stay. Then, N times each (default 3):
  cargo build   every package of the workspace cleaned, `cargo build -p PACKAGE`
  cargo check   the same with `cargo check`; build minus check is rustc's code generation
  ra-jit cold    ra-jit on every crate of the workspace the program uses, with an empty store
  ra-jit warm    the same again with the store the cold run filled
  ra-jit none    the same with no store
For ra-jit the time is the process's, loading included, with the split its `RA_JIT_STATS`
lines give, and user and system CPU seconds next to the wall time. Every run is printed.
"""
import json, os, re, resource, shutil, statistics, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
INCREMENTAL = os.environ.get("CARGO_INCREMENTAL")
from difftest import Fixture, RA_JIT, TARGET_ARGS, sysroot_rlibs
if INCREMENTAL is None:
    os.environ.pop("CARGO_INCREMENTAL", None)


def timed(command, **kwargs):
    """Wall seconds, CPU seconds, peak RSS in MB and stderr of a command."""
    before, start = resource.getrusage(resource.RUSAGE_CHILDREN), time.monotonic()
    run = subprocess.run(command, capture_output=True, text=True, **kwargs)
    wall, after = time.monotonic() - start, resource.getrusage(resource.RUSAGE_CHILDREN)
    if run.returncode != 0:
        raise SystemExit(f"{command[:3]} failed:\n{run.stderr[-3000:]}")
    cpu = after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime
    return dict(wall=round(wall, 2), cpu=round(cpu, 2), rss=after.ru_maxrss // 1024, stderr=run.stderr)


def seconds(text):
    """A `Duration` as `{:?}` prints it."""
    number, unit = re.fullmatch(r"([\d.]+)(ns|µs|ms|s)", text).groups()
    return float(number) * {"ns": 1e-9, "µs": 1e-6, "ms": 1e-3, "s": 1}[unit]


def split(stderr):
    """What ra-jit says of where its time went."""
    found = {}
    for name, pattern in [("load", r"^load(?:ed)?[^\n]*? in ([\d.]+\S+)"), ("roots", r"roots: (\S+),"), ("revalidate", r"revalidate: (\S+),"),
                          ("compile", r"compile: (\S+)$"), ("emit", r"\(\+(\S+) emit\)")]:
        if hit := re.search(pattern, stderr, re.M):
            found[name] = round(seconds(hit[1]), 2)
    if hit := re.search(r"compiled (\d+) of (\d+) fns \(\+(\d+) instances\)", stderr):
        found.update(functions=int(hit[1]), of=int(hit[2]), instances=int(hit[3]))
    if hit := re.search(r"stats: (\d+) of (\d+) functions compiled, (\d+) reused", stderr):
        found.update(compiled=int(hit[1]), bodies=int(hit[2]), reused=int(hit[3]))
    if hit := re.search(r"^phases: (.*)$", stderr, re.M):
        found["phases"] = hit[1]
    return found


def main():
    argv = sys.argv[1:]
    option = lambda name, default: argv[argv.index(name) + 1] if name in argv else default
    runs, out, work = int(option("--runs", 3)), option("--out", None), option("--keep", None)
    source, package = argv[0], argv[1]
    ws = work or tempfile.mkdtemp(prefix="ra-jit-base-")
    if not os.path.exists(os.path.join(ws, "Cargo.toml")):
        shutil.copytree(source, ws, dirs_exist_ok=True, ignore=shutil.ignore_patterns("target*", ".git"))
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1"], cwd=ws))
    members = set(meta["workspace_members"])
    packages = {p["id"]: p for p in meta["packages"]}
    # The workspace's packages the program is built from.
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(i for i in members if packages[i]["name"] == package)
    used, todo = set(), [root]
    while todo:
        at = todo.pop()
        if at in used:
            continue
        used.add(at)
        todo.extend(d["pkg"] for d in nodes[at]["deps"] if any(k["kind"] is None for k in d["dep_kinds"]))
    ours = [packages[i] for i in used & members]
    crates = [t["name"].replace("-", "_") for p in ours for t in p["targets"] if "lib" in t["kind"] or (p["id"] == root and "bin" in t["kind"])]
    clean = ["cargo", "clean", *(x for p in ours for x in ("-p", p["name"]))]
    result = dict(workspace=source, package=package, crates=sorted(set(crates)), ra_jit=RA_JIT, rows={})
    print(f"work {ws}; {len(ours)} packages; crates {' '.join(sorted(set(crates)))}", flush=True)

    subprocess.run(["cargo", "build", "-p", package], cwd=ws, check=True, capture_output=True)
    subprocess.run(["cargo", "check", "-p", package], cwd=ws, check=True, capture_output=True)
    for name in ("build", "check"):
        rows = []
        for _ in range(runs):
            subprocess.run(clean, cwd=ws, check=True, capture_output=True)
            row = timed(["cargo", name, "-p", package], cwd=ws)
            del row["stderr"]
            rows.append(row)
            print(f"cargo {name:5}  {row['wall']:7.2f}s wall {row['cpu']:7.2f}s cpu", flush=True)
        result["rows"][f"cargo {name}"] = rows
    subprocess.run(["cargo", "build", "-p", package], cwd=ws, check=True, capture_output=True)

    fx = Fixture(ws, [",".join(sorted(set(crates)))], ["-p", package])
    fx.rustc_build()
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    obj = os.path.join(fx.work, "all.o")
    command = [RA_JIT, ws, *(x for c in sorted(set(crates)) for x in ("--crate", c)), "--out", obj, *TARGET_ARGS, *symbols]
    open(os.path.join(fx.work, "command.json"), "w").write(json.dumps(command))
    print(f"command in {fx.work}/command.json", flush=True)
    store = os.path.join(fx.work, "store")
    for name in ("cold", "warm", "none"):
        rows = []
        for _ in range(runs):
            env = dict(os.environ, RA_JIT_STATS="1", RA_JIT_CACHE="off" if name == "none" else store)
            if name == "cold":
                shutil.rmtree(store, ignore_errors=True)
            row = timed(command, env=env)
            stderr = row.pop("stderr")
            open(os.path.join(fx.work, f"{name}.stderr"), "w").write(stderr)
            row.update(split(stderr))
            rows.append(row)
            print(f"ra-jit {name:5}  {row['wall']:7.2f}s wall {row['cpu']:7.2f}s cpu {row['rss']:6} MB  " +
                  " ".join(f"{k} {v}" for k, v in row.items() if k not in ("wall", "cpu", "rss")), flush=True)
        result["rows"][f"ra-jit {name}"] = rows
    fx.link([obj])
    ran = subprocess.run([fx.binary, "--version"], capture_output=True, text=True)
    print(f"linked: `--version` exits {ran.returncode}: {ran.stdout.strip()[:80]}")
    median = lambda name: statistics.median(r["wall"] for r in result["rows"][name])
    print("medians: " + ", ".join(f"{name} {median(name):.2f}s" for name in result["rows"]))
    if out:
        open(out, "w").write(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
