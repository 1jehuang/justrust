#!/usr/bin/env python3
"""How much of a crate rust-analyzer redoes after different kinds of edit.

usage: invalidation.py [crate[,crate...]] [--only TEXT] [--perf DIR] [--check] [--cargo]

`--only` keeps the scenarios whose name contains TEXT, plus one per entry of BODIES. `--perf`
records the daemon with `perf` during each edit, into DIR/<n>.data. `--check` compares the object
the daemon wrote after each edit with the one a new ra-jit process writes from the same source.
`--cargo` has the daemon link the program after each build, times `cargo build` on the same edit
in a target directory of its own, and compares what the two programs print.

Starts the daemon on bench2 for one crate (default regex_automata) with RA_JIT_STATS set, applies
each scenario, and records the time to apply the change, the time spent in MIR queries, the
total rebuild time, where it went, and how many functions were compiled again. Writes INVALIDATION.md.
"""
import os, re, shlex, signal, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
os.environ["RA_JIT_STATS"] = "1"
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs
from editbench import millis

V = os.path.join(HERE, "vendor")
A, S = os.path.join(V, "regex-automata/src"), os.path.join(V, "regex-syntax/src")
B = os.path.join(HERE, "bench2/src")
BODY = "let _ra_jit_edit = 1u32;"
# Edited files are read and written byte for byte: one of them is not text.
RAW = {"encoding": "latin-1", "newline": ""}


def insert_after(path, line, text):
    """Inserts `text` after 1-based `line`."""
    def apply(lines):
        lines.insert(line, text + "\n")
    return path, apply


def top(path):
    def apply(lines):
        lines.insert(0, "\n")
    return path, apply


def substitute(path, old, new):
    def apply(lines):
        text = "".join(lines)
        assert old in text, old
        lines[:] = text.replace(old, new).splitlines(keepends=True)
    return path, apply


BODIES = [(f"{A}/util/look.rs", 291), (f"{A}/util/sparse_set.rs", 151), (f"{A}/nfa/thompson/builder.rs", 149),
          (f"{A}/meta/strategy.rs", 383), (f"{A}/nfa/thompson/pikevm.rs", 165)]
