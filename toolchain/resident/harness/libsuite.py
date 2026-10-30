#!/usr/bin/env python3
"""The standard library's own test suites, as far as they build on stable, through cratetest.py.

usage: libsuite.py <rust checkout> <coretests|alloctests|stdtests> [--work dir] [--prepare-only]
                   [-- <cratetest.py arguments>]

`coretests`, `alloctests` and the tests under `library/std/tests` are written against the
library's unstable interface: each starts with a list of `#![feature]`s, which a stable
compiler turns down. So a suite is first cut down to what stable builds. The feature lines are
taken out, and then, until `cargo check` has nothing left to say, every item an error points
into is deleted: the test function when the error is in one, otherwise the `use`, helper or
macro it is in, whose users go in a later round. What was deleted is counted by the error
that deleted it and written to `<work>/<suite>/pruned.json`. Nothing is rewritten: a test either
runs as the Rust project wrote it or is not there.

The package that is left is then given to cratetest.py, which builds each test binary with
rustc and again with ra-jit's objects in front, runs both and compares every test by name.
"""
import argparse, collections, json, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
FEATURE = re.compile(r"^#!\[(?:cfg_attr\([^\n]*?, *)?feature\((?:[^()]|\([^()]*\))*\)\)?\]\n", re.M | re.S)
HEADER = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))? )?(?:(?:const|async|unsafe|default|extern(?: \"[\w-]+\")?) )*"
    r"(?:fn|mod|impl|struct|enum|union|trait|use|static|const|type|extern crate)\b|^\s*macro_rules!|^\s*[\w:]+! *[({\[]")
ATTRIBUTE = re.compile(r"^\s*(#\[|///|//)")
TEST = re.compile(r"^\s*#\[test\]", re.M)
# What does not count when brackets are matched: comments, strings and character literals.
QUIET = re.compile(r'//.*|r#".*?"#|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')


class Suite:
    """Where a suite's sources are in the checkout and which of its files are test binaries."""

    def __init__(self, source, tests, targets, dependencies):
        self.source, self.tests, self.targets, self.dependencies = source, tests, targets, dependencies


SUITES = {
    "coretests": Suite("library/coretests", "tests", {"lib": "tests/lib.rs"},
                       'rand = { version = "0.9.0", default-features = false }\n'
                       'rand_xorshift = { version = "0.4.0", default-features = false }\n'),
    "alloctests": Suite("library/alloctests", "tests",
                        {"lib": "tests/lib.rs", "c_str_alloc_error": "tests/c_str_alloc_error.rs",
                         "vec_deque_alloc_error": "tests/vec_deque_alloc_error.rs"},
                        'rand = { version = "0.9.0", default-features = false, features = ["alloc"] }\n'
                        'rand_xorshift = "0.4.0"\n'),
    "stdtests": Suite("library/std", "tests", None,
                      'rand = { version = "0.9.0", default-features = false, features = ["alloc"] }\n'
                      'rand_xorshift = "0.4.0"\n'),
}


def lay_out(checkout, name, directory):
    """Copies the suite into a package of its own with an empty library."""
    suite = SUITES[name]
    shutil.rmtree(directory, ignore_errors=True)
    source = os.path.join(checkout, suite.source)
    shutil.copytree(os.path.join(source, suite.tests), os.path.join(directory, "tests"))
    for extra in ("testing",):
        if os.path.isdir(os.path.join(source, extra)):
            shutil.copytree(os.path.join(source, extra), os.path.join(directory, extra))
    os.makedirs(os.path.join(directory, "src"))
    open(os.path.join(directory, "src/lib.rs"), "w").write("")
    targets = suite.targets
    if targets is None:
        # Every file of the directory is a test binary, and so is each directory with a `lib.rs`.
        tests = os.path.join(directory, "tests")
        targets = {f[:-3].replace("-", "_"): f"tests/{f}" for f in sorted(os.listdir(tests)) if f.endswith(".rs")}
        targets.update({d: f"tests/{d}/lib.rs" for d in sorted(os.listdir(tests))
                        if os.path.exists(os.path.join(tests, d, "lib.rs"))})
    manifest = f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2024"\nautotests = false\n'
    manifest += "".join(f'[[test]]\nname = "{target}"\npath = "{path}"\n' for target, path in targets.items())
    manifest += f"[dev-dependencies]\n{suite.dependencies}[workspace]\n"
    open(os.path.join(directory, "Cargo.toml"), "w").write(manifest)
    return targets


def sources(directory):
    for base, _, files in os.walk(directory):
        if "/target" in base:
            continue
        for file in files:
            if file.endswith(".rs"):
                yield os.path.join(base, file)


def count_tests(directory):
    return sum(len(TEST.findall(open(path, errors="replace").read())) for path in sources(directory))


def indent(line):
    return len(line) - len(line.lstrip())


FUNCTION = re.compile(r"^\s*(?:pub(?:\([^)]*\))? )?(?:(?:const|async|unsafe|default|extern(?: \"[\w-]+\")?) )*fn\b")


def extent(lines, start):
    """The last line of the item whose header is on line `start`, or None when it does not end."""
    depth = 0
    for i in range(start, len(lines)):
        quiet = QUIET.sub("", lines[i])
        depth += sum(quiet.count(c) for c in "({[") - sum(quiet.count(c) for c in ")}]")
        if depth <= 0 and quiet.rstrip().endswith((";", "}", "},")):
            return i
    return None


def holders(lines, at):
    """The items that hold line `at`, innermost first, each as its first and last line."""
    # A line that closes or continues a header is as far in as the header is.
    inside = indent(lines[at]) + lines[at].lstrip().startswith((")", "}", "]", "{", "where"))
    for start in range(at, -1, -1):
        if not HEADER.match(lines[start]):
            continue
        if start != at and indent(lines[start]) >= inside and lines[at].strip():
            continue
        end = extent(lines, start)
        if end is not None and end >= at:
            yield start, end


def item_around(lines, at):
    """The first and last line of the item that holds line `at`, with the attributes and
    comments written over it: the innermost function when it is in one, since a function with a
    statement missing is another function, and otherwise the innermost item. None when no item
    holds it."""
    while at + 1 < len(lines) and lines[at].lstrip().startswith("#["):
        at += 1
    found = list(holders(lines, at))
    if not found:
        return None
    start, end = next(((s, e) for s, e in found if FUNCTION.match(lines[s])), found[0])
    while start > 0 and ATTRIBUTE.match(lines[start - 1]) and indent(lines[start - 1]) == indent(lines[start]):
        start -= 1
    return start, end


def place_of(message, directory):
    """The file and line of the package an error is about: where it is written, or for code
    from a macro of another crate, where the macro is called."""
    for span in message.get("spans", []):
        if not span.get("is_primary"):
            continue
        while span:
            path = os.path.join(directory, span["file_name"])
            if os.path.exists(path) and not os.path.isabs(span["file_name"]):
                return path, span["line_start"] - 1
            span = (span.get("expansion") or {}).get("span")
    for span in message.get("spans", []):
        if not os.path.isabs(span["file_name"]) and os.path.exists(os.path.join(directory, span["file_name"])):
            return os.path.join(directory, span["file_name"]), span["line_start"] - 1
    return None


def errors_of(directory):
    """The errors of one `cargo check` of the tests, each with its code and place."""
    run = subprocess.run(["cargo", "check", "--tests", "--keep-going", "--message-format=json"], cwd=directory,
                         capture_output=True, text=True)
    found = []
    for line in run.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line).get("message") or {}
        if message.get("level") != "error" or "aborting due to" in message.get("message", ""):
            continue
        place = place_of(message, directory)
        found.append(((message.get("code") or {}).get("code") or "syntax", place, message["message"]))
    return found, run.returncode


def prune(directory):
    """Deletes what stable does not build until the tests build. Returns what was deleted, by
    the code of the error that deleted it."""
    for path in sources(directory):
        text = open(path, errors="replace").read()
        if FEATURE.search(text):
            open(path, "w").write(FEATURE.sub("", text))
    deleted, rounds = collections.Counter(), 0
    while True:
        errors, status = errors_of(directory)
        if status == 0:
            return deleted, rounds
        rounds += 1
        by_file = collections.defaultdict(dict)
        for code, place, text in errors:
            if place is None:
                raise SystemExit(f"an error with no place in the package: {text}")
            by_file[place[0]].setdefault(place[1], code)
        if not by_file:
            raise SystemExit("cargo check fails without an error to act on")
        for path, at in by_file.items():
            lines = open(path, errors="replace").read().split("\n")
            spans = {}
            for line, code in at.items():
                item = item_around(lines, min(line, len(lines) - 1))
                if item is None and not lines[line].lstrip().startswith("#!["):
                    raise SystemExit(f"no item around {path}:{line + 1}: {lines[line]}")
                # An inner attribute is in no item and goes alone.
                spans.setdefault(item or (line, line), code)
            for (start, end), code in sorted(spans.items(), reverse=True):
                if any(s <= start and end <= e and (s, e) != (start, end) for s, e in spans):
                    continue
                deleted[code] += 1
                del lines[start:end + 1]
            open(path, "w").write("\n".join(lines))


def binary_of(directory, target):
    """The test binary cargo builds for one target of the package."""
    run = subprocess.run(["cargo", "test", "--no-run", "--test", target, "--message-format=json"], cwd=directory,
                         capture_output=True, text=True)
    built = [json.loads(line) for line in run.stdout.splitlines() if line.startswith("{")]
    return next(it["executable"] for it in built if it.get("executable") and it["target"]["name"] == target)


def unfinished(directory, target, limit):
    """The tests rustc's own binary does not finish within `limit` seconds each. The suites are
    written to run optimised; cargo's test profile is not, and a loop over `usize::MAX` unit
    values that an optimiser deletes then runs for years."""
    binary, slow = binary_of(directory, target), []
    while True:
        skips = [x for name in slow for x in ("--skip", name)]
        try:
            subprocess.run([binary, "--exact", *skips], cwd=directory, capture_output=True, timeout=limit + 60)
            return slow
        except subprocess.TimeoutExpired as stopped:
            output = (stopped.stdout or b"").decode(errors="replace")
        started = re.findall(r"^test (\S+) has been running for over", output, re.M)
        found = [name for name in started if not re.search(rf"^test {re.escape(name)} \.\.\. ", output, re.M)]
        if not found:
            raise SystemExit(f"{target} does not finish, and no test of it says it is slow")
        slow += found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checkout")
    ap.add_argument("suite", choices=sorted(SUITES))
    ap.add_argument("--work", default=os.path.expanduser("~/ra-jit-lib"))
    ap.add_argument("--prepare-only", action="store_true")
    ap.add_argument("--limit", type=int, default=60, help="seconds a test may run in rustc's own binary")
    own = sys.argv[1:sys.argv.index("--")] if "--" in sys.argv else sys.argv[1:]
    rest = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    a = ap.parse_args(own)
    directory = os.path.join(os.path.abspath(a.work), a.suite)
    record = os.path.join(directory, "pruned.json")
    if not os.path.exists(record):
        targets = lay_out(os.path.abspath(a.checkout), a.suite, directory)
        written = count_tests(directory)
        deleted, rounds = prune(directory)
        slow = {target: unfinished(directory, target, a.limit) for target in sorted(targets)}
        json.dump({"targets": sorted(targets), "tests_written": written, "tests_left": count_tests(directory),
                   "rounds": rounds, "items_deleted": dict(deleted.most_common()),
                   "unfinished": {target: names for target, names in slow.items() if names}},
                  open(record, "w"), indent=1)
    kept = json.load(open(record))
    print(f"{a.suite}: {kept['tests_left']} of {kept['tests_written']} `#[test]` functions build on stable, "
          f"after {kept['rounds']} rounds; items deleted by error: {kept['items_deleted']}", flush=True)
    skips = sorted({name for names in kept["unfinished"].values() for name in names})
    print(f"left out, not finished by rustc's own unoptimised binary in {a.limit}s: {len(skips)} {skips}", flush=True)
    if a.prepare_only:
        return
    tests = [x for target in kept["targets"] for x in ("--test", target)]
    sys.exit(subprocess.run([sys.executable, os.path.join(HERE, "cratetest.py"), directory, "--crate", a.suite,
                             *tests, *(x for name in skips for x in ("--skip", name)), *rest]).returncode)


if __name__ == "__main__":
    main()
