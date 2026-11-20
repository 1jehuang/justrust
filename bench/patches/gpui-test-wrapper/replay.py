#!/usr/bin/env python3
"""Instrument a held GPUI macro pair only in the new output directory."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

GPUI = "be875c56343dcbd45127a9805781a9e128057dd7"
DESKTOP = "7d38fb72a0beadbca366da1104c915689cc0fd8e"
HERE = Path(__file__).resolve().parent


def git(repo, *args):
    return subprocess.check_output(["git", "-C", repo, *args])


if not __debug__:
    sys.exit("correctness checks require Python without -O")
p = argparse.ArgumentParser(description=__doc__)
p.add_argument(
    "--work-dir",
    required=True,
    type=Path,
    help="New directory outside both source repositories",
)
p.add_argument(
    "--desktop", required=True, type=Path, help="Read-only Desktop git object store"
)
p.add_argument(
    "--gpui", required=True, type=Path, help="Read-only GPUI git object store"
)
p.add_argument(
    "--rustc",
    required=True,
    type=Path,
    help="Actual rustc binary matching the saved rlibs",
)
p.add_argument(
    "--fingerprint",
    required=True,
    type=Path,
    help="Cargo lib-gpui_macros.json from an already built local debug host target",
)
a = p.parse_args()
r = a.work_dir.resolve()
if any(r.is_relative_to(p.resolve()) for p in (a.desktop, a.gpui)):
    sys.exit("work directory must be outside external repositories")
r.mkdir(exist_ok=False)
d = a.fingerprint.resolve().parents[2] / "deps"
env = {**os.environ, "RUSTC_BOOTSTRAP": "1"}
shutil.copy(HERE / "replay.rs", r / "replay.rs")
src = r / "candidate-src/crates/gpui_macros/src"
src.mkdir(parents=True)
for name in ("gpui_macros.rs", "test.rs"):
    (src / name).write_bytes(
        git(a.gpui, "show", f"{GPUI}:crates/gpui_macros/src/{name}")
    )
base = r / "original.rs"
shutil.copy(src / "test.rs", base)
subprocess.run(
    ["patch", "--batch", "--fuzz=0", "-p1", "-i", HERE / "candidate.patch"],
    cwd=r / "candidate-src",
    check=True,
)
shutil.copytree(src / "fast", r / "fast")
rc = str(a.rustc.absolute())
fp = json.loads(a.fingerprint.read_text())
extern = []
for _, name, _, h in fp["deps"]:
    if name not in ("syn", "quote", "proc_macro2"):
        continue
    tag = h.to_bytes(8, "little").hex()
    (f,) = [
        f
        for f in a.fingerprint.parent.parent.glob("*/lib-" + name)
        if f.read_text() == tag
    ]
    extern += [
        "--extern",
        f"{name}={d}/lib{name}-{f.parent.name.rsplit('-', 1)[1]}.rlib",
    ]
for arm, source in [("control", base), ("candidate", src / "test.rs")]:
    s = source.read_text()
    old = "match syn::parse::<ItemFn>(function)"
    assert s.count(old) == 1
    s = s.replace(
        old,
        f'match crate::timed("{arm}.input_parse", || syn::parse::<ItemFn>(function))',
    )
    if arm == "control":
        assert s.count("parse_quote! {") == 2
        s = s.replace("parse_quote! {", "split_parse_quote! {")
        s = (
            """macro_rules! split_parse_quote { ($($tt:tt)*) => {{ let ts=crate::timed("control.generated_emit",|| syn::__private::quote::quote!($($tt)*)); crate::timed("control.generated_parse",||syn::__private::parse_quote(ts)) }}; }\n"""
            + s
        )
        s = s.replace("    parse_quote,\n", "").replace(
            "Ok(TokenStream::from(quote!(#outer_fn)))",
            'Ok(crate::timed("control.final_emit",||TokenStream::from(quote!(#outer_fn))))',
        )
    (r / f"{arm}.rs").write_text(s)
subprocess.run(
    [
        rc,
        "--edition=2024",
        "--crate-type",
        "proc-macro",
        "replay.rs",
        "-L",
        f"dependency={d}",
        *extern,
        "-o",
        "libreplay.so",
    ],
    cwd=r,
    env=env,
    check=True,
)
files = (
    git(
        a.desktop,
        "grep",
        "-l",
        "-F",
        "#[gpui::test",
        DESKTOP,
        "--",
        "crates/jcode-desktop-ui/src/**/*.rs",
        "crates/jcode-desktop-ui/src/*.rs",
    )
    .decode()
    .splitlines()
)
assert files
# Read pinned Git blobs, never current external source or worktree files.
rows = ["extern crate replay;"]
for f in sorted(files):
    rows += ["replay::replay!{", git(a.desktop, "show", f).decode(), "}"]
(r / "held.rs").write_text("\n".join(rows))
(r / "negative.rs").write_text("""extern crate replay; replay::replay!{
#[gpui::test(unknown=3)] fn a(){} #[gpui::test] fn b(x:bool){} #[gpui::test] fn c(){let x=;}
#[gpui::test(iterations="oops")] fn d(){} #[gpui::test(seeds("oops"))] fn e(){}
}""")
(
    r / "edge.rs"
).write_text("""extern crate replay; macro_rules! generated { ($id:ident, $value:expr) => {replay::replay! { #[gpui::test] fn $id() { let n = -1i32; if n < $value { dbg!(n); } } }}; } generated!(from_macro, 1 + 2);
replay::replay!{
#[gpui::test(seed=27,retries=1,on_failure="on_failure")] #[should_panic(expected="sentinel")] fn sync(cx: &mut TestAppContext, _: &mut App, mut rng:StdRng){#![allow(unused_variables)] let x=r#"opaque"#;}
#[gpui::test(seeds(13,19))] #[ignore="why"] async fn asynchronous(cx:&mut TestAppContext, other:&mut TestAppContext,_: BackgroundExecutor){std::future::ready(()).await;}
}""")
for file in ["held", "negative", "edge"]:
    p = subprocess.run(
        [
            rc,
            "--edition=2024",
            "--crate-type",
            "lib",
            "--emit=metadata",
            f"{file}.rs",
            "--extern",
            "replay=libreplay.so",
        ],
        check=False,
        cwd=r,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    (r / f"{file}.log").write_bytes(p.stdout)
    p.check_returncode()
    rows = re.findall(rb"GPUI_REPLAY (\d+) (\d+) (\d+)", p.stdout)
    assert rows
    print(
        file,
        "calls, differences, control errors:",
        [sum(map(int, x)) for x in zip(*rows)],
        flush=True,
    )
    print(
        file,
        "phase ms:",
        {
            k.decode(): round(
                sum(
                    int(v)
                    for a, v in re.findall(rb"GPUI_PHASE (\S+) (\d+)", p.stdout)
                    if a == k
                )
                / 1e6,
                3,
            )
            for k in set(re.findall(rb"GPUI_PHASE (\S+) ", p.stdout))
        },
        flush=True,
    )
    sums = [sum(map(int, x)) for x in zip(*rows)]
    assert (
        sums == {"held": [724, 0, 0], "negative": [5, 0, 5], "edge": [3, 0, 0]}[file]
    ), sums
# Validate diagnostics in the real attribute expansion path. Same source, compiler, filename and command.
(r / "fail.rs").write_text("""#[replay::probe(unknown=3)] fn a() {}
#[replay::probe] fn b(x:bool) {}\n#[replay::probe(iterations="oops")] fn d() {}\n#[replay::probe(seeds("oops"))] fn e() {}\n#[replay::probe] struct NotAFunction;""")
logs = []
for arm in ["control", "candidate"]:
    env.pop("PROBE_CANDIDATE", None)
    if arm == "candidate":
        env["PROBE_CANDIDATE"] = "1"
    p = subprocess.run(
        [
            rc,
            "--edition=2024",
            "--crate-type",
            "lib",
            "--emit=metadata",
            "--error-format=json",
            "fail.rs",
            "--extern",
            "replay=libreplay.so",
        ],
        check=False,
        cwd=r,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    assert p.returncode == 1
    (r / f"{arm}.diag").write_bytes(p.stdout)
    logs.append(p.stdout)
assert logs[0] == logs[1]
msgs = [json.loads(s) for s in logs[0].splitlines()]
assert sum(m["level"] == "error" and bool(m["spans"]) for m in msgs) == 5
print("five negative diagnostics byte-identical, including spans and notes")
