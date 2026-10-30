#!/usr/bin/env python3
"""editrows.py WS BIN RUNS edit[,edit..]: time each genws edit under `cargo ra-jit build` and `cargo build`, compare program output.
Each run: apply edit, build ra-jit, build cargo, compare outputs; revert, build both (reverts timed separately)."""
import sys, os, subprocess, time, glob, json, statistics, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import genws
T = {"ra-jit": "/tmp/edit-target-ra-jit", "cargo": "/tmp/edit-target-cargo"}
def build(way):
    cmd = ["cargo", "ra-jit", "build"] if way == "ra-jit" else ["cargo", "build"]
    s = time.time(); p = subprocess.run(cmd, env=dict(env, CARGO_TARGET_DIR=T[way]), capture_output=True, text=True); t = time.time() - s
    if p.returncode: print(f"  {way} FAILED rc{p.returncode}: {p.stderr[-600:]}")
    return t, p.returncode
def out(way):
    p = subprocess.run([f"{T[way]}/debug/{binname}", "5"], capture_output=True, text=True)
    return p.returncode, p.stdout[-400:]
def apply(edit, back=False):
    n = 0
    for pat, old, new in edit:
        if back: old, new = new, old
        for f in glob.glob(pat):
            s = open(f).read()
            if old in s: open(f, "w").write(s.replace(old, new)); n += 1
    assert n, edit
    return n
def daemon_stats():
    logs = glob.glob("/tmp/edit-home/spares/*/log")
    txt = "".join(open(l, errors="replace").read() for l in logs)
    e = re.findall(r"edit-to-binary: (\S+)", txt); s = re.findall(r"stats: (\d+) of", txt); l = re.findall(r"link: \S+ \((\w+)", txt)
    return (e[-1] if e else "?"), (s[-1] if s else "?"), (l[-1] if l else "?"), len(e)
def main():
    global binname, env
    ws, binname, runs, names = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4].split(",")
    os.chdir(ws)
    env = dict(os.environ, RA_JIT_HOME="/tmp/edit-home", PATH=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release") + ":" + os.environ["PATH"])
    all_edits = genws.edits(ws)
    for name in names:
        edit = genws.scattered(ws, int(name[1:])) if name.startswith("s") and name[1:].isdigit() else all_edits[name]
        rows = []
        for r in range(runs):
            for back in (False, True):
                files = apply(edit, back); time.sleep(0.05)
                before = daemon_stats()[3]
                tr, rc_r = build("ra-jit"); st = daemon_stats(); served = st[3] > before
                tc, rc_c = build("cargo")
                same = out("ra-jit") == out("cargo")
                rows.append((back, tr, tc))
                print(f"{name:10} {'revert' if back else 'apply '} files={files} ra-jit={tr:6.2f} cargo={tc:6.2f} x{tc/tr:5.1f} daemon={st[0]} fns={st[1]} link={st[2]} served={served} same_output={same}", flush=True)
        for back in (False, True):
            a = [x for x in rows if x[0] == back]
            mr, mc = statistics.median(x[1] for x in a), statistics.median(x[2] for x in a)
            print(f"== {name:10} {'revert' if back else 'apply '} median ra-jit {mr:.2f} cargo {mc:.2f} ratio {mc/mr:.1f}x (n={len(a)})", flush=True)


if __name__ == "__main__":
    main()
