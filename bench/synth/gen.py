#!/usr/bin/env python3
"""Synthetic agent workload generator: weighted points -> concrete edits.

A *point* is one agent build call, described along the axes mined from
real sessions by `bench/synth/mine.py` (command, change, spread, relation,
...). `sample(seed, n)` draws points in proportion to how often agents
made that kind of call, then `plan()` maps each point onto the pinned
snapshots: which workspace, which `cargo` command, and which concrete
edits (files, lines) to make before it.

Edits are placed only at locations proven live: once per crate, `probe()`
inserts a type error after every candidate `fn ... {` line and runs
`cargo check`; a location whose probe errors is compiled code (right cfg,
not inside a string or an unexpanded macro). Locations live only with
`--tests` are test code. Results are cached per snapshot.

Same seed -> same points -> same edits, so two stacks, two justrust
versions, or two dates are compared on identical work.

  bench/synth/gen.py show [--seed S] [-n N]     print the sampled plan
  bench/synth/gen.py probe [CRATE...]           (re)probe crates now
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import suite  # noqa: E402

WEIGHTS = HERE / "weights.json"
WORK = Path(os.environ.get("JUSTRUST_SYNTH_WORK", suite.JR_HOME / "bench" / "synth"))
STOCK_CARGO = os.environ.get("VERSUS_STOCK_CARGO", "/usr/bin/cargo")

# Mined repo name -> (suite snapshot, workspace dir inside it).
WORKSPACES = {
    "jcode-desktop": ("desktop", "jcode-desktop"),
    "jcode": ("desktop", "jcode"),
    "justrust": ("justrust", "justrust"),
}

FN_LINE = re.compile(
    r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?'
    r"fn\s+\w+.*\{\s*$"
)
PROBES_PER_CRATE = 300


# --- snapshots and metadata ------------------------------------------------


def snapshot_root(snap: str) -> Path:
    """The pinned suite snapshot. Every benchmark holds the suite's lock
    (taken in prepare()) while it edits the sources, so sharing the
    extracted tree is safe, and versus's warm target dirs stay usable."""
    return suite.prepare(snap)


def stock_env() -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith(("JUSTRUST_", "CARGO_", "RUSTC", "RUSTFLAGS"))}
    env["PATH"] = ":".join(p for p in env.get("PATH", "").split(":")
                           if not (Path(p) / "cargo").resolve().name.startswith("justrust"))
    return env


@dataclass
class Pkg:
    name: str
    dir: Path
    manifest: Path
    targets: list[dict]
    member: bool
    deps: set[str] = field(default_factory=set)  # local deps, direct

    def root_file(self) -> Path | None:
        for kinds in (("lib", "rlib", "proc-macro", "cdylib", "staticlib", "dylib"), ("bin",)):
            for t in self.targets:
                if set(t["kind"]) & set(kinds):
                    return Path(t["src_path"])
        return None

    def has(self, kind: str) -> bool:
        return any(kind in t["kind"] or (kind == "lib" and set(t["kind"]) & {"rlib", "proc-macro", "cdylib", "dylib"})
                   for t in self.targets)


@dataclass
class Workspace:
    repo: str
    snap: str
    root: Path  # snapshot root
    cwd: Path
    pkgs: dict[str, Pkg]  # every local (path) package
    members: list[str]
    default: list[str]  # what a bare `cargo build` builds

    def closure(self, names: list[str]) -> set[str]:
        out, todo = set(), list(names)
        while todo:
            n = todo.pop()
            if n in out or n not in self.pkgs:
                continue
            out.add(n)
            todo.extend(self.pkgs[n].deps)
        return out


_WS: dict[str, Workspace] = {}


