"""appstests.py: the tests stage of apps.py.

`tests` of an application has two kinds of test:

program  arguments of `cargo test` that pick test binaries which start the program
         (`["--test", "integration"]`). Cargo builds them once; each is run with cargo's
         program beside it and then with ra-jit's in its place.
crates   directories of workspace crates whose unit and integration tests are linked again
         with ra-jit's objects for the crate, by cratetest.py.

A test counts by its name and its result in both runs; the ones that differ are listed.
"""
import json, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def results(binary, cwd, env, skip):
    """What each test of a test binary came to."""
    argv = [binary, *(x for name in skip for x in ("--skip", name))]
    try:
        ran = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, errors="replace",
                             stdin=subprocess.DEVNULL, timeout=3600)
    except subprocess.TimeoutExpired:
        return {"(the binary)": "timeout"}
    found = dict(re.findall(r"^test (.*?) \.\.\. (\w+)", ran.stdout, re.M))
    return found or {"(the binary)": f"status {ran.returncode}"}


def program_tests(fx, ws, spec, args, skip):
    """Runs the test binaries twice; returns (tests, differing names)."""
    target = os.path.join(ws, "target-tests")
    env = {**os.environ, "CARGO_TARGET_DIR": target, "RUSTFLAGS": "--cap-lints allow"}
    built = subprocess.run(["cargo", "test", "-p", spec["package"], *args, "--no-run", "--message-format=json"],
                           cwd=ws, env=env, capture_output=True, text=True)
    if built.returncode != 0:
        raise SystemExit(f"cargo test --no-run failed:\n{built.stderr[-2000:]}")
    name = spec.get("bin", spec["package"])
    tests, program = [], os.path.join(target, "debug", name)
    for line in built.stdout.splitlines():
        unit = json.loads(line) if line.startswith("{") else {}
        if unit.get("profile", {}).get("test") and unit.get("executable"):
            tests.append((unit["executable"], os.path.dirname(unit["manifest_path"])))
    kept = program + ".cargo"
    shutil.copy2(program, kept)
    seen = []
    try:
        for stand_in in (kept, fx.binary):
            shutil.copy2(stand_in, program)
            run_env = {**fx.run_env, "CARGO": shutil.which("cargo")}
            seen.append({f"{os.path.basename(exe).rsplit('-', 1)[0]}::{test}": result
                         for exe, cwd in tests
                         for test, result in results(exe, cwd, {**run_env, "CARGO_MANIFEST_DIR": cwd}, skip).items()})
    finally:
        shutil.move(kept, program)
    want, got = seen
    differing = sorted(test for test in want.keys() | got.keys() if want.get(test) != got.get(test))
    for test in differing[:40]:
        print(f"  DIFFERS {test}: with cargo's program {want.get(test)}, with ra-jit's {got.get(test)}")
    return len(want), differing


def crate_tests(ws, directory):
    """cratetest.py on a crate of the workspace; returns (tests, differing names, error)."""
    out = os.path.join(ws, ".apps-cratetest.json")
    if os.path.exists(out):
        os.remove(out)
    ran = subprocess.run([sys.executable, os.path.join(HERE, "cratetest.py"), os.path.join(ws, directory), "--json", out],
                         capture_output=True, text=True)
    last = (ran.stdout.strip().splitlines() or [ran.stderr.strip()[-300:]])[-1]
    print(f"  {directory}: {last}")
    if not os.path.exists(out):
        return 0, [], last[:300]
    merged = json.load(open(out))
    differing = [f"{directory}::{test}" for binary in merged["binaries"] for test in binary.get("differing_tests", [])]
    broken = [f"{directory}: {binary['error']}" for binary in merged["binaries"] if "error" in binary]
    return merged["tests_run"], differing + broken, None


def run(fx, ws, spec, result):
    tests = spec.get("tests")
    if not tests:
        return True
    row = {"run": 0, "differing": [], "errors": []}
    if "program" in tests:
        try:
            count, differing = program_tests(fx, ws, spec, tests["program"], tests.get("skip", []))
            row["run"] += count
            row["differing"] += differing
        except SystemExit as failed:
            row["errors"].append(str(failed)[-600:])
    for directory in tests.get("crates", []):
        count, differing, error = crate_tests(ws, directory)
        row["run"] += count
        row["differing"] += differing
        if error:
            row["errors"].append(f"{directory}: {error}")
    result["tests"] = row
    print(f"tests: {row['run']} run, {len(row['differing'])} differing, {len(row['errors'])} test targets not run")
    for error in row["errors"]:
        print(f"  NOT RUN {error[:300]}")
    return not row["differing"] and not row["errors"]
