# Matched compiled-test Blaze experiment, 2026-10-30

The earlier `check --lib` result compared different checking engines. This follow-up
requires compilation, linking and execution of the same four existing tests in
pinned justrust `f97fcc2`, with exact matching discovered names, pass/fail/ignored
and filtered counts. It does not claim equivalent behavior for all Rust programs.

## Prospective contract

All primary stacks run `test --lib -- summary::tests:: --test-threads=1 --nocapture`
on the same private source path. Dev/test profiles explicitly use optimization 0,
debug information 0, incremental compilation enabled. Targets, homes, daemon
state and disk stores are isolated. Prepared compilers and shared downloaded
registry sources are not cold OS caches. Owned stacks use one frozen compiler
and client, with all three retention/lint experiments either off or on.

Before timing, each stack must pass the same test set, reject an inserted E0308
with exit 101, recover, execute a deliberately failing assertion in the selected
test with exit 101 and the exact expected test failure, and recover again.
Every measured edit changes the executed test, prints a unique runtime marker
exactly once and checks the changed value. A stale executable, missing/extra test,
unexpected diagnostic, failure, signal or timeout cannot become a fast sample.

Five repetitions rotate stack order and apply identical edit bytes per round.
All original source bytes must be restored and compiler/driver/helper identities
remain unchanged. Timing uses blocking process wait plus a watchdog thread, not
bounded-wait polling. Noise coverage remains explicit and unknown, not assumed
quiet. No native compiler bootstrap runs during this measurement window.

## Backend boundary established before timing

The pinned Blaze stable `0.20261027.0` successfully compiles and runs the four
real tests with its Cranelift backend. Its LLVM backend fails before execution
with unsupported writable statics `__RUST_STD_INTERNAL_VAL` and `MIN`. Full
compatibility logs are retained privately under
`/home/jeremy/.jcode/scratch/ba30/smoke-{cranelift,llvm}/`.

The primary comparison therefore matches the requested observable test workflow,
not the backend implementation: Blaze/Cranelift versus owned rustc/LLVM. We do
not pretend this isolates in-memory reuse from codegen backend differences, nor
rank the unsupported LLVM attempt as a successful result. The test fixture and
acceptance gates are not weakened to accommodate that failure.

## Optimization hypothesis

Existing real native profiles identify the whole-crate dead-code liveness query
as a roughly 139 ms inclusive elapsed scope. This is not exclusive CPU or a
saved-time estimate. Its individual HIR item scans may be reusable independently
of the global reachability traversal. A new default-off, dependency-tracked
per-item usage-query experiment is being implemented in owned rustc to test that
fine-grained invalidation idea. It must preserve diagnostics, allow/expect
propagation, trait reachability, field usage and errors/recovery. Only measured
correctness and performance acceptance can justify keeping or promoting it.

This is a focused real compiled-test diagnostic. Comprehensive session-weighted
before/after results remain required for any general speed or default claim.