def workspace(repo: str) -> Workspace:
    if repo in _WS:
        return _WS[repo]
    snap, sub = WORKSPACES[repo]
    root = snapshot_root(snap)
    cwd = root / sub
    cache = WORK / "meta" / f"{snap}-{suite.snapshot_dir(snap).name}-{sub}.json"
    if cache.exists():
        meta = json.loads(cache.read_text())
    else:
        out = subprocess.run([STOCK_CARGO, "metadata", "--format-version", "1", "--offline"], cwd=cwd,
                             env=stock_env(), capture_output=True, text=True, check=True).stdout
        meta = json.loads(out)
        cache.parent.mkdir(parents=True, exist_ok=True)
        cache.write_text(out)
    ids = {p["id"]: p for p in meta["packages"]}
    members = {ids[i]["name"] for i in meta["workspace_members"]}
    pkgs = {}
    for p in meta["packages"]:
        if p["source"] is None:
            mf = Path(p["manifest_path"])
            pkgs[p["name"]] = Pkg(p["name"], mf.parent, mf, p["targets"], p["name"] in members)
    for node in meta["resolve"]["nodes"]:
        name = ids[node["id"]]["name"]
        if name in pkgs and ids[node["id"]]["source"] is None:
            pkgs[name].deps = {ids[d["pkg"]]["name"] for d in node["deps"]
                               if ids[d["pkg"]]["source"] is None}
    rootpkg = [n for n, p in pkgs.items() if p.member and p.dir == cwd]
    default = [ids[i]["name"] for i in meta.get("workspace_default_members", [])] or rootpkg or sorted(members)
    ws = Workspace(repo, snap, root, cwd, pkgs, sorted(members), default)
    _WS[repo] = ws
    return ws


def home_workspace(pkg: str) -> Workspace | None:
    """The workspace where `pkg` is a member (where an agent can `test -p` it)."""
    for repo in WORKSPACES:
        ws = workspace(repo)
        if pkg in ws.members:
            return ws
    return None


# --- probing live locations ------------------------------------------------


def rs_files(p: Pkg, ws: Workspace) -> list[Path]:
    """Source files of a package (not of packages nested inside it). Build
    scripts are left out: a probe there fails the script and hides every
    other probe. Examples and benches are not built by the commands agents run."""
    nested = [q.dir for q in ws.pkgs.values() if q.dir != p.dir and p.dir in q.dir.parents]
    skip = {Path(t["src_path"]) for t in p.targets if set(t["kind"]) & {"custom-build", "example", "bench"}}
    out = []
    for f in sorted(p.dir.rglob("*.rs")):
        parts = f.relative_to(p.dir).parts
        if "target" in parts or "examples" in parts or "benches" in parts or f in skip:
            continue
        if any(n in f.parents for n in nested):
            continue
        out.append(f)
    return out


def candidates(f: Path) -> list[int]:
    lines = f.read_text(errors="replace").splitlines()
    out = []
    for i, line in enumerate(lines):
        s = line.lstrip()
        if s.startswith("//") or "const fn" in line or line.count("{") != 1 or "}" in line:
            continue
        if FN_LINE.match(line):
            out.append(i)
    return out


def _check_targets(p: Pkg, tests: bool) -> list[str]:
    if tests:
        return ["--tests"]
    out = []
    if p.has("lib"):
        out.append("--lib")
    if p.has("bin"):
        out.append("--bins")
    return out


JOURNAL = WORK / "probe-journal"


def _recover_journal() -> None:
    """Undo a probe that was killed before it restored its files."""
    if not JOURNAL.exists():
        return
    idx = JOURNAL / "index.json"
    if idx.exists():
        for k, dst in json.loads(idx.read_text()).items():
            src = JOURNAL / k
            if src.exists():
                shutil.copy2(src, dst)
                print(f"synth: restored {dst} after an interrupted probe", file=sys.stderr)
    shutil.rmtree(JOURNAL)


def _locked() -> None:
    """Probes edit the shared snapshot sources: always under the bench lock,
    even with BENCH_NO_LOCK=1."""
    v = os.environ.pop("BENCH_NO_LOCK", None)
    try:
        suite.bench_lock()
    finally:
        if v is not None:
            os.environ["BENCH_NO_LOCK"] = v
    _recover_journal()


