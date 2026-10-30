#!/usr/bin/env python3
"""The verdict on a `uitest.py --json` run: every test that ran has to match rustc, except the
ones named in `ui-known.txt` (one test per line, `#` for why), which differ in a way that is
understood. A test that is listed and now matches is said too, so the list shrinks. A test
followed by backends (`panics/x.rs llvm  # why`) is known on those alone, the one
`RA_JIT_BACKEND` names.

usage: uigate.py results.json
"""
import collections
import json
import os
import sys
from pathlib import Path

KNOWN = Path(__file__).with_name("ui-known.txt")


def main() -> None:
    results = json.load(open(sys.argv[1]))
    backend = os.environ.get("RA_JIT_BACKEND", "cranelift")
    entries = [line.split("#")[0].split() for line in KNOWN.read_text().splitlines()]
    known = {test for test, *on in filter(None, entries) if not on or backend in on}
    counts = collections.Counter(r["outcome"] for r in results)
    print("tests/ui run-pass:", ", ".join(f"{n} {k}" for k, n in sorted(counts.items())))
    bad = sorted({r["test"] for r in results if r["outcome"] not in ("match", "excluded")})
    new = [t for t in bad if t not in known]
    fixed = sorted(k for k in known if k not in bad and any(r["test"] == k for r in results))
    for t in new:
        print(f"NEW difference: {t}")
    for t in fixed:
        print(f"known difference now matches (remove from ui-known.txt): {t}")
    if new:
        sys.exit(1)


if __name__ == "__main__":
    main()
