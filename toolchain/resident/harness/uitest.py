#!/usr/bin/env python3
"""Runs rustc's own run-pass tests through the differential harness.

usage: uitest.py <rust checkout> [--filter <substring>] [--tests list.txt] [--limit N] [--jobs N]
                 [--work dir] [--json out.json] [--rerun out.json] [--fresh]

Every `tests/ui` file marked `//@ run-pass` is accounted for: it is either run, once per
revision, or left out with a reason that is counted. A test that is run becomes a package
whose library is the test and whose binary calls its `main`; its `//@ aux-build`, `aux-crate`
and `proc-macro` files become packages it depends on, and the files it names with `mod`,
`#[path]` and `include!` are copied beside it. rustc builds the package, ra-jit rebuilds the
library, and the two programs have to exit and print the same.

Tests are built a workspace at a time, `--jobs` workspaces at once. The result of a workspace
is kept under `<work>/results`, named by the tests in it, the ra-jit binary and the backend, so
a run that is stopped picks up where it was; `--fresh` forgets them.

The summary lists differences first, then what was left out and why, then why functions were
left to rustc, most common reason first: that list is the worklist.
"""
import argparse, collections, concurrent.futures, glob, hashlib, json, os, re, shlex, shutil, subprocess, sys
import tempfile, time

import difftest
import linkline

LINE = r"^//@ *(?:\[(?P<rev>[\w-]+)\] *)?"
DIRECTIVE = re.compile(LINE + r"(?P<name>[\w-]+)(?: *: *| +|$)(?P<value>.*)$", re.M)
# How many more times a program that differed once is run, to tell a race from a difference.
RERUNS = 6
# How long rustc's program is run over and over to see whether it too does what ra-jit's did.
RACE_SECONDS = 20
MAIN = re.compile(r"^(pub )?fn +main *\(\)(?P<returns> *->)?", re.M)
FEATURE = re.compile(r"#!\[(cfg_attr\(.*, *)?feature\(")
# What a test names beside itself, relative to its own directory.
MODULE = re.compile(r'^\s*(?:#\[path *= *"(?P<path>[^"]+)"\]\s*)?(?:pub(?:\([\w: ]+\))? )?mod (?P<name>\w+);', re.M)
INCLUDE = re.compile(r'\binclude(?:_str|_bytes)?! *\( *"(?P<path>[^"]+)"')
PATH_ATTR = re.compile(r'#\[path *= *"(?P<path>[^"]+)"\]')

# Directives that say nothing about how the program is built or run here.
PLAIN = {
    "run-pass", "check-run-results", "reference", "edition", "aux-build", "aux-crate", "proc-macro",
    "exec-env", "run-flags", "compile-flags", "revisions", "no-prefer-dynamic", "known-bug", "run-rustfix",
    "rustfix-only-machine-applicable", "dont-check-compiler-stderr", "dont-check-compiler-stdout",
    "normalize-stdout", "normalize-stderr", "pretty-expanded", "ignore-backends", "min-llvm-version",
    "needs-unwind", "needs-threads", "needs-subprocess", "needs-asm-support", "needs-dynamic-linking",
    "needs-crate-type", "needs-rust-lld", "needs-deterministic-layouts", "needs-target-std", "needs-relocation-model-pic",
    "needs-symlink", "needs-git-hash", "forbid-output", "error-pattern", "dont-require-annotations", "ignore-pass",
    "ignore-compare-mode-next-solver", "ignore-compare-mode-polonius", "ignore-cross-compile", "ignore-remote",
    "ignore-stage1", "ignore-stage2", "ignore-debug", "ignore-std-debug-assertions", "ignore-rustc-debug-assertions",
    "only-linux", "only-x86_64", "only-unix", "only-64bit", "only-x86_64-unknown-linux-gnu", "only-elf",
    "only-gnu", "only-endian-little", "ignore-endian-big", "filecheck-flags", "doc-flags", "ignore-parallel-frontend",
    "ignore-gdb", "ignore-lldb", "ignore-cdb", "min-gdb-version", "min-lldb-version", "min-cdb-version", "ignore-llvm-version",
}
# `ignore-<these>` leaves the test out on this machine.
HERE = {"linux", "x86_64", "unix", "64bit", "test", "gnu", "elf", "x86_64-unknown-linux-gnu", "endian-little",
        "stable", "pass", "auxiliary"}