def _probe_run(ws: Workspace, p: Pkg, probes: dict[int, tuple[Path, int]], tests: bool) -> tuple[set[int], set[int], set[Path]]:
    """Insert every probe, check, restore.

    Returns (live probe ids, broken probe ids, files with other errors). Only
    E0308 (the mismatched type we inserted) proves a location live. A probe
    named in any other error (a parse error: inside a macro, an expression
    position) is broken; it may also have hidden the others, since parse
    errors stop the crate before type checking, so the caller retries.
    """
    by_file: dict[Path, list[tuple[int, int]]] = collections.defaultdict(list)
    for pid, (f, line) in probes.items():
        by_file[f].append((line, pid))
    _locked()  # before reading: another benchmark may have an edit in flight
    saved = {f: f.read_text() for f in by_file}
    JOURNAL.mkdir(parents=True, exist_ok=True)
    index = {}
    for k, f in enumerate(saved):
        (JOURNAL / str(k)).write_text(saved[f])
        index[str(k)] = str(f)
    (JOURNAL / "index.json").write_text(json.dumps(index))
    old_term = signal.signal(signal.SIGTERM, _raise_interrupt)
    try:
        for f, items in by_file.items():
            lines = saved[f].splitlines(keepends=True)
            for line, pid in sorted(items, reverse=True):
                lines.insert(line + 1, f'let _: u32 = "jrprobe_{pid}";\n')
            f.write_text("".join(lines))
        targets = _check_targets(p, tests)
        if not targets:
            return set(), set(), set()
        env = stock_env()
        env["CARGO_TARGET_DIR"] = str(WORK / "probe-target" / f"{ws.snap}-{ws.cwd.name}")
        home = home_workspace(p.name) or ws
        r = subprocess.run([STOCK_CARGO, "check", "-p", p.name, *targets, "--offline", "--message-format=json"],
                           cwd=home.cwd, env=env, capture_output=True, text=True)
        live, broken, bad = set(), set(), set()
        for line in r.stdout.splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("reason") != "compiler-message" or m["message"]["level"] not in ("error", "error: internal compiler error"):
                continue
            rendered = m["message"].get("rendered") or ""
            ids = {int(x) for x in re.findall(r"jrprobe_(\d+)", rendered)}
            for sp in m["message"].get("spans", []):
                for t in sp.get("text", []):
                    ids |= {int(x) for x in re.findall(r"jrprobe_(\d+)", t.get("text", ""))}
            if ids:
                if (m["message"].get("code") or {}).get("code") == "E0308":
                    live |= ids
                else:
                    broken |= ids
            elif m["message"].get("code") or "aborting" not in rendered:
                for sp in m["message"].get("spans", []):
                    bad.add((home.cwd / sp["file_name"]).resolve())
        if r.returncode != 0 and not live and not bad and not broken and "error: could not compile" not in r.stderr:
            sys.exit(f"synth: probe of {p.name} failed before compiling:\n{r.stderr[-2000:]}")
        return live, broken, bad
    finally:
        for f, s in saved.items():
            f.write_text(s)
        shutil.rmtree(JOURNAL, ignore_errors=True)
        signal.signal(signal.SIGTERM, old_term)


def _raise_interrupt(*_a) -> None:
    raise KeyboardInterrupt


