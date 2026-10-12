#!/usr/bin/env python3
"""Early exit accuracy at scale: rust-analyzer's answer vs rustc on mutations
drawn from the error kinds agents actually hit (bench/early_exit/mine_codes.py
over ~/.justrust/runs: first error E0308 52%, syntax 7%, E0425 5%, E0063,
E0433, E0599, E0061, E0432, E0277, E0609, E0004, E0384...).

For each mutation of one file in the pinned Jcode or Desktop sources:
  1. ask the workspace's warm early daemon, exactly as an agent build does
     (op `diagnostics`: the files changed since the last query),
  2. run `justrust check -p <crate> --all-targets` (rustc's verdict),
  3. restore the file (new mtime, so cargo never trusts a stale fingerprint).

Early exit (`JUSTRUST_EARLY_EXIT=1`) fires when the answer is non-empty and
every error carries a rustc code (`early::exit_eligible`). Per mutation:
  fired          early exit would have returned
  correct        fired, rustc failed, every early error at a rustc error line
  wrong          fired but rustc passed, or an early error rustc did not report
  partial        correct, but rustc reported more errors in the changed file
  near           same file and code, rustc's line within 5 (same expression)
  superset       every rustc error of the file, plus ones rustc did not report
Clean edits (should compile) and a clean sweep of every file count
false exits on code that compiles.

Needs a private copy of the snapshot (EARLY_EXIT_ROOT with jcode/ and
jcode-desktop/, target dirs inside) and `justrust early start` in both
workspaces. Untimed (no bench lock): rust-analyzer latency is recorded but
the bench numbers that matter are counts.

Usage: mutations.py [--n 70] [--seed 1] [--only desktop|jcode]
Writes bench/results/early-exit-mutations-<time>.json.
"""
import argparse
import signal
import json
import os
import random
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent / "results"
J = os.environ.get("JUSTRUST", str(Path.home() / ".cargo/bin/justrust"))
ROOT = Path(os.environ.get("EARLY_EXIT_ROOT", Path.home() / ".justrust/bench/early-exit-work"))
JR_HOME = Path(os.environ.get("JUSTRUST_HOME", Path.home() / ".justrust"))
WS = {"desktop": ROOT / "jcode-desktop", "jcode": ROOT / "jcode"}
TARGET = {"desktop": ROOT / "target", "jcode": ROOT / "target-jcode"}
# Crates mutated per workspace (where agents edit; jcode-base/app-core/tui
# are 140-220k lines, check -p of them is the slow end).
CRATES = {
    "desktop": ["jcode-desktop-ui", "jcode-desktop-ui-core", "jcode-desktop-model",
                "jcode-desktop-harness", "jcode-desktop-api", "jcode-desktop-accounts-ui"],
    "jcode": ["jcode-tui", "jcode-app-core", "jcode-base", "jcode-protocol", "jcode-plan",
              "jcode-provider-core", "jcode-tui-markdown", "jcode-telemetry-core"],
}

ID = r"[a-z_][a-z0-9_]*"


