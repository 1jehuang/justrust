# Native proc-macro input correctness gate

`bench/reuse-macro-input-accept.py` is a correctness experiment, not permission to
promote a macro cache or assume proc macros are pure. It changes no compiler
sources and uses no bootstrap build. The default compiler is the frozen binary:

```
/home/jeremy/.jcode/scratch/dead-item-cache-20261030/sysroot/bin/rustc
```

## Running

Coordinate with the owner of native profiling before starting. Use a new, private
fixture directory, never the profiler's source directory:

```sh
python3 -m unittest bench/test_reuse_macro_input_accept.py -v
python3 bench/reuse-macro-input-accept.py \
  --work-dir "$JCODE_SCRATCH_DIR/macro-input-fixture" \
  --output "$JCODE_SCRATCH_DIR/macro-input-result.json"
```

`--work-dir` must not exist. Omitting it creates a retained temporary directory.
The script removes inherited `RUSTC_BOOTSTRAP`, records `rustc -vV`, and hashes
the resolved compiler binary. A new run must use new incremental directories.
All native invocations are direct frozen-rustc invocations because the variable
under test is its native cache, not the installed client or hosted build path.

## Matrix and observable contract

Five small proc-macro fixtures emit a Rust string literal: a constant, ordinary
`std::env::var`, tracked environment input, ordinary filesystem input, and tracked
filesystem input. Each starts warm, changes the actual consuming `main` body,
then changes its external input together with another body edit where applicable.
Finally the proc-macro crate is rebuilt and replaced at the same `.so` path with
a changed expansion, again accompanied by a consuming-body edit. Macro invocation
tokens stay fixed. This intentionally probes reuse across real consuming edits.

Each state is compiled and executed in six configurations:

1. Cache off, persistent incremental directory.
2. Cache off, fresh incremental reference.
3. Cache on, separate persistent incremental directory.
4. Cache on, fresh incremental reference.
5. Cache off, shared persistent incremental directory.
6. Cache on, the exact same shared incremental directory as step 5.

The separate persistent cache-on lane prevents an off compile from hiding stale
on-only reuse. The shared directory lane explicitly tests option switching.
Fresh directories are unique per state and option. Compiler output paths and
source paths remain fixed, and an old executable is removed before compiling.

Pass requires exact compile exit status, stdout and stderr equality across all
six configurations, followed by exact executable exit status, stdout and stderr
equality. Only the private fixture root is normalized in diagnostic strings.
Additionally, the fresh-off reference must succeed and print the independently
expected input, macro revision and consuming-body marker. Equality between six
identically failing compilations cannot pass. Failed observations remain in JSON,
including complete macro-build probe diagnostics and each consumer observation.

## Tracked APIs, uncertainty, and scope

The tracked fixtures first try `proc_macro::tracked::{env_var,path}` with the
corresponding feature gates. If building the fixture fails, they also try the
older `proc_macro::tracked_env::var` or `proc_macro::tracked_path::path` spelling.
Both unsuccessful probes are retained and the case is `unsupported`, not passed.
This label means neither probe compiled, not proof that every possible tracked
API spelling is unavailable. Inspect diagnostics to distinguish unavailable APIs,
feature restrictions, and other fixture/toolchain problems.

An unsupported case makes the overall result `incomplete` and exits nonzero.
Any supported-case observable mismatch or ordinary fixture setup failure makes
it `fail`. Only a complete passing matrix exits zero. Python tests use mocks and
validate harness behavior, not compiler correctness. Native findings must cite
an actual retained report before claiming any stale or unsafe reuse.

Ordinary environment and filesystem reads are deliberate observational probes.
They must not be silently converted into a guarantee that arbitrary untracked
inputs can be cached soundly. A mismatch requires investigation into cache policy
and supported contracts, not automatically a claim that all supported Rust is
incorrect. Tracked-input mismatches need independent verification too.

This gate does not prove arbitrary side-effect correctness, Cargo dep-info
integration, cross-target loading, parallel processes, diagnostics from failing
macro expansion, deletion/unset inputs, or all proc-macro kinds. It does not show
that a cache hit occurred: a correct result may mean conservative recomputation.
No timing or speedup claim follows from this correctness fixture.

## Evidence

Harness unit tests: six tests passed on October 30, 2026. Native execution is
pending coordination with the profiler owner. No compiler safety conclusion has
yet been drawn from this harness.