def probe(ws: Workspace, name: str, force: bool = False) -> dict:
    """{rel file: {"body": [line...], "test": [line...]}} for one package."""
    p = ws.pkgs[name]
    cache = WORK / "probe" / suite.snapshot_dir(ws.snap).name / f"{name}.json"
    if cache.exists() and not force:
        return json.loads(cache.read_text())
    _locked()  # candidate lines must come from unedited sources
    rng = random.Random(name)
    cands = [(f, line) for f in rs_files(p, ws) for line in candidates(f)]
    rng.shuffle(cands)
    cands = cands[:PROBES_PER_CRATE]
    print(f"synth: probing {name} ({len(cands)} locations)", file=sys.stderr)
    result: dict[str, dict[str, list[int]]] = {}
    found = {}
    for tests in (False, True):
        probes = {i: c for i, c in enumerate(cands)}
        live: set[int] = set()
        for _ in range(6):
            live, broken, bad = _probe_run(ws, p, probes, tests)
            if not broken and not bad:
                break
            # Drop broken probes (and every probe in a file with an error we
            # did not insert), then retry: the rest may have been hidden.
            probes = {i: c for i, c in probes.items() if i not in broken and c[0].resolve() not in bad}
        else:
            live = set()
        found[tests] = {probes[i] for i in live if i in probes}
    for (f, line) in found[False] | found[True]:
        rel = str(f.relative_to(ws.root))
        kind = "body" if (f, line) in found[False] else "test"
        result.setdefault(rel, {"body": [], "test": []})[kind].append(line)
    for v in result.values():
        v["body"].sort()
        v["test"].sort()
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(result, indent=0) + "\n")
    nb = sum(len(v["body"]) for v in result.values())
    nt = sum(len(v["test"]) for v in result.values())
    print(f"synth: {name}: {nb} live body, {nt} live test locations in {len(result)} files", file=sys.stderr)
    return result


# --- sampling --------------------------------------------------------------


@dataclass
class Point:
    i: int
    shape: dict  # the mined axes
    calls: int  # how many real calls had this shape


def mappable(sh: dict) -> bool:
    if sh["repo"] not in WORKSPACES:
        return False
    if sh["pkg"]:
        return home_workspace(sh["pkg"]) is not None
    return True


def sample(seed: int, n: int) -> tuple[list[Point], dict]:
    """n points, systematic sampling in proportion to real call counts."""
    w = json.loads(WEIGHTS.read_text())
    fields = w["shape_fields"]
    shapes = [dict(zip(fields, s[:-1])) | {"_n": s[-1]} for s in w["shapes"]]
    total = sum(s["_n"] for s in shapes)
    ok = [s for s in shapes if mappable(s)]
    covered = sum(s["_n"] for s in ok)
    rng = random.Random(seed)
    rng.shuffle(ok)  # the seed picks the order, so the systematic grid hits different shapes
    cum, acc = [], 0
    for s in ok:
        acc += s["_n"]
        cum.append(acc)
    start = rng.random() * covered / n
    pts, j = [], 0
    for k in range(n):
        x = start + k * covered / n
        while cum[j] <= x:
            j += 1
        s = dict(ok[j])
        pts.append(Point(k, {f: s[f] for f in fields}, s["_n"]))
    info = {"shapes": len(shapes), "calls": total, "covered_calls": covered,
            "coverage": round(covered / total, 4), "mined": w.get("mined", {})}
    return pts, info


# --- planning --------------------------------------------------------------


@dataclass
class EditOp:
    kind: str  # body | test | type-error | item | new-file | manifest
    crate: str
    file: str  # relative to the snapshot root
    line: int = -1  # insert after this 0-based line (body/test/type-error)

    def label(self) -> str:
        return f"{self.kind}:{self.crate}:{Path(self.file).name}" + (f":{self.line + 1}" if self.line >= 0 else "")


@dataclass
class Plan:
    point: Point
    repo: str  # workspace actually used
    snap: str
    cwd: str  # relative to the snapshot root
    cmd: list[str]
    pre: list[str]  # brings the target dir to the pristine state, unmeasured
    prev: list[str] | None  # run after the edit, before the measured command (a "switch")
    edits: list[EditOp]
    expect_fail: bool
    note: str = ""

    def to_json(self) -> dict:
        return {"i": self.point.i, "shape": self.point.shape, "calls": self.point.calls, "repo": self.repo,
                "snap": self.snap, "cwd": self.cwd, "cmd": self.cmd, "pre": self.pre, "prev": self.prev,
                "edits": [e.label() for e in self.edits], "expect_fail": self.expect_fail, "note": self.note}


