#!/usr/bin/env python3
"""fmtdiff.py: what blaze's formatter writes against what the pinned nightly's rustfmt writes,
byte for byte. Stock rustfmt and cargo-fmt are the oracle; blaze never runs them outside this
harness.

usage: fmtdiff.py [CORPUS...] [--work DIR] [--apps DIR] [--app NAME]... [--show N] [--jobs N]
                  [--ours-cargo-fmt CMD] [--ours-rustfmt CMD] [--out FILE]

The corpora are
  apps    the bench apps (ripgrep, nushell, fd, helix, biome) as they are, with their own
          rustfmt.toml: `cargo fmt --all` from each root
  apps60  the same with `max_width = 60` in every rustfmt.toml (one written at the root if
          there is none), so that most lines are rewrapped
  tests   rustfmt's own tests/source files, each run as rustfmt's test harness configures it
          (its `//@ rustfmt-<option>: <value>` lines, or tests/config/<name>.toml) and printed
          with `--emit stdout -q`, its out-of-line modules included
and all three run when none is named.

An app is copied from --apps (default /dev/shm/bsub/apps, read only) into --work (default
$TMPDIR/blaze-fmtdiff) three times: one each tool formats and one left as it was. A file is
counted when the oracle's verbose output names it (the files rustfmt reaches from the targets,
modules followed); it is byte-equal when blaze wrote the same bytes the oracle did. `--check`
is run by both on the untouched copy: the exit codes must agree, and the diffs printed are
compared as text. A file blaze refuses (its parser or lowering does not cover something yet) is
reported with the reason and counted as not equal. The first --show differences (default 5)
are printed as short unified diffs.

The oracle is `cargo +<harness/toolchain> fmt` and that toolchain's rustfmt. Ours is the
`cargo-fmt` and `rustfmt` examples of crates/blaze-fmt (built in release unless the commands
are given), until `cargo blaze fmt` dispatches to it. TERM is unset for both, so neither
colours its output.
"""
import argparse, collections, concurrent.futures, difflib, json, os, re, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PIN = open(os.path.join(HERE, "toolchain")).read().strip()
APPS = ["ripgrep", "nushell", "fd", "helix", "biome"]
RUSTFMT_TESTS = "/Volumes/Projects/tmp/blaze-subcmds/rust/src/tools/rustfmt/tests"
REFUSED = re.compile(r"blaze fmt cannot format this file yet: (.*)")
DIRECTIVE = re.compile(r"^//@ rustfmt-(?P<name>[A-Za-z_]+): (?P<value>\S+)$")
CONFIG_NAMES = ("rustfmt.toml", ".rustfmt.toml")


def env():
    e = {k: v for k, v in os.environ.items() if k != "TERM"}
    e.pop("RUSTFMT", None)
    return e


def run(cmd, cwd=None, timeout=1800):
    p = subprocess.run(cmd, cwd=cwd, env=env(), capture_output=True, timeout=timeout)
    return p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace")


def oracle_rustfmt():
    code, out, err = run(["rustup", "which", "rustfmt", "--toolchain", PIN])
    if code != 0:
        sys.exit(f"no rustfmt in {PIN}: {err.strip()}")
    return out.strip()


def build_ours():
    code, out, err = run(["cargo", f"+{PIN}", "build", "--release", "-q", "-p", "blaze-fmt",
                          "--example", "rustfmt", "--example", "cargo-fmt"], cwd=ROOT)
    if code != 0:
        sys.exit(f"building blaze-fmt's examples failed:\n{err}")
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    examples = os.path.join(target, "release", "examples")
    return [os.path.join(examples, "cargo-fmt")], [os.path.join(examples, "rustfmt")]


def copy_tree(src, dst):
    if os.path.exists(dst):
        shutil.rmtree(dst)
    shutil.copytree(src, dst, symlinks=True, ignore=shutil.ignore_patterns("target", ".git"))


def set_max_width(root, width):
    """Sets `max_width` in every rustfmt.toml under `root`, writing one at the root if none."""
    found = False
    for d, dirs, files in os.walk(root):
        for name in files:
            if name not in CONFIG_NAMES:
                continue
            found = True
            path = os.path.join(d, name)
            lines = [l for l in open(path).read().splitlines() if not re.match(r"\s*max_width\s*=", l)]
            # A top-level key must come before the first table.
            at = next((i for i, l in enumerate(lines) if l.lstrip().startswith("[")), len(lines))
            lines.insert(at, f"max_width = {width}")
            open(path, "w").write("\n".join(lines) + "\n")
    if not found:
        open(os.path.join(root, "rustfmt.toml"), "w").write(f"max_width = {width}\n")


