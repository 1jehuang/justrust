#!/usr/bin/env python3
"""unittests.py: the crates' own tests, with the ones known to fail named.

usage: unittests.py [cargo test argument ...]

Runs `cargo test --release --no-fail-fast` of the crates below, in two cargo runs: ROOT_CRATES
in the root workspace, and FORK_CRATES (the rust-analyzer fork's) in vendor/rust-analyzer's own
workspace (run in vendor/rust-analyzer, so that its own .cargo/config.toml sets
CARGO_WORKSPACE_DIR, where expect-test finds the parser's .rast files), since the root does not claim
them as members (#293: so that `cargo install --git` builds ra-jit), and cargo only tests a
member's dev-dependencies. Both share one target directory and the arguments given. A failing
test that is not in OPEN fails the run, and so does a test in OPEN that passes or no longer
exists: take it out; so does either cargo run failing before any of its tests failed.
`hir-ide` is rust-analyzer's type inference with its own tests; the ones in OPEN are what the
fork does differently from upstream that its expectations have not caught up with.
"""
import os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Members of the root workspace.
ROOT_CRATES = ["hir-ty-codegen", "ra-jit-store", "blaze-registry", "blaze-resolve", "blaze-build-script", "blaze-manifest", "ra-jit"]
# Members of vendor/rust-analyzer's workspace (its own Cargo.toml and Cargo.lock).
FORK_CRATES = ["hir-ty", "hir-ty-borrowck", "hir-ty-wfcheck", "hir-def", "parser", "syntax", "hir-ide"]
FORK = os.path.join(ROOT, "vendor", "rust-analyzer")

# crate::test, as libtest names it under the crate's test binary.
OPEN = {
    "hir_ide::tests::consteval::bit_op",
    "hir_ide::tests::display_source_code::render_dyn_ty_independent_of_order",
    "hir_ide::tests::display_source_code::render_dyn_ty_under_enclosing_binder",
    "hir_ide::tests::incremental::add_struct_invalidates_trait_solve",
    "hir_ide::tests::incremental::adding_enum_query_log",
    "hir_ide::tests::incremental::adding_impl_query_log",
    "hir_ide::tests::incremental::adding_struct_invalidates_infer",
    "hir_ide::tests::incremental::adding_use_query_log",
    "hir_ide::tests::mir::eval::caller_location_intrinsic",
    "hir_ide::tests::regression::issue_6628",
    "hir_ide::tests::regression::new_solver::trait_solving_with_error",
    "hir_ide::tests::regression::regression_21605",
    "hir_ide::tests::simple::box_into_vec",
    "hir_ide::tests::traits::assoc_types_from_bounds",
    "hir_ide::tests::traits::closure_signature_from_struct_where_clause",
    "hir_ide::tests::traits::infer_box_fn_arg",
    "hir_ide::tests::traits::infer_from_return_pos_impl_trait",
    "hir_ide::tests::traits::nested_impl_traits",
}

COLOR = re.compile(r"\x1b\[[0-9;]*m")
# A test binary is `deps/<name>-<hash>` in cargo's first layout of the build directory and
# `build/<package>/<hash>/out/<name>-<hash>` in its second (nightly cargo's default).
RUNNING = re.compile(r"^\s+Running .*[/\\](?:deps|out)[/\\](\w+?)-[0-9a-f]{16}\b")
DOCTESTS = re.compile(r"^\s+Doc-tests (\w+)")
RESULT = re.compile(r"^test (\S+)(?: - .*)? \.\.\. (ok|FAILED|ignored)")


def run(command, cwd, env, seen, failed):
    """Runs one `cargo test`, adding its tests to seen and failed; returns its exit status and
    whether any of its tests failed."""
    print(" ".join(command), flush=True)
    proc = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
    binary, any_failed = "?", False
    for line in proc.stdout:
        sys.stdout.write(line)
        line = COLOR.sub("", line)
        if m := RUNNING.match(line) or DOCTESTS.match(line):
            binary = m.group(1)
        elif m := RESULT.match(line):
            name = f"{binary}::{m.group(1)}"
            seen.add(name)
            if m.group(2) == "FAILED":
                failed.add(name)
                any_failed = True
    return proc.wait(), any_failed


def main():
    test = ["cargo", "test", "--release", "--no-fail-fast"]
    commands = [
        (ROOT, [*test, *(f"-p{c}" for c in ROOT_CRATES), *sys.argv[1:]]),
        # cargo reads .cargo/config.toml from where it runs, not from --manifest-path.
        (FORK, [*test, *(f"-p{c}" for c in FORK_CRATES), *sys.argv[1:]]),
    ]
    # Under CI rust-analyzer's slow tests run, and mark that they did in the target directory,
    # which they take to be vendor/rust-analyzer/target unless told.
    env = {"CARGO_TARGET_DIR": os.path.join(ROOT, "target"), **os.environ}
    seen, failed, broken = set(), set(), []
    for cwd, command in commands:
        status, any_failed = run(command, cwd, env, seen, failed)
        if status and not any_failed:
            broken.append((command, status))
    sys.stdout.flush()
    new, fixed, gone = sorted(failed - OPEN), sorted(OPEN & seen - failed), sorted(OPEN - seen)
    print(f"\n{len(seen)} tests, {len(failed)} failed, {len(OPEN & failed)} of them known (OPEN in harness/unittests.py)")
    for name in new:
        print(f"::error::{name} fails, and is not in OPEN in harness/unittests.py")
    for name in fixed:
        print(f"::error::{name} passes now: take it out of OPEN in harness/unittests.py")
    for name in gone:
        print(f"::error::{name} did not run: take it out of OPEN in harness/unittests.py")
    for command, status in broken:
        print(f"::error::{' '.join(command)} failed (exit status {status}) before any test failed: the build, most likely")
    if broken:
        sys.exit(1)
    sys.exit(1 if new or fixed or gone else 0)


if __name__ == "__main__":
    main()
