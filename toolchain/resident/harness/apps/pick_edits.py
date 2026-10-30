#!/usr/bin/env python3
"""pick_edits.py: chooses the ten edits of an application's edit loop and writes them to apps.json.

usage: pick_edits.py <name> [...]

The edits are of three kinds, each the size of what one types between two runs of a program:

text       a word added to a string a function prints or formats
statement  a `let` added after another in a function's body
condition  `x.is_empty()` written as `x.len() == 0`

Candidates are taken from the package's own `src`, spread over its files in a fixed order, and
each is kept only if `cargo check` still passes with it on top of the ones before it, as the
edit loop applies them. Nothing is chosen by what ra-jit does with it.
"""
import json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import apps

WANTED = {"text": 4, "statement": 3, "condition": 3}
TEXT = re.compile(r'^( {4,}.*(?:println|eprintln|format|write|writeln|bail|anyhow|panic)!\(.*)"((?:[A-Za-z][a-z]+ ){2,}[a-z]+)(["{:.,])')
STATEMENT = re.compile(r"^( {8,})let (?:mut )?[a-z_]+ = [^;{}\n]+;\n$")
CONDITION = re.compile(r"(?<![!.\w])([a-z_]+)\.is_empty\(\)")


def candidates(path, text):
    """(kind, old, new, what) for every line of the file an edit could be made at."""
    lines = text.splitlines(keepends=True)
    for number, line in enumerate(lines):
        if line.lstrip().startswith(("//", "#")):
            continue
        once = text.count(line) == 1
        if (found := TEXT.match(line)) and once:
            old = f'"{found[2]}{found[3]}'
            if line.count(old) == 1:
                yield "text", line, line.replace(old, f'"{found[2]} indeed{found[3]}'), "a word in a message"
        elif STATEMENT.match(line) and once:
            indent = STATEMENT.match(line)[1]
            yield "statement", line, f"{line}{indent}let _edited = {number};\n", "a statement added"
        elif (found := CONDITION.search(line)) and once and line.startswith("        "):
            yield "condition", line, line.replace(found[0], f"{found[1]}.len() == 0", 1), "a condition rewritten"


def checks(ws, spec):
    env = {**os.environ, "CARGO_TARGET_DIR": os.path.join(ws, "target-debug"), "RUSTFLAGS": "--cap-lints allow"}
    argv = ["cargo", "check", "-q", "-p", spec["package"], *spec.get("cargo_args", [])]
    return subprocess.run(argv, cwd=ws, env=env, capture_output=True).returncode == 0


def pick(name, spec):
    ws = apps.checkout(name, spec)
    project = apps.package_directory(ws, spec["package"])
    files = sorted(os.path.join(d, f) for d, _, fs in os.walk(os.path.join(project, "src")) for f in fs if f.endswith(".rs"))
    originals = {path: open(path).read() for path in files}
    found = {path: list(candidates(path, text)) for path, text in originals.items()}
    edits, counts, used = [], dict.fromkeys(WANTED, 0), set()
    try:
        # One edit a file while there are files left, then a second round over them.
        for again in (False, True):
            for path in files:
                for kind, old, new, what in found[path]:
                    if counts[kind] >= WANTED[kind] or (path in used) != again or len(edits) >= 10:
                        continue
                    text = open(path).read()
                    if old not in text:
                        continue
                    open(path, "w").write(text.replace(old, new, 1))
                    if not checks(ws, spec):
                        open(path, "w").write(text)
                        continue
                    counts[kind] += 1
                    used.add(path)
                    edits.append({"file": os.path.relpath(path, ws), "old": old, "new": new, "what": what})
                    break
    finally:
        for path, text in originals.items():
            open(path, "w").write(text)
    return edits


def main():
    path = os.path.join(HERE, "apps.json")
    specs = json.load(open(path))
    for name in sys.argv[1:]:
        specs[name]["edits"] = pick(name, specs[name])
        print(f"{name}: {len(specs[name]['edits'])} edits")
        with open(path, "w") as out:
            json.dump(specs, out, indent=1)
            out.write("\n")


main()
