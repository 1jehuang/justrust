#!/usr/bin/env python3
"""Stress the interface hash: random body-only edits must keep it, random
interface edits must change it. Builds each variant with --emit=metadata.

Usage: jr/iface-stress.py RUSTC SRC.rs WORKDIR [N]
SRC.rs must contain fn bodies marked with `// BODY` on a line inside them
(body edits insert lines there) and `// IFACE` on a line where a new pub
item may be added.
"""
import os, random, subprocess, sys
from pathlib import Path

rustc, src, work = sys.argv[1:4]
n = int(sys.argv[4]) if len(sys.argv) > 4 else 20
work = Path(work)
work.mkdir(parents=True, exist_ok=True)
base = Path(src).read_text()
lines = base.splitlines()
body_lines = [i for i, l in enumerate(lines) if l.strip() == "// BODY"]
iface_lines = [i for i, l in enumerate(lines) if l.strip() == "// IFACE"]
assert body_lines and iface_lines


def build(text: str, tag: str) -> str:
    d = work / tag
    d.mkdir(exist_ok=True)
    f = d / "a.rs"
    f.write_text(text)
    env = dict(os.environ, JUSTRUST_RUSTC_IFACE_HASH="1")
    r = subprocess.run([rustc, "--edition", "2021", "--crate-type", "lib", "--crate-name", "a",
                        "--emit=metadata", "--out-dir", str(d), str(f)], env=env,
                       capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(f"{tag}: build failed\n{r.stderr[-2000:]}")
    return (d / "liba.rmeta.jriface").read_text().strip()


rng = random.Random(1)
h0 = build(base, "base")
bad = 0
for k in range(n):
    out = list(lines)
    # body edit: insert 1..400 lines (with long text, so later spans change byte
    # width) at random body markers, sometimes delete comment lines
    for i in sorted(rng.sample(body_lines, rng.randint(1, min(3, len(body_lines)))), reverse=True):
        extra = [f"    let _x{k}_{j} = {rng.randint(0, 10**9)}u64; // {'pad' * rng.randint(0, 40)}"
                 for j in range(rng.randint(1, 400))]
        out[i + 1:i + 1] = extra
    h = build("\n".join(out) + "\n", f"body{k}")
    if h != h0:
        bad += 1
        print(f"FAIL body edit {k} changed iface")
    # comment-only lines before everything (shifts every span)
    shifted = "\n".join(["// x" * rng.randint(1, 50)] * rng.randint(1, 300)) + "\n" + base
    if build(shifted, f"shift{k}") != h0:
        bad += 1
        print(f"FAIL leading comments {k} changed iface")
    # interface edit
    out = list(lines)
    i = rng.choice(iface_lines)
    out.insert(i + 1, f"pub fn added_{k}() -> u{rng.choice([8, 16, 32])} {{ 0 }}")
    if build("\n".join(out) + "\n", f"iface{k}") == h0:
        bad += 1
        print(f"FAIL interface edit {k} kept iface")
print("ok" if bad == 0 else f"{bad} failures", f"({n} rounds)")
sys.exit(1 if bad else 0)