# Flags that change nothing the two programs could differ in.
INERT = re.compile(
    r"^(-[AWDF] ?\S*|--cap-lints(=| )\S+|-g|-C ?debuginfo=\S+|-C ?codegen[-_]units=\S+|-C ?split-debuginfo=\S+|-C ?force-unwind-tables(=\S+)?|-C ?strip=\S+|--crate-type(=| )bin"
    r"|-C ?prefer-dynamic=(off|no)|-C ?force-frame-pointers(=\S+)?|--check-cfg(=| )\S+|-C ?opt-level=0|-C ?incremental=\S+"
    r"|-C ?target-feature=-crt-static|-C ?debug[-_]assertions(=(yes|on|y|true))?|-C ?overflow-checks(=(yes|on|y|true))?"
    r"|-C ?lto=(no|off)|--error-format(=| )\S+|--diagnostic-width(=| )\S+|-C ?symbol-mangling-version=v0|-C ?panic=unwind"
    r"|--remap-path-prefix(=| )\S+|-C ?link-dead-code|-l ?rust_test_helpers)$")
# Flags that say how the code is generated, which rustc and ra-jit are both given.
HONOURED = re.compile(r"^(-O|-C ?opt-level=\S+|-C ?debug[-_]assertions(=\S+)?|-C ?overflow-checks(=\S+)?|-C ?panic=abort"
                      r"|-C ?target-(feature|cpu)=\S+)$")
# What an auxiliary crate's flags may ask of its own package's profile.
PACKAGE_SETTING = re.compile(r"^-C ?(?P<key>opt-level|overflow-checks|debug[-_]assertions)=(?P<value>\S+)$")
ON = {"yes": "true", "on": "true", "y": "true", "true": "true", "no": "false", "off": "false", "n": "false", "false": "false"}
CHUNK = 150
STALL = 20


# What the program calls in a test whose `main` is not written where the harness can make it public.
ENTRY = "ra_jit_harness_entry"


class Unit:
    """One test in one revision, and what building and running it takes."""

    def __init__(self, test, path, revision):
        self.test, self.path, self.revision = test, path, revision
        self.edition, self.cfgs, self.env, self.args = "2015", [], {}, []
        self.aux = []
        # Code generation flags, and the environment of the compiler: set, or None for unset.
        self.flags, self.rustc_env = [], {}
        # The rustc issue the test is there to show (`known-bug`), and whether its output is
        # also held against a file (`check-run-results`).
        self.known_bug, self.checked_output = None, False

    @property
    def build(self):
        """What a workspace of tests has to share: how its members are compiled."""
        # A profile setting of one package changes what ra-jit can say of the others.
        alone = self.label if any(aux.settings for aux in self.aux) else None
        return (tuple(self.cfgs), tuple(self.flags), tuple(sorted(self.rustc_env.items(), key=str)), alone)

    @property
    def label(self):
        return f"{self.test}#{self.revision}" if self.revision else self.test


class Aux:
    """A file a test is built against: an ordinary library or a procedural macro."""

    def __init__(self, path, name, proc_macro):
        self.path, self.name, self.proc_macro = path, name, proc_macro
        # What its own flags ask of its package's profile.
        self.settings = {}


def flags_of(text):
    """The flags of a `compile-flags` line, an option and its separate value as one."""
    words, flags = shlex.split(text), []
    while words:
        word = words.pop(0)
        takes = word in ("-C", "-Z", "--cfg", "--check-cfg", "--edition", "--crate-type", "--cap-lints", "-A", "-W", "-D",
                         "-F", "--crate-name", "--extern", "-L", "-l", "--emit", "--target", "--error-format",
                         "--remap-path-prefix", "--diagnostic-width")
        flags.append(f"{word} {words.pop(0)}" if takes and words else word)
    return flags


def refuse(flag):
    """Why a compile flag cannot be honoured, the same wording for the same kind of flag."""
    flag = flag.replace("-C ", "-C").replace("-Z ", "-Z")
    if flag.startswith("-Z"):
        return "compile flag: -Z (nightly only)"
    if flag == "--test":
        return "compile flag: --test (the program is libtest's, not the test's `main`)"
    if flag.startswith("-Cpanic"):
        return f"compile flag: {flag}"
    if flag.startswith(("-Clto", "-Clinker-plugin-lto", "-Cembed-bitcode")):
        return "compile flag: LTO (rustc would inline the test into the program, leaving nothing of ra-jit's to run)"
    return f"compile flag: {flag.split('=')[0].split(' ')[0]}"


