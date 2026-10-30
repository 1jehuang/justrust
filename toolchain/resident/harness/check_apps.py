#!/usr/bin/env python3
"""check_apps.py: `cargo blaze check` against `cargo check` on the applications of apps/apps.json.

usage: check_apps.py [NAME...] [--bin ra-jit] [--apps-dir dir] [--work dir] [--json out.json]
                     [--skip NAME] [--timeout secs]

Each application (cloned and prepared as borrowck_apps.py does it) is checked twice with
`check --workspace --all-targets --message-format=json`, which is every crate and target
`cargo blaze check` looks at: by plain cargo (rustc), the reference, and by `cargo blaze`
(the daemon's checker, which runs no rustc), each into a target directory and a daemon home
of its own. The errors and warnings of the workspace's files are compared by file, line and
code (rustc's `E` code, or the lint's name):

  invented  an error or warning `cargo blaze check` reports that rustc does not: an invented
            error fails a build rustc accepts
  missed    one rustc reports that `cargo blaze check` does not

Errors come first. An error of blaze's with no code (a lint a source denies, whose name blaze
does not write) is taken for rustc's lint error on the same line. The differences are printed
by app, then grouped by (invented/missed, level, code) over every app. Exits 1 when an error is
invented or missed, or an app could not be checked; missed warnings alone do not fail it.
"""
import argparse, asyncio, collections, json, os, shutil, subprocess, sys, time

import borrowck_apps

HERE = os.path.dirname(os.path.abspath(__file__))

TOOLCHAIN = open(os.path.join(HERE, "toolchain")).read().strip()
ARGS = ["check", "--workspace", "--all-targets", "--message-format=json"]
# Errors whose primary span rust-analyzer places otherwise than rustc (check_json.py's): only
# their file and code are compared.
PLACED_OTHERWISE = {"E0061"}


def env(home, target, bin_dir):
    it = dict(os.environ, RUSTUP_TOOLCHAIN=TOOLCHAIN, RA_JIT_HOME=home, RUST_BACKTRACE="0",
              CARGO_TERM_COLOR="never", CARGO_TARGET_DIR=target)
    it["RA_JIT_CACHE"] = os.path.join(home, "store")
    it["PATH"] = bin_dir + os.pathsep + it["PATH"]
    for name in ("RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_TARGET"):
        it.pop(name, None)
    return it


def diagnostics(stdout, ws):
    """The errors and warnings of `cargo ... --message-format=json` in the files of `ws`, as
    (level, code, file, line, message)."""
    out = []
    for line in stdout.splitlines():
        if not line.startswith("{"):
            continue
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-message":
            continue
        d = m["message"]
        if d["level"] not in ("error", "warning"):
            continue
        spans = [s for s in d["spans"] if s["is_primary"]] or d["spans"]
        # rustc's summaries ("aborting due to", "N warnings emitted") are of no place.
        if not spans:
            continue
        span = spans[0]
        path = span["file_name"]
        if os.path.isabs(path):
            if not path.startswith(ws.rstrip("/") + "/"):
                continue
            path = os.path.relpath(path, ws)
        code = d["code"] and d["code"]["code"]
        out.append((d["level"], code, path, span["line_start"], d["message"]))
    # A file of a library is its tests' too, and rustc says the same of it in each.
    return sorted(set(out), key=str)


def compare(ours, theirs):
    """(invented, missed): what only blaze reports and what only rustc does, as multisets."""
    def key(d):
        level, code, path, line, _ = d
        return (level, code, path, None if code in PLACED_OTHERWISE else line)

    left = collections.Counter(map(key, theirs))
    invented = []
    for d in ours:
        k = key(d)
        if left[k] > 0:
            left[k] -= 1
            continue
        # A denied lint blaze writes without its name.
        level, code, path, line, _ = d
        named = next((t for t in left if left[t] > 0 and code is None and t[0] == level and t[2] == path
                      and t[3] == line and t[1] and not t[1].startswith("E")), None)
        if named:
            left[named] -= 1
            continue
        invented.append(d)
    missed = []
    for d in theirs:
        k = key(d)
        if left[k] > 0:
            left[k] -= 1
            missed.append(d)
    return invented, missed


def run(argv, cwd, environ, timeout):
    began = time.monotonic()
    try:
        done = subprocess.run(argv, cwd=cwd, env=environ, capture_output=True, text=True, errors="replace",
                              timeout=timeout, stdin=subprocess.DEVNULL)
        return done.returncode, done.stdout, done.stderr, round(time.monotonic() - began)
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return None, out, f"timed out after {timeout}s", round(time.monotonic() - began)


