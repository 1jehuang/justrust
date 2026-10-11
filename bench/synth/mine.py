#!/usr/bin/env python3
"""Mine Jcode sessions for what agents edit before each build, and what it costs.

For every foreground cargo/justrust call in a Rust repo, collect the edits
the agent made since its previous build call (edit, write, replace,
apply_patch, also inside batch), classify them along the synthetic
benchmark's axes, and attribute the call's wall time to that cell:

  command   check | test | test-filtered | test-workspace | build | build-release | clippy
  change    none | body | test | item | type-error | manifest | new-file
  spread    one-file | multi-file | multi-crate
  relation  none | target (edited the crate being built) | upstream (another crate) | outside
  outcome   ok | compile-error

`change` is the heaviest edit in the set (manifest > new-file > item > test
> body). `type-error` is assigned from the outcome: the build failed to
compile, which is what the benchmark has to reproduce, whatever the edit.

Output: bench/synth/weights.json (aggregates only, no paths or code), which
bench/synth/gen.py samples from.

  bench/synth/mine.py [--sessions DIR] [--since YYYY-MM-DD] [--out FILE]
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TIMING = re.compile(r"tool timing: start=\S+ finish=\S+ duration=([\d.]+)(ms|s)")
CARGO_SUB = re.compile(r"(?:cargo|justrust)\s+(?:\+\S+\s+)?(?:-\S+\s+)*([a-z-]+)")
ERROR = re.compile(r"(?m)^error(\[E\d+\])?:")
ASYNC = re.compile(r'queued in background|^\s*\{\s*"acknowledged"|^Reload initiated', re.M)
PKG = re.compile(r"(?:-p|--package)\s+(\S+)")
# A changed line that declares or changes an item's interface.
ITEM = re.compile(
    r"^\s*(pub(\([^)]*\))?\s+)?(async\s+|const\s+|unsafe\s+|extern\s+\"C\"\s+)*"
    r"(fn|struct|enum|trait|type|impl|mod|use|const|static|macro_rules!)\b|^\s*#\[derive"
)
TESTISH = re.compile(r"#\[(tokio::)?test\]|\bassert(_eq|_ne)?!|mod tests\b")
RANK = {"body": 1, "test": 2, "item": 3, "new-file": 4, "vcs": 5, "manifest": 6}


def ts(v) -> float | None:
    try:
        return dt.datetime.fromisoformat(v.replace("Z", "+00:00")).timestamp()
    except Exception:
        return None


def result_text(c) -> str:
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "\n".join(i.get("text", "") for i in c if isinstance(i, dict))
    return ""


def crate_of(path: str, cache: dict) -> str | None:
    """Package name owning `path`: the nearest Cargo.toml with [package]."""
    p = Path(path)
    for d in [p.parent, *p.parent.parents]:
        if d in cache:
            return cache[d]
        m = d / "Cargo.toml"
        if m.exists():
            name = None
            try:
                txt = m.read_text(errors="replace")
                sec = re.search(r"(?ms)^\[package\](.*?)(^\[|\Z)", txt)
                if sec:
                    nm = re.search(r'(?m)^name\s*=\s*"([^"]+)"', sec.group(1))
                    name = nm.group(1) if nm else None
            except OSError:
                pass
            if name:
                cache[d] = name
                return name
        if d == Path.home() or str(d) == "/":
            break
    # File gone since: guess from a crates/<name>/ layout.
    m = re.search(r"/crates/([^/]+)/", path)
    return m.group(1) if m else None


def changed_lines(old: str, new: str) -> list[str]:
    a, b = old.splitlines(), new.splitlines()
    sa, sb = set(a), set(b)
    return [x for x in a if x not in sb] + [x for x in b if x not in sa]


def classify_edit(path: str, old: str, new: str, is_write: bool) -> str:
    if path.endswith("Cargo.toml"):
        return "manifest"
    if is_write:
        return "new-file"
    lines = changed_lines(old, new)
    if any(ITEM.match(x) for x in lines):
        return "item"
    if TESTISH.search(old) or TESTISH.search(new) or "/tests/" in path:
        return "test"
    return "body"


def patch_edits(text: str, wd: str) -> list[tuple[str, str]]:
    """(path, kind) for a Codex-style patch."""
    out = []
    for blk in re.split(r"(?m)^\*\*\* (?=(?:Update|Add|Delete) File: )", text)[1:]:
        head, _, body = blk.partition("\n")
        op, _, rel = head.partition(" File: ")
        path = rel.strip()
        if not path.startswith("/"):
            path = os.path.join(wd, path)
        old = "\n".join(l[1:] for l in body.splitlines() if l.startswith("-"))
        new = "\n".join(l[1:] for l in body.splitlines() if l.startswith("+"))
        out.append((path, classify_edit(path, old, new, op == "Add")))
    return out


# Shell commands that change sources without an edit tool. Classified
# coarsely: the exact edit is unknown.
BASH_GIT = re.compile(r"\bgit\s+(checkout|switch|pull|rebase|merge|stash|reset|apply|am|cherry-pick|restore)\b")
BASH_WRITE = re.compile(r"\bsed\s+-i|>\s*\S+\.rs\b|\btee\s+\S+\.rs\b|\bcargo\s+(add|remove|update)\b|"
                        r"\bperl\s+-[a-z]*i|write_text|\.write\(|\bmv\s+\S+\.rs\b|\bcp\s+\S+\s+\S+\.rs\b")


RS_PATH = re.compile(r"(?<![\w$])((?:~|/|\.{0,2}/)?[\w.-]+(?:/[\w.-]+)*\.rs|(?:[\w.-]+/)*Cargo\.toml)\b")
CD = re.compile(r"^\s*cd\s+(\S+)")


def bash_edits(cmd: str, wd: str) -> list[tuple[str, str]]:
    out = []
    if BASH_GIT.search(cmd):
        out.append((os.path.join(wd, "<git>.rs"), "vcs"))
    if re.search(r"\bcargo\s+(add|remove|update)\b", cmd):
        out.append((os.path.join(wd, "Cargo.toml"), "manifest"))
    elif BASH_WRITE.search(cmd):
        # Scripted edits (python, sed -i, heredocs) name the files they
        # change. The kind of change is not parsed: count it as a body edit.
        base = wd
        m = CD.match(cmd)
        if m:
            d = os.path.expanduser(m.group(1).strip("'\""))
            base = d if d.startswith("/") else os.path.join(wd, d)
        paths = []
        for p in RS_PATH.findall(cmd):
            if "JCODE_SCRATCH" in p or "/tmp/" in p or "/scratch/" in p or "target/" in p:
                continue
            p = os.path.expanduser(p)
            paths.append(os.path.normpath(p if p.startswith("/") else os.path.join(base, p)))
        if paths:
            out += [(p, "manifest" if p.endswith("Cargo.toml") else "body") for p in dict.fromkeys(paths)]
        elif not re.search(r"\.write\(|write_text", cmd):
            out.append((os.path.join(wd, "<bash>.rs"), "body"))
    return out


def edits_of(name: str, inp: dict, wd: str) -> list[tuple[str, str]]:
    def absp(p):
        return p if p.startswith("/") else os.path.join(wd, p)

    name = name.lower()
    if name in ("bash",):
        return bash_edits(inp.get("command", "") or "", wd)
    if name == "multiedit":
        name = "edit"
    if name == "edit":
        path = absp(inp.get("file_path", ""))
        eds = inp.get("edits") or [{"old_string": inp.get("old_string", ""), "new_string": inp.get("new_string", "")}]
        return [(path, classify_edit(path, e.get("old_string", "") or "", e.get("new_string", "") or "", False))
                for e in eds]
    if name == "write":
        return [(absp(inp.get("file_path", "")), "new-file")]
    if name == "replace":
        path = absp(inp.get("path", "") or ".")
        return [(path, classify_edit(path, inp.get("pattern", ""), inp.get("replacement", ""), False))]
    if name == "apply_patch":
        return patch_edits(inp.get("patch_text", ""), wd)
    return []


def build_command(cmds: list[str]) -> tuple[str, list[str]] | None:
    """Normalized command kind and its -p packages, or None if not a build."""
    for c in cmds:
        if c.startswith("<"):  # selfdev tools: count as a scoped test/build
            if ":test>" in c:
                return "test", []
            if ":build" in c:
                return "build", []
        for m in CARGO_SUB.finditer(c):
            sub = m.group(1)
            rest = c[m.end():].split("|")[0].split("&&")[0].split(";")[0]
            pkgs = PKG.findall(rest)
            flags = sorted({f for f in ("--lib", "--tests", "--all-targets", "--bins", "--examples")
                            if f in rest.split(" -- ")[0].split()})
            pkgs = [p.strip("'\"") for p in pkgs]
            pkgs = [p for p in pkgs if re.fullmatch(r"[A-Za-z0-9_-]+", p)] + ["|".join(flags)]
            if sub in ("test", "nextest"):
                if "--workspace" in rest or "--all" in rest.split():
                    return "test-workspace", pkgs
                tail = rest.split(" -- ")[0].split()
                # A positional filter (not a flag or a flag's value).
                filt, skip = False, False
                for w in tail:
                    if skip:
                        skip = False
                        continue
                    if w in ("-p", "--package", "--features", "--test", "--bin", "-j", "--target-dir", "--profile"):
                        skip = True
                        continue
                    if not w.startswith("-"):
                        filt = True
                return ("test-filtered" if filt else "test"), pkgs
            if sub == "check":
                return "check", pkgs
            if sub == "clippy":
                return "clippy", pkgs
            if sub in ("build", "run"):
                return ("build-release" if "--release" in rest else "build"), pkgs
    return None


def is_rust_repo(wd: str, cache: dict) -> bool:
    if wd not in cache:
        cache[wd] = (Path(wd) / "Cargo.toml").exists()
    return cache[wd]


def mine(sessions: Path, since: float) -> tuple[list[dict], dict]:
    rows = []
    crate_cache: dict = {}
    repo_cache: dict = {}
    stats = collections.Counter()
    for f in sessions.glob("session_*.json"):
        if f.stat().st_mtime < since:
            continue
        try:
            s = json.loads(f.read_bytes())
        except Exception:
            continue
        wd = s.get("working_dir") or ""
        if not wd or not is_rust_repo(wd, repo_cache):
            continue
        pending_edits: list[tuple[str, str]] = []
        open_calls: dict[str, dict] = {}
        nth = 0  # build calls so far in this session
        last_cmd = None
        for m in s.get("messages", []):
            for b in m.get("content") or []:
                if not isinstance(b, dict):
                    continue
                if b.get("type") == "tool_use":
                    name, inp = b.get("name", ""), b.get("input") or {}
                    subs = [(x.get("tool", ""), x) for x in inp.get("tool_calls", [])] if name == "batch" else [(name, inp)]
                    cmds = []
                    for n, i in subs:
                        pending_edits.extend(edits_of(n, i, wd))
                        if n in ("bash", "Bash"):
                            c = i.get("command", "")
                            if "cargo" in c or "justrust" in c:
                                cmds.append(c)
                        elif n in ("selfdev", "desktop_selfdev"):
                            cmds.append(f"<{n}:{i.get('action', '')}> {i.get('command', '')}")
                    bc = build_command(cmds)
                    if not bc:
                        continue
                    stats["build_calls"] += 1
                    flags = bc[1].pop() if bc[1] else ""
                    open_calls[b.get("id", "")] = {
                        "command": bc[0], "pkgs": bc[1], "flags": flags, "edits": pending_edits,
                        "background": bool(inp.get("run_in_background")), "t": ts(m.get("timestamp")),
                        "first": nth == 0, "same_as_last": last_cmd == bc,
                        "prev": last_cmd[0] if last_cmd else "",
                    }
                    nth += 1
                    last_cmd = bc
                    pending_edits = []
                elif b.get("type") == "tool_result" and b.get("tool_use_id") in open_calls:
                    call = open_calls.pop(b["tool_use_id"])
                    text = result_text(b.get("content"))
                    t = TIMING.search(text)
                    if t:
                        wall = float(t.group(1)) / (1000 if t.group(2) == "ms" else 1)
                    elif call["t"] and ts(m.get("timestamp")):
                        wall = ts(m.get("timestamp")) - call["t"]
                    else:
                        continue
                    if call["background"] or ASYNC.search(text) or wall <= 0 or wall > 1800:
                        stats["skipped_background_or_bad"] += 1
                        continue
                    rows.append(classify_call(call, wall, text, wd, crate_cache))
    return rows, dict(stats)


def classify_call(call: dict, wall: float, text: str, wd: str, cache: dict) -> dict:
    rs = [(p, k) for p, k in call["edits"] if p.endswith(".rs") or p.endswith("Cargo.toml")]
    outside = [p for p, _ in rs if not p.startswith(wd.rstrip("/") + "/")]
    crates = {crate_of(p, cache) for p, _ in rs if "<" not in p} - {None}
    change = max((k for _, k in rs), key=lambda k: RANK[k], default="none")
    known = [(p, k) for p, k in rs if "<" not in p]
    files = {p for p, _ in known}
    if not known:
        spread = "none" if not rs else "unknown"  # git or shell edits only
    else:
        spread = "multi-crate" if len(crates) > 1 else ("multi-file" if len(files) > 1 else "one-file")
    targets = set(call["pkgs"])
    if not rs:
        # Why nothing was edited: the session's first build (state unknown,
        # often stale after other agents' commits), the same command again
        # (rerun: flaky test, more output), or a different command on the
        # same sources (check then test, another -p).
        relation = "none"
        change = "first" if call["first"] else ("rerun" if call["same_as_last"] else "switch")
    elif not known:
        relation = "unknown"
    elif outside and len(outside) == len(rs):
        relation = "outside"  # e.g. a sibling repo used as a path dependency
    elif not targets or crates & targets:
        relation = "target"  # no -p: the workspace (or root crate) is the target
    else:
        relation = "upstream"
    failed = bool(ERROR.search(text)) and ("could not compile" in text or "error[E" in text)
    if failed:
        change = "type-error"
    return {"command": call["command"], "change": change, "spread": spread, "relation": relation,
            "outcome": "compile-error" if failed else "ok", "wall": round(wall, 2),
            "repo": Path(wd).name, "pkg": call["pkgs"][0] if call["pkgs"] else "", "flags": call["flags"],
            "files": min(len(files), 9), "crates": min(len(crates), 9), "edited": sorted(crates),
            "prev": call["prev"] if change == "switch" else ""}


def aggregate(rows: list[dict]) -> dict:
    total = sum(r["wall"] for r in rows)
    axes = ["command", "change", "spread", "relation", "outcome"]
    out = {"calls": len(rows), "wall_hours": round(total / 3600, 2), "axes": {}, "cells": []}
    for a in axes:
        c = collections.defaultdict(lambda: [0, 0.0])
        for r in rows:
            c[r[a]][0] += 1
            c[r[a]][1] += r["wall"]
        out["axes"][a] = {k: {"calls": n, "wall_share": round(w / total, 4), "mean_wall": round(w / n, 1)}
                          for k, (n, w) in sorted(c.items(), key=lambda kv: -kv[1][1])}
    cells = collections.defaultdict(lambda: [0, 0.0])
    for r in rows:
        key = (r["command"], r["change"], r["spread"], r["relation"])
        cells[key][0] += 1
        cells[key][1] += r["wall"]
    for (cmd, ch, sp, rel), (n, w) in sorted(cells.items(), key=lambda kv: -kv[1][1]):
        out["cells"].append({"command": cmd, "change": ch, "spread": sp, "relation": rel,
                             "calls": n, "wall_share": round(w / total, 4), "mean_wall": round(w / n, 1)})
    # The sampling table for bench/synth/gen.py: every distinct call shape
    # (categorical fields only, no paths or code) with its count.
    shape = ("repo", "command", "pkg", "flags", "change", "spread", "relation", "files", "crates", "prev")
    pts = collections.Counter(tuple(r[k] for k in shape) for r in rows)
    out["shape_fields"] = list(shape)
    out["shapes"] = [[*k, n] for k, n in sorted(pts.items(), key=lambda kv: (-kv[1], kv[0]))]
    # Which crates agents edit, per repo: the generator places edits by this.
    ed = collections.defaultdict(collections.Counter)
    for r in rows:
        for c in r["edited"]:
            ed[r["repo"]][c] += 1
    out["edited_crates"] = {repo: dict(c.most_common()) for repo, c in ed.items()}
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sessions", default=str(Path.home() / ".jcode" / "sessions"))
    ap.add_argument("--since", default="2026-09-01")
    ap.add_argument("--out", default=str(HERE / "weights.json"))
    ap.add_argument("--dump", help="also write every classified call as JSON lines")
    args = ap.parse_args()
    since = dt.datetime.fromisoformat(args.since).timestamp()
    rows, stats = mine(Path(args.sessions), since)
    if not rows:
        sys.exit("mine: no build calls found")
    agg = aggregate(rows)
    agg["mined"] = {"since": args.since, "at": dt.datetime.now().strftime("%Y-%m-%d"), **stats,
                    "repos": dict(collections.Counter(r["repo"] for r in rows).most_common(10))}
    Path(args.out).write_text(json.dumps(agg, indent=1) + "\n")
    if args.dump:
        Path(args.dump).write_text("".join(json.dumps(r) + "\n" for r in rows))
    print(f"{agg['calls']} foreground build calls, {agg['wall_hours']} h of agent wait -> {args.out}")
    for a, vals in agg["axes"].items():
        print(f"\n{a}")
        for k, v in vals.items():
            print(f"  {k:16s} {100 * v['wall_share']:5.1f}%  n={v['calls']:5d}  mean {v['mean_wall']:6.1f}s")
    print("\ntop cells")
    for c in agg["cells"][:20]:
        print(f"  {100 * c['wall_share']:5.1f}%  n={c['calls']:4d}  {c['mean_wall']:6.1f}s  "
              f"{c['command']:15s} {c['change']:10s} {c['spread']:11s} {c['relation']}")


if __name__ == "__main__":
    main()