NEW_FN = "\n/// Added by the harness.\npub fn ra_jit_new_item(x: u32) -> u32 { x + 1 }\n"
SCENARIOS = {
    "comment appended to one file": [insert_after(f"{A}/util/look.rs", 10**9, "// edit")],
    "blank line at top of one file": [top(f"{A}/nfa/thompson/builder.rs")],
    "blank line at top of 5 files": [top(p) for p, _ in BODIES],
    "1 function body": [insert_after(*BODIES[0], BODY)],
    "5 function bodies in 5 files": [insert_after(p, l, BODY) for p, l in BODIES],
    "new private fn added": [insert_after(f"{A}/util/look.rs", 10**9, NEW_FN.replace("pub ", ""))],
    "signature of a private method": [
        substitute(f"{A}/nfa/thompson/builder.rs", "    fn memory_usage(&self) -> usize {\n        match *self {\n            State::Empty",
                   "    fn memory_usage(&self, _extra: ()) -> usize {\n        match *self {\n            State::Empty"),
        substitute(f"{A}/nfa/thompson/builder.rs", "state.memory_usage()", "state.memory_usage(())")],
    "upstream crate: 1 function body": [insert_after(f"{S}/hir/mod.rs", 1008, BODY)],
    "upstream crate: blank line at top": [top(f"{S}/hir/mod.rs")],
    "upstream crate: new pub fn": [insert_after(f"{S}/hir/mod.rs", 10**9, NEW_FN)],
    "both crates: 3 bodies each + new pub fn upstream": [
        *[insert_after(p, l, BODY) for p, l in BODIES[:3]],
        insert_after(f"{S}/ast/mod.rs", 36, BODY), insert_after(f"{S}/ast/mod.rs", 10**9, NEW_FN),
        insert_after(f"{S}/hir/mod.rs", 1008, BODY), insert_after(f"{S}/hir/literal.rs", 10**9, "// edit")],
    "upstream crate: what the program prints": [substitute(f"{S}/hir/print.rs", 'r"(?:"', 'r"(?~"')],
    "upstream crate: new pub fn, called from the binary": [
        insert_after(f"{S}/hir/mod.rs", 10**9, NEW_FN),
        substitute(f"{B}/main.rs", "    let re = Regex::new(",
                   '    println!("{}", regex_syntax::hir::ra_jit_new_item(41));\n    let re = Regex::new(')],
    "generic fn with a closure removed, its one call written out": [
        substitute(f"{B}/main.rs", "        show(m.range(), &hay[m.range()]);\n",
                   '        println!("{:?} {}", m.range(), &hay[m.range()]);\n'),
        substitute(f"{B}/main.rs", "\nfn show<T: std::fmt::Debug>(value: T, text: &str) {\n"
                   "    let line = |value: &T| format!(\"{value:?} {text}\");\n"
                   "    println!(\"{}\", line(&value));\n}\n", "")],
    "closure body": [substitute(f"{B}/main.rs", "map(|n| n * 2)", "map(|n| n * 3)")],
    "const value": [substitute(f"{B}/main.rs", "const LIMIT: usize = 3;", "const LIMIT: usize = 1;")],
    "static initializer": [substitute(f"{B}/main.rs", 'static GREETING: &str = "matches";', 'static GREETING: &str = "hits";')],
    "file of an include_str! (not a source file)": [substitute(f"{B}/motto.txt", "quick", "slow")],
    "file of an include_bytes! (binary)": [substitute(f"{B}/blob.bin", "\x02", "\x03\x04")],
    "string literal in a body": [substitute(f"{B}/main.rs", '"on 2026-10 the quick', '"in 2026-10 the quick')],
    "println! added to a body": [
        substitute(f"{B}/main.rs", "    let twice = ", '    println!("counted {count}");\n    let twice = ')],
    "macro_rules! body": [substitute(f"{B}/main.rs", 'format!("<{}>", $value)', 'format!("[{}]", $value)')],
    "default method body of a trait": [substitute(f"{B}/main.rs", 'String::from("thing")', 'String::from("item")')],
    "method added to a trait impl": [
        substitute(f"{B}/main.rs", "impl Describe for Hit {}",
                   "impl Describe for Hit {\n    fn describe(&self) -> String {\n        self.0.to_string()\n    }\n}")],
    "trait impl added, not used": [
        insert_after(f"{B}/main.rs", 10**9, "\nimpl std::fmt::Display for Hit {\n"
                     "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n"
                     "        write!(f, \"{}\", self.0)\n    }\n}\n")],
    "private fn renamed, with its one call": [
        substitute(f"{B}/main.rs", "    tally(hay, &re);", "    tally_up(hay, &re);"),
        substitute(f"{B}/main.rs", "fn tally(hay: &str", "fn tally_up(hay: &str")],
    "two items swapped": [
        substitute(f"{B}/main.rs", "const LIMIT: usize = 3;\nstatic GREETING: &str = \"matches\";\n",
                   "static GREETING: &str = \"matches\";\nconst LIMIT: usize = 3;\n")],
    "derive added to a struct (Span: Debug is hand-written, add PartialOrd)": [
        substitute(f"{A}/util/search.rs", "#[derive(Clone, Copy, Eq, Hash, PartialEq)]\npub struct Span {",
                   "#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd)]\npub struct Span {")],
    "attribute added to a function (#[inline(never)])": [
        substitute(f"{A}/util/prefilter/mod.rs", "    fn is_fast(&self) -> bool {", "    #[inline(never)]\n    fn is_fast(&self) -> bool {")],
    "field added to a struct used everywhere (Input)": [
        substitute(f"{A}/util/search.rs", "    earliest: bool,\n}", "    earliest: bool,\n    extra: u64,\n}"),
        substitute(f"{A}/util/search.rs", "            earliest: false,\n        }", "            earliest: false,\n            extra: 0,\n        }")],
}


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("crate", nargs="?", default="regex_automata")
    ap.add_argument("--only")
    ap.add_argument("--perf")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--cargo", action="store_true")
    ap.add_argument("--patch", action="store_true",
                    help="with --cargo: the daemon answers an edit with a patch object linked ahead of the first one")
    a = ap.parse_args()
    crate = a.crate
    crates = [x for name in crate.split(",") for x in ("--crate", name)]
    scenarios = SCENARIOS
    if a.only:
        scenarios = {f"body in {os.path.basename(p)}": [insert_after(p, l, BODY)] for p, l in BODIES}
        scenarios.update(SCENARIOS)
        scenarios = {name: it for name, it in scenarios.items() if a.only in name}
    ws = os.path.join(HERE, "bench2")
    fx = Fixture(ws, crate.split(","), ["-p", "bench2"])
    fx.rustc_build()
    link, cargo_env, cargo_bin = [], None, None
    if a.cargo:
        args = list(fx.link_args)
        patch = os.path.join(fx.work, "ra-jit.patch.o")
        objects = [patch] * a.patch + [os.path.join(fx.work, "ra-jit.o")]
        args = linkline.ahead(args, objects, RA_JIT)
        fx.binary = args[args.index("-o") + 1] = os.path.join(fx.work, "bin-ra-jit")
        link = ["--link-cmd", shlex.join(args)] + ["--patch", patch] * a.patch
        # cargo as it comes: incremental, and none of the flags the harness builds with.
        cargo_env = {k: v for k, v in os.environ.items() if k not in ("RUSTFLAGS", "CARGO_INCREMENTAL")}
        cargo_env["CARGO_TARGET_DIR"] = os.path.join(fx.work, "cargo")
        cargo_bin = os.path.join(fx.work, "cargo/debug/bench2")

    def cargo_build():
        start = time.time()
        subprocess.run(["cargo", "build"], cwd=ws, env=cargo_env, check=True, capture_output=True)
        return (time.time() - start) * 1000

    if a.cargo:
        cargo_build()
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    # The daemon appends through its own handle: a shared one would share the reader's offset.
    log_path = os.path.join(fx.work, "daemon.log")
    log_out = open(log_path, "a")
    log = open(log_path)
    obj = os.path.join(fx.work, "ra-jit.o")
    mismatches = 0
    daemon = subprocess.Popen([RA_JIT, ws, *crates, "--out", obj, "--watch", *link, *symbols],
                              stderr=log_out, stdout=log_out)

    def wait(pattern, seen):
        while True:
            log.seek(0)
            found = re.findall(pattern, log.read())
            if len(found) > seen:
                return
            if daemon.poll() is not None:
                log.seek(0)
                raise SystemExit(f"daemon died:\n{log.read()[-3000:]}")
            time.sleep(0.01)

    originals, rows, edits = {}, [], 0

    def settle():
        """Waits until the daemon has finished every build the writes set off."""
        nonlocal edits
        wait(r"edit-to-binary: \S+", edits)
        while True:
            time.sleep(0.3)
            log.seek(0)
            text = log.read()
            edits = len(re.findall(r"edit-to-binary: \S+", text))
            if edits == len(re.findall(r"change: \d+ files", text)):
                return

    def write(changes):
        nonlocal edits
        files = {}
        for path, apply in changes:
            originals.setdefault(path, open(path, **RAW).read())
            lines = files.setdefault(path, originals[path].splitlines(keepends=True))
            apply(lines)
        time.sleep(0.2)
        for path, lines in files.items():
            open(path, "w", **RAW).write("".join(lines))
        settle()

    def restore(paths):
        nonlocal edits
        time.sleep(0.2)
        for path in paths:
            open(path, "w", **RAW).write(originals[path])
        settle()

    try:
        wait(r"watching \d+ files", 0)
        for n, (name, changes) in enumerate(scenarios.items()):
            before = edits
            perf = a.perf and subprocess.Popen(
                ["perf", "record", "--call-graph", "dwarf,16384", "-F", "499", "-q", "-p", str(daemon.pid), "-o", f"{a.perf}/{n}.data"],
                stderr=subprocess.DEVNULL)
            write(changes)
            if perf:
                perf.send_signal(signal.SIGINT)
                perf.wait()
            log.seek(0)
            text = log.read()
            builds = re.findall(
                r"edit \(apply (\S+)\): compiled (\d+) of (\d+) fns \(\+(\d+) instances\) in (\S+) .*\n(?:   .*\n)*stats: (\d+) of (\d+) functions compiled, \d+ reused\n(?:(?:no )?patch: .*\n)?phases: compile (\S+), resolve (\S+), define (\S+), (?:fallback \S+, )?object (\S+), \d+ rounds\n(?:link: (\S+) .*\n)?(?:timeline-us: .*\n)?edit-to-binary: (\S+)",
                text)[before:edits]
            apply = sum(millis(b[0]) for b in builds)
            total = sum(millis(b[4]) for b in builds)
            phases = [sum(millis(b[i]) for b in builds) for i in (7, 8, 9, 10)]
            row = {"name": name, "builds": len(builds), "apply": apply, "codegen": total,
                   "compiled": sum(int(b[5]) for b in builds), "bodies": int(builds[-1][6]), "phases": phases,
                   "link": sum(millis(b[11]) for b in builds if b[11]),
                   "total": sum(millis(b[12]) for b in builds)}
            if a.cargo:
                row["cargo"] = cargo_build()
                ours, theirs = (subprocess.run([binary], capture_output=True) for binary in (fx.binary, cargo_bin))
                row["same"] = (ours.returncode, ours.stdout) == (theirs.returncode, theirs.stdout)
                mismatches += not row["same"]
                if not row["same"]:
                    for who, run in (("ra-jit", ours), ("cargo", theirs)):
                        print(f"  {who}: exit {run.returncode}\n" + run.stdout.decode(errors="replace")[-400:], flush=True)
            rows.append(row)
            print(row, flush=True)
            if a.check:
                scratch = os.path.join(fx.work, "scratch.o")
                subprocess.run([RA_JIT, ws, *crates, "--out", scratch, *symbols],
                               check=True, capture_output=True)
                same = open(scratch, "rb").read() == open(obj, "rb").read()
                print("  from scratch:", "same object" if same else "DIFFERENT OBJECT", flush=True)
                mismatches += not same
            restore({path for path, _ in changes})
            if a.cargo:
                cargo_build()
        log.seek(0)
        text = log.read()
        # INVALIDATION_LOG: where to keep what the daemon printed.
        if keep := os.environ.get("INVALIDATION_LOG"):
            open(keep, "w").write(text)
        rss = int(re.search(r"VmRSS:\s+(\d+)", open(f"/proc/{daemon.pid}/status").read())[1]) // 1024
    finally:
        daemon.kill()
        for path, text_ in originals.items():
            open(path, "w", **RAW).write(text_)
    cold = re.search(r"cold: compiled (\d+) of (\d+) fns \(\+(\d+) instances\) in (\S+) .*\n(?:   .*\n)*stats: (\d+) of (\d+) functions compiled", text)
    load = re.search(r"load workspace: (\S+)", text)
    load = load[1] if load else "?"
    out = [f"# What an edit invalidates ({crate})", "",
           "Generated by `harness/invalidation.py`. The daemon holds bench2 (regex-syntax and regex-automata, vendored).",
           f"Cold: load {load}, {cold[1]} of {cold[2]} functions + {cold[3]} instances in {cold[4]}. Daemon RSS {rss} MB.",
           "", "Each row is one save, followed by a restore that is not counted. Times in ms.", ""]
    head = ["Edit", "Builds", "Apply change", "Codegen", "of which compile / resolve / define / object"]
    head += ["Link", "Edit to binary", "`cargo build`", "Program output"] if a.cargo else ["Edit to binary"]
    head.append("Functions recompiled")
    out += ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    for r in rows:
        cells = [r["name"], r["builds"], f"{r['apply']:.1f}", f"{r['codegen']:.0f}",
                 " / ".join(f"{p:.0f}" for p in r["phases"])]
        if a.cargo:
            cells += [f"{r['link']:.0f}", f"{r['total']:.0f}", f"{r['cargo']:.0f}", "same" if r["same"] else "DIFFERS"]
        else:
            cells.append(f"{r['total']:.0f}")
        cells.append(f"{r['compiled']} of {r['bodies']}")
        out.append("| " + " | ".join(map(str, cells)) + " |")
    if not a.only:
        open(os.path.join(HERE, f"INVALIDATION-{crate.replace(',', '+')}.md"), "w").write("\n".join(out) + "\n")
    print("\n".join(out))
    if mismatches:
        raise SystemExit(f"{mismatches} edits differ from a build from scratch")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(1))
    main()
