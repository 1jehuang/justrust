#!/usr/bin/env python3
"""expansions.py: a daemon that loads a workspace again finds its proc-macro expansions in the
store instead of expanding them (crates/ra-jit-expand).

A workspace with a proc-macro crate of its own (a derive, an attribute, and a derive that reads
the text of a span, which asks the database) is built by three daemons on one store:

  - the first, on an empty store, keeps what it expanded that asked nothing of the database;
  - the second finds all of that, and expands only what asked;
  - after an edit to a struct and to a function, the third runs under
    `RA_JIT_PROC_MACRO_CACHE=check`, which expands every call anyway: nothing may expand
    otherwise than it was kept.

Each program has to print what cargo's does. The toolchain's proc-macro server answers no
question of a macro (`source_text` is `None`, and nothing asked), so the program prints only
that the text is not longer than rustc's `#`.

    python3 expansions.py
"""
import os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, TARGET_ARGS, built
from storelib import run, scratch

STRUCTS = 40
TRACED = 20
REPORT = re.compile(r"^proc macros: (\d+) expanded in [\d.]+s, (\d+) from the store in [\d.]+s.*$", re.M)

TOML = """[package]
name = "app"
version = "0.1.0"
edition = "2024"

[dependencies]
pm = { path = "pm" }

[workspace]
"""
PM_TOML = """[package]
name = "pm"
version = "0.1.0"
edition = "2024"

[lib]
proc-macro = true
"""
PM = r'''use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

fn name_of(input: &TokenStream) -> String {
    let mut tokens = input.clone().into_iter();
    while let Some(token) = tokens.next() {
        if matches!(&token, TokenTree::Ident(it) if it.to_string() == "struct" || it.to_string() == "fn") {
            return tokens.next().map(|it| it.to_string()).unwrap_or_default();
        }
    }
    String::new()
}

/// `fields()`: the names of the struct's fields.
#[proc_macro_derive(Fields)]
pub fn fields(input: TokenStream) -> TokenStream {
    let name = name_of(&input);
    let body = input.into_iter().find_map(|it| match it {
        TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => Some(group.stream()),
        _ => None,
    });
    let tokens: Vec<TokenTree> = body.into_iter().flatten().collect();
    let mut names = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let colon = |at: usize| matches!(tokens.get(at), Some(TokenTree::Punct(p)) if p.as_char() == ':');
        if let TokenTree::Ident(ident) = token
            && colon(i + 1)
            && !colon(i + 2)
            && (i == 0 || !colon(i - 1))
        {
            names.push(format!("{:?}", ident.to_string()));
        }
    }
    format!("impl {name} {{ fn fields() -> &'static [&'static str] {{ &[{}] }} }}", names.join(", "))
        .parse()
        .unwrap()
}

/// The function says its name when it is called.
#[proc_macro_attribute]
pub fn traced(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = name_of(&item);
    let mut tokens: Vec<TokenTree> = item.into_iter().collect();
    if let Some(TokenTree::Group(body)) = tokens.pop() {
        let enter: TokenStream = format!("println!(\"enter {name}\");").parse().unwrap();
        tokens.push(TokenTree::Group(Group::new(Delimiter::Brace, enter.into_iter().chain(body.stream()).collect())));
    }
    tokens.into_iter().collect()
}

/// `source()`: the text the struct is written with, which the macro asks for.
#[proc_macro_derive(Located)]
pub fn located(input: TokenStream) -> TokenStream {
    let name = name_of(&input);
    let first = input.into_iter().next().map(|it| it.span());
    let text = first.and_then(|it| it.source_text()).unwrap_or_default();
    format!("impl {name} {{ fn source() -> &'static str {{ {text:?} }} }}").parse().unwrap()
}
'''


def program():
    lines = ["use pm::{Fields, Located, traced};", ""]
    for k in range(STRUCTS):
        fields = "".join(f"    f{j}: u{8 << (j % 4)},\n" for j in range(k % 5 + 1))
        lines.append(f"#[derive(Fields)]\n#[allow(dead_code)]\nstruct S{k} {{\n{fields}}}\n")
    for k in range(TRACED):
        lines.append(f"#[traced]\n#[inline(never)]\nfn t{k}(x: u64) -> u64 {{\n    x * {k + 2}\n}}\n")
    lines.append("#[derive(Located)]\n#[allow(dead_code)]\nstruct Here;\n")
    calls = "".join(f"    println!(\"{{:?}}\", S{k}::fields());\n" for k in range(STRUCTS))
    calls += "".join(f"    acc = t{k}(acc) % 1_000_003;\n" for k in range(TRACED))
    lines.append(f"fn main() {{\n    let mut acc = std::env::args().count() as u64;\n{calls}"
                 f"    println!(\"{{acc}} {{}}\", Here::source().len() < 2);\n}}\n")
    return "\n".join(lines)


