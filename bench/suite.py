#!/usr/bin/env python3
"""justrust benchmark suite: the agent edit -> check/test loop, reproducibly.

Every scenario runs on a pinned source snapshot (a `git archive` of a fixed
commit, extracted once under ~/.justrust/bench/work), with its own target dir,
so results do not depend on what is checked out in ~/jcode-desktop today or on
another agent's build state. Each iteration applies a unique one-line edit
(never reverting to an already-compiled state), runs the command through
justrust, and reads the recorded summary.json for the phase breakdown.

  bench/suite.py list                      scenarios and what they measure
  bench/suite.py run [-n 5] [scenario...]  run (default: every non-cold scenario)
  bench/suite.py compare A.json B.json     per-scenario median delta
  bench/suite.py show RESULT.json          print a saved result

Results go to bench/results/<timestamp>-<justrust rev>.json (commit them).
A sample is flagged `noisy` when another rustc was running at its start or
other processes averaged more than BENCH_NOISY_CORES (default 4) cores during
the build. Medians are over all samples, `quiet` medians over the rest. Use --quiet-wait to wait for a quiet machine before each sample.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import random
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field, replace
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOME = Path.home()
JR_HOME = Path(os.environ.get("JUSTRUST_HOME", HOME / ".justrust"))
WORK = Path(os.environ.get("JUSTRUST_BENCH_WORK", JR_HOME / "bench" / "work"))
RESULTS = ROOT / "bench" / "results"


def resolve_justrust_binary(env=None) -> str:
    """Explicit overrides must never silently benchmark a different executable."""
    env = os.environ if env is None else env
    selected = []
    for key in ("JUSTRUST", "JUSTRUST_BIN"):
        if key in env:
            path = shutil.which(env[key]) if env[key] else None
            if path is None:
                raise ValueError(f"bench: {key} must name an executable: {env[key]!r}")
            selected.append((key, str(Path(path).resolve())))
    if len(selected) == 2 and selected[0][1] != selected[1][1]:
        raise ValueError("bench: JUSTRUST and JUSTRUST_BIN select different executables")
    return selected[0][1] if selected else shutil.which("justrust") or "justrust"


def binary_sha256(binary=None) -> str:
    """Hash outside timed samples, and reject a replacement while reading."""
    binary = J if binary is None else binary
    path = Path(shutil.which(str(binary)) or binary)
    try:
        with path.open("rb") as source:
            before = os.fstat(source.fileno())
            digest = hashlib.file_digest(source, "sha256").hexdigest()
            after = os.fstat(source.fileno())
        current = path.stat()
    except OSError as err:
        raise ValueError(f"bench: cannot identify executable {path.name}: {err.strerror}") from err
    signature = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
    if signature(before) != signature(after) or signature(after) != signature(current):
        raise ValueError("bench: justrust executable changed while recording its identity")
    return digest


def verify_binary_identity(info: dict) -> None:
    if binary_sha256() != info["justrust_binary_sha256"]:
        raise ValueError("bench: justrust executable changed during the benchmark; refusing mixed results")


# Absolute, so a child env (versus stacks, os.execvpe) never resolves it
# differently from this process.
try:
    J = resolve_justrust_binary()
except ValueError as err:
    sys.exit(str(err))
# `run --toolchain SPEC`: build with a justrust-pinned toolchain
# (JUSTRUST_TOOLCHAIN) in its own target dir. "system" = the system rustc.
TOOLCHAIN = "system"
RUSTC_THREADS = None
WORKSPACE_THREADS = None
PASS_TIMES = False
# A sample is noisy when other processes averaged more than this many cores
# or another rustc was running when it started.
NOISY_CORES = float(os.environ.get("BENCH_NOISY_CORES", "4"))

# Pinned sources. Bump deliberately (and say so in FINDINGS.md): results from
# different pins are not comparable, and `compare` refuses to mix them.
SNAPSHOTS = {
    "desktop": {
        # Jcode Desktop needs the Jcode repo as a sibling (path dependencies
        # `../../../jcode/crates/...`).
        "repos": {
            "jcode-desktop": (HOME / "jcode-desktop", "7d38fb7"),
            "jcode": (HOME / "jcode", "abff94eeb"),
        },
        "cwd": "jcode-desktop",
        # 7d38fb7's lib tests miss a field its harness already uses (fixed
        # later upstream). Exact replacements, applied once at extraction.
        "fixups": [
            (
                "jcode-desktop/crates/jcode-desktop-ui/src/panel.rs",
                'ApiEvent::SessionStatus { session_id: "session-a".into(), status: "idle".into() }',
                'ApiEvent::SessionStatus { session_id: "session-a".into(), status: "idle".into(), pending_soft_interrupts: None }',
                2,
            ),
        ],
    },
    "justrust": {
        "repos": {"justrust": (ROOT, "f97fcc2")},
        "cwd": "justrust",
    },
}


@dataclass
class Edit:
    file: str  # relative to the snapshot root
    anchor: str  # must occur exactly once
    # Replacement for the anchor; `{n}` is a unique per-iteration number.
    template: str


@dataclass
class Scenario:
    name: str
    snapshot: str
    cmd: list[str]
    edit: Edit | None
    what: str
    expect_fail: bool = False
    expect_test_fail: bool = False  # a test must fail (the timing is to its report)
    cold: bool = False  # fresh target dir every iteration; opt-in
    env: dict[str, str] = field(default_factory=dict)
    # Cold, with an empty local depcache every iteration and a prebuilt
    # store (JUSTRUST_PREBUILT) published once from another cargo home.
    prebuilt: bool = False


FPS = "jcode-desktop/crates/jcode-desktop-ui/src/fps_counter.rs"
DESKTOP_TEST = ["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"]
DESKTOP_CHECK = ["check", "-p", "jcode-desktop-ui"]

SCENARIOS = [
    Scenario(
        "desktop-test-edit",
        "desktop",
        DESKTOP_TEST,
        Edit(
            FPS,
            "let mut counter = FpsCounter::default();\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
            "let mut counter = FpsCounter::default();\n        std::hint::black_box({n}u64);\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        ),
        "THE reference loop: one line inside a #[test] fn of the 118k-line UI crate, run its tests",
    ),
    Scenario(
        "desktop-body-test",
        "desktop",
        DESKTOP_TEST,
        Edit(
            FPS,
            "            let current = snapshot();\n",
            "            let current = snapshot();\n            std::hint::black_box({n}u64);\n",
        ),
        "one-line body edit in non-test code of the UI crate, run its tests",
    ),
    Scenario(
        "desktop-body-check",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            FPS,
            "            let current = snapshot();\n",
            "            let current = snapshot();\n            std::hint::black_box({n}u64);\n",
        ),
        "same body edit, `check` only (the inner type-check loop)",
    ),
    Scenario(
        "desktop-type-error",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            FPS,
            "    ) -> String {\n        if self",
            '    ) -> String {\n        let _: u32 = "type error {n}";\n        if self',
        ),
        "time to the first type error after an edit (the check must fail)",
        expect_fail=True,
    ),
    Scenario(
        "desktop-type-error-early",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            FPS,
            "    ) -> String {\n        if self",
            '    ) -> String {\n        let _: u32 = "type error {n}";\n        if self',
        ),
        "desktop-type-error with early diagnostics (JUSTRUST_EARLY=1, warm rust-analyzer daemon started untimed)",
        expect_fail=True,
        env={"JUSTRUST_EARLY": "1"},
    ),
    Scenario(
        "desktop-noop-test",
        "desktop",
        DESKTOP_TEST,
        None,
        "no edit: pure overhead of a fresh `test` (cargo startup, fingerprints, test run)",
    ),
    Scenario(
        "desktop-upstream-body",
        "desktop",
        DESKTOP_TEST,
        Edit(
            "jcode-desktop/crates/jcode-desktop-ui-core/src/render_stats.rs",
            "        stat.renders += 1;\n",
            "        stat.renders += 1;\n        std::hint::black_box({n}u64);\n",
        ),
        "body edit in jcode-desktop-ui-core, a dependency of the UI crate, run UI tests",
    ),
    Scenario(
        "desktop-upstream-sig",
        "desktop",
        DESKTOP_CHECK,
        Edit(
            "jcode-desktop/crates/jcode-desktop-ui-core/src/render_stats.rs",
            "/// Times one render of `name` until the returned guard drops.\n",
            "pub fn bench_probe_{n}() {}\n\n/// Times one render of `name` until the returned guard drops.\n",
        ),
        "new pub item in jcode-desktop-ui-core: metadata changes, downstream re-checks",
    ),
    Scenario(
        "desktop-jcode-core-body",
        "desktop",
        DESKTOP_TEST,
        Edit(
            "jcode/crates/jcode-core/src/id.rs",
            "pub fn session_icon(name: &str) -> &'static str {\n",
            "pub fn session_icon(name: &str) -> &'static str {\n    std::hint::black_box({n}u64);\n",
        ),
        "body edit in jcode-core (sibling Jcode repo, deep in the graph), run UI tests",
    ),
    Scenario(
        "justrust-body-test",
        "justrust",
        ["test"],
        Edit(
            "justrust/src/runs.rs",
            "    let d = (paths::now() - t).max(0.0);\n",
            "    let d = (paths::now() - t).max(0.0);\n    std::hint::black_box({n}u64);\n",
        ),
        "small crate: body edit in this repo, full `test` (the old bench/edit-loop.sh)",
    ),
    Scenario(
        "desktop-workspace-test",
        "desktop",
        ["test", "--workspace", "--no-fail-fast"],
        None,
        "no edit, every test binary of the workspace (11 binaries + doc tests): test execution only",
    ),
    Scenario(
        "desktop-ui-full-test",
        "desktop",
        ["test", "-p", "jcode-desktop-ui", "--lib"],
        Edit(
            FPS,
            "let mut counter = FpsCounter::default();\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
            "let mut counter = FpsCounter::default();\n        std::hint::black_box({n}u64);\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        ),
        "test-fn edit, the whole UI lib suite (1290 tests, no filter): when do the edited module's tests report",
    ),
    Scenario(
        "desktop-ui-full-test-fail",
        "desktop",
        ["test", "-p", "jcode-desktop-ui", "--lib"],
        Edit(
            FPS,
            "let mut counter = FpsCounter::default();\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
            "let mut counter = FpsCounter::default();\n        assert!(std::hint::black_box({n}u64) == 0, \"bench failure\");\n        let now = Instant::now();\n        assert_eq!(counter.label(now, || snapshot(&[])),",
        ),
        "the edit breaks an fps_counter test, whole UI lib suite: `first_fail` = when the agent sees the panic",
        expect_test_fail=True,
    ),
    Scenario(
        "desktop-cold-check",
        "desktop",
        DESKTOP_CHECK,
        None,
        "fresh target dir, shared depcache on: what a new agent checkout pays",
        cold=True,
    ),
    Scenario(
        "desktop-cold-check-nocache",
        "desktop",
        DESKTOP_CHECK,
        None,
        "fresh target dir, every justrust cache off: the from-scratch floor",
        cold=True,
        env={"JUSTRUST_DEPCACHE": "0", "JUSTRUST_BUILD_SCRIPT_CACHE": "0"},
    ),
    Scenario(
        "desktop-cold-check-prebuilt",
        "desktop",
        DESKTOP_CHECK,
        None,
        "fresh target dir and empty local depcache, dependencies from a prebuilt store "
        "(published once from another cargo home, served over local HTTP): a new machine",
        cold=True,
        prebuilt=True,
    ),
]
BY_NAME = {s.name: s for s in SCENARIOS}


def sh(cmd: list[str], cwd: Path | None = None) -> str:
    return subprocess.run(
        cmd, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def snapshot_dir(name: str) -> Path:
    spec = SNAPSHOTS[name]
    key = "-".join(f"{r}@{rev}" for r, (_, rev) in sorted(spec["repos"].items()))
    return WORK / name / key


_BENCH_LOCK = None


def lock_holders(path: Path) -> str:
    """Processes with `path` open (from /proc), e.g. "1234 python3, 99 wl-copy".

    A shell `flock` on the lock file leaks its fd into every child, so a
    daemonized grandchild (a clipboard helper, a server) can keep holding the
    lock after the benchmark exits. Naming the holders makes that visible.
    """
    target = str(path.resolve())
    out = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            fds = os.listdir(f"/proc/{pid}/fd")
            if any(os.readlink(f"/proc/{pid}/fd/{fd}") == target for fd in fds):
                out.append(f"{pid} {Path(f'/proc/{pid}/comm').read_text().strip()}")
        except OSError:
            continue
    return ", ".join(out) or "unknown"


def bench_lock() -> None:
    """Hold an exclusive lock on the pinned snapshots for this process's life.

    suite.py, factors.py and versus.py edit the same extracted sources and
    restore them per run, so two concurrent runs corrupt each other (one
    sees the other's injected type error) and skew each other's wall times.
    Taken in prepare(), which every runner calls before touching a snapshot;
    blocks until the other run exits. BENCH_NO_LOCK=1 skips it. A parent
    process that already holds the lock (e.g. a shell `flock` on the same
    file) must export BENCH_LOCK_HELD=1, or this would wait on it forever.
    """
    global _BENCH_LOCK
    if _BENCH_LOCK is not None or "1" in (os.environ.get("BENCH_NO_LOCK"), os.environ.get("BENCH_LOCK_HELD")):
        return
    import fcntl

    WORK.mkdir(parents=True, exist_ok=True)
    f = open(WORK / ".bench.lock", "w")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("bench: another benchmark holds the snapshot lock, waiting; holders: "
              + lock_holders(WORK / ".bench.lock"), file=sys.stderr)
        fcntl.flock(f, fcntl.LOCK_EX)
    _BENCH_LOCK = f
    # Children (other bench scripts started from this one) share the lock.
    os.environ["BENCH_LOCK_HELD"] = "1"


def prepare(name: str) -> Path:
    """Extract the pinned snapshot once. Source files are restored per run."""
    bench_lock()
    root = snapshot_dir(name)
    # bench/on-remote.sh: pinned trees come as tarballs, <dir>/<repo>@<rev>.tar.
    src_dir = os.environ.get("JUSTRUST_BENCH_SRC")
    for repo, (src, rev) in SNAPSHOTS[name]["repos"].items():
        dst = root / repo
        stamp = dst / ".bench-snapshot"
        if stamp.exists():
            continue
        print(f"bench: extracting {src}@{rev} -> {dst}", file=sys.stderr)
        if dst.exists():
            shutil.rmtree(dst)
        dst.mkdir(parents=True)
        if src_dir:
            tar = Path(src_dir) / f"{repo}@{rev}.tar"
            subprocess.run(["tar", "-x", "-C", str(dst), "-f", str(tar)], check=True)
        else:
            archive = subprocess.Popen(
                ["git", "-C", str(src), "archive", rev], stdout=subprocess.PIPE
            )
            subprocess.run(["tar", "-x", "-C", str(dst)], stdin=archive.stdout, check=True)
            if archive.wait() != 0:
                sys.exit(f"bench: git archive {src}@{rev} failed")
        for rel, old, new, count in SNAPSHOTS[name].get("fixups", []):
            if not rel.startswith(repo + "/"):
                continue
            p = root / rel
            s = p.read_text()
            if s.count(old) != count:
                sys.exit(f"bench: fixup for {rel} matches {s.count(old)} times, expected {count}")
            p.write_text(s.replace(old, new))
        stamp.write_text(rev + "\n")
    return root


def pristine(root: Path, rel: str) -> Path:
    """Saved copy of a file as extracted, used to restore it."""
    p = root / ".pristine" / rel
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / rel, p)
    return p


def apply_edit(root: Path, edit: Edit, n: int) -> None:
    orig = pristine(root, edit.file).read_text()
    if orig.count(edit.anchor) != 1:
        sys.exit(f"bench: anchor occurs {orig.count(edit.anchor)} times in {edit.file}")
    new = edit.template.replace("{n}", str(n))
    (root / edit.file).write_text(orig.replace(edit.anchor, new, 1))


def restore(root: Path, edit: Edit | None) -> None:
    if edit:
        shutil.copy2(pristine(root, edit.file), root / edit.file)


def foreign_load() -> tuple[float, int]:
    load = float(Path("/proc/loadavg").read_text().split()[0])
    try:
        rustc = int(sh(["pgrep", "-c", "-x", "rustc"]))
    except subprocess.CalledProcessError:
        rustc = 0
    return load, rustc


def wait_quiet(max_secs: float) -> None:
    deadline = time.time() + max_secs
    while time.time() < deadline:
        load, rustc = foreign_load()
        if rustc == 0 and load < 2.0:
            return
        time.sleep(5)


def new_id() -> str:
    t = time.time()
    stamp = time.strftime("%Y%m%d-%H%M%S", time.localtime(t)) + f"{int(t * 1000) % 1000:03d}"
    return f"{stamp}-{os.getpid()}{random.randrange(100, 999)}"


def reuse_ledger(sm: dict, run_dir: Path) -> dict:
    """Work this run did not redo, and what it would have cost.

    fresh: units cargo found up to date. depcache: rustc units restored from
    the shared cache (saved = the original compile's recorded wall time).
    scripts: build-script runs replayed. compiled: rustc units that ran.
    """
    u = sm["units"]
    hits = u.get("depcache_hits", 0)
    s_hits, s_saved = 0, 0.0
    sp = run_dir / "scripts.jsonl"
    if sp.exists():
        for line in sp.read_text().splitlines():
            try:
                x = json.loads(line)
            except ValueError:
                continue
            if x.get("hit"):
                s_hits += 1
                s_saved += max(0.0, x.get("saved_secs", 0.0) - x.get("wall", 0.0))
    slot = sm.get("slot") or {}
    return {
        "compiled": u["compiled"] - hits,
        "fresh": u.get("fresh", 0),
        "depcache_hits": hits,
        "depcache_misses": u.get("depcache_misses", 0),
        "depcache_saved_secs": round(max(0.0, u.get("depcache_saved_secs", 0.0)), 2),
        "script_hits": s_hits,
        "script_saved_secs": round(s_saved, 2),
        "slot_refreshed_units": slot.get("refreshed_units", 0) or 0,
        "local_hits": u.get("localcache_hits", 0),
        "local_saved_secs": round(u.get("localcache_saved_secs", 0.0), 2),
    }


# Set by `prebuilt_store()` for prebuilt scenarios.
PREBUILT_URL = ""
REMOTE = False


def remote_measurement(summary: dict) -> dict:
    """A remote sample must prove it ran remotely, never a silent fallback."""
    remote = summary.get("remote")
    if not isinstance(remote, dict) or not remote.get("machine"):
        raise ValueError("forced remote benchmark fell back locally")
    actual_toolchain = summary.get("toolchain") or "system"
    if actual_toolchain != TOOLCHAIN:
        raise ValueError(f"forced remote benchmark used {actual_toolchain}, expected {TOOLCHAIN}")
    return {**remote, "host_wall": summary["wall"], "toolchain": actual_toolchain,
            "target_state": "persistent remote target"}


def prebuilt_store(s: Scenario, root: Path) -> None:
    """Publish the snapshot's dependencies to a store once (untimed), from a
    different cargo home path and target dir than the measured runs, and
    serve it over local HTTP. `BENCH_PREBUILT_URL` uses an existing store."""
    global PREBUILT_URL
    if os.environ.get("BENCH_PREBUILT_URL"):
        PREBUILT_URL = os.environ["BENCH_PREBUILT_URL"]
        return
    tag = f"{root.name}-{TOOLCHAIN}-{'-'.join(s.cmd)}"
    pub = WORK / "prebuilt" / tag
    store = pub / "store"
    if not (store / "v1" / "index").exists():
        home = pub / "publisher-home" / "cargo"
        home.mkdir(parents=True, exist_ok=True)
        cargo = Path(os.environ.get("CARGO_HOME", HOME / ".cargo"))
        for sub in ("registry", "git"):
            link = home / sub
            if (cargo / sub).exists() and not link.exists():
                link.symlink_to(cargo / sub)
        env = dict(os.environ)
        env.update({
            "CARGO_HOME": str(home),
            "CARGO_TARGET_DIR": str(pub / "target"),
            "JUSTRUST_DEPCACHE_DIR": str(pub / "depcache"),
            "JUSTRUST_REMOTE": "0",
            "JUSTRUST_QUIET": "1",
            "JUSTRUST_TOOLCHAIN": TOOLCHAIN,
        })
        env.pop("JUSTRUST_PREBUILT", None)
        print(f"bench: publishing {s.name} dependencies to {store}", file=sys.stderr)
        cwd = root / SNAPSHOTS[s.snapshot]["cwd"]
        subprocess.run([J, *s.cmd], cwd=cwd, env=env, check=True, capture_output=True)
        out = subprocess.run([J, "cache", "--publish", str(store)], env=env, check=True,
                             capture_output=True, text=True).stdout.strip()
        print(f"bench: {out}", file=sys.stderr)
        shutil.rmtree(pub / "target", ignore_errors=True)
    import http.server
    import functools

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Quiet, directory=str(store)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    PREBUILT_URL = f"http://127.0.0.1:{srv.server_address[1]}"


def hotpatch_outcomes(run_dir: Path) -> list[str] | None:
    """Outcome of each hot-patchable link in a run (None: hot patching off)."""
    p = run_dir / "hotpatch.jsonl"
    if not p.exists():
        return None
    out = []
    for line in p.read_text().splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        out.append(f"patched {d['ms']:.0f}ms" if d["outcome"] == "patched" else f"{d['outcome']}: {d['detail'][:80]}")
    return out


def admission_transport(stderr: str, requested: bool) -> str:
    if not requested:
        return "curl"
    if "justrust trace: pooled hosted admission" not in stderr:
        raise ValueError("requested pooled admission was not used, not a pooled measurement")
    return "pooled"


def remote_trace_timings(stderr: str) -> dict:
    """Keep fixed numeric client diagnostics, never raw headers or credentials."""
    steps = ("roots", "charged", "flushed", "first output", "exit", "recording")
    elapsed, duplicates = {}, set()
    admission = []
    number = r"([0-9]+(?:\.[0-9]+)?)"
    phase_pattern = re.compile(
        r"justrust trace: admission identity " + number + r" ms \| charge "
        + number + r" ms \| identity_wait " + number + r" ms \| queue "
        + number + r" ms \| total " + number
        + r" ms \(awaited paths, identity overlaps\)"
    )
    for line in stderr.splitlines():
        if len(line) > 512 or not line.isascii():
            continue
        step = re.fullmatch(r"justrust trace:\s+" + number + r" ms (" + "|".join(steps) + r")", line)
        if step:
            value, name = float(step[1]), step[2]
            if name in elapsed:
                duplicates.add(name)
            if value <= 86_400_000:
                elapsed[name] = value
        phase = phase_pattern.fullmatch(line)
        if phase:
            values = [float(value) for value in phase.groups()]
            if all(value <= 60_000 for value in values):
                admission.append(dict(zip(("identity", "charge", "identity_wait", "queue", "total"), values)))
    elapsed = {name: elapsed[name] for name in steps if name in elapsed and name not in duplicates}
    values = list(elapsed.values())
    if values != sorted(values):
        elapsed = {}
    return {
        **({"client_elapsed_ms": {name.replace(" ", "_"): value for name, value in elapsed.items()}} if elapsed else {}),
        **({"admission_awaited_ms": admission[0]} if len(admission) == 1 else {}),
    }


def hosted_early_provenance(summary: dict) -> dict:
    early = summary.get("early") or {}
    if (not early.get("ready") or not early.get("shown") or early.get("exited")
            or not early.get("matched") or early.get("early_only", 0)):
        raise ValueError("requested hosted provisional diagnostics were not shown and rustc-confirmed")
    return early


def run_once(s: Scenario, root: Path, target: Path) -> dict:
    rid = new_id()
    env = dict(os.environ)
    env.update(
        {
            "CARGO_TARGET_DIR": str(target),
            "JUSTRUST_RUN_ID_OVERRIDE": rid,
            "JUSTRUST_REMOTE": "0",
            "JUSTRUST_QUIET": "1",
            # Per-pass split for every crate. Safe here: the bench owns its
            # target dirs, so the mode never flips under a warm cache. Not for
            # cold runs: units compiled with -Ztime-passes are never stored
            # in the depcache, so forcing it would keep the cache empty.
            **({} if s.cold else {"JUSTRUST_PASSES": "always"}),
        }
    )
    env.pop("JUSTRUST_DISABLE", None)
    env["JUSTRUST_TOOLCHAIN"] = TOOLCHAIN
    # The public store is on by default for users. Benchmarks compare
    # against their own caches, so it is off unless the scenario asks.
    env["JUSTRUST_PREBUILT"] = "off"
    # The build daemon (default on) would prebuild restored files between
    # samples; scenarios that measure it turn it on.
    env.setdefault("JUSTRUST_BUILDD", "0")
    env.update(s.env)
    if REMOTE:
        env["JUSTRUST_REMOTE"] = "force"
        if env.get("JUSTRUST_HOSTED_HTTP_POOL") in ("1", "on", "true"):
            env["JUSTRUST_REMOTE_TRACE"] = "1"
    if s.prebuilt:
        # A new machine: nothing in the local depcache, the store has it.
        cache = target.with_name(target.name + "-depcache")
        shutil.rmtree(cache, ignore_errors=True)
        env["JUSTRUST_DEPCACHE_DIR"] = str(cache)
        env["JUSTRUST_PREBUILT"] = PREBUILT_URL
    cwd = root / SNAPSHOTS[s.snapshot]["cwd"]
    load, rustc = foreign_load()
    t0 = time.monotonic()
    # Stream stderr to timestamp the first error line the agent sees (early
    # rust-analyzer errors or rustc's), not only the exit.
    p = subprocess.Popen([J, *s.cmd], cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    first_error = None
    err_lines = []
    first_fail = []

    def read_out():
        # Seconds until the agent sees a failing test's panic message.
        for line in p.stdout:
            if not first_fail and ("panicked at" in line or line.startswith("---- ")):
                first_fail.append(time.monotonic() - t0)

    out_reader = threading.Thread(target=read_out, daemon=True)
    out_reader.start()
    for line in p.stderr:
        if first_error is None and line.startswith("error"):
            first_error = time.monotonic() - t0
        err_lines.append(line)
    out_reader.join()
    p.wait()
    wall = time.monotonic() - t0
    p.stderr = "".join(err_lines)
    summ_path = JR_HOME / "runs" / rid / "summary.json"
    if env.get("JUSTRUST_EARLY_EXIT") == "1":
        # An early exit returns before cargo finishes; the run's summary is
        # written by the build the daemon keeps running. Wait for it
        # (untimed) so samples never overlap.
        deadline = time.monotonic() + 300
        while not summ_path.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
    if not summ_path.exists():
        sys.exit(f"bench: {s.name}: no summary for run {rid} (exit {p.returncode})\n{p.stderr[-3000:]}")
    sm = json.loads(summ_path.read_text())
    remote = None
    admission = None
    trace = {}
    if REMOTE:
        try:
            remote = remote_measurement(sm)
            admission = admission_transport(p.stderr, env.get("JUSTRUST_HOSTED_HTTP_POOL") in ("1", "on", "true"))
            trace = remote_trace_timings(p.stderr)
            if env.get("JUSTRUST_EARLY") == "query":
                hosted_early_provenance(sm)
        except ValueError as e:
            sys.exit(f"bench: {s.name}: {e} (justrust log {rid})\n{p.stderr[-3000:]}")
    failed = p.returncode != 0
    compile_failed = sm.get("diagnostics", {}).get("compile_failed", False)
    tests_failed = sm.get("tests", {}).get("failed", 0)
    if s.expect_test_fail and not tests_failed:
        sys.exit(f"bench: {s.name}: expected a failing test (justrust log {rid})\n{p.stderr[-3000:]}")
    if s.expect_fail != compile_failed or (failed and not compile_failed and not tests_failed):
        sys.exit(
            f"bench: {s.name}: exit {p.returncode}, expected {'failure' if s.expect_fail else 'success'}"
            f" (justrust log {rid})\n{p.stderr[-3000:]}"
        )
    if tests_failed and not s.expect_test_fail:
        # The timing is still valid; a flaky test in a pinned snapshot must
        # not abort an hour of benchmarking. Recorded per sample.
        print(f"  note {s.name}: {tests_failed} test(s) failed (justrust log {rid})", file=sys.stderr)
    try:
        workspace_threads = workspace_thread_provenance(sm, WORKSPACE_THREADS)
    except ValueError as e:
        sys.exit(f"bench: {s.name}: {e} (justrust log {rid})")
    local = [u for u in sm.get("top_units", []) if u.get("local")]
    split = {"frontend": 0.0, "codegen": 0.0, "link": 0.0, "incremental": 0.0, "other": 0.0}
    for u in local:
        for k, v in (u.get("split") or {}).items():
            split[k] = split.get(k, 0.0) + v
    res = sm.get("resources", {})
    early = sm.get("early") or {}
    return {
        "id": rid,
        "wall": round(wall, 3),
        # Seconds until the first `error` line on stderr (None: no error).
        "first_error": None if first_error is None else round(first_error, 3),
        "first_fail": round(first_fail[0], 3) if first_fail else None,
        "early": {k: early.get(k) for k in ("secs", "ready", "shown", "matched", "early_only", "rustc_only", "exited")}
        if early else None,
        "recorded_wall": round(sm["wall"], 3),
        **({"remote": remote} if remote is not None else {}),
        **({"admission_transport": admission} if admission is not None else {}),
        **({"remote_trace": trace} if trace else {}),
        "cpu": round(sm.get("cpu_secs", 0.0), 2),
        "startup": round(sm["phases"]["startup"], 3),
        "compile": round(sm["phases"]["compile"], 3),
        "test_run": round(sm["phases"]["test_run"], 3),
        "units": sm["units"]["compiled"],
        "local_units": sm["units"]["local"],
        "dep_units": sm["units"]["dependencies"],
        "reuse": reuse_ledger(sm, JR_HOME / "runs" / rid),
        "split": {k: round(v, 3) for k, v in split.items()},
        "link_secs": round(sm.get("link", {}).get("link_pass_secs", 0.0), 3),
        # JUSTRUST_HOTPATCH=1: "patched 41ms", "full-link: <why>" per link.
        "hotpatch": hotpatch_outcomes(JR_HOME / "runs" / rid),
        "tests_passed": sm.get("tests", {}).get("passed", 0),
        "tests_failed": tests_failed,
        "other_cores": round(res.get("avg_other_cores", 0.0), 2),
        "foreign_rustc_before": rustc,
        "load_before": load,
        "noisy": res.get("avg_other_cores", 0.0) > NOISY_CORES or rustc > 0,
        "top_unit": local[0]["name"] if local else None,
        "workspace_frontend_threads": workspace_threads,
    }


def med(xs: list[float]) -> float:
    return round(statistics.median(xs), 3) if xs else 0.0


def stats(samples: list[dict]) -> dict:
    walls = [x["wall"] for x in samples]
    quiet = [x["wall"] for x in samples if not x["noisy"]]
    fe = [x["first_error"] for x in samples if x.get("first_error") is not None]
    ff = [x["first_fail"] for x in samples if x.get("first_fail") is not None]
    return {
        "first_fail": med(ff) if ff else None,
        "first_fail_min": round(min(ff), 3) if ff else None,
        "first_fail_max": round(max(ff), 3) if ff else None,
        "n": len(walls),
        "first_error": med(fe) if fe else None,
        "first_error_min": round(min(fe), 3) if fe else None,
        "first_error_max": round(max(fe), 3) if fe else None,
        "median": med(walls),
        "min": round(min(walls), 3),
        "max": round(max(walls), 3),
        "quiet_n": len(quiet),
        "quiet_median": med(quiet),
        "phases": {
            k: med([x[k] for x in samples]) for k in ("startup", "compile", "test_run", "cpu", "link_secs")
        },
        "split": {k: med([x["split"][k] for x in samples]) for k in samples[0]["split"]},
        "units": med([x["units"] for x in samples]),
    }


def run_scenario(s: Scenario, n: int, warmup: int, quiet_wait: float) -> dict:
    root = prepare(s.snapshot)
    if s.prebuilt:
        prebuilt_store(s, root)
    # Each toolchain keeps its own warm target dir: switching rustc in one
    # dir would rebuild everything on every switch.
    tdir = "target" if TOOLCHAIN == "system" else f"target-{TOOLCHAIN}"
    target = root / tdir
    samples = []
    # Unique edit numbers per invocation, so a re-run never lands on a state
    # the incremental cache has already seen.
    base = int(time.time()) % 100000 * 100
    # Cold runs still get a warmup: it fills the shared depcache, so measured
    # runs see the state a second fresh checkout sees.
    warm = warmup if (not s.cold or s.env.get("JUSTRUST_DEPCACHE") != "0") else 0
    if s.env.get("JUSTRUST_EARLY") == "1":
        # The resident daemon is the point: start it and wait until it has
        # loaded the workspace, outside the timing.
        restore(root, s.edit)
        subprocess.run([J, "early", "start"], cwd=root / SNAPSHOTS[s.snapshot]["cwd"], check=True)
    total = warm + n
    try:
        for i in range(total):
            if s.cold:
                target = root / f"{tdir}-cold-{os.getpid()}-{i}"
                restore(root, s.edit)
            elif s.edit:
                apply_edit(root, s.edit, base + i)
            if quiet_wait:
                wait_quiet(quiet_wait)
            r = run_once(s, root, target)
            measured = i >= warm
            tag = "" if measured else " (warmup)"
            flag = " noisy" if r["noisy"] else ""
            print(
                f"  {s.name} {i + 1}/{total}: {r['wall']:.2f}s compile {r['compile']:.2f}s "
                f"units {r['units']} id {r['id']}{flag}{tag}",
                file=sys.stderr,
            )
            if measured:
                samples.append(r)
            if s.cold:
                shutil.rmtree(target, ignore_errors=True)
                shutil.rmtree(target.with_name(target.name + "-depcache"), ignore_errors=True)
    finally:
        restore(root, s.edit)
    if s.edit is None and not s.cold and any(x["units"] for x in samples):
        print(f"  WARNING {s.name}: a no-op run compiled units (cache invalidation?)", file=sys.stderr)
    return {"what": s.what, "cmd": s.cmd, "samples": samples, "stats": stats(samples)}


def toolchain_rustc(spec: str) -> Path:
    """Explicit staged compilers are not in the automatic toolchain namespace."""
    installed = JR_HOME / "toolchains" / spec / "bin" / "rustc"
    staged = JR_HOME / "toolchains" / "staged" / spec / "bin" / "rustc"
    return staged if not installed.is_file() and staged.is_file() else installed


def workspace_thread_provenance(summary: dict, expected: int | None) -> int | None:
    """Verify the requested wrapper setting on actual compiled UI units.

    No-op and interface-cutoff runs may have no UI compiler invocation, so
    they carry no observed thread count instead of inventing one.
    """
    units = [u for u in summary.get("top_units", []) if u.get("local")
             and (u["name"] == "jcode_desktop_ui"
                  or u["name"].startswith("jcode_desktop_ui ("))]
    counts = {u.get("frontend_threads") for u in units}
    if expected is not None and units and counts != {expected}:
        raise ValueError(f"workspace wrapper did not use {expected} threads: {counts}")
    return next(iter(counts)) if len(counts) == 1 else None


def compiler_scenario(s: Scenario, threads: int | None, passes: bool,
                      workspace_threads: int | None = None) -> Scenario:
    """Explicit Cargo config survives hosted transport, unlike arbitrary env.

    Never mutate shared scenarios or put Cargo options after libtest's --.
    The effective command is saved with every result for reproducibility.
    """
    if threads is not None and workspace_threads is not None:
        raise ValueError("global rustc threads override the workspace wrapper control")
    configs = []
    if workspace_threads is not None:
        configs += ["--config", f'env.JCODE_FRONTEND_THREADS.value="{workspace_threads}"',
                    "--config", "env.JCODE_FRONTEND_THREADS.force=true"]
    if threads is not None:
        configs += ["--config", f'build.rustflags=["-Zthreads={threads}"]']
    if passes:
        configs += ["--config", 'env.JUSTRUST_PASSES="on"']
    return replace(s, cmd=s.cmd[:1] + configs + s.cmd[1:]) if configs else s


def optimization_env() -> dict:
    """Only public performance controls, never credential-bearing environment."""
    names = (
        "JUSTRUST_SHADOW",
        "JUSTRUST_TEST_FILTER_COMPILE", "JUSTRUST_CACHE_PROC_MACROS",
        "JUSTRUST_CUTOFF", "JUSTRUST_CARGO_CUTOFF", "JUSTRUST_CARGO_FORK",
        "JUSTRUST_CARGO_DAEMON", "JUSTRUST_CARGO_DAEMON_TRACE",
        "JUSTRUST_UNIFY_FEATURES",
        "JUSTRUST_HOSTED_HTTP_POOL",
    )
    return {name: os.environ[name] for name in names if name in os.environ}


def env_info() -> dict:
    def tryrun(cmd):
        try:
            return sh(cmd)
        except Exception:
            return None

    cpu = None
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            cpu = line.split(":", 1)[1].strip()
            break
    repo_rev = (tryrun(["git", "-C", str(ROOT), "rev-parse", "--short=12", "HEAD"])
                or os.environ.get("JUSTRUST_BENCH_REPO_REV") or None)
    declared_rev = os.environ.get("JUSTRUST_BENCH_REV") or None
    digest = binary_sha256()
    version = tryrun([J, "-V"])
    verify_binary_identity({"justrust_binary_sha256": digest})
    return {
        "host": platform.node(),
        "cpu": cpu,
        "ncpu": os.cpu_count(),
        "mem_gb": round(os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 2**30, 1),
        "kernel": platform.release(),
        "justrust": version,
        # Legacy label retained for old compare consumers. Repository HEAD is
        # not necessarily the installed executable's build revision.
        "justrust_rev": declared_rev or repo_rev,
        "benchmark_repo_rev": repo_rev,
        "justrust_binary_revision": declared_rev,
        "justrust_revision_source": "declared JUSTRUST_BENCH_REV" if declared_rev else "repository HEAD, not binary revision",
        "justrust_binary_sha256": digest,
        "justrust_dirty": os.environ.get("JUSTRUST_BENCH_DIRTY") == "1"
        if "JUSTRUST_BENCH_DIRTY" in os.environ
        else bool(tryrun(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"])),
        "rustc": tryrun(["rustc", "-V"]) if TOOLCHAIN == "system" else tryrun(
            [str(toolchain_rustc(TOOLCHAIN)), "-V"]),
        "toolchain": TOOLCHAIN,
        "optimization_env": optimization_env(),
        "rustc_threads": RUSTC_THREADS,
        "workspace_threads": WORKSPACE_THREADS,
        "pass_times": PASS_TIMES,
        "cargo": tryrun(["cargo", "-V"]),
        "governor": tryrun(["cat", "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"]),
        "on_ac": tryrun(["cat", "/sys/class/power_supply/AC/online"]),
        "snapshots": {k: {r: rev for r, (_, rev) in v["repos"].items()} for k, v in SNAPSHOTS.items()},
    }


def table(result: dict) -> str:
    rows = [
        f"{'scenario':28s} {'median':>7s} {'min':>7s} {'max':>7s} {'n':>3s} {'quiet':>7s}"
        f" {'1st err':>7s} {'start':>6s} {'compile':>7s} {'tests':>6s} {'front':>6s} {'codegen':>7s} {'incr':>6s} {'link':>6s} {'units':>5s}"
    ]
    for name, sc in result["scenarios"].items():
        st = sc["stats"]
        ph, sp = st["phases"], st["split"]
        q = f"{st['quiet_median']:.2f}" if st["quiet_n"] else "-"
        fe = f"{st['first_error']:.2f}" if st.get("first_error") is not None else "-"
        if st.get("first_fail") is not None:
            fe = f"{st['first_fail']:.2f}"
        rows.append(
            f"{name:28s} {st['median']:7.2f} {st['min']:7.2f} {st['max']:7.2f} {st['n']:3d} {q:>7s}"
            f" {fe:>7s} {ph['startup']:6.2f} {ph['compile']:7.2f} {ph['test_run']:6.2f}"
            f" {sp['frontend']:6.2f} {sp['codegen']:7.2f} {sp['incremental']:6.2f} {sp['link']:6.2f} {st['units']:5.0f}"
        )
    return "\n".join(rows)


def cmd_list(_args) -> None:
    for s in SCENARIOS:
        tag = " [cold, opt-in]" if s.cold else ""
        print(f"{s.name:28s} {s.what}{tag}\n{'':28s} justrust {' '.join(s.cmd)}")


def cmd_run(args) -> None:
    global TOOLCHAIN, REMOTE, RUSTC_THREADS, WORKSPACE_THREADS, PASS_TIMES
    TOOLCHAIN = args.toolchain
    RUSTC_THREADS = getattr(args, "rustc_threads", None)
    WORKSPACE_THREADS = getattr(args, "workspace_threads", None)
    PASS_TIMES = getattr(args, "pass_times", False)
    if WORKSPACE_THREADS is not None:
        if RUSTC_THREADS is not None:
            sys.exit("bench: global rustc threads override workspace threads; choose only one")
        if (args.synth is not None or not getattr(args, "scenarios", None)
                or any(n not in BY_NAME or BY_NAME[n].snapshot != "desktop" for n in args.scenarios)):
            sys.exit("bench: workspace threads require named Desktop wrapper scenarios")
    if (RUSTC_THREADS is not None or WORKSPACE_THREADS is not None or PASS_TIMES) and not TOOLCHAIN.startswith(("nightly-", "jr-")):
        sys.exit("bench: compiler thread/pass options require an explicit nightly or JR toolchain")
    if (RUSTC_THREADS is not None or WORKSPACE_THREADS is not None or PASS_TIMES) and args.synth is not None:
        sys.exit("bench: compiler thread/pass options require named pinned scenarios")
    REMOTE = getattr(args, "remote", False)
    hosted_early = getattr(args, "hosted_early", False)
    if hosted_early and (not REMOTE or args.synth is not None or not getattr(args, "scenarios", None)
                        or any(n not in BY_NAME or not BY_NAME[n].expect_fail for n in args.scenarios)):
        sys.exit("bench: --hosted-early requires --remote and named compile-error scenarios")
    if REMOTE and any(BY_NAME[n].env.get("JUSTRUST_EARLY") == "1"
                      for n in (getattr(args, "scenarios", None) or []) if n in BY_NAME):
        sys.exit("bench: local analyzer startup is not a hosted measurement; use --hosted-early on error scenarios")
    if REMOTE and args.synth is not None:
        sys.exit("bench: hosted synthetic measurements use bench/versus.py --remote")
    if TOOLCHAIN != "system":
        # Install up front so the first sample does not include the download.
        subprocess.run([J, "toolchain", "install", TOOLCHAIN], check=True, stdout=subprocess.DEVNULL)
    if args.synth is not None:
        return cmd_run_synth(args)
    names = args.scenarios or [s.name for s in SCENARIOS if not s.cold
                               and not (REMOTE and s.env.get("JUSTRUST_EARLY") == "1")]
    if args.cold:
        names += [s.name for s in SCENARIOS if s.cold and s.name not in names]
    for n in names:
        if n not in BY_NAME:
            sys.exit(f"bench: unknown scenario {n} (see `bench/suite.py list`)")
        if REMOTE and (BY_NAME[n].cold or BY_NAME[n].prebuilt):
            sys.exit("bench: --remote supports warm scenarios only. Clearing a local target/cache "
                     "does not clear the persistent remote target/cache; it is not a cold hosted build.")
    # Fail before hours of building, not after: every anchor must exist once.
    for n in names:
        s = BY_NAME[n]
        if s.edit:
            root = prepare(s.snapshot)
            c = pristine(root, s.edit.file).read_text().count(s.edit.anchor)
            if c != 1:
                sys.exit(f"bench: {n}: anchor occurs {c} times in {s.edit.file}")
    info = env_info()
    info["execution"] = "remote" if REMOTE else "local"
    info["hosted_early"] = hosted_early
    if REMOTE:
        info["target_state"] = "persistent remote target; first run is warmup"
    started = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    result = {"started": started, "env": info, "runs_per_scenario": args.n, "scenarios": {}}
    for k, name in enumerate(names):
        print(f"bench: {name} ({k + 1}/{len(names)})", file=sys.stderr)
        scenario = compiler_scenario(BY_NAME[name], RUSTC_THREADS, PASS_TIMES, WORKSPACE_THREADS)
        if hosted_early:
            scenario = replace(scenario, env={**scenario.env, "JUSTRUST_EARLY": "query", "JUSTRUST_EARLY_EXIT": "0"})
        result["scenarios"][name] = run_scenario(scenario, args.n, args.warmup, args.quiet_wait)
    verify_binary_identity(info)
    result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    out = Path(args.out) if args.out else RESULTS / (
        time.strftime("%Y%m%d-%H%M%S") + f"-{info['justrust_rev']}{'-dirty' if info['justrust_dirty'] else ''}"
        + ("" if TOOLCHAIN == "system" else f"-{TOOLCHAIN}") + ("-remote" if REMOTE else "") + ".json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1) + "\n")
    print(table(result))
    print(f"\nsaved {out}")


def _synth():
    sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
    import run as synth_run  # noqa: E402
    import versus  # noqa: E402

    return synth_run, versus


def cmd_run_synth(args) -> None:
    """The synthetic usage-weighted workload (bench/synth/) on justrust as
    agents run it: one stack, for comparing justrust commits."""
    synth_run, versus = _synth()
    # Its own target dir per toolchain, kept warm across runs and commits.
    st = versus.Stack("suite" if TOOLCHAIN == "system" else f"suite-{TOOLCHAIN}", "justrust defaults", True,
                      {} if TOOLCHAIN == "system" else {"JUSTRUST_TOOLCHAIN": TOOLCHAIN}, private_cache=True)
    info = env_info()
    result = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info,
              "synth": synth_run.run([st], args.synth, args.points, args.n, args.quiet_wait, partial=RESULTS / "partial" / f"synth{args.synth}.json", resume=args.resume)}
    verify_binary_identity(info)
    result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    out = Path(args.out) if args.out else RESULTS / (
        f"synth{args.synth}-" + time.strftime("%Y%m%d-%H%M%S")
        + f"-{info['justrust_rev']}{'-dirty' if info['justrust_dirty'] else ''}"
        + ("" if TOOLCHAIN == "system" else f"-{TOOLCHAIN}") + ".json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1) + "\n")
    print(synth_run.table(result["synth"]))
    print(f"\nsaved {out}")


def cmd_show(args) -> None:
    r = json.loads(Path(args.file).read_text())
    if "synth" in r:
        synth_run, _ = _synth()
        print(synth_run.table(r["synth"]))
        print(synth_run.detail(r["synth"]))
        return
    e = r["env"]
    print(f"{args.file}: justrust {e['justrust_rev']}{' dirty' if e['justrust_dirty'] else ''}, {e['rustc']}, {e['cpu']}, {r['started']}")
    print(table(r))


def cmd_compare(args) -> None:
    a, b = (json.loads(Path(f).read_text()) for f in (args.a, args.b))
    if "synth" in a or "synth" in b:
        if not ("synth" in a and "synth" in b):
            sys.exit("bench: compare a synthetic result with another synthetic result")
        synth_run, _ = _synth()
        print(f"A {args.a} (justrust {a['env']['justrust_rev']})\nB {args.b} (justrust {b['env']['justrust_rev']})")
        # Stack names can differ (toolchain suffix): compare position by position.
        b["synth"]["stack_names"] = list(a["synth"]["stack_names"])
        for p in b["synth"]["results"]:
            p["stats"] = dict(zip(a["synth"]["stack_names"], p["stats"].values()))
        print(synth_run.compare(a["synth"], b["synth"]))
        return
    if a["env"]["snapshots"] != b["env"]["snapshots"]:
        sys.exit("bench: different pinned snapshots, numbers are not comparable")
    if a["env"].get("host") != b["env"].get("host"):
        print("WARNING: different machines", file=sys.stderr)
    if a["env"].get("execution", "local") != b["env"].get("execution", "local"):
        print("WARNING: comparing local and remote execution; remote wall includes transport", file=sys.stderr)
    print(f"A {args.a} (justrust {a['env']['justrust_rev']})\nB {args.b} (justrust {b['env']['justrust_rev']})")
    print(f"{'scenario':28s} {'A med':>7s} {'B med':>7s} {'delta':>7s} {'%':>6s}  verdict")
    for name in a["scenarios"]:
        if name not in b["scenarios"]:
            continue
        sa, sb = a["scenarios"][name]["stats"], b["scenarios"][name]["stats"]
        d = sb["median"] - sa["median"]
        pct = 100 * d / sa["median"] if sa["median"] else 0.0
        # Conservative: the sample ranges must not overlap AND the median must
        # move by more than the noise floor. Re-running the same binary in a
        # later session moved medians by up to 9% with disjoint ranges
        # (FINDINGS.md section 20), so range separation alone is not enough.
        big = abs(pct) > args.floor
        if sb["max"] < sa["min"] and big:
            verdict = "faster"
        elif sb["min"] > sa["max"] and big:
            verdict = "slower"
        else:
            verdict = "within noise"
        samples = [a["scenarios"][name].get("samples", []), b["scenarios"][name].get("samples", [])]
        if any(not group or any(x.get("noisy") is not False for x in group) for group in samples):
            verdict = "uncontrolled (noisy or unknown samples)"
        print(f"{name:28s} {sa['median']:7.2f} {sb['median']:7.2f} {d:+7.2f} {pct:+5.0f}%  {verdict}")
        if sa.get("first_error") is not None and sb.get("first_error") is not None:
            print(f"{'  first error':28s} {sa['first_error']:7.2f} {sb['first_error']:7.2f}"
                  f" {sb['first_error'] - sa['first_error']:+7.2f}"
                  f" (ranges {sa['first_error_min']:.2f}-{sa['first_error_max']:.2f}"
                  f" / {sb['first_error_min']:.2f}-{sb['first_error_max']:.2f})")
        if sa.get("first_fail") is not None and sb.get("first_fail") is not None:
            print(f"{'  first failing test':28s} {sa['first_fail']:7.2f} {sb['first_fail']:7.2f}"
                  f" {sb['first_fail'] - sa['first_fail']:+7.2f}"
                  f" (ranges {sa['first_fail_min']:.2f}-{sa['first_fail_max']:.2f}"
                  f" / {sb['first_fail_min']:.2f}-{sb['first_fail_max']:.2f})")

    # Overhead justrust adds around cargo: agent-visible wall minus the wall
    # justrust recorded. Independent of compile noise, so small changes show.
    def overhead(r):
        g = sorted(x["wall"] - x["recorded_wall"] for s in r["scenarios"].values() for x in s["samples"])
        return (g[len(g) // 2], g[-1]) if g else (0.0, 0.0)

    (ma, xa), (mb, xb) = overhead(a), overhead(b)
    print(f"{'exit overhead (med/max)':28s} {ma:.3f}/{xa:.3f}s -> {mb:.3f}/{xb:.3f}s")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    r = sub.add_parser("run")
    r.add_argument("scenarios", nargs="*")
    r.add_argument("-n", type=int, default=5, help="measured runs per scenario (default 5)")
    r.add_argument("--warmup", type=int, default=1, help="discarded runs first (default 1)")
    r.add_argument("--cold", action="store_true", help="also run the cold (fresh target dir) scenarios")
    r.add_argument("--hosted-early", action="store_true",
                   help="query a prewarmed bounded hosted analyzer on compile-error scenarios, preserving full cargo exit")
    r.add_argument("--remote", action="store_true",
                   help="force warm scenarios through the real remote route and reject local fallback; "
                        "not a cold hosted benchmark")
    r.add_argument("--rustc-threads", type=int, choices=(1, 2, 4, 8, 16),
                   help="explicit nightly/JR frontend threads via Cargo config; changes dependency artifacts, warm up first")
    r.add_argument("--workspace-threads", type=int, choices=(1, 2, 4, 6, 8, 12, 16),
                   help="Desktop workspace wrapper threads via forced Cargo env config; "
                        "registry compiler flags stay unchanged, warm up first")
    r.add_argument("--pass-times", action="store_true",
                   help="local-crate pass timings through explicit Cargo environment config; warmup required")
    r.add_argument("--quiet-wait", type=float, default=0, metavar="SECS",
                   help="before each run wait up to SECS for no foreign rustc and load < 2")
    r.add_argument("-o", "--out", help="result file (default bench/results/<time>-<rev>.json)")
    r.add_argument("--synth", type=int, metavar="SEED",
                   help="run the synthetic usage-weighted workload (bench/synth/) instead of the named "
                        "scenarios; -n is then runs per point")
    r.add_argument("--points", type=int, default=40, help="synthetic points (default 40)")
    r.add_argument("--resume", action="store_true",
                   help="synthetic: keep the points of an interrupted run of the same seed")
    r.add_argument("--toolchain", default="system", metavar="SPEC",
                   help="pinned toolchain (nightly-YYYY-MM-DD) via JUSTRUST_TOOLCHAIN, own target dir; "
                        "default: the system rustc")
    r.set_defaults(fn=cmd_run)
    s = sub.add_parser("show")
    s.add_argument("file")
    s.set_defaults(fn=cmd_show)
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.add_argument("--floor", type=float, default=10.0, metavar="PCT",
                   help="minimum median change to call a difference (default 10%%)")
    c.set_defaults(fn=cmd_compare)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