def short_diff(a, b, name, limit=40):
    lines = list(difflib.unified_diff(a.splitlines(keepends=True), b.splitlines(keepends=True),
                                      f"oracle/{name}", f"blaze/{name}", n=2))
    more = f"... {len(lines) - limit} more lines\n" if len(lines) > limit else ""
    return "".join(lines[:limit]) + more


def read(path):
    try:
        return open(path, "rb").read()
    except FileNotFoundError:
        return None


def refusals(stderr):
    return [m.group(1) for m in REFUSED.finditer(stderr)]


def app_case(app, corpus, args, ours_cargo_fmt):
    base = os.path.join(args.work, corpus, app)
    orig, oracle, ours = (os.path.join(base, d) for d in ("orig", "oracle", "ours"))
    copy_tree(os.path.join(args.apps, app), orig)
    if corpus == "apps60":
        set_max_width(orig, 60)
    copy_tree(orig, oracle)
    copy_tree(orig, ours)

    check_oracle = run(["cargo", f"+{PIN}", "fmt", "--all", "--check"], cwd=orig)
    check_ours = run(ours_cargo_fmt + ["--all", "--check"], cwd=orig)

    o_code, o_out, o_err = run(["cargo", f"+{PIN}", "fmt", "--all", "--", "-v"], cwd=oracle)
    b_code, b_out, b_err = run(ours_cargo_fmt + ["--all"], cwd=ours)
    reached = sorted({os.path.relpath(l[len("Formatting "):].strip(), oracle)
                      for l in o_out.splitlines() if l.startswith("Formatting ")})

    equal, changed, differ = 0, 0, []
    for rel in reached:
        want, got, was = read(os.path.join(oracle, rel)), read(os.path.join(ours, rel)), read(os.path.join(orig, rel))
        changed += want != was
        if want == got:
            equal += 1
        else:
            differ.append(rel)
    shown = []
    for rel in differ[: args.show]:
        want = (read(os.path.join(oracle, rel)) or b"").decode(errors="replace")
        got = (read(os.path.join(ours, rel)) or b"").decode(errors="replace")
        shown.append(short_diff(want, got, f"{app}/{rel}"))
    return {
        "case": app,
        "files": len(reached),
        "equal": equal,
        "changed_by_oracle": changed,
        "differ": differ,
        "refused": collections.Counter(refusals(b_err)),
        "exit": {"oracle": o_code, "ours": b_code},
        "check": {
            "oracle": check_oracle[0],
            "ours": check_ours[0],
            "agree": check_oracle[0] == check_ours[0],
            "same_output": check_oracle[1] == check_ours[1],
        },
        "oracle_stderr": o_err[-2000:] if o_code else "",
        "ours_stderr": b_err[-2000:] if b_code and not refusals(b_err) else "",
        "shown": shown,
    }


def test_args(path, tests):
    """The rustfmt arguments that configure `path` as rustfmt's test harness does, or None for
    a test the CLI cannot express (line ranges)."""
    directives = {}
    for line in open(path, errors="replace"):
        m = DIRECTIVE.match(line.rstrip("\n"))
        if m:
            directives[m["name"]] = m["value"]
    if "file_lines" in directives:
        return None
    config = os.path.join(tests, "config")
    if directives:
        name = directives.get("config")
    else:
        name = os.path.basename(path)[: -len(".rs")] + ".toml"
    config_path = os.path.join(config, name) if name else None
    if not config_path or not os.path.exists(config_path):
        config_path = os.path.join(tests, "empty.toml")
    out = ["--config-path", config_path, "--emit", "stdout", "-q", "--unstable-features"]
    opts = [f"{k}={v}" for k, v in directives.items() if k not in ("target", "config", "unstable", "stable")]
    if opts:
        out += ["--config", ",".join(opts)]
    return out


def test_case(path, tests, oracle_bin, ours_rustfmt):
    rel = os.path.relpath(path, os.path.join(tests, "source"))
    cli = test_args(path, tests)
    if cli is None:
        return {"case": rel, "skipped": "line ranges (`file_lines`)"}
    want = run([oracle_bin] + cli + [path], timeout=120)
    got = run(ours_rustfmt + cli + [path], timeout=120)
    return {
        "case": rel,
        "equal": want[0] == got[0] and want[1] == got[1],
        "exit": {"oracle": want[0], "ours": got[0]},
        "refused": collections.Counter(refusals(got[2])),
        "diff": "" if want[1] == got[1] else short_diff(want[1], got[1], rel),
        "ours_stderr": got[2][-1000:] if got[0] and not refusals(got[2]) else "",
    }


