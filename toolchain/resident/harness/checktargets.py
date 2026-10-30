#!/usr/bin/env python3
"""checktargets.py: a plain check leaves dev targets alone, as cargo check does (#634).

Workspace and registry dev-dependencies must resolve when tests are explicitly selected.
Errors in tests, examples and benches must fail an all-targets check, but not a plain one;
errors in a library or binary must still fail the plain check. Every verdict is compared
with the pinned nightly's cargo, using the same daemon across edits and selections.
"""
import json, os, shutil, subprocess, tempfile

from check_json import RA_JIT, env

PACKAGE = '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n'


def write(at, path, text):
    path = os.path.join(at, path)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as file:
        file.write(text)


def verdict(at, home, blaze, flags):
    command = ["cargo", *(["blaze"] if blaze else []), "check", "--message-format=json", *flags]
    done = subprocess.run(command, cwd=at, env=env(home), capture_output=True, text=True, timeout=600)
    errors = set()
    for line in done.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") == "compiler-message" and message["message"]["level"] == "error":
            code = message["message"]["code"]
            target = message["target"]
            errors.add((tuple(target["kind"]), target["name"], code and code["code"]))
    return done.returncode, errors, done.stdout + done.stderr


def compare(at, home, what, flags, expected):
    theirs = verdict(at, home, False, flags)
    ours = verdict(at, home, True, flags)
    assert theirs[0] == expected, f"{what}: cargo exited {theirs[0]}, expected {expected}\n{theirs[2]}"
    assert ours[:2] == theirs[:2], f"{what}: blaze {ours[:2]}, cargo {theirs[:2]}\n{ours[2]}"
    print(f"  {what}: status {ours[0]}, errors {sorted(ours[1])}, as cargo's", flush=True)