def test_filter(ws: Workspace, crate: str, edits: list[EditOp], rng: random.Random) -> str:
    """A module-name filter that selects some tests: the edited file's, like an
    agent testing what it just changed, else any test module of the crate."""
    pr = probe(ws, crate)
    for e in edits:
        if e.crate == crate and pr.get(e.file, {}).get("test"):
            return Path(e.file).stem
    files = sorted(f for f, v in pr.items() if v["test"])
    if not files:
        return "tests"
    stem = Path(rng.choice(files)).stem
    return Path(rng.choice(files)).parent.name if stem in ("mod", "lib", "main", "tests") else stem


def command(kind: str, pkg: str, flags: str, filt: str | None) -> list[str]:
    p = ["-p", pkg] if pkg else []
    fl = [f for f in flags.split("|") if f]
    if kind == "check":
        return ["check", *p, *fl]
    if kind == "clippy":
        return ["clippy", *p, *fl]
    if kind == "build":
        return ["build", *p, *[f for f in fl if f in ("--lib", "--bins")]]
    if kind == "build-release":
        return ["build", "--release", *p]
    if kind == "test-workspace":
        return ["test", "--workspace", *fl]
    if kind == "test":
        return ["test", *p, *fl]
    if kind == "test-filtered":
        return ["test", *p, *fl, "--", filt or "tests"]
    raise ValueError(kind)


def pre_command(cmd: list[str]) -> list[str]:
    """Build what `cmd` builds without running tests."""
    if cmd[0] != "test":
        return cmd
    base = cmd[:cmd.index("--")] if "--" in cmd else cmd
    return [*base, "--no-run"]


def weighted(rng: random.Random, names: list[str], weights: dict[str, int]) -> str:
    return rng.choices(names, [weights.get(n, 0) + 1 for n in names])[0]