def auxiliary(path, value, proc_macro, seen=()):
    """The file a directive names beside `path`, or the reason it cannot be wired up."""
    name, _, file = value.strip().rpartition("=")
    if name.startswith(("noprelude:", "priv:")):
        return f"auxiliary crate with the `{name.split(':')[0]}` modifier"
    found = os.path.join(os.path.dirname(path), "auxiliary", file)
    if not os.path.exists(found):
        return "auxiliary file is not there"
    text = open(found, errors="replace").read()
    if FEATURE.search(text):
        return "auxiliary crate needs a nightly feature"
    named = re.search(r'#!\[crate_name *= *"(\w+)"\]', text)
    aux = Aux(found, name or (named.group(1) if named else os.path.basename(file)[:-3].replace("-", "_")), proc_macro)
    for d in DIRECTIVE.finditer(text):
        for flag in flags_of(d["value"]) if d["name"] == "compile-flags" else []:
            setting = PACKAGE_SETTING.match(flag)
            if setting and (setting["key"] == "opt-level" or setting["value"] in ON):
                aux.settings[setting["key"].replace("_", "-")] = ON.get(setting["value"], setting["value"])
            elif not INERT.match(flag) and not re.match(r"--crate-type[= ](lib|rlib)$", flag):
                return f"auxiliary crate has a compile flag of its own: {flag.split('=')[0].split(' ')[0]}"
        if d["name"] in ("aux-build", "aux-crate", "proc-macro"):
            if found in seen:
                return "auxiliary crates name each other"
            inner = auxiliary(found, d["value"], d["name"] == "proc-macro", (*seen, found))
            if isinstance(inner, str):
                return inner
            aux.needs = [*getattr(aux, "needs", []), inner]
    return aux


def classify(test, path):
    """The units a run-pass test is run as, or the reason it is left out."""
    text = open(path, errors="replace").read()
    directives = list(DIRECTIVE.finditer(text))
    if not any(d["name"] == "run-pass" for d in directives):
        return None
    if FEATURE.search(text):
        return "needs a nightly feature (`#![feature]`)"
    for shape, why in ((r"#!\[no_main\]", "`#![no_main]`"),
                       (r"#!\[crate_type", "`#![crate_type]`"), (r"#!\[no_core\]", "`#![no_core]`")):
        if re.search(shape, text):
            return f"{why}: not a program with a `main` of its own"
    revisions = [r for d in directives if d["name"] == "revisions" for r in d["value"].split()] or [None]
    units = []
    for revision in revisions:
        unit = Unit(test, path, revision)
        if revision:
            unit.cfgs.append(f"--cfg {revision}")
        passes = False
        for d in directives:
            if d["rev"] not in (None, revision):
                continue
            name, value = d["name"], d["value"].strip()
            passes |= name == "run-pass"
            away = name.startswith("only-") and name not in PLAIN or name.startswith("ignore-") and name[7:] in HERE
            if away or name.startswith("needs-") and name not in PLAIN:
                return f"not for stable x86_64-unknown-linux-gnu (`{re.sub(r'-(kcfi|cfi|support|address|leak|memory|thread|hwaddress|shadow-call-stack|dataflow|memtag|safestack)$', '', name)}`)"
            if name == "edition":
                unit.edition = re.match(r"\d+", value).group(0) if re.match(r"\d+", value) else "2015"
            elif name == "known-bug":
                unit.known_bug = value
            elif name == "check-run-results":
                unit.checked_output = True
            elif name == "exec-env":
                key, _, val = value.partition("=")
                unit.env[key] = val
            elif name == "unset-exec-env":
                unit.env[value] = None
            elif name == "rustc-env":
                key, _, val = value.partition("=")
                unit.rustc_env[key.strip()] = val.strip()
            elif name == "unset-rustc-env":
                unit.rustc_env[value] = None
            elif name == "run-flags":
                unit.args += shlex.split(value)
            elif name in ("aux-build", "aux-crate", "proc-macro"):
                aux = auxiliary(path, value, name == "proc-macro")
                if isinstance(aux, str):
                    return aux
                if any(other.name == aux.name and other.path != aux.path for other in unit.aux):
                    return "two auxiliary crates of one name"
                if all(other.path != aux.path for other in unit.aux):
                    unit.aux.append(aux)
            elif name == "compile-flags":
                for flag in flags_of(value):
                    edition = re.match(r"--edition[= ](\d+)$", flag)
                    if edition:
                        unit.edition = edition.group(1)
                    elif re.match(r"--cfg[= ]", flag):
                        unit.cfgs.append("--cfg " + flag[6:])
                    elif HONOURED.match(flag):
                        unit.flags.append(flag)
                    elif not INERT.match(flag):
                        return refuse(flag)
            elif name not in PLAIN and not name.startswith("ignore-"):
                return f"directive `{name}`"
        if passes:
            units.append(unit)
    return units


def environment(changes):
    """The environment a test's program runs in: this one with what the test sets and unsets."""
    env = {**os.environ, **{key: value for key, value in changes.items() if value is not None}}
    for key, value in changes.items():
        if value is None:
            env.pop(key, None)
    return env


def ignored_directories(root):
    """The directories compiletest is told to skip: their files are parts of other tests."""
    return [os.path.dirname(p) + "/" for p in glob.glob(f"{root}/**/compiletest-ignore-dir", recursive=True)]


