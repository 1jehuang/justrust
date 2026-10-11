#!/usr/bin/env python3
"""Accuracy of early diagnostics (`src/early.rs`) against rustc.

Two measurements on the pinned Desktop snapshot (bench/suite.py prepares it):

1. clean sweep: every .rs file of the workspace crates on pristine sources
   (which compile with warnings only). Every early error is a false positive.
2. mutations: a corpus of one-error edits of the kinds agents make. For each,
   the daemon's answer (as an agent build would get it) vs the errors of a
   real `justrust check -p jcode-desktop-ui` on the same source.

Untimed: no bench lock needed. Usage: bench/early_accuracy.py [--sweep-only]
Writes bench/results/early-accuracy-<time>.json.
"""
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

J = os.environ.get("JUSTRUST", str(Path.home() / ".cargo/bin/justrust"))
ROOT = suite.prepare("desktop")
WS = ROOT / "jcode-desktop"
UI = "jcode-desktop/crates/jcode-desktop-ui/src"
CORE = "jcode/crates/jcode-core/src/id.rs"
FPS_ANCHOR = "    ) -> String {\n        if self"

# (name, file, anchor, replacement, what rustc should say)
M = [
    ("type mismatch", f"{UI}/fps_counter.rs", FPS_ANCHOR,
     '    ) -> String {\n        let _: u32 = "x";\n        if self', "E0308"),
    ("unknown method", f"{UI}/fps_counter.rs", FPS_ANCHOR,
     "    ) -> String {\n        self.fps.no_such_method();\n        if self", "E0599"),
    ("unresolved name", f"{UI}/fps_counter.rs", FPS_ANCHOR,
     "    ) -> String {\n        let _ = undefined_name;\n        if self", "E0425"),
    ("use after move", f"{UI}/fps_counter.rs", "            let current = snapshot();\n",
     "            let current = snapshot();\n            let _again = snapshot();\n", "E0382"),
    ("wrong return type", f"{UI}/effort.rs", '        "none" => "None",', '        "none" => 5,', "E0308"),
    ("integer type mix", f"{UI}/effort.rs", "        index + 1\n", "        index + 1u8\n", "E0308"),
    ("unresolved import", f"{UI}/effort.rs", "use gpui::{App, KeyBinding, Keystroke};",
     "use gpui::{App, KeyBinding, Keystroke, NoSuchItem};", "E0432"),
    ("non-exhaustive match", f"{UI}/effort.rs", '        "swarm-deep" => "Swarm Deep",\n        other => other,\n',
     '        "swarm-deep" => "Swarm Deep",\n', "E0004"),
    ("unknown field", f"{UI}/panel_effort.rs", "        let ladder = self.effort_ladder();\n        if ladder.is_empty() {\n            self.items",
     "        let ladder = self.effort_ladder();\n        let _ = self.no_such_field;\n        if ladder.is_empty() {\n            self.items", "E0609"),
    ("wrong arg count", f"{UI}/panel_effort.rs", 'format!("Effort: {}", effort::label(next))',
     'format!("Effort: {}", effort::label(next, 1))', "E0061"),
    ("missing struct field", f"{UI}/panel_effort.rs",
     "            previous: self.reasoning_effort.clone(),\n            requested: VecDeque::new(),\n",
     "            previous: self.reasoning_effort.clone(),\n", "E0063"),
    ("borrow conflict", f"{UI}/panel_effort.rs", "        pending.requested.push_back(level.to_string());\n",
     "        pending.requested.push_back(level.to_string());\n        let _p = &mut self.pending_effort;\n        let _q = &mut self.pending_effort;\n        _p.take();\n",
     "E0499"),
    ("tuple arity", "jcode-desktop/crates/jcode-desktop-ui/src/ack.rs", "        return (0.0, 1.0, false);",
     "        return (0.0, 1.0);", "E0308"),
    ("trait bound", "jcode-desktop/crates/jcode-desktop-ui/src/ack.rs",
     "    let t = elapsed.as_secs_f32()", "    let _ = 1.0f32 + 1u32;\n    let t = elapsed.as_secs_f32()", "E0277"),
    ("assign immutable", "jcode-desktop/crates/jcode-desktop-ui/src/ack.rs",
     "    let decay = 1.0 - t;\n", "    let decay = 1.0 - t;\n    decay = 2.0;\n", "E0384"),
    ("format args", "jcode-desktop/crates/jcode-desktop-ui/src/ack.rs",
     "    let decay = 1.0 - t;\n", '    let decay = 1.0 - t;\n    let _ = format!("{}");\n', ""),
    ("syntax error", "jcode-desktop/crates/jcode-desktop-ui/src/ack.rs",
     "    let decay = 1.0 - t;\n", "    let decay = 1.0 - ;\n", ""),
    ("signature change (callers elsewhere)", f"{UI}/effort.rs",
     "pub(crate) fn label(effort: &str) -> &str {", "pub(crate) fn label(effort: &str, _x: u8) -> &str {", "E0061"),
    ("dependency crate (jcode-core)", CORE, "pub fn session_icon(name: &str) -> &'static str {\n",
     "pub fn session_icon(name: &str) -> &'static str {\n    let _: u32 = name;\n", "E0308"),
]