def op_sites(lines):
    """(op, line index, new line(s)) candidates for one file."""
    out = []
    for i, l in enumerate(lines):
        s = l.strip()
        if s.startswith("//") or s.startswith("#[") or s.startswith("///"):
            continue
        ind = l[: len(l) - len(l.lstrip())]
        # E0308 family (half of real first errors).
        if re.search(r"\bOk\(\(\)\)\s*$", s) and not s.startswith("return"):
            out.append(("ok_unit_to_unit", i, l.replace("Ok(())", "()")))
        if s.endswith("?;") and "let " in s:
            out.append(("drop_question_mark", i, l[: l.rindex("?;")] + ";"))
        if ".to_string()" in s and s.count(".to_string()") == 1:
            out.append(("drop_to_string", i, l.replace(".to_string()", "", 1)))
        m = re.search(r" as (usize|u32|u64|i64|f32|f64|u16|u8)\b", l)
        if m:
            out.append(("drop_as_cast", i, l[: m.start()] + l[m.end():]))
        if re.search(r"\) -> [A-Za-z&]", l) and s.endswith("{") and "fn " in s:
            out.append(("let_u32_str", i, l + "\n" + ind + '    let _: u32 = "mutation";'))
        m = re.search(r"Some\(([a-z_][a-z0-9_.]*)\)", l)
        if m and "=>" not in l and "if let" not in l and "let Some" not in l:
            out.append(("unwrap_some", i, l[: m.start()] + m.group(1) + l[m.end():]))
        m = re.search(r"= (true|false);", l)
        if m:
            out.append(("bool_to_int", i, l[: m.start()] + "= 1;" + l[m.end():]))
        # Syntax.
        if s.startswith("let ") and s.endswith(";") and len(s) > 12:
            out.append(("syntax_drop_semicolon", i, l.rstrip()[:-1]))
        if s.endswith(");") and s.count("(") == s.count(")") and s.count("(") == 1:
            out.append(("syntax_drop_paren", i, l.rstrip()[:-2] + ";"))
        # E0425: typo in a free function call or a local.
        m = re.search(r"(?<![.:\w])(" + ID + r")\((?!\))", l)
        if m and m.group(1) not in ("if", "while", "match", "for", "fn", "Some", "Ok", "Err", "return",
                                    "vec", "matches", "assert", "format", "println", "write", "writeln"):
            out.append(("typo_fn_call", i, l[: m.start(1)] + m.group(1) + "_x" + l[m.end(1):]))
        m = re.match(r"\s*let (?:mut )?(" + ID + r")\b", l)
        if m and len(m.group(1)) > 2 and i + 1 < len(lines):
            name = m.group(1)
            for j in range(i + 1, min(i + 12, len(lines))):
                mm = re.search(r"(?<![\w.:])" + name + r"(?!\w)(?!\s*:)", lines[j])
                if mm and not lines[j].strip().startswith("let ") and not lines[j].strip().startswith("//"):
                    out.append(("typo_local", j, lines[j][: mm.start()] + name + "x" + lines[j][mm.end():]))
                    break
            # E0384: assign to an immutable binding.
            if "let mut" not in l and s.endswith(";") and "=" in s:
                out.append(("assign_immutable", i, l + "\n" + ind + f"{name} = {name};"))
        # E0433: misspelled type path.
        m = re.search(r"(?<![\w:])([A-Z][A-Za-z0-9]+)::[a-z_]", l)
        if m and m.group(1) not in ("Self",):
            out.append(("typo_type_path", i, l[: m.start(1)] + m.group(1) + "x" + l[m.end(1):]))
        # E0599: misspelled method.
        m = re.search(r"\.(" + ID + r")\(", l)
        if m and len(m.group(1)) > 2:
            out.append(("typo_method", i, l[: m.start(1)] + m.group(1) + "_x" + l[m.end(1):]))
        # E0061: extra argument.
        m = re.search(r"\.(" + ID + r")\(([^()]+)\)", l)
        if m:
            out.append(("extra_arg", i, l[: m.end(2)] + ", 0" + l[m.end(2):]))
        # E0609: misspelled field.
        m = re.search(r"\bself\.(" + ID + r")\b(?!\s*\()", l)
        if m:
            out.append(("typo_field", i, l[: m.start(1)] + m.group(1) + "x" + l[m.end(1):]))
        # E0063 (struct literal) / E0027 (pattern): a field initializer gone.
        if re.match(r"\s+" + ID + r": [^:].*,$", l) and not s.startswith("pub ") and "=>" not in s:
            prev = lines[i - 1].rstrip() if i else ""
            if prev.endswith("{") or prev.endswith(","):
                out.append(("drop_field_init", i, None))
        # E0432: unknown item in an import list.
        if s.startswith("use ") and "{" in s and s.endswith("};"):
            out.append(("bad_import", i, l.replace("};", ", NoSuchItemMutation};", 1)))
        # E0277: `?` on a non-Result.
        if re.search(r"\.len\(\)", l) and "?" not in l:
            out.append(("question_on_usize", i, l.replace(".len()", ".len()?", 1)))
        # E0004: drop a wildcard arm.
        if re.match(r"\s*_ => .*,$", l):
            out.append(("drop_wildcard_arm", i, None))
    return out


# Weights follow the mined first-error mix (E0308 52%, syntax 7%, E0425 5%,
# E0063 5%, E0433 4%, E0599 3%, E0061 3%, E0432 2%, E0277 2%, E0609 2%...).
WEIGHTS = {
    "ok_unit_to_unit": 6, "drop_question_mark": 6, "drop_to_string": 7, "drop_as_cast": 5,
    "let_u32_str": 4, "unwrap_some": 6, "bool_to_int": 4,
    "syntax_drop_semicolon": 4, "syntax_drop_paren": 3,
    "typo_fn_call": 3, "typo_local": 3, "assign_immutable": 2,
    "typo_type_path": 4, "typo_method": 3, "extra_arg": 3, "typo_field": 2,
    "drop_field_init": 5, "bad_import": 2, "question_on_usize": 2, "drop_wildcard_arm": 2,
}