def select(checkout, needle):
    """Every run-pass test under the checkout: the units to run, and the tests left out by reason."""
    root = f"{checkout}/tests/ui"
    units, excluded = [], []
    ignored = ignored_directories(root)
    for path in sorted(glob.glob(f"{root}/**/*.rs", recursive=True)):
        if needle not in path or "/auxiliary/" in path or any(path.startswith(d) for d in ignored):
            continue
        test = os.path.relpath(path, root)
        found = classify(test, path)
        if isinstance(found, str):
            excluded.append({"test": test, "outcome": "excluded", "reason": found})
        elif found:
            units += found
    return units, excluded


def copy_named(source, dest, root):
    """Copies the files `source` names with `mod`, `#[path]` and `include!` to the same place
    beside `dest`. Returns the reason when one of them cannot be put there."""
    text = open(source, errors="replace").read()
    here, there = os.path.dirname(source), os.path.dirname(dest)
    named = [m["path"] for m in PATH_ATTR.finditer(text)] + [m["path"] for m in INCLUDE.finditer(text)]
    for m in MODULE.finditer(text):
        if m["path"]:
            continue
        # A module of the root file is beside it; one of another file is in the directory named after that file.
        inner = "" if source == root or os.path.basename(source) == "mod.rs" else os.path.basename(source)[:-3]
        found = [p for p in (os.path.join(inner, m["name"] + ".rs"), os.path.join(inner, m["name"], "mod.rs"))
                 if os.path.exists(os.path.join(here, p))]
        if not found:
            return "names a module file that is not there"
        named.append(found[0])
    for relative in named:
        if relative.startswith(("/", "..")) or "/../" in relative:
            return "names a file outside its own directory"
        origin, copy = os.path.join(here, relative), os.path.join(there, relative)
        if not os.path.isfile(origin):
            return "names a file that is not there"
        if os.path.exists(copy):
            continue
        os.makedirs(os.path.dirname(copy), exist_ok=True)
        shutil.copyfile(origin, copy)
        if relative.endswith(".rs"):
            why = copy_named(origin, copy, root)
            if why:
                return why
    return None


def write_aux(ws, owner, aux):
    """Writes the package of an auxiliary crate and returns the line that depends on it."""
    package = f"{owner}_{os.path.basename(aux.path)[:-3].replace('-', '_').replace('.', '_')}"
    directory = f"{ws}/crates/{package}"
    if not os.path.exists(directory):
        os.makedirs(f"{directory}/src")
        text = open(aux.path, errors="replace").read()
        edition = re.search(LINE + r"edition *: *(\d+)", text, re.M)
        needs = "".join(write_aux(ws, owner, inner) for inner in getattr(aux, "needs", []))
        open(f"{directory}/Cargo.toml", "w").write(
            f'[package]\nname = "{package}"\nversion = "0.0.0"\nedition = "{edition.group(2) if edition else "2015"}"\n'
            f'[lib]\nname = "{aux.name}"\npath = "src/lib.rs"\nproc-macro = {str(aux.proc_macro).lower()}\n'
            f"[dependencies]\n{needs}")
        shutil.copyfile(aux.path, f"{directory}/src/lib.rs")
        copy_named(aux.path, f"{directory}/src/lib.rs", aux.path)
        if aux.settings:
            settings = "".join(f"{key} = {value}\n" for key, value in aux.settings.items())
            open(f"{ws}/profiles.toml", "a").write(f"[profile.dev.package.{package}]\n{settings}")
    return f'{aux.name} = {{ path = "../{package}", package = "{package}" }}\n'