def main():
    work = tempfile.mkdtemp(prefix="check-targets-", dir="/tmp" if os.name != "nt" else None)
    home = os.path.join(work, "home")
    passed = False
    try:
        at = os.path.join(work, "workspace")
        write(at, "Cargo.toml", '[workspace]\nmembers = ["app", "helper"]\nresolver = "2"\n'
              '[workspace.dependencies]\nhelper = { path = "helper" }\n')
        write(at, "helper/Cargo.toml", PACKAGE.replace('"app"', '"helper"'))
        write(at, "helper/src/lib.rs", "pub struct Grid { pub value: u32 }\npub fn value() -> u32 { 7 }\n")
        write(at, "app/Cargo.toml", PACKAGE + '[dev-dependencies]\nhelper.workspace = true\n')
        write(at, "app/src/lib.rs", "pub fn value() -> u32 { 7 }\n"
              "#[cfg(test)] mod tests { #[test] fn helper_works() { assert_eq!(helper::value(), 7); } }\n")
        write(at, "app/src/main.rs", "fn main() { println!(\"{}\", app::value()); }\n")
        write(at, "app/tests/helper.rs", "use helper::{value, Grid};\n"
              "#[test] fn works() { let g = Grid { value: value() }; assert_eq!(g.value, app::value()); }\n")
        for flags in ([], ["-p", "app"], ["-p", "app", "--tests"], ["--all-targets"]):
            compare(at, home, f"workspace dev-dependency {flags}", flags, 0)
        for path in ("tests/broken.rs", "examples/broken.rs", "benches/broken.rs"):
            write(at, "app/" + path, "fn main() { let _ = missing; }\n")
        for flags in ([], ["-p", "app"], ["-p", "app", "--lib"], ["-p", "app", "--bins"]):
            compare(at, home, f"unselected dev targets {flags}", flags, 0)
        compare(at, home, "selected dev targets have real errors", ["--all-targets"], 101)
        # Each flag picks its own targets of the dev ones (#632): `--tests` checks no example
        # or bench, `--test NAME` the one test.
        for flags, status in ((["--tests"], 101), (["--examples"], 101), (["--benches"], 101),
                              (["--test", "helper"], 0), (["--example", "broken"], 101),
                              (["--bench", "broken"], 101), (["--lib", "--bins"], 0)):
            compare(at, home, f"picked dev targets {flags}", ["-p", "app", *flags], status)
        os.unlink(os.path.join(at, "app", "tests/broken.rs"))
        compare(at, home, "an unpicked example and bench", ["-p", "app", "--tests"], 0)
        for path in ("examples/broken.rs", "benches/broken.rs"):
            os.unlink(os.path.join(at, "app", path))
        # An example may use a dev-dependency; `--tests` does not check it.
        write(at, "app/examples/uses_helper.rs", "fn main() { let _ = helper::Grid { value: 1 }; }\n")
        for flags in (["--tests"], ["--examples"], ["--all-targets"]):
            compare(at, home, f"example with a dev-dependency {flags}", ["-p", "app", *flags], 0)
        # What is under `cfg(test)` in the library is its unit tests', which `--lib` does not check.
        lib = "pub fn value() -> u32 { 7 }\n"
        write(at, "app/src/lib.rs", lib + "#[cfg(test)] mod tests { fn broken() { let _ = helper::value(); let _ = missing; } }\n")
        for flags, status in ((["--lib"], 0), (["--tests"], 101), (["--test", "helper"], 0)):
            compare(at, home, f"unit tests with an error {flags}", ["-p", "app", *flags], status)
        write(at, "app/src/lib.rs", lib)
        # A member that is only depended on is checked as its library, not as its unit tests
        # or its tests, whose dev-dependencies the build does not resolve.
        helper = "pub struct Grid { pub value: u32 }\npub fn value() -> u32 { 7 }\n"
        write(at, "helper/src/lib.rs", helper + "#[cfg(test)] mod tests { fn broken() { let _ = missing; } }\n")
        write(at, "helper/tests/broken.rs", "fn main() { let _ = missing; }\n")
        for flags, status in ((["-p", "app", "--tests"], 0), (["-p", "app", "--all-targets"], 0),
                              (["--workspace", "--tests"], 101)):
            compare(at, home, f"a member depended on {flags}", flags, status)
        write(at, "helper/src/lib.rs", helper)
        os.unlink(os.path.join(at, "helper/tests/broken.rs"))
        # A dev target may share a root file with a normal one: filtering by path alone
        # would also hide the binary's real error below.
        write(at, "app/Cargo.toml", PACKAGE + '[dev-dependencies]\nhelper.workspace = true\n'
              '[[test]]\nname = "shared"\npath = "src/main.rs"\n')
        for path, good in (("src/lib.rs", "pub fn value() -> u32 { 7 }\n"),
                           ("src/main.rs", "fn main() {}\n")):
            write(at, "app/" + path, good + "fn broken() { let _ = missing; }\n")
            compare(at, home, f"default target error in {path}", ["-p", "app"], 101)
            write(at, "app/" + path, good)

        at = os.path.join(work, "registry")
        write(at, "Cargo.toml", PACKAGE + '[dev-dependencies]\ntempfile = "3"\n')
        write(at, "src/main.rs", "fn main() {}\n")
        write(at, "tests/path.rs", "use std::path::Path;\nfn takes_path(_: &Path) {}\n"
              "#[test] fn path() { let dir = tempfile::tempdir().unwrap();\n"
              'let copy = dir.path().join("a.txt"); let _ = copy.to_str().unwrap(); takes_path(&copy); }\n')
        for flags in ([], ["--tests"], ["--all-targets"], []):
            compare(at, home, f"registry dev-dependency {flags}", flags, 0)
        passed = True
    finally:
        subprocess.run([RA_JIT, "stop"], env=env(home), capture_output=True)
        if passed:
            shutil.rmtree(work)
        else:
            print(f"kept {work}", flush=True)
    print("checktargets: all verdicts agree")


if __name__ == "__main__":
    main()