def report_apps(corpus, cases):
    files = sum(c["files"] for c in cases)
    equal = sum(c["equal"] for c in cases)
    print(f"\n== {corpus}: {equal}/{files} files byte-equal ({pct(equal, files)})")
    for c in cases:
        check = c["check"]
        print(f"  {c['case']:<8} {c['equal']}/{c['files']} ({pct(c['equal'], c['files'])}), "
              f"{c['changed_by_oracle']} changed by rustfmt; --check exit oracle {check['oracle']} "
              f"blaze {check['ours']} ({'agree' if check['agree'] else 'DISAGREE'}"
              f"{', same diff' if check['same_output'] else ', diff text differs'})")
        for why, n in c["refused"].most_common(8):
            print(f"      refused {n}x: {why}")
        for what in ("oracle_stderr", "ours_stderr"):
            if c[what]:
                print(f"      {what}: {c[what].strip()[:600]}")
    return {"files": files, "equal": equal}


def report_tests(cases, show):
    run_cases = [c for c in cases if "skipped" not in c]
    equal = sum(c["equal"] for c in run_cases)
    print(f"\n== tests: {equal}/{len(run_cases)} files byte-equal ({pct(equal, len(run_cases))}), "
          f"{len(cases) - len(run_cases)} skipped (line ranges)")
    reasons = collections.Counter()
    for c in run_cases:
        reasons.update(c["refused"])
    for why, n in reasons.most_common(15):
        print(f"      refused {n}x: {why}")
    for c in [c for c in run_cases if not c["equal"] and not c["refused"]][:show]:
        print(c["diff"] or f"  {c['case']}: exit oracle {c['exit']['oracle']} blaze {c['exit']['ours']} {c['ours_stderr'][:300]}")
    return {"files": len(run_cases), "equal": equal}


def pct(a, b):
    return f"{100.0 * a / b:.1f}%" if b else "n/a"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("corpora", nargs="*", metavar="CORPUS", help="apps, apps60 or tests")
    ap.add_argument("--work", default=os.path.join(tempfile.gettempdir(), "blaze-fmtdiff"))
    ap.add_argument("--apps", default="/dev/shm/bsub/apps")
    ap.add_argument("--app", action="append", choices=APPS)
    ap.add_argument("--tests", default=RUSTFMT_TESTS, help="rustfmt's tests directory")
    ap.add_argument("--show", type=int, default=5)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--ours-cargo-fmt", help="the command blaze's `cargo fmt` runs as")
    ap.add_argument("--ours-rustfmt", help="the command blaze's `rustfmt` runs as")
    ap.add_argument("--out", help="write every case as JSON")
    ap.add_argument("--commit", help="the blaze commit to report (default: this checkout's HEAD)")
    args = ap.parse_args()
    corpora = args.corpora or ["apps", "apps60", "tests"]
    for c in corpora:
        if c not in ("apps", "apps60", "tests"):
            ap.error(f"no corpus {c!r}: apps, apps60 or tests")
    os.makedirs(args.work, exist_ok=True)

    if args.ours_cargo_fmt and args.ours_rustfmt:
        ours_cargo_fmt, ours_rustfmt = args.ours_cargo_fmt.split(), args.ours_rustfmt.split()
    else:
        ours_cargo_fmt, ours_rustfmt = build_ours()
    oracle_bin = oracle_rustfmt()
    commit = args.commit or run(["git", "rev-parse", "--short=10", "HEAD"], cwd=ROOT)[1].strip()
    host = os.uname().nodename
    print(f"blaze-fmt at {commit} on {host}; oracle rustfmt {run([oracle_bin, '--version'])[1].strip()}")

    results, totals = {}, {}
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        for corpus in corpora:
            if corpus == "tests":
                tests = os.path.join(args.work, "tests")
                copy_tree(args.tests, tests)
                open(os.path.join(tests, "empty.toml"), "w").close()
                paths = sorted(os.path.join(d, f) for d, _, fs in os.walk(os.path.join(tests, "source"))
                               for f in fs if f.endswith(".rs"))
                cases = list(pool.map(lambda p: test_case(p, tests, oracle_bin, ours_rustfmt), paths))
                totals[corpus] = report_tests(cases, args.show)
            else:
                apps = args.app or APPS
                cases = list(pool.map(lambda a: app_case(a, corpus, args, ours_cargo_fmt), apps))
                totals[corpus] = report_apps(corpus, cases)
                for c in cases:
                    for d in c["shown"]:
                        print(d)
            results[corpus] = cases
    print("\n" + "; ".join(f"{k} {v['equal']}/{v['files']} ({pct(v['equal'], v['files'])})" for k, v in totals.items())
          + f" at {commit} on {host}")
    if args.out:
        json.dump({"commit": commit, "host": host, "totals": totals, "cases": results}, open(args.out, "w"), indent=1, default=dict)


if __name__ == "__main__":
    main()