def write_package(ws, name, unit):
    """Writes the package of a test. Returns the reason when it cannot be laid out."""
    directory = f"{ws}/crates/{name}"
    os.makedirs(f"{directory}/src")
    text = open(unit.path, errors="replace").read()
    main = MAIN.search(text)
    if main:
        entry, library = "main", MAIN.sub(lambda m: "pub fn main()" + (m["returns"] or ""), text, count=1)
    else:
        # A `main` a macro writes, or one written in a way the pattern does not know: the
        # library calls it, whatever it returns.
        entry, library = ENTRY, f"{text}\npub fn {ENTRY}() -> impl std::process::Termination {{\n    main()\n}}\n"
    wrapper = f"fn main() {{\n    {name}::{entry}()\n}}\n" if main and not main["returns"] \
        else f"fn main() -> impl std::process::Termination {{\n    {name}::{entry}()\n}}\n"
    deps = "".join(write_aux(ws, name, aux) for aux in unit.aux)
    open(f"{directory}/Cargo.toml", "w").write(
        f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "{unit.edition}"\n[dependencies]\n{deps}')
    open(f"{directory}/src/lib.rs", "w").write(library)
    open(f"{directory}/src/main.rs", "w").write(wrapper)
    if copy_named(unit.path, f"{directory}/src/lib.rs", unit.path):
        return beside(unit.path, f"{directory}/src")
    return None


def beside(source, directory):
    """Puts everything that is beside a test beside its copy, for a test whose modules are not
    where the names alone say. Returns the reason when a file of the package is in the way."""
    for entry in os.listdir(os.path.dirname(source)):
        if entry in ("lib.rs", "main.rs"):
            return "has a `lib.rs` or `main.rs` beside it, as its package does"
        if not os.path.lexists(f"{directory}/{entry}"):
            os.symlink(os.path.join(os.path.dirname(os.path.abspath(source)), entry), f"{directory}/{entry}")
    return None


def run_ra_jit(ws, work, built, rlib, symbols, env):
    """Builds the libraries of `built` with one ra-jit. One program can make ra-jit hang or
    exit: the batch is watched for progress, and when it stalls or ends early the program it
    was on is recorded and the rest are started again without it. Returns the programs ra-jit
    hung on and what it said last when it exited."""
    pending, hung, failure = list(built), set(), None
    while pending:
        crates = [x for name in pending for x in ("--crate", name)]
        files = [x for file in [rlib[n] for n in pending] + symbols for x in ("--symbols", file)]
        ra_jit = subprocess.Popen([difftest.RA_JIT, ws, *crates, "--out", work, "--report", work, *files],
                                 stdout=subprocess.DEVNULL, stderr=open(f"{work}/stderr.txt", "w"), env=env)
        done = lambda: [n for n in pending if os.path.exists(f"{work}/{n}.tsv") or os.path.exists(f"{work}/{n}.error")]
        seen, since = 0, time.time()
        while ra_jit.poll() is None:
            time.sleep(0.5)
            if len(done()) != seen:
                seen, since = len(done()), time.time()
            elif time.time() - since > (STALL if seen else 300):
                ra_jit.kill()
                ra_jit.wait()
                break
        finished = set(done())
        rest = [n for n in pending if n not in finished]
        if ra_jit.returncode == 0 or not rest:
            break
        if ra_jit.returncode < 0:
            hung.add(rest[0])
        else:
            failure = open(f"{work}/stderr.txt").read()[-300:]
            open(f"{work}/{rest[0]}.error", "w").write(f"ra-jit exited: {failure}")
        pending = rest[1:]
    return hung, failure


def run_alone(ws, work, name, link, env):
    """Builds the library of one program with a ra-jit of its own. Returns whether ra-jit hung."""
    symbols = [x for a in link if a.endswith(".rlib") for x in ("--symbols", a)]
    try:
        run = subprocess.run([difftest.RA_JIT, ws, "--crate", name, "--out", f"{work}/{name}.o", "--report",
                              f"{work}/{name}.tsv", *symbols], capture_output=True, text=True, timeout=300, env=env)
    except subprocess.TimeoutExpired:
        return True
    if run.returncode != 0:
        open(f"{work}/{name}.error", "w").write(f"ra-jit exited: {run.stderr[-300:]}")
    return False


def races_to(reference, unit, got):
    """Whether rustc's program, run again and again for RACE_SECONDS, once does what `got` is."""
    end = time.monotonic() + RACE_SECONDS
    while time.monotonic() < end:
        if behaviour(reference, unit) == got:
            return True
    return False


def behaviour(binary, unit):
    """What the program does: status, stdout and stderr, run in a directory of its own."""
    cwd = tempfile.mkdtemp(prefix="ra-jit-ui-run-")
    try:
        run = subprocess.run([binary, *unit.args], capture_output=True, timeout=60, cwd=cwd,
                             env=environment(unit.env))
        # The thread id in a panic message differs from run to run, and the program names itself.
        clean = lambda out: re.sub(rb"' \(\d+\) panicked", b"' panicked", out).replace(
            binary.encode(), b"<program>").replace(os.path.dirname(binary).encode(), b"<directory>")
        return run.returncode, clean(run.stdout), clean(run.stderr)
    except subprocess.TimeoutExpired:
        return "timeout", b"", b""
    finally:
        shutil.rmtree(cwd, ignore_errors=True)


def plain(out):
    """Program output with what a build puts into it taken out: where a source file is, the
    place in it, and the id of a thread."""
    out = re.sub(r"[^\s'\"]*\.rs(:\d+:\d+|:LL:CC)", "<source>", out.replace("\r\n", "\n"))
    return out.replace(" ($TID)", "")


def held(unit, got):
    """Whether what the program printed is what the files beside a `check-run-results` test
    say, a missing file saying nothing is printed. The test's own `normalize-` rules are not
    applied, so a test that needs them is seen to differ."""
    stem = unit.path[:-3] + (f".{unit.revision}" if unit.revision else "")
    for kind, text in (("stdout", got[1]), ("stderr", got[2])):
        file = f"{stem}.run.{kind}"
        want = open(file).read() if os.path.exists(file) else ""
        if plain(want) != plain(text.decode(errors="replace")):
            return False
    return True


def relinked(link, binary, unit):
    """What the program does when linked again from rustc's objects with nothing of ra-jit's: a
    difference here is one of the link line and not of ra-jit's code."""
    link = list(link)
    link[link.index("-o") + 1] = binary
    if subprocess.run(link, capture_output=True).returncode != 0:
        return None
    return behaviour(binary, unit)


def native_helpers(checkout, base):
    """Builds `rust_test_helpers`, the C library the tests of foreign calls link, as compiletest
    does. Returns the directory it is in."""
    source = f"{checkout}/tests/auxiliary/rust_test_helpers.c"
    archive = f"{base}/native/librust_test_helpers.a"
    if os.path.exists(source) and not os.path.exists(archive):
        os.makedirs(f"{base}/native", exist_ok=True)
        subprocess.run(["cc", "-c", "-fPIC", "-o", f"{base}/native/rust_test_helpers.o", source], check=True)
        subprocess.run(["ar", "crs", archive, f"{base}/native/rust_test_helpers.o"], check=True)
    return f"{base}/native"


def errors_of(output):
    """The first error cargo printed for each package it could not compile."""
    errors, last = {}, ""
    for line in output.splitlines():
        failed = re.match(r"error: could not compile `(\w+)`", line)
        if failed:
            errors.setdefault(failed.group(1), last)
            last = ""
        elif re.match(r"error(\[E\d+\])?: ", line) and not last:
            last = line
    return errors


def count_functions(result, report):
    """Adds what the report says of each function: from ra-jit, or left to rustc and why."""
    rows = [line.rstrip("\n").split("\t") for line in open(report)]
    for row in rows:
        if row[0] == "skipped" and "no rustc symbol for function" not in row[2]:
            key = difftest.bucket(row[2])
            result["skipped"][key] = result["skipped"].get(key, 0) + 1
            full = re.sub(r"Id\(\w+\)", "Id", row[2])[:200]
            result["reasons"][full] = result["reasons"].get(full, 0) + 1
    result["compiled"] = sum(r[0] == "compiled" for r in rows)
    result["roots"] = result["compiled"] + sum(result["skipped"].values())


def run_chunk(key, units, base):
    """Builds a workspace of tests with one cargo run and one ra-jit run, then links, runs and
    compares each. Loading std into ra-jit is most of what a test costs, so it is shared."""
    ws = f"{base}/ws-{key}"
    shutil.rmtree(ws, ignore_errors=True)
    names = {f"t{key}_{i}": unit for i, unit in enumerate(units)}
    results = {name: {"test": unit.test, "revision": unit.revision, "outcome": "excluded", "skipped": {},
                      "known_bug": unit.known_bug, "checked_output": unit.checked_output,
                      "reasons": {}, "compiled": 0, "roots": 0} for name, unit in names.items()}
    for name, unit in names.items():
        why = write_package(ws, name, unit)
        if why:
            results[name]["reason"] = why
            shutil.rmtree(f"{ws}/crates/{name}")
    profiles = open(f"{ws}/profiles.toml").read() if os.path.exists(f"{ws}/profiles.toml") else ""
    open(f"{ws}/Cargo.toml", "w").write(
        f'[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n[profile.dev]\ndebug = false\n{profiles}')
    flags = " ".join(["--print link-args -C save-temps -C link-dead-code --cap-lints allow",
                      f"-L native={base}/native", *units[0].cfgs, *units[0].flags])
    env = {**os.environ, "RUSTFLAGS": flags}
    for key, value in units[0].rustc_env.items():
        env.pop(key, None)
        if value is not None:
            env[key] = value
    out = subprocess.run(["cargo", "build", "--workspace", "--keep-going"], cwd=ws, capture_output=True, text=True, env=env)
    links = {}
    for line in (out.stdout + out.stderr).splitlines():
        if '"cc"' not in line:
            continue
        args = shlex.split(line)
        args = [a for a in args[args.index("cc"):] if not a.endswith("symbols.o")]
        binary = os.path.basename(args[args.index("-o") + 1]).rsplit("-", 1)[0]
        if binary in names:
            links[binary] = args
    built = sorted(links)
    errors = errors_of(out.stderr)
    for name in names:
        if name not in links and "reason" not in results[name]:
            results[name]["reason"] = "rustc does not build it as a package"
            results[name]["detail"] = errors.get(name, "a crate it depends on does not build")[:200]
    if not built:
        shutil.rmtree(ws, ignore_errors=True)
        return list(results.values())
    work = f"{ws}/ra-jit"
    os.makedirs(work)
    rlib = {name: next(a for a in links[name] if os.path.basename(a).startswith(f"lib{name}-")) for name in built}
    # A test built against other crates is given to a ra-jit of its own, with the symbols of what
    # it is linked with: in a batch every program would be shown the libraries of all of them.
    alone = [name for name in built if names[name].aux]
    batch = [name for name in built if name not in alone]
    hung, failure = run_ra_jit(ws, work, batch, rlib, difftest.sysroot_rlibs(), env) if batch else (set(), None)
    with concurrent.futures.ThreadPoolExecutor(2) as pool:
        hung |= {name for name, stuck in zip(alone, pool.map(lambda n: run_alone(ws, work, n, links[n], env), alone)) if stuck}

    def finish(name):
        result, unit, args = results[name], names[name], list(links[name])
        result["outcome"] = "match"
        if name in hung:
            result.update(outcome="hang", detail="ra-jit made no progress")
            return
        report, obj = f"{work}/{name}.tsv", f"{work}/{name}.o"
        if os.path.exists(f"{work}/{name}.error") or not os.path.exists(report):
            detail = open(f"{work}/{name}.error").read() if os.path.exists(f"{work}/{name}.error") \
                else f"batch ended early: {failure}"
            result.update(outcome="crash", detail=detail.strip()[:300])
            return
        count_functions(result, report)
        reference = args[args.index("-o") + 1]
        args = linkline.ahead(args, [obj], difftest.RA_JIT)
        binary = f"{work}/{name}"
        args[args.index("-o") + 1] = binary
        link = subprocess.run(args, capture_output=True, text=True)
        if link.returncode != 0:
            result.update(outcome="link", detail=" ".join(link.stderr.strip().splitlines()[-4:])[-300:])
            return
        expected = behaviour(reference, unit)
        got = behaviour(binary, unit)
        if unit.checked_output:
            result["held"] = held(unit, got)
        # A test that fails as rustc built it does not survive being wrapped in a package, and one
        # that prints something else each time (an address, say) has no behaviour to compare.
        if expected[0] != 0:
            result.update(outcome="excluded", reason="fails as rustc builds it in a package")
        elif behaviour(reference, unit) != expected:
            result.update(outcome="excluded", reason="prints something else in every run")
        elif got == expected:
            pass
        # A rare difference between runs (the order of a hash map) takes a few more to show.
        elif any(behaviour(reference, unit) != expected for _ in range(6)):
            result.update(outcome="excluded", reason="prints something else in every run")
        elif relinked(links[name], f"{work}/{name}.control", unit) != expected:
            result.update(outcome="excluded", reason="differs when relinked from rustc's objects alone "
                          "(the link line the harness keeps lacks rustc's `symbols.o`)")
        else:
            detail = difftest.symptom(expected, got)
            # Said and counted on its own, never passed over: a difference that one run showed
            # and the next runs do not is of a program that races, under rustc as well or under
            # ra-jit alone.
            again = sum(behaviour(binary, unit) == expected for _ in range(RERUNS))
            if races_to(reference, unit, got):
                result.update(outcome="excluded", reason="races: rustc's own program is seen to do what ra-jit's did")
            elif again == RERUNS:
                result.update(outcome="unsteady", detail=f"{detail} in one run, and in none of {RERUNS} more")
            else:
                result.update(outcome="mismatch", detail=detail, stderr=got[2].decode(errors="replace")[-300:])

    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        list(pool.map(finish, built))
    failed = [name for name, r in results.items() if r["outcome"] not in ("match", "excluded")]
    for name in failed:
        result = results[name]
        result["package"] = f"{ws}/crates/{name}"
        print(f"{result['outcome']} {names[name].label}: {result.get('detail', '')}", file=sys.stderr, flush=True)
    # What cargo built is most of the disk a run takes; the packages of what failed stay.
    shutil.rmtree(f"{ws}/target", ignore_errors=True)
    if not failed:
        shutil.rmtree(ws, ignore_errors=True)
    return list(results.values())


def chunks_of(units):
    """Workspaces of tests that are built with the same flags and compiler environment, which
    cargo takes for all of one."""
    by_cfg = collections.defaultdict(list)
    for unit in units:
        by_cfg[unit.build].append(unit)
    return [group[i:i + CHUNK] for group in by_cfg.values() for i in range(0, len(group), CHUNK)]


def identity():
    """What a kept result is a result of: this ra-jit and this backend."""
    stat = os.stat(difftest.RA_JIT)
    backend = [os.environ.get(k, "") for k in ("RA_JIT_BACKEND", "RA_JIT_OPT", "RA_JIT_EMIT")]
    return f"{os.path.abspath(difftest.RA_JIT)} {stat.st_size} {stat.st_mtime_ns} {backend}"


def run_kept(chunk, base, stamp):
    """The result of a workspace, from an earlier run when there is one."""
    key = hashlib.sha256("\n".join([stamp, *(u.label for u in chunk)]).encode()).hexdigest()[:12]
    kept = f"{base}/results/{key}.json"
    if os.path.exists(kept):
        return json.load(open(kept))
    results = run_chunk(key, chunk, base)
    json.dump(results, open(kept + ".part", "w"))
    os.replace(kept + ".part", kept)
    return results


def summarize(results):
    outcomes = collections.Counter(r["outcome"] for r in results)
    ran = [r for r in results if r["outcome"] != "excluded"]
    full = sum(r["outcome"] == "match" and not r["skipped"] for r in ran)
    print(f"{len(ran)} tests: {outcomes['match']} match ({full} with every function from ra-jit), "
          f"{outcomes['mismatch']} mismatch, {outcomes['unsteady']} differing in one run only, "
          f"{outcomes['crash']} ra-jit crashes, {outcomes['hang']} ra-jit hangs, "
          f"{outcomes['link']} link errors; {outcomes['excluded']} left out")
    pinned = sum(r["outcome"] == "mismatch" and bool(r.get("known_bug")) for r in ran)
    print(f"of the mismatches, {pinned} are of tests that are there to show a bug of rustc (`known-bug`)")
    checked = [r for r in ran if r.get("checked_output")]
    apart = [r["test"] for r in checked if not r.get("held")]
    print(f"{len(checked)} of the tests run have `check-run-results`: ra-jit's program prints what "
          f"the test's files say in {len(checked) - len(apart)}, not in {len(apart)} {sorted(set(apart))}")
    compiled, roots = sum(r["compiled"] for r in ran), sum(r["roots"] for r in ran)
    print(f"functions: {compiled} of {roots} from ra-jit, {roots - compiled} left to rustc")
    for outcome in ("mismatch", "unsteady", "crash", "hang", "link"):
        for r in ran:
            if r["outcome"] == outcome:
                revision = f"#{r['revision']}" if r.get("revision") else ""
                pins = f" (known-bug {r['known_bug']})" if r.get("known_bug") else ""
                print(f"  {outcome.upper()} {r['test']}{revision}{pins}: {r.get('detail', '')}")
    left = collections.Counter(r["reason"] for r in results if r["outcome"] == "excluded")
    print("left out by reason:")
    for reason, n in left.most_common():
        print(f"  {n:4}  {reason}")
    reasons, example = collections.Counter(), {}
    for r in ran:
        for reason, n in r["skipped"].items():
            reasons[reason] += n
            example.setdefault(reason, r["test"])
    print("functions left to rustc by reason:")
    for reason, n in reasons.most_common(60):
        print(f"  {n:4}  {reason}  (e.g. {example[reason]})")
    return outcomes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checkout")
    ap.add_argument("--filter", default="")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2), help="workspaces at a time")
    ap.add_argument("--rerun", help="only the tests that failed in this earlier --json output")
    ap.add_argument("--tests", help="only the tests this file names, one per line (as ui-pinned.txt)")
    ap.add_argument("--work", default=os.path.expanduser("~/ra-jit-ui"), help="where the workspaces and results go")
    ap.add_argument("--fresh", action="store_true", help="forget the results kept under --work")
    ap.add_argument("--json")
    a = ap.parse_args()
    a.work = os.path.abspath(a.work)

    units, excluded = select(os.path.abspath(a.checkout), a.filter)
    if a.rerun:
        again = {r["test"] for r in json.load(open(a.rerun)) if r["outcome"] not in ("match", "excluded")}
        units, excluded = [u for u in units if u.test in again], []
    if a.tests:
        named = {line.split("#")[0].strip() for line in open(a.tests)} - {""}
        missing = named - {u.test for u in units} - {r["test"] for r in excluded}
        if missing:
            sys.exit(f"{a.tests} names tests that are not in the checkout: {sorted(missing)}")
        units, excluded = [u for u in units if u.test in named], [r for r in excluded if r["test"] in named]
    units = units[:a.limit]
    if a.fresh:
        shutil.rmtree(f"{a.work}/results", ignore_errors=True)
    os.makedirs(f"{a.work}/results", exist_ok=True)
    native_helpers(os.path.abspath(a.checkout), a.work)
    stamp = identity()
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        results = [r for chunk in pool.map(lambda c: run_kept(c, a.work, stamp), chunks_of(units)) for r in chunk]
    results += excluded
    outcomes = summarize(results)
    if a.json:
        json.dump(results, open(a.json, "w"), indent=1)
    sys.exit(1 if outcomes["mismatch"] or outcomes["unsteady"] or outcomes["crash"] or outcomes["hang"] or outcomes["link"] else 0)


if __name__ == "__main__":
    main()