def plan(pt: Point, seed: int) -> Plan:
    sh = pt.shape
    rng = random.Random(f"{seed}-{pt.i}-{json.dumps(sh, sort_keys=True)}")
    w = json.loads(WEIGHTS.read_text())
    edited = w.get("edited_crates", {}).get(sh["repo"], {})
    pkg = sh["pkg"]
    ws = home_workspace(pkg) if pkg else workspace(sh["repo"])
    assert ws is not None
    flags = sh["flags"]
    if pkg and "--lib" in flags and not ws.pkgs[pkg].has("lib"):
        flags = "|".join(f for f in flags.split("|") if f != "--lib")
    targets = [pkg] if pkg else (ws.members if sh["command"] == "test-workspace" else ws.default)
    deps = sorted(ws.closure(targets) - set(targets))
    change = sh["change"]
    if change == "first":
        change = "vcs"  # session start: sources moved since the last build (pulls, other agents)
    nfiles, ncrates = max(sh["files"], 1), max(sh["crates"], 1)
    rel = sh["relation"]
    edits: list[EditOp] = []
    note = ""

    def pick_crates(k: int) -> list[str]:
        if rel == "upstream" and deps:
            pool = deps
        elif rel == "outside":
            # Edited outside the repo: a sibling repo's crate this build
            # depends on, or something unrelated (no effect on the build).
            pool = [c for c in edited if c not in ws.members and c in ws.pkgs and c in ws.closure(targets)]
            if not pool:
                return []
        elif rel == "target":
            pool = sorted(targets) + (deps if k > 1 else [])
        else:  # unknown (scripted or git): anywhere in what this build compiles
            pool = sorted(targets) + deps
        pool = [c for c in pool if c in ws.pkgs]
        out = []
        for _ in range(min(k, len(pool))):
            c = weighted(rng, [c for c in pool if c not in out], edited)
            out.append(c)
        return out

    if change in ("rerun", "switch"):
        crates = []
    elif change == "vcs":
        crates = pick_crates(rng.randint(1, 3))
        nfiles = rng.randint(2, 6)
    else:
        crates = pick_crates(ncrates)
        if not crates and change == "type-error":
            # The build failed, so the error is in something it compiles.
            rel = "target"
            crates = pick_crates(ncrates)
        if not crates:
            note = "edit outside this build's crates: no effect"

    def live(crate: str, kind: str) -> list[tuple[str, int]]:
        pr = probe(home_workspace(crate) or ws, crate)
        return [(f, ln) for f, v in sorted(pr.items()) for ln in v[kind]]

    def live_any(crate: str) -> bool:
        # Only crates agents edit (or the build's own targets): probing every
        # dependency of a workspace would take an hour.
        return (crate in edited or crate in targets) and bool(live(crate, "body"))

    used: set[tuple[str, int]] = set()

    def place(kind: str, crate: str) -> EditOp | None:
        loc_kind = "test" if kind == "test" else "body"
        locs = [x for x in live(crate, loc_kind) if x not in used]
        if not locs and kind == "test":
            locs = [x for x in live(crate, "body") if x not in used]
        if kind in ("item",):
            files = sorted({f for f, _ in live(crate, "body")})
            return EditOp(kind, crate, rng.choice(files)) if files else None
        if kind == "new-file":
            rf = ws.pkgs[crate].root_file() if crate in ws.pkgs else None
            return EditOp(kind, crate, str(rf.relative_to(ws.root))) if rf else None
        if kind == "manifest":
            return EditOp(kind, crate, str(ws.pkgs[crate].manifest.relative_to(ws.root)))
        if not locs:
            return None
        f, ln = rng.choice(locs)
        used.add((f, ln))
        return EditOp(kind, crate, f, ln)

    if crates:
        main_kind = {"vcs": "body", "type-error": "type-error"}.get(change, change)
        kinds = [main_kind] + ["body"] * (max(nfiles, len(crates)) - 1)
        if change == "vcs":
            kinds = [rng.choice(["body", "body", "item"]) for _ in kinds]
        for j, kind in enumerate(kinds):
            crate = crates[j % len(crates)]
            op = place(kind, crate)
            if op is None and kind not in ("body", "type-error"):
                op = place("body", crate)
            if op:
                edits.append(op)
        if change == "type-error" and not any(e.kind == "type-error" for e in edits):
            note = "no live location for the type error"
    filt = None
    if sh["command"] == "test-filtered":
        filt = test_filter(ws, targets[0], edits, rng) if targets[0] in ws.pkgs else "tests"
    cmd = command(sh["command"], pkg, flags, filt)
    prev = None
    if change == "switch" and sh.get("prev"):
        pf = test_filter(ws, targets[0], [], rng) if sh["prev"] == "test-filtered" and targets[0] in ws.pkgs else None
        prev = command(sh["prev"], pkg, flags, pf)
        if prev[0] == "test":
            prev = pre_command(prev)
        if prev == pre_command(cmd):
            # Same kind of command, so the agent built another package
            # first: one that shares a dependency with this build.
            others = [m for m in ws.members if m not in targets and ws.closure([m]) & ws.closure(targets)]
            other = weighted(rng, others, edited) if others else None
            prev = pre_command(command(sh["prev"], other, "", pf)) if other else None
    if change == "switch" and prev is not None:
        # Switch = same sources, another command: something was edited and
        # built by the previous command; this one sees it for the first time.
        prev_pkg = prev[prev.index("-p") + 1] if "-p" in prev else None
        shared = sorted(ws.closure(targets) & ws.closure([prev_pkg] if prev_pkg else ws.default))
        pool = [c for c in shared if c in ws.pkgs and live_any(c)]
        if pool:
            op = place("body", weighted(rng, pool, edited))
            if op:
                edits.append(op)
    return Plan(pt, ws.repo, ws.snap, str(ws.cwd.relative_to(ws.root)), cmd, pre_command(cmd), prev, edits,
                expect_fail=any(e.kind == "type-error" for e in edits), note=note)


# --- applying edits --------------------------------------------------------


def _pristine(root: Path, rel: str) -> Path:
    return suite.pristine(root, rel)