def daemon(op: dict, timeout=600) -> dict:
    out = subprocess.run([J, "early", "status"], cwd=WS, capture_output=True, text=True).stdout
    if "not running" in out:
        sys.exit("start the daemon first: (cd snapshot && justrust early start)")
    import hashlib  # noqa: F401  (key computed by justrust; find the socket)
    socks = list((suite.JR_HOME / "early").glob("*/sock"))
    for sp in socks:
        log = (sp.parent / "daemon.log").read_text(errors="replace")
        if f"daemon up for {WS}" in log:
            s = socket.socket(socket.AF_UNIX)
            s.settimeout(timeout)
            s.connect(str(sp))
            s.sendall((json.dumps(op) + "\n").encode())
            buf = b""
            while not buf.endswith(b"\n"):
                c = s.recv(65536)
                if not c:
                    break
                buf += c
            return json.loads(buf)
    sys.exit("no daemon socket for the snapshot")


LOC = re.compile(r"^\s*--> (.+):(\d+):(\d+)$")


def rustc_errors(stderr: str) -> list[tuple[str, int, str]]:
    out, code = [], None
    for line in stderr.splitlines():
        t = line.lstrip()
        if t.startswith("error["):
            code = t[6:].split("]")[0]
        elif t.startswith("error:") and "could not compile" not in t and "aborting" not in t:
            code = ""
        elif t.startswith("warning"):
            code = None
        else:
            m = LOC.match(line)
            if m and code is not None:
                f = os.path.normpath(WS / m.group(1)) if not m.group(1).startswith("/") else m.group(1)
                out.append((f, int(m.group(2)), code))
                code = None
    return out


def sweep() -> dict:
    files = sorted(str(p) for p in (WS / "crates").rglob("*.rs") if "target" not in p.parts)
    files += sorted(str(p) for p in (WS / "src").rglob("*.rs"))
    t = time.monotonic()
    r = daemon({"op": "pull", "files": files, "raw": True})
    secs = time.monotonic() - t
    print(f"clean sweep: {len(r['files'])}/{len(files)} files pulled in {secs:.1f}s, "
          f"{len(r['diags'])} early errors (all false positives)", flush=True)
    for d in r["diags"]:
        print(f"  FP {d['code']} {d['file'].replace(str(ROOT) + '/', '')}:{d['line']} {d['message'][:100]}")
    sev1 = [d for d in r.get("raw", []) if d["severity"] == 1]
    print(f"  raw severity-1 diagnostics from rust-analyzer (all codes): {len(sev1)}")
    for d in sev1[:30]:
        print(f"    {d['code']} {d['file'].replace(str(ROOT) + '/', '')}:{d['line']} {str(d['message'])[:90]}")
    return {"raw_errors": sev1, "files": len(files), "pulled": len(r["files"]), "secs": round(secs, 1), "false_positives": r["diags"]}


