#!/usr/bin/env python3
"""Milestone evaluator: how far the clean-room compiler is from full Rust.

  M1 core           tests/fixtures/full_programs (manifest `pass` lines)
  M2 idiomatic std  tests/milestones/m2/*.rs
  M3 real programs  tests/milestones/m3 (single files and multi-file src/main.rs)
  M4 crates         tests/milestones/m4/crates.txt, unchanged crates.io sources
  M5 rustc UI       deterministic sample of rust-lang/rust tests/ui (pinned tag)

M1-M3 pass = native BUILD succeeds and stdout matches the stored stock output.
M4 pass = `jr-rust check` then `jr-rust test --lib` succeed where stock cargo does.
M5 pass = same verdict as stock rustc (accept/reject) and, for run-pass tests,
identical stdout and exit status. Stock rustc results are cached per test.

Usage: scripts/milestones.py [--only m2,m5] [--jobs N] [--ui-sample N] [-v]
Exit 0 always unless infrastructure breaks; this reports progress, it is not a gate.
"""
import argparse
import concurrent.futures as cf
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
MS = ROOT / "tests/milestones"
CACHE = Path(os.environ.get("JR_MILESTONE_CACHE", Path.home() / ".cache/justrust-milestones"))
RUST_TAG = "1.98.1"
TIMEOUT = 120


def jr_binary():
    # JR_RUST overrides. Otherwise use the most recently built binary: hosted
    # machines only bring back optimized (musl) builds, never target/debug.
    if os.environ.get("JR_RUST"):
        return Path(os.environ["JR_RUST"])
    found = [p for p in (ROOT / "target/debug/jr-rust", ROOT / "target/release/jr-rust",
                         ROOT / "target/x86_64-unknown-linux-musl/release/jr-rust") if p.exists()]
    if not found:
        raise SystemExit("build the compiler first: justrust build --bin jr-rust")
    return max(found, key=lambda p: p.stat().st_mtime)


def run(cmd, cwd=None, timeout=TIMEOUT, env=None):
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, timeout=timeout, env=env)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, b"", b"timeout"


def first_error(stderr):
    lines = [l for l in stderr.decode(errors="replace").splitlines()
             if l.strip() and not l.startswith("{") and not l.startswith("Input capture:")
             and not l.strip().startswith("Checking ") and not l.strip().startswith("Compiling ")]
    errs = [l for l in lines if "error" in l.lower() and "could not compile" not in l]
    for line in errs or lines:
        if line.strip():
            line = re.sub(r"^jr-rust: ", "", line.strip())
            line = re.sub(r"(/[^\s:]+)+\.rs(:\d+:\d+)?:\s*", "", line)
            return line[:160]
    return ""


def native_program(jr, src, expected, work):
    exe = work / (hashlib.sha1(str(src).encode()).hexdigest()[:12])
    code, _, err = run([jr, src, "-o", exe])
    if code != 0:
        return False, "compile: " + first_error(err)
    code, out, err = run([exe], timeout=30)
    if code != 0:
        return False, f"exit {code}"
    if out != expected:
        return False, "wrong output (miscompile)"
    return True, ""


def program_cases(level):
    if level == "m1":
        d = ROOT / "tests/fixtures/full_programs"
        for line in (d / "manifest.tsv").read_text().splitlines():
            if line.startswith("#") or not line.strip():
                continue
            name = line.split("\t")[0]
            yield name, d / f"{name}.rs", d / f"{name}.stdout"
        return
    d = MS / level
    for entry in sorted(d.iterdir()):
        if entry.suffix == ".rs":
            yield entry.stem, entry, d / f"{entry.stem}.stdout"
        elif entry.is_dir() and (entry / "src/main.rs").exists():
            yield entry.name, entry / "src/main.rs", d / f"{entry.name}.stdout"


def eval_programs(level, jr, jobs, work):
    cases = list(program_cases(level))
    with cf.ThreadPoolExecutor(jobs) as pool:
        futs = {pool.submit(native_program, jr, src, out.read_bytes(), work): name
                for name, src, out in cases}
        return [(futs[f], *f.result()) for f in futs]


