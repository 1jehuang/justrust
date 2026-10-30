#!/usr/bin/env python3
"""Compares `cargo blaze clippy` with stock `cargo clippy` on the bench apps and a fixture.

For each app it lints a copy with both, `--all-targets --message-format=json` (or the
arguments `--args` gives, with `--message-format=json`), and compares
the clippy diagnostics by where they point: (lint, file, line, column), each counted once
however many builds report it. A site both report is matched, and its rendered text is
compared byte for byte; one only stock reports is missed, one only blaze reports is extra.
rustc's own warnings are counted apart: blaze does not run rustc's lints. The exit statuses
are compared too.

    harness/clippydiff.py --blaze target/release/blaze --work /dev/shm/x/work \
        [--args "--workspace --all-targets"] [--stock-dir DIR] [--out report.json] APP_DIR...

`--stock-dir` holds stock runs made before (`<app>.json` and `<app>.json.status`), so the
oracle is not rebuilt each time. Coverage is matched over stock's sites, overall and over
the lints blaze implements (read from crates/blaze-lints/src/lints/mod.rs).
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
LINTS = HERE.parent / "crates/blaze-lints/src/lints/mod.rs"


def implemented():
    text = LINTS.read_text()
    table = text[text.index("IMPLEMENTED"):]
    table = table[: table.index("];")]
    return set(re.findall(r'\("([a-z0-9_]+)"', table))


def sites(stdout):
    """The clippy diagnostics in cargo's JSON, by site, and rustc's own, counted."""
    clippy = {}
    rustc = Counter()
    for line in stdout.splitlines():
        if not line.startswith("{"):
            continue
        try:
            it = json.loads(line)
        except json.JSONDecodeError:
            continue
        if it.get("reason") != "compiler-message":
            continue
        message = it["message"]
        code = (message.get("code") or {}).get("code") or ""
        if not code.startswith("clippy::"):
            if message.get("level") in ("warning", "error") and code:
                rustc[code] += 1
            continue
        spans = [s for s in message.get("spans", []) if s.get("is_primary")]
        if not spans:
            continue
        span = spans[0]
        key = (code[len("clippy::"):], span["file_name"], span["line_start"], span["column_start"])
        clippy.setdefault(key, message.get("rendered") or "")
    return clippy, rustc


def run(command, cwd, env, log):
    start = time.time()
    with open(log, "w") as err:
        done = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=err, text=True)
    return done.stdout, done.returncode, time.time() - start


def stock(app, copy, args, env):
    if args.stock_dir:
        out = Path(args.stock_dir) / f"{app.name}.json"
        status = Path(args.stock_dir) / f"{app.name}.json.status"
        if out.exists() and status.exists():
            return out.read_text(), int(status.read_text().split()[0]), None
    env = dict(env, CARGO_TARGET_DIR=str(Path(args.work) / "stock-target" / app.name))
    return run(
        ["cargo", "clippy", *args.args.split(), "--message-format=json"],
        copy,
        env,
        Path(args.work) / f"{app.name}.stock.err",
    )


def blaze(app, copy, args, env):
    return run(
        [args.blaze, "clippy", *args.args.split(), "--message-format=json"],
        copy,
        env,
        Path(args.work) / f"{app.name}.blaze.err",
    )


def compare(app, args, ours):
    work = Path(args.work)
    copy = work / "apps" / app.name
    if not copy.exists():
        copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(app, copy, symlinks=True, ignore=shutil.ignore_patterns("target"))
    env = dict(os.environ)
    stock_out, stock_status, stock_secs = stock(app, copy, args, env)
    blaze_out, blaze_status, blaze_secs = blaze(app, copy, args, env)
    (work / f"{app.name}.blaze.json").write_text(blaze_out)
    want, rustc = sites(stock_out)
    got, _ = sites(blaze_out)
    per_lint = defaultdict(Counter)
    unequal = []
    for key, rendered in want.items():
        lint = key[0]
        if key in got:
            per_lint[lint]["matched"] += 1
            if got[key] == rendered:
                per_lint[lint]["equal"] += 1
            else:
                unequal.append(key)
        else:
            per_lint[lint]["missed"] += 1
    for key in got:
        if key not in want:
            per_lint[key[0]]["extra"] += 1
    return {
        "app": app.name,
        "stock_status": stock_status,
        "blaze_status": blaze_status,
        "stock_secs": stock_secs,
        "blaze_secs": blaze_secs,
        "rustc_warnings": sum(rustc.values()),
        "per_lint": {lint: dict(c) for lint, c in per_lint.items()},
        "unequal": [list(k) for k in unequal],
        "missed": [list(k) for k in want if k not in got and k[0] in ours],
        "extra": [list(k) for k in got if k not in want],
    }


def totals(per_lint, only=None):
    total = Counter()
    for lint, c in per_lint.items():
        if only is None or lint in only:
            total.update(c)
    return total


def line(name, c):
    stock = c["matched"] + c["missed"]
    cover = 100.0 * c["matched"] / stock if stock else 100.0
    return (
        f"{name:<40} stock {stock:>5}  matched {c['matched']:>5}  missed {c['missed']:>5}  "
        f"extra {c['extra']:>4}  rendered-equal {c['equal']:>5}  coverage {cover:5.1f}%"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--blaze", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--args", default="--all-targets")
    parser.add_argument("--stock-dir")
    parser.add_argument("--out")
    parser.add_argument("apps", nargs="+")
    args = parser.parse_args()
    Path(args.work).mkdir(parents=True, exist_ok=True)
    ours = implemented()
    reports = []
    overall = defaultdict(Counter)
    for app in map(Path, args.apps):
        report = compare(app, args, ours)
        reports.append(report)
        for lint, c in report["per_lint"].items():
            overall[lint].update(c)
        per = report["per_lint"]
        agree = "agree" if (report["stock_status"] == 0) == (report["blaze_status"] == 0) else "DIFFER"
        print(f"== {app.name}: exit stock {report['stock_status']} blaze {report['blaze_status']} ({agree}); "
              f"rustc's own warnings (not blaze's) {report['rustc_warnings']}; blaze {report['blaze_secs']:.1f}s")
        print(line("  all lints", totals(per)))
        print(line("  implemented lints", totals(per, ours)))
        for key in report["unequal"][:5]:
            print(f"  rendered differs: {key}")
        for key in report["missed"][:5]:
            print(f"  missed (implemented): {key}")
        for key in report["extra"][:5]:
            print(f"  extra: {key}")
    print("== per lint, all apps")
    for lint, c in sorted(overall.items(), key=lambda it: -(it[1]["matched"] + it[1]["missed"] + it[1]["extra"])):
        print(line(("* " if lint in ours else "  ") + lint, c))
    print(line("== overall, all lints", totals(overall)))
    print(line("== overall, implemented lints", totals(overall, ours)))
    if args.out:
        Path(args.out).write_text(json.dumps({"implemented": sorted(ours), "apps": reports}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