def mutations() -> list[dict]:
    rows = []
    target = ROOT / "target"
    for name, rel, anchor, repl, want in M:
        path = ROOT / rel
        pristine = suite.pristine(ROOT, rel)
        orig = pristine.read_text()
        assert orig.count(anchor) == 1, f"{name}: anchor occurs {orig.count(anchor)} times"
        line = orig[: orig.index(anchor)].count("\n") + 1
        try:
            path.write_text(orig.replace(anchor, repl, 1))
            t = time.monotonic()
            early = daemon({"op": "diagnostics", "raw": True}, timeout=30)
            early_ms = (time.monotonic() - t) * 1000
            env = {**os.environ, "CARGO_TARGET_DIR": str(target), "JUSTRUST_REMOTE": "0", "JUSTRUST_TOOLCHAIN": "system"}
            env.pop("JUSTRUST_EARLY", None)
            p = subprocess.run([J, "check", "-p", "jcode-desktop-ui"], cwd=WS, env=env, capture_output=True, text=True)
            rerr = rustc_errors(p.stderr)
        finally:
            shutil.copy2(pristine, path)
        ed = [(d["file"], d["line"], d["code"]) for d in early.get("diags", [])]
        rset = {(f, l) for f, l, _ in rerr}
        eset = {(f, l) for f, l, _ in ed}
        tp = len(eset & rset)
        fp = len(eset - rset)
        fn = len(rset - eset)
        first_ok = bool(rerr) and (rerr[0][0], rerr[0][1]) in eset
        row = {
            "name": name, "file": rel, "line": line, "want": want,
            "early_ready": early.get("ready"), "early_ms": round(early_ms, 1),
            "early": ed, "rustc": rerr, "ra_raw": early.get("raw", []), "tp": tp, "fp": fp, "fn": fn,
            "first_rustc_error_found": first_ok,
        }
        rows.append(row)
        verdict = "HIT" if first_ok and fp == 0 else ("MISS" if not ed else ("PARTIAL" if tp else "WRONG"))
        print(f"{verdict:8s} {name:38s} rustc {[c for *_, c in rerr]} early {[c for *_, c in ed]} "
              f"tp {tp} fp {fp} fn {fn} ({early_ms:.0f} ms)", flush=True)
        if verdict != "HIT":
            for d in early.get("raw", []):
                print(f"           ra: sev {d['severity']} {d['code']} line {d['line']}: {str(d['message'])[:90]}")
    # Back to pristine in the daemon too.
    daemon({"op": "diagnostics"}, timeout=30)
    return rows


def main():
    res = {"experimental": os.environ.get("JUSTRUST_EARLY_EXPERIMENTAL", "1") != "0", "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "snapshot": suite.SNAPSHOTS["desktop"]["repos"].__repr__()}
    daemon({"op": "diagnostics"}, timeout=30)
    res["sweep"] = sweep()
    if "--sweep-only" not in sys.argv:
        rows = mutations()
        res["mutations"] = rows
        n = len(rows)
        hit = sum(r["first_rustc_error_found"] for r in rows)
        clean = sum(r["first_rustc_error_found"] and r["fp"] == 0 for r in rows)
        fp = sum(r["fp"] for r in rows)
        print(f"\nfirst rustc error found early: {hit}/{n}, with no false positive: {clean}/{n}, "
              f"false positives in mutations: {fp}, clean-sweep false positives: {len(res['sweep']['false_positives'])}")
        res["totals"] = {"n": n, "first_found": hit, "clean": clean, "fp": fp,
                         "tp": sum(r["tp"] for r in rows), "fn": sum(r["fn"] for r in rows)}
    tag = "" if res["experimental"] else "-stable-only"
    out = suite.RESULTS / f"early-accuracy-{time.strftime('%Y%m%d-%H%M%S')}{tag}.json"
    out.write_text(json.dumps(res, indent=1) + "\n")
    print(f"saved {out}")


if __name__ == "__main__":
    main()