# ---------------------------------------------------------------- M4 crates

def crate_cases():
    for line in (MS / "m4/crates.txt").read_text().splitlines():
        line = line.split("#")[0].strip()
        if line:
            name, version = line.split()
            yield name, version


def unpack_crate(name, version):
    dest = CACHE / "crates" / f"{name}-{version}"
    if dest.exists():
        return dest
    home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    found = sorted((home / "registry/cache").glob(f"*/{name}-{version}.crate"))
    if not found:
        return None
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(found[0]) as t:
        t.extractall(dest.parent, filter="data")
    return dest


def eval_crate(jr, name, version, work):
    src = unpack_crate(name, version)
    if src is None:
        return f"{name}-{version}", False, "not in cargo cache (cargo fetch it)"
    target = work / f"crate-{name}"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target / "stock"))
    stock = CACHE / "stock" / f"crate-{name}-{version}.json"
    if stock.exists():
        base = json.loads(stock.read_text())
    else:
        c, _, _ = run(["cargo", "test", "--lib", "--offline", "-q"], cwd=src, timeout=600, env=env)
        base = {"test": c == 0}
        stock.parent.mkdir(parents=True, exist_ok=True)
        stock.write_text(json.dumps(base))
    manifest = src / "Cargo.toml"
    c, _, err = run([jr, "check", "--manifest-path", manifest, "--lib"], cwd=src, timeout=600)
    if c != 0:
        return f"{name}-{version}", False, "check: " + first_error(err)
    if not base["test"]:
        return f"{name}-{version}", True, "check only (stock tests unavailable offline)"
    c, _, err = run([jr, "test", "--manifest-path", manifest, "--lib"], cwd=src, timeout=600)
    if c != 0:
        return f"{name}-{version}", False, "test: " + first_error(err)
    return f"{name}-{version}", True, ""