CLEAN = ["black_box_stmt", "new_private_fn", "comment_line"]


def crate_of(path: Path) -> str | None:
    for d in path.parents:
        c = d / "Cargo.toml"
        if c.exists():
            m = re.search(r'^name\s*=\s*"([^"]+)"', c.read_text(), re.M)
            return m.group(1) if m else None
    return None


def crate_dir(ws: Path, name: str) -> Path:
    for c in [ws / "crates" / name, ws / name]:
        if (c / "Cargo.toml").exists():
            return c
    sys.exit(f"no crate {name} in {ws}")


def daemon(ws: Path, op: dict, timeout=120) -> dict:
    for sp in (JR_HOME / "early").glob("*/sock"):
        log = (sp.parent / "daemon.log").read_text(errors="replace")
        if f"daemon up for {ws}\n" in log or log.rstrip().endswith(f"daemon up for {ws}") or f"daemon up for {ws} " in log:
            s = socket.socket(socket.AF_UNIX)
            s.settimeout(timeout)
            try:
                s.connect(str(sp))
            except OSError:
                continue
            s.sendall((json.dumps(op) + "\n").encode())
            buf = b""
            while not buf.endswith(b"\n"):
                c = s.recv(1 << 20)
                if not c:
                    break
                buf += c
            return json.loads(buf)
    sys.exit(f"no early daemon for {ws}: (cd {ws} && justrust early start)")


LOC = re.compile(r"^\s*--> (.+):(\d+):(\d+)$")


def rustc_errors(stderr: str, ws: Path):
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
                f = m.group(1)
                f = f if f.startswith("/") else os.path.normpath(ws / f)
                out.append((f, int(m.group(2)), code))
                code = None
    return out


def check(kind: str, crate: str):
    env = {**os.environ, "CARGO_TARGET_DIR": str(TARGET[kind]), "JUSTRUST_REMOTE": "0",
           "JUSTRUST_TOOLCHAIN": "system", "JUSTRUST_BUILDD": "0", "JUSTRUST_PREBUILT": "off"}
    for k in ("JUSTRUST_EARLY", "JUSTRUST_EARLY_EXIT"):
        env.pop(k, None)
    t = time.monotonic()
    p = subprocess.run([J, "check", "-p", crate, "--all-targets"], cwd=WS[kind], env=env,
                       capture_output=True, text=True)
    return p.returncode, rustc_errors(p.stderr, WS[kind]), time.monotonic() - t


def eligible(diags):
    return bool(diags) and all(d["code"] for d in diags)


def classify(diags, rerr, rc, path):
    fired = eligible(diags)
    rset = {(f, l) for f, l, _ in rerr}
    in_file = {(f, l) for f, l, _ in rerr if f == str(path)}
    eset = {(d["file"], d["line"]) for d in diags}
    if not fired:
        return "no_exit"
    if rc == 0:
        return "wrong_build_passed"
    if not eset <= rset:
        # Same error, reported on another line of the same expression
        # (rust-analyzer marks the end of a method chain, rustc its start).
        near = all(any(f == d["file"] and c == d["code"] and abs(l - d["line"]) <= 5 for f, l, c in rerr)
                   for d in diags)
        if near:
            return "near"
        # Every rustc error of the file found, plus errors rustc did not
        # report (rust-analyzer's set is bigger): right place, extra noise.
        if in_file and in_file <= eset:
            return "superset"
        return "wrong_location"
    if in_file - eset:
        return "partial"
    return "correct"


def mutate_text(lines, op, i, new):
    lines = list(lines)
    if new is None:
        del lines[i]
    else:
        lines[i] = new
    return "\n".join(lines) + "\n"


def clean_text(lines, op, rng):
    lines = list(lines)
    if op == "comment_line":
        i = rng.randrange(len(lines))
        lines.insert(i, "// early-exit bench comment")
    elif op == "new_private_fn":
        lines.append("#[allow(dead_code)]\nfn early_exit_bench_probe() -> u32 {\n    1\n}")
    else:
        cands = [i for i, l in enumerate(lines)
                 if re.search(r"\bfn \w+\(.*\)( -> .*)? \{$", l.strip()) and not l.strip().startswith("const")]
        if not cands:
            return None
        i = rng.choice(cands)
        ind = lines[i][: len(lines[i]) - len(lines[i].lstrip())]
        lines.insert(i + 1, ind + "    std::hint::black_box(0u64);")
    return "\n".join(lines) + "\n"


