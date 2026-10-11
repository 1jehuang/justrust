#!/usr/bin/env python3
"""Probe: apply one corpus mutation, query the early daemon repeatedly (raw),
to tell "rust-analyzer does not report this" from "reported late".
Usage: bench/early_probe.py <mutation name substring> [polls]"""
import sys
import time

sys.argv += [] if len(sys.argv) > 2 else ["4"]
import early_accuracy as ea  # noqa: E402
import shutil  # noqa: E402

name = sys.argv[1]
polls = int(sys.argv[2])
m = next(x for x in ea.M if name in x[0])
_, rel, anchor, repl, want = m
path = ea.ROOT / rel
pristine = ea.suite.pristine(ea.ROOT, rel)
orig = pristine.read_text()
try:
    path.write_text(orig.replace(anchor, repl, 1))
    for i in range(polls):
        t = time.monotonic()
        r = ea.daemon({"op": "pull", "files": [str(path)], "raw": True}, timeout=60)
        print(f"poll {i} {1000 * (time.monotonic() - t):.0f} ms:")
        for d in r.get("raw", []):
            print(f"   sev {d['severity']} {d['code']} line {d['line']}: {str(d['message'])[:110]}")
        time.sleep(1)
finally:
    shutil.copy2(pristine, path)
    ea.daemon({"op": "diagnostics"}, timeout=60)