def eval_crates(jr, jobs, work):
    with cf.ThreadPoolExecutor(max(1, jobs // 2)) as pool:
        futs = [pool.submit(eval_crate, jr, n, v, work) for n, v in crate_cases()]
        return [f.result() for f in futs]


# ---------------------------------------------------------------- M5 rustc UI

SKIP = re.compile(r"//@\s*(aux-build|aux-crate|aux-bin|proc-macro|compile-flags|rustc-env|"
                  r"only-|ignore-|needs-|revisions|edition|exec-env|run-flags|"
                  r"known-bug|no-prefer-dynamic|build-aux-docs|check-run-results|"
                  r"run-fail|should-fail|incremental|normalize|dont-check|failure-status)")


def ui_tests(sample):
    ui = CACHE / "rust/tests/ui"
    if not ui.exists():
        subprocess.run(["git", "clone", "--depth", "1", "--branch", RUST_TAG, "--filter=blob:none",
                        "--sparse", "https://github.com/rust-lang/rust.git", str(CACHE / "rust")],
                       check=True)
        subprocess.run(["git", "sparse-checkout", "set", "tests/ui"], cwd=CACHE / "rust", check=True)
    pool = []
    for path in sorted(ui.rglob("*.rs")):
        if "auxiliary" in path.parts:
            continue
        text = path.read_text(errors="replace")
        if SKIP.search(text):
            continue
        pool.append(path)
    # Deterministic, evenly spread sample across every UI directory.
    step = max(1, len(pool) // sample)
    return pool[::step][:sample], len(pool)


def ui_kind(text):
    if "//@ run-pass" in text:
        return "run"
    if "//@ check-pass" in text or "//@ build-pass" in text:
        return "accept"
    return "reject" if "//~" in text else "accept"


def stock_ui(path, work):
    key = hashlib.sha1(path.read_bytes()).hexdigest()
    cached = CACHE / "stock/ui" / f"{key}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    exe = work / f"stock-{key}"
    kind = ui_kind(path.read_text(errors="replace"))
    flags = ["--edition", "2024", "-C", "panic=abort", "--cap-lints", "allow"]
    if kind == "run":
        c, _, _ = run(["rustc", *flags, "-o", exe, path])
        res = {"accept": c == 0}
        if c == 0:
            rc, out, _ = run([exe], timeout=30)
            res.update(exit=rc, stdout=out.decode(errors="replace"))
    else:
        c, _, _ = run(["rustc", *flags, "--emit=metadata", "-o", exe, path])
        res = {"accept": c == 0}
    res["kind"] = kind
    cached.parent.mkdir(parents=True, exist_ok=True)
    cached.write_text(json.dumps(res))
    return res


def eval_ui_one(jr, path, work):
    rel = str(path.relative_to(CACHE / "rust/tests/ui"))
    stock = stock_ui(path, work)
    exe = work / ("jr-" + hashlib.sha1(rel.encode()).hexdigest()[:12])
    if stock["kind"] == "run" and stock["accept"]:
        c, _, err = run([jr, path, "--cap-lints", "allow", "-o", exe])
        if c != 0:
            return rel, False, "compile: " + first_error(err), True
        rc, out, _ = run([exe], timeout=30)
        ok = rc == stock["exit"] and out.decode(errors="replace") == stock["stdout"]
        return rel, ok, "" if ok else "wrong run result (miscompile)", True
    c, _, err = run([jr, path, "--cap-lints", "allow", "--check"])
    if (c == 0) == stock["accept"]:
        return rel, True, "", stock["accept"]
    if stock["accept"]:
        return rel, False, "rejected valid: " + first_error(err), True
    return rel, False, "accepted invalid program", False


def eval_ui(jr, jobs, work, sample):
    tests, eligible = ui_tests(sample)
    with cf.ThreadPoolExecutor(jobs) as pool:
        res = list(pool.map(lambda p: eval_ui_one(jr, p, work), tests))
    return res, eligible


# ---------------------------------------------------------------- report

TITLES = {"m1": "M1 core programs", "m2": "M2 idiomatic std", "m3": "M3 real programs",
          "m4": "M4 crates.io crates", "m5": "M5 rustc UI sample"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="m1,m2,m3,m4,m5")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--ui-sample", type=int, default=500)
    ap.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args()
    jr = jr_binary()
    summary = []
    with tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR")) as tmp:
        work = Path(tmp)
        for level in a.only.split(","):
            extra = ""
            if level in ("m1", "m2", "m3"):
                res = eval_programs(level, jr, a.jobs, work)
            elif level == "m4":
                res = eval_crates(jr, a.jobs, work)
            elif level == "m5":
                res, eligible = eval_ui(jr, a.jobs, work, a.ui_sample)
                extra = f" (sample of {eligible} eligible tests, rust {RUST_TAG})"
            else:
                raise SystemExit(f"unknown milestone {level}")
            passed = sum(1 for r in res if r[1])
            if level == "m5":
                # Rejecting everything would pass every invalid test, so the
                # headline is valid programs accepted (and run correctly).
                valid = [r for r in res if r[3]]
                invalid = [r for r in res if not r[3]]
                vp = sum(1 for r in valid if r[1])
                ip = sum(1 for r in invalid if r[1])
                print(f"== {TITLES[level]}: valid accepted {vp}/{len(valid)} ({100 * vp / max(1, len(valid)):.1f}%), "
                      f"invalid rejected {ip}/{len(invalid)}{extra}")
                res = [r[:3] for r in res]
                summary.append(f"M5 {vp}/{len(valid)} valid")
            else:
                print(f"== {TITLES[level]}: {passed}/{len(res)} ({100 * passed / max(1, len(res)):.1f}%){extra}")
            for name, ok, why in sorted(res):
                if a.verbose or (not ok and level != "m5"):
                    print(f"   {'PASS' if ok else 'FAIL'} {name}{'  ' + why if why else ''}")
            if level == "m5" and not a.verbose:
                reasons = {}
                for _, ok, why in res:
                    if not ok:
                        k = why.split(":")[0]
                        reasons[k] = reasons.get(k, 0) + 1
                for k, n in sorted(reasons.items(), key=lambda x: -x[1]):
                    print(f"   {n:4} {k}")
            if level != "m5":
                summary.append(f"{level.upper()} {passed}/{len(res)}")
    print("SUMMARY " + " | ".join(summary))


if __name__ == "__main__":
    main()