def files_of(kind):
    out = []
    for c in CRATES[kind]:
        d = crate_dir(WS[kind], c) if kind == "desktop" else crate_dir(WS[kind], c)
        for p in sorted((d / "src").rglob("*.rs")):
            out.append((c, p))
    return out


def run_kind(kind, n, n_clean, rng, rows):
    files = files_of(kind)
    daemon(WS[kind], {"op": "diagnostics"})
    plan = []
    ops = list(WEIGHTS)
    tries = 0
    while len(plan) < n and tries < n * 50:
        tries += 1
        op = rng.choices(ops, weights=[WEIGHTS[o] for o in ops])[0]
        crate, path = rng.choice(files)
        lines = path.read_text().split("\n")
        if lines and lines[-1] == "":
            lines.pop()
        sites = [s for s in op_sites(lines) if s[0] == op]
        if not sites:
            continue
        _, i, new = rng.choice(sites)
        if any(p[1] == path and p[3] == i for p in plan):
            continue
        plan.append((op, path, crate, i, new, lines))
    for _ in range(n_clean):
        crate, path = rng.choice(files)
        lines = path.read_text().split("\n")
        if lines and lines[-1] == "":
            lines.pop()
        plan.append((rng.choice(CLEAN), path, crate, None, None, lines))
    for k, (op, path, crate, i, new, lines) in enumerate(plan):
        orig = path.read_text()
        text = mutate_text(lines, op, i, new) if op not in CLEAN else clean_text(lines, op, rng)
        if text is None:
            continue
        try:
            path.write_text(text)
            t = time.monotonic()
            r = daemon(WS[kind], {"op": "diagnostics", "raw": True}, timeout=30)
            ra_ms = (time.monotonic() - t) * 1000
            rc, rerr, rustc_s = check(kind, crate)
        finally:
            path.write_text(orig)  # new mtime: cargo re-checks it
        daemon(WS[kind], {"op": "diagnostics"})
        diags = r.get("diags", []) if r.get("ready") else []
        verdict = classify(diags, rerr, rc, path)
        row = {
            "kind": kind, "op": op, "clean": op in CLEAN, "file": str(path.relative_to(ROOT)),
            "line": None if i is None else i + 1, "crate": crate, "ready": r.get("ready"),
            "ra_ms": round(ra_ms, 1), "rustc_s": round(rustc_s, 2), "rc": rc,
            "early": [(d["file"].replace(str(ROOT) + "/", ""), d["line"], d["code"]) for d in diags],
            "rustc": [(f.replace(str(ROOT) + "/", ""), l, c) for f, l, c in rerr],
            "ra_raw_sev1": [d for d in r.get("raw", []) if d.get("severity") == 1][:8],
            "verdict": verdict,
        }
        rows.append(row)
        print(f"[{kind} {k + 1}/{len(plan)}] {verdict:18s} {op:22s} rc {rc} rustc {[c for *_, c in rerr][:4]} "
              f"early {[c for *_, c in row['early']][:4]} ra {ra_ms:.0f} ms rustc {rustc_s:.1f}s "
              f"{row['file']}:{row['line']}", flush=True)