def apply(p: Plan, n: int) -> list[Path]:
    """Make the plan's edits with unique number n. Returns files created."""
    root = snapshot_root(p.snap)
    created: list[Path] = []
    by_file: dict[str, list[EditOp]] = collections.defaultdict(list)
    for e in p.edits:
        by_file[e.file].append(e)
    for rel, ops in by_file.items():
        text = _pristine(root, rel).read_text()
        lines = text.splitlines(keepends=True)
        tail = ""
        for k, e in enumerate(sorted(ops, key=lambda e: -e.line)):
            tag = f"{n}{k}"
            if e.kind in ("body", "test"):
                indent = re.match(r"\s*", lines[e.line + 1] if e.line + 1 < len(lines) else "").group(0) or "    "
                lines.insert(e.line + 1, f"{indent}::core::hint::black_box({tag}u64);\n")
            elif e.kind == "type-error":
                lines.insert(e.line + 1, f'    let _: u32 = "jr synth {tag}";\n')
            elif e.kind == "item":
                tail += f"\n#[allow(dead_code)]\npub fn jr_synth_item_{tag}() -> u64 {{\n    {tag}\n}}\n"
            elif e.kind == "new-file":
                mod = f"jr_synth_mod_{tag}"
                tail += f"\n#[allow(dead_code)]\nmod {mod};\n"
                nf = (root / rel).parent / f"{mod}.rs"
                nf.write_text(f"pub fn value() -> u64 {{\n    {tag}\n}}\n")
                created.append(nf)
            elif e.kind == "manifest":
                body = "".join(lines)
                feat = f"jr-synth-{tag} = []\n"
                if re.search(r"(?m)^\[features\]\s*$", body):
                    body = re.sub(r"(?m)^\[features\]\s*$\n?", lambda m: m.group(0) + feat, body, count=1)
                else:
                    body = body.rstrip("\n") + "\n\n[features]\n" + feat
                lines = body.splitlines(keepends=True)
        out = "".join(lines)
        if tail:
            out = out.rstrip("\n") + "\n" + tail
        (root / rel).write_text(out)
    return created


def restore(p: Plan, created: list[Path]) -> None:
    root = snapshot_root(p.snap)
    for e in p.edits:
        shutil.copy2(_pristine(root, e.file), root / e.file)
    for f in created:
        f.unlink(missing_ok=True)


def restore_all(snap: str) -> None:
    """Undo every edit in a snapshot (after an interrupted run)."""
    root = snapshot_root(snap)
    pr = root / ".pristine"
    if pr.exists():
        for f in pr.rglob("*"):
            if f.is_file():
                shutil.copy2(f, root / f.relative_to(pr))
    for f in root.rglob("jr_synth_mod_*.rs"):
        f.unlink()


def fingerprint() -> str:
    """Identifies the generator + weights: results are comparable only within one."""
    h = hashlib.sha256()
    for f in (Path(__file__), WEIGHTS):
        h.update(f.read_bytes())
    return h.hexdigest()[:12]


# --- CLI -------------------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("show")
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("-n", type=int, default=40)
    s.add_argument("--apply-check", action="store_true", help="apply and restore each edit (syntax smoke test)")
    pr = sub.add_parser("probe")
    pr.add_argument("crates", nargs="*")
    pr.add_argument("--repo", default="jcode-desktop")
    args = ap.parse_args()
    if args.cmd == "probe":
        ws = workspace(args.repo)
        for c in args.crates or ws.members:
            probe(home_workspace(c) or ws, c, force=True)
        return
    pts, info = sample(args.seed, args.n)
    print(f"seed {args.seed}: {args.n} points covering {100 * info['coverage']:.1f}% of {info['calls']} mined calls")
    for pt in pts:
        p = plan(pt, args.seed)
        sh = pt.shape
        print(f"{pt.i:3d} {sh['command']:14s} {sh['change']:10s} {sh['spread']:11s} {sh['relation']:8s} "
              f"[{p.repo}] {' '.join(p.cmd)}")
        if p.prev:
            print(f"      after: {' '.join(p.prev)}")
        for e in p.edits:
            print(f"      {e.label()}")
        if p.note:
            print(f"      note: {p.note}")
        if args.apply_check:
            created = apply(p, 1)
            restore(p, created)


if __name__ == "__main__":
    main()
