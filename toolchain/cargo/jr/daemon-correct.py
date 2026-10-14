#!/usr/bin/env python3
"""Correctness check for the jr-cargo daemon.

Two private copies of the pinned Desktop snapshot: A runs cargo with
JUSTRUST_CARGO_DAEMON=1, B plain. The same edit sequence is applied to both;
per step the per-unit decisions (Fresh / Dirty / Compiling), exit codes and
stdout must match, and a plain cargo run on A right after must be all Fresh
(fingerprints on disk consistent for plain cargo).

Usage: correct.py CARGO [setup]
"""
import os, re, shutil, subprocess, sys, time
from pathlib import Path

D = Path.home() / ".jcode/scratch/frog2"
cargo = sys.argv[1]
snap = next((Path.home() / ".justrust/bench/work/desktop").glob("*/"))
tc = Path.home() / ".justrust/toolchains/nightly-2026-10-08"
roots = {"A": D / "wA", "B": D / "wB"}

if len(sys.argv) > 2 and sys.argv[2] == "setup":
    for r in roots.values():
        if r.exists():
            continue
        r.mkdir(parents=True)
        for sub in ["jcode", "jcode-desktop", "target-nightly-2026-10-08"]:
            subprocess.run(["cp", "-a", "--reflink=auto", str(snap / sub), str(r / sub)], check=True)

def env(root, daemon):
    e = dict(os.environ)
    e.update(
        CARGO_TARGET_DIR=str(root / "target-nightly-2026-10-08"),
        RUSTC=str(tc / "bin/rustc"), RUSTDOC=str(tc / "bin/rustdoc"),
        PATH=f"{tc}/bin:" + e["PATH"],
        CARGO_RESOLVER_FEATURE_UNIFICATION="workspace", CARGO_UNSTABLE_FEATURE_UNIFICATION="true",
        __CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS="nightly", CARGO_TERM_VERBOSE="true",
    )
    e.pop("JUSTRUST_CARGO_DAEMON", None)
    if daemon:
        e["JUSTRUST_CARGO_DAEMON"] = "1"
        e["JUSTRUST_CARGO_DAEMON_TRACE"] = "1"
    return e

UNIT = re.compile(r"^\s+(Fresh|Dirty|Compiling|Checking|Running|Executable)\s+(\S+)")

def run(root, args, daemon):
    t = time.monotonic()
    p = subprocess.run([cargo, *args], cwd=root / "jcode-desktop", env=env(root, daemon),
                       capture_output=True, text=True)
    wall = time.monotonic() - t
    s = re.sub(r"[0-9a-f]{16}", "<H>", p.stderr.replace(str(root), "<R>"))
    units = sorted(f"{m.group(1)} {m.group(2)}" for l in s.splitlines() if (m := UNIT.match(l)))
    out = re.sub(r"finished in [0-9.]+s", "finished in Xs", p.stdout.replace(str(root), "<R>"))
    out = "\n".join(sorted(re.sub(r"[0-9a-f]{16}", "<H>", out).splitlines()))
    served = "served by a parked server" in s
    s = s.replace("jr-cargo daemon: served by a parked server\n", "")
    return p.returncode, units, out, wall, s, served

def edit(root, rel, fn):
    p = root / rel
    p.write_text(fn(p.read_text()))

def add(root, rel, text):
    (root / rel).write_text(text)

def rm(root, rel):
    (root / rel).unlink()

UI = "jcode-desktop/crates/jcode-desktop-ui"
FPS = f"{UI}/src/fps_counter.rs"
CORE = "jcode/crates/jcode-core/src/id.rs"
CHECK = ["check", "-p", "jcode-desktop-ui"]
TEST = ["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"]
steps = [
    ("warm check", None, CHECK),
    ("warm check again (daemon parked now)", None, CHECK),
    ("no-op check", None, CHECK),
    ("fps edit, check", lambda r: edit(r, FPS, lambda s: s + "\n// e1\n"), CHECK),
    ("fps edit 2, check", lambda r: edit(r, FPS, lambda s: s + "// e2\n"), CHECK),
    ("type error, check", lambda r: edit(r, FPS, lambda s: s + "fn jr_bad() -> u32 { \"x\" }\n"), CHECK),
    ("fix type error, check", lambda r: edit(r, FPS, lambda s: s.replace("fn jr_bad() -> u32 { \"x\" }\n", "")), CHECK),
    ("core edit, check", lambda r: edit(r, CORE, lambda s: s + "\n// core e1\n"), CHECK),
    ("new test file (target discovery), check", lambda r: add(r, f"{UI}/tests/jr_daemon_probe.rs", "#[test] fn probe() {}\n"), CHECK),
    ("remove test file, check", lambda r: rm(r, f"{UI}/tests/jr_daemon_probe.rs"), CHECK),
    ("manifest comment, check", lambda r: edit(r, f"{UI}/Cargo.toml", lambda s: s + "\n# jr probe\n"), CHECK),
    ("no-op check after manifest", None, CHECK),
    ("warm test", None, TEST),
    ("warm test again", None, TEST),
    ("fps edit, test", lambda r: edit(r, FPS, lambda s: s + "// e3\n"), TEST),
    ("failing test, test", lambda r: edit(r, FPS, lambda s: s + "#[test] fn jr_fail() { panic!(\"jr boom\") }\n"), TEST),
    ("fix failing test, test", lambda r: edit(r, FPS, lambda s: s.replace("#[test] fn jr_fail() { panic!(\"jr boom\") }\n", "")), TEST),
    ("RUSTFLAGS-free env change: CARGO_TERM_VERBOSE kept, no-op test", None, TEST),
]

bad = 0
for name, mut, args in steps:
    res = {}
    for k, r in roots.items():
        if mut:
            mut(r)
        res[k] = run(r, args, daemon=(k == "A"))
    plain = run(roots["A"], args, daemon=False)
    a, b = res["A"], res["B"]
    ok = a[0] == b[0] and a[1] == b[1] and a[2] == b[2]
    stale = [u for u in plain[1] if not u.startswith(("Fresh", "Running", "Executable"))]
    if a[0] != 0:
        stale = []  # a failed compile leaves its unit dirty in plain cargo too
    if a[0] != plain[0]:
        stale.append(f"exit {plain[0]} vs {a[0]}")
    print(f"{'OK ' if ok and not stale else 'BAD'} {name:45s} exit A{a[0]} B{b[0]} units {len(a[1])}/{len(b[1])} "
          f"{'D' if a[5] else '-'} A {a[3]:.2f}s B {b[3]:.2f}s plain-after {plain[3]:.2f}s", flush=True)
    if not ok:
        bad += 1
        sa, sb = set(a[1]), set(b[1])
        print("   only A:", sorted(sa - sb)[:10], "\n   only B:", sorted(sb - sa)[:10])
        if a[2] != b[2]:
            print("   stdout differs")
        print("   A stderr tail:", a[4][-1500:])
    if stale:
        bad += 1
        print("   plain run after daemon run not fresh:", stale[:10])
# restore the edited files from the snapshot
for r in roots.values():
    for rel in [FPS, CORE, f"{UI}/Cargo.toml"]:
        shutil.copy2(snap / rel, r / rel)
print("FAILURES:", bad)
sys.exit(1 if bad else 0)