def check(name, spec, args):
    try:
        ws = borrowck_apps.LOCAL.get(name) or asyncio.run(borrowck_apps.checkout(name, spec, args.apps_dir))
    except borrowck_apps.StepError as e:
        return {"failed": str(e)}
    ws = os.path.realpath(ws)
    work = os.path.join(args.work, name)
    shutil.rmtree(work, ignore_errors=True)
    bin_dir = os.path.dirname(args.bin)
    result = {}
    for side, argv in (("rustc", ["cargo", *ARGS]), ("blaze", ["cargo", "blaze", *ARGS])):
        home = os.path.join(work, side + "-home")
        os.makedirs(home)
        environ = env(home, os.path.join(work, side + "-target"), bin_dir)
        code, out, err, secs = run(argv, ws, environ, args.timeout)
        if side == "blaze":
            subprocess.run([args.bin, "stop"], env=environ, capture_output=True)
        found = diagnostics(out, ws)
        result[side] = {"status": code, "secs": secs, "found": found, "stderr": err[-2000:]}
        # A check that ended without saying anything of the source did not check it.
        if code != 0 and not any(level == "error" for level, *_ in found):
            result["failed"] = f"{side}: `{' '.join(argv)}` exited {code} with no error of the source: {err[-800:]}"
    invented, missed = compare(result["blaze"]["found"], result["rustc"]["found"])
    result.update(invented=invented, missed=missed)
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    count = lambda ds, level: sum(1 for d in ds if d[0] == level)
    print(f"{name}: invented {count(invented, 'error')} errors {count(invented, 'warning')} warnings, "
          f"missed {count(missed, 'error')} errors {count(missed, 'warning')} warnings"
          f"{', FAILED' if 'failed' in result else ''} (rustc {result['rustc']['secs']}s, blaze {result['blaze']['secs']}s)",
          file=sys.stderr, flush=True)
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("names", nargs="*")
    ap.add_argument("--bin", default=os.environ.get("RA_JIT_BIN", "ra-jit"))
    ap.add_argument("--apps-dir", default=os.environ.get("APPS_DIR") or os.path.join(HERE, "vendor", "apps"))
    ap.add_argument("--work", default=os.path.join(os.environ.get("TMPDIR", "/tmp"), "check-apps"))
    ap.add_argument("--timeout", type=int, default=3600)
    ap.add_argument("--json")
    ap.add_argument("--keep", action="store_true", help="keep the target directories and homes")
    ap.add_argument("--skip", action="append", default=[], help="apps left out")
    args = ap.parse_args()
    args.bin = os.path.abspath(args.bin) if os.sep in args.bin else shutil.which(args.bin) or args.bin
    specs = json.load(open(borrowck_apps.SPECS))
    specs.update({name: {} for name in borrowck_apps.LOCAL})
    names = args.names or [n for n in specs if n not in args.skip]
    results = {}
    for name in names:
        results[name] = check(name, specs[name], args)
        if args.json:
            json.dump(results, open(args.json, "w"), indent=1)
    groups = collections.Counter()
    bad = 0
    for name, r in results.items():
        if "failed" in r:
            bad += 1
            print(f"\n== {name}: FAILED: {r['failed']}")
        if "invented" not in r:
            continue
        for what in ("invented", "missed"):
            for level, code, path, line, message in sorted(r[what], key=lambda d: (d[0] != "error", d[2], d[3])):
                groups[(what, level, code)] += 1
                if level == "error":
                    bad += 1
        errors = [(w, d) for w in ("invented", "missed") for d in r[w] if d[0] == "error"]
        warnings = collections.Counter((w, d[1]) for w in ("invented", "missed") for d in r[w] if d[0] == "warning")
        if errors or warnings:
            print(f"\n== {name}")
        for what, (level, code, path, line, message) in errors:
            print(f"   {what} error {code or '-'} {path}:{line}: {message[:160]}")
        # The warnings themselves are in --json.
        if warnings:
            print("   warnings: " + ", ".join(f"{n} {w} {c or '-'}" for (w, c), n in warnings.most_common()))
    print("\nby kind (errors first):")
    for (what, level, code), n in sorted(groups.items(), key=lambda kv: (kv[0][1] != "error", kv[0][0], -kv[1])):
        print(f"  {n:6} {what} {level} {code or '-'}")
    clean = sum(1 for r in results.values() if "failed" not in r and not any(d[0] == "error" for w in ("invented", "missed") for d in r.get(w, [])))
    print(f"\n{clean} of {len(results)} apps have the errors rustc has")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