def ran(binary):
    """What the program prints. Just linked, it can still be open in a process that forked as
    it was written (Linux says ETXTBSY), for a moment."""
    deadline = time.monotonic() + 5
    while "Text file busy" in (got := run(binary)) and time.monotonic() < deadline:
        time.sleep(0.1)
    return got


def counts(log):
    """How many the daemon's last report says were expanded and found, and the report."""
    reports = list(REPORT.finditer(log))
    if not reports:
        return 0, 0, "no report"
    return int(reports[-1][1]), int(reports[-1][2]), reports[-1][0]


def main():
    work, store = scratch("ra-jit-expansions-"), scratch("ra-jit-expansions-store-")
    ws = os.path.join(work, "app")
    os.makedirs(os.path.join(ws, "src"))
    os.makedirs(os.path.join(ws, "pm", "src"))
    files = (("Cargo.toml", TOML), ("src/main.rs", program()), ("pm/Cargo.toml", PM_TOML), ("pm/src/lib.rs", PM))
    for path, text in files:
        open(os.path.join(ws, path), "w").write(text)
    fx = Fixture(ws, ["app"], ["-p", "app"])
    fx.rustc_build()
    obj, patch, binary = (os.path.join(work, f"app.{n}") for n in ("o", "patch.o", "bin"))
    link = linkline.join(linkline.with_output(linkline.ahead(list(fx.link_args), [patch, obj], RA_JIT), binary))
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    ask = ["--crate", "app", "--out", obj, "--patch", patch, "--link-cmd", link, "--binary", binary, *symbols]
    reference = os.path.join(work, "reference")

    def cargo():
        build = subprocess.run(["cargo", "build", "-q", *TARGET_ARGS], cwd=ws, capture_output=True, text=True,
                               env={**os.environ, "CARGO_TARGET_DIR": reference})
        assert build.returncode == 0, build.stderr[-2000:]
        return run(built(reference, "app"))

    def daemon(name, **env):
        log_path, socket = os.path.join(work, f"{name}.log"), os.path.join(work, f"{name}.sock")
        want = cargo()
        with open(log_path, "w") as out:
            proc = subprocess.Popen([RA_JIT, ws, *TARGET_ARGS, "--socket", socket, *ask], stderr=out, stdout=out,
                                    env={**os.environ, "RA_JIT_CACHE": store, **env})
        try:
            while "watching" not in open(log_path).read():
                if proc.poll() is not None:
                    raise SystemExit(f"FAIL: the {name} daemon did not start: {log_path}")
                time.sleep(0.05)
            reply = subprocess.run([RA_JIT, "sync", socket, *ask], capture_output=True, text=True, timeout=300)
            got = ran(binary)
            if got != want:
                raise SystemExit(f"FAIL: the {name} daemon's program prints\n{got}\nand cargo's\n{want}\n"
                                 f"reply: {(reply.stdout + reply.stderr).strip()}\nlog: {log_path}")
        finally:
            proc.kill()
            proc.wait()
        log = open(log_path).read()
        expanded, found, line = counts(log)
        print(f"{name} daemon: {line}", flush=True)
        return log, expanded, found

    passed = False
    try:
        first, expanded, found = daemon("first")
        kept = int((re.findall(r"; (\d+) kept", first) or ["0"])[-1])
        if found != 0 or kept < STRUCTS + TRACED:
            raise SystemExit(f"FAIL: the first daemon kept {kept} expansions, fewer than the {STRUCTS + TRACED} "
                             f"that ask nothing, or found {found} in an empty store: {work}/first.log")
        second, again, found = daemon("second")
        if found != kept or again != expanded - kept:
            raise SystemExit(f"FAIL: the second daemon found {found} of the {kept} kept and expanded {again}, "
                             f"not the {expanded - kept} that asked: {work}/second.log")
        main_rs = os.path.join(ws, "src", "main.rs")
        text = open(main_rs).read()
        text = text.replace("struct S3 {\n", "struct S3 {\n    added: u8,\n", 1).replace("    x * 2\n", "    x * 7\n", 1)
        open(main_rs, "w").write(text)
        third, _, found = daemon("third", RA_JIT_PROC_MACRO_CACHE="check")
        if "expanded otherwise" in third or found < kept - 2:
            raise SystemExit(f"FAIL: under check, {found} were found again of {kept}, or one expanded otherwise "
                             f"than it was kept: {work}/third.log")
        print(f"PASS: {kept} expansions kept, all found again by a second daemon, none otherwise after an edit")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)
            shutil.rmtree(store, ignore_errors=True)


if __name__ == "__main__":
    main()
