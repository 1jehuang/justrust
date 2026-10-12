#!/usr/bin/env python3
"""Correctness of the build daemon's prebuild: for the same edited sources,
the replayed prebuild result vs a direct build (daemon off) in its own warm
target dir. Compares exit code, test result lines, error lines, then checks
that a plain build right after compiles nothing (no Dirty units).

Holds the bench snapshot lock. Usage: bench/buildd-correctness.py
"""
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

suite.bench_lock()
root = suite.prepare("desktop")
cwd = root / "jcode-desktop"
T = suite.JR_HOME / "bench" / "buildd-correctness"
CMD = ["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"]
edit = suite.BY_NAME["desktop-test-edit"].edit
FPS = root / edit.file


def run(target: str, daemon: bool, extra=None):
    env = dict(os.environ, CARGO_TARGET_DIR=str(T / target), JUSTRUST_REMOTE="0",
               JUSTRUST_TOOLCHAIN=suite.TOOLCHAIN, JUSTRUST_PREBUILT="off",
               JUSTRUST_BUILDD="1" if daemon else "0", JCODE_SESSION_ID=f"bench-correct-{target}")
    env.update(extra or {})
    p = subprocess.run([suite.J, *CMD], cwd=cwd, env=env, capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def norm(out: str, err: str) -> list[str]:
    keep = []
    for line in (out + err).splitlines():
        if re.match(r"^(test |error|failures:|---- |thread '|test result|\s+left|\s+right|assertion)", line):
            line = re.sub(r"thread '([^']*)' \(\d+\)", r"thread '\1'", line)
            line = re.sub(r"finished in [0-9.]+s", "finished in Xs", line)
            line = re.sub(r"<[0-9.]+m?s>", "<Xs>", line)
            # Pass counts of a failing run depend on test order: justrust
            # runs likely failures first with fail-fast, ordered by recorded
            # history, so two direct runs differ too (2 passed, then 0).
            line = re.sub(r"\d+ passed; (\d+ failed)", r"N passed; \1", line)
            if line.startswith("test ") and line.rstrip().endswith(("ok", "ok <Xs>")):
                continue
            keep.append(line)
    return sorted(keep)


def idle():
    for _ in range(240):
        o = subprocess.run([suite.J, "daemon", "status"], capture_output=True, text=True).stdout
        if " 0 running" in o or "no build daemon" in o:
            return
        time.sleep(0.5)


cases = [
    ("pass", edit.template.replace("{n}", "424242")),
    # A failing assertion: exit 101 and the panic text must match.
    ("fail", edit.template.replace("std::hint::black_box({n}u64);", "assert_eq!(1 + 1, 3, \"buildd check\");")),
]
ok = True
try:
    orig = suite.pristine(root, edit.file).read_text()
    # Warm both targets on pristine sources; the daemon one teaches the
    # daemon this request.
    FPS.write_text(orig)
    run("direct", False)
    run("daemon", True)
    idle()
    for name, new in cases:
        FPS.write_text(orig.replace(edit.anchor, new, 1))
        time.sleep(1.0)
        idle()  # the prebuild ran and finished
        c1, o1, e1 = run("daemon", True)
        # `test` prebuilds compile only (--no-run): the agent's run must find
        # every unit Fresh and run the tests itself.
        m = re.search(r"units\s+(\d+) compiled", e1)
        replayed = "result of a prebuild" in e1 or (m is not None and m.group(1) == "0")
        c2, o2, e2 = run("direct", False)
        same = norm(o1, e1) == norm(o2, e2)
        # After both: a plain build of the same sources compiles nothing.
        _, _, e3 = run("daemon", False)
        dirty = re.search(r"units\s+(\d+) compiled", e3)
        nodirty = dirty is not None and dirty.group(1) == "0"
        print(f"{name}: prebuilt={replayed} exit daemon={c1} direct={c2} same_output={same} "
              f"no_dirty_after={nodirty}")
        if not same:
            a, b = norm(o1, e1), norm(o2, e2)
            print("  only daemon:", [x for x in a if x not in b])
            print("  only direct:", [x for x in b if x not in a])
        ok &= replayed and c1 == c2 and same and nodirty
finally:
    suite.restore(root, edit)
    idle()
print("OK" if ok else "MISMATCH")
sys.exit(0 if ok else 1)
