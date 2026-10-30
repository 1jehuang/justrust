# Opt-in module-local Clippy late-lint experiment

The shipping path remains the crate-wide statically combined pass. Invoke the
native Clippy driver with `--cfg=jr_clippy_module_lints` to move only the audited
subset below into `register_late_lint_mod_pass`. The module subset is itself
statically combined, not one dynamically dispatched pass per lint.

The toggle is a rustc cfg, not an untracked environment variable. Importantly,
custom cfgs alone do **not** invalidate green `lint_mod` queries when source code
never reads the cfg. Actual acceptance caught missing diagnostics when switching
partitions within one incremental directory. The native Clippy driver therefore
places the opt-in partition under `jr-clippy-module-lints-v2` inside the requested
incremental directory. Default cache locations remain unchanged. Bump this
namespace when changing the partition or its invalidation contract.
Do not promote this experiment by default without representative benchmark gates.

## Audit boundary

* `NegMultiply`: zero-sized pass, `check_expr`; reads typeck and parent expression,
  computes literal constants locally, no crate initialization.
* `NeedlessQuestionMark`: zero-sized pass, `check_expr` and `check_body`; reads
  lang items and typeck through compiler queries, no crate/body accumulator.

These two passes cover two lint IDs. The crate-wide factory suppresses exactly
those pass names only while opted in. Every module receives a fresh subset.
`Methods`, `UnusedAsync`, and every pass with shared `FormatArgsStorage` or
`AttrStorage` remain crate-wide. This is not evidence that other passes are safe.
Changes to any selected pass require re-auditing this boundary.

The initial four-pass experiment also moved the zero-state `NeedlessBool` and
`BoolComparison` passes. Ordered-edit diagnostics matched, but the official UI
runner found reordered diagnostics in `needless_bool_assign` and `nonminimal_bool`
(1874 passed, 2 failed, 5 ignored). These passes therefore remain crate-wide:
statelessness alone does not preserve interleaving with other passes. No snapshots
were blessed to conceal the ordering regression. The acceptance harness retains
their fixtures and lint levels as cross-partition regression coverage.

## Acceptance

`bench/clippy-module-accept.py` executes the actual compiler/Clippy driver,
not copied pass code or a mock query provider. It compares full JSON diagnostics
(spans, child messages, replacements, applicability and rendered output) and exit
status, allowing only top-level emission ordering to differ. Each ordered edit
compares the default path, warm module path, and a fresh module path. Every run
uses `-Zincremental-verify-ich`.

The sequence removes lints from the first module, removes lints from the second,
reintroduces first-module lints, changes a scoped allow in the second module,
denies a selected lint, introduces a compiler type error, and recovers. A nested
module tests separate-module visitation. Fulfilled and unfulfilled lint
expectations exercise diagnostic side effects across module edits. Repeated cfg
switches share one
incremental directory to exercise partition invalidation. `--ui-root` adds exact
diagnostic/status comparisons for the existing selected-lint UI sources. These
comparisons supplement, rather than replace, the official UI snapshot runner.

Example (use a driver built against the exact same sysroot):

```sh
python3 bench/clippy-module-accept.py \
  --driver toolchain/rustc/build/host/stage1-tools/x86_64-unknown-linux-gnu/release/clippy-driver \
  --sysroot toolchain/rustc/build/host/stage1 \
  --ui-root toolchain/rustc/src/tools/clippy/tests/ui \
  --output "$JCODE_SCRATCH_DIR/clippy-module-accept.json"
```

Wall times in that output are diagnostic only. They are not a speedup claim or
the comprehensive session-weighted benchmark required for default promotion.

## Native build and snapshot runner

Against an existing stage compiler with matching std, the real bootstrap build
is feasible and reuses compiler artifacts:

```sh
cd toolchain/rustc
CLIPPY_ARGS=--cfg=jr_clippy_module_lints ./x test \
  --stage 1 --keep-stage 0 --keep-stage-std 1 src/tools/clippy
```

`--keep-stage 0` preserves the compiler artifacts built *by* stage 0. Using
`--keep-stage 1` instead can rebuild the compiler while retaining old stage-1 std,
causing E0514 before any lint runs. If necessary rebuild matching std with
`./x build --stage 1 --keep-stage 0 library` first. Do not claim that matching
infrastructure errors constitute diagnostic equivalence. The acceptance harness
explicitly rejects missing/incompatible std (E0463/E0514) and compiler ICEs.

## Observed result, 2026-10-30 UTC

The final two-pass v2 experiment passed all 48 real-driver invocations: 10 ordered
states compared across default/warm-module/cold-module modes, four shared-directory
cfg switches, and seven existing UI-source comparisons. The official bootstrap
runner also passed: 1877 main UI cases with five ignored, 194 secondary UI cases,
and all remaining test groups. No snapshot files were changed.

A separate native self-profile fixture used `lib.rs` containing `pub mod a; pub
mod b;`, `a.rs` containing `pub fn f(x: i32) -> i32 { x * -1 }`, and `b.rs` containing
`pub fn f(x: Option<u8>) -> Option<u8> { Some(x?) }`. With the opt-in flag, a shared
incremental directory, `-Zincremental-verify-ich`, and `-Zself-profile-events=default,args`,
the summary showed three `lint_mod` executions cold, no `lint_mod` execution row
on an unchanged warm run, and two executions after replacing only `a`'s body with
`(x + 1) * -1`. Both warnings remained visible, including the untouched module's
cached diagnostic. This measures cache reuse, not an end-to-end speedup.

The path-redacted record is `bench/results/clippy-module-accept-2026-10-30.json`.
The comprehensive performance gate remains unrun, so the default stays off.