def summarize(rows):
    def agg(rs):
        failing = [r for r in rs if not r["clean"] and r["rc"] != 0]
        fired = [r for r in rs if r["verdict"] != "no_exit"]
        return {
            "mutations": sum(not r["clean"] for r in rs),
            "rustc_failed": len(failing),
            "fired": len(fired),
            "correct": sum(r["verdict"] == "correct" for r in rs),
            "partial": sum(r["verdict"] == "partial" for r in rs),
            "near": sum(r["verdict"] == "near" for r in rs),
            "superset": sum(r["verdict"] == "superset" for r in rs),
            "wrong_build_passed": sum(r["verdict"] == "wrong_build_passed" for r in rs),
            "wrong_location": sum(r["verdict"] == "wrong_location" for r in rs),
            "recall": round(sum(r["verdict"] in ("correct", "partial", "near", "superset") for r in failing) / max(1, len(failing)), 3),
            "clean_edits": sum(r["clean"] for r in rs),
            "clean_false_exits": sum(r["clean"] and r["verdict"] != "no_exit" for r in rs),
            "ra_ms_median": sorted(r["ra_ms"] for r in rs)[len(rs) // 2] if rs else None,
            "rustc_s_median_fired": sorted(r["rustc_s"] for r in fired)[len(fired) // 2] if fired else None,
        }
    out = {"all": agg(rows)}
    for k in ("desktop", "jcode"):
        rs = [r for r in rows if r["kind"] == k]
        if rs:
            out[k] = agg(rs)
    by_op = {}
    for r in rows:
        if r["clean"]:
            continue
        b = by_op.setdefault(r["op"], {"n": 0, "rustc_failed": 0, "fired_correct": 0, "partial": 0, "near": 0, "superset": 0, "wrong": 0})
        b["n"] += 1
        b["rustc_failed"] += r["rc"] != 0
        b["fired_correct"] += r["verdict"] == "correct"
        b["partial"] += r["verdict"] == "partial"
        b["near"] += r["verdict"] == "near"
        b["superset"] += r["verdict"] == "superset"
        b["wrong"] += r["verdict"].startswith("wrong")
    out["by_op"] = by_op
    return out


def sweep(kind):
    files = [str(p) for _, p in files_of(kind)]
    t = time.monotonic()
    r = daemon(WS[kind], {"op": "pull", "files": files, "raw": True}, timeout=900)
    secs = time.monotonic() - t
    # The daemon treats errors in never-edited files as their baseline and
    # returns none; the raw list shows what rust-analyzer flags (coded
    # severity-1 errors, before justrust's filters).
    ex = [d for d in r.get("raw", []) if d.get("severity") == 1
          and re.fullmatch(r"E\d{4}", str(d.get("code") or ""))]
    print(f"clean sweep {kind}: {len(r.get('files', []))}/{len(files)} files in {secs:.0f}s, "
          f"{len(ex)} coded errors on unedited sources", flush=True)
    for d in ex[:20]:
        print(f"  flagged {d['code']} {d['file'].replace(str(ROOT) + '/', '')}:{d['line']} {d['message'][:90]}")
    # What an agent sees: a clean edit (a comment line) of each flagged
    # file, asked as a build asks. The daemon's per-file baseline (errors
    # before the first edit) should cancel them.
    false_exits = []
    for f in sorted({d["file"] for d in ex}):
        p = Path(f)
        orig = p.read_text()
        try:
            p.write_text("// early-exit sweep\n" + orig)
            a = daemon(ws_of(p), {"op": "diagnostics"}, timeout=60)
        finally:
            p.write_text(orig)
        daemon(ws_of(p), {"op": "diagnostics"})
        if a.get("ready") and eligible(a.get("diags", [])):
            false_exits.append({"file": f, "diags": a["diags"]})
            print(f"  FALSE EXIT after a clean edit: {f}")
    print(f"  clean edits of the {len({d['file'] for d in ex})} flagged files: {len(false_exits)} false exits")
    return {"files": len(files), "pulled": len(r.get("files", [])), "secs": round(secs, 1),
            "flagged_unedited": len(ex), "flagged_files": len({d["file"] for d in ex}),
            "false_exits": len(false_exits), "false_exit_files": false_exits, "diags": ex[:50]}


def ws_of(p: Path) -> Path:
    return WS["desktop"] if WS["desktop"] in p.parents else WS["jcode"]


def main():
    # A kill (pkill, a server restart) must still restore the mutated file:
    # SIGTERM/SIGHUP unwind through the `finally` that writes it back.
    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda *_: sys.exit(1))
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=70)
    ap.add_argument("--clean", type=int, default=12)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--only")
    ap.add_argument("--no-sweep", action="store_true")
    a = ap.parse_args()
    rng = random.Random(a.seed)
    rows = []
    res = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "seed": a.seed, "root": str(ROOT),
           "justrust": subprocess.run(["git", "-C", str(HERE.parent.parent), "rev-parse", "--short", "HEAD"],
                                      capture_output=True, text=True).stdout.strip()}
    kinds = [a.only] if a.only else ["desktop", "jcode"]
    if not a.no_sweep:
        res["sweep"] = {k: sweep(k) for k in kinds}
    for k in kinds:
        run_kind(k, a.n, a.clean, rng, rows)
    res["rows"] = rows
    res["totals"] = summarize(rows)
    print(json.dumps(res["totals"], indent=1))
    out = RESULTS / f"early-exit-mutations-{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.write_text(json.dumps(res, indent=1) + "\n")
    print(f"saved {out}")


if __name__ == "__main__":
    main()
