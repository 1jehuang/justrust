# Native query admission acceptance, 2026-10-14

Actual owned stage1 compiler acceptance passed at 16:55 UTC and repeated at
16:56 UTC (1 test in 2.004s). No `RUSTFLAGS` or sysroot environment override was
present. The host alias resolved to `build/x86_64-unknown-linux-gnu/stage1/bin/rustc`.

SHA-256 identities at the repeated check:

```text
rustc: 38a70d2401a26eddad5d9ed99ffefb99689f857cfcbb6721fcf15241c9e70042
harness: d4df83b24f92892293a2b30b99d3412c9b0a1947efb7a5372e1763f682881709
```

The stage1 sysroot contained `libstd-fdfb42d0feb061d9` artifacts. A coordinator
attempt at 16:56 UTC reported E0463, then the same exact invocation at 16:56:39 UTC
passed (1 test in 0.741s), with std files present and no source change by the
coordinator. Concurrent sysroot assembly is a plausible explanation for the
transient failure, not a confirmed diagnosis.

```text
JR_QUERY_RUSTC="$PWD/toolchain/rustc/build/host/stage1/bin/rustc" python3 tests/test_execution_graph_query_native.py -v
Ran 1 test in 2.205s
OK
native query admission acceptance: allocator accounting, serial/parallel, artifact identity, failed interface and fail-open sink passed
```

The executable fixture checks independently accounted contexts/jobs and matching
terminal inventories in serial and parallel captures, byte-identical executable
outputs across capture-off/on/off warm incremental compilations, an intentional
type error with incomplete interface receipt, and a sink occupied by a regular
file that must not prevent compilation or change that file. These observations
are acceptance of the admission lifecycle slice, not full pipeline coverage.
CPU work and execution span remain null; no CPU/span ceiling is established.

## Bootstrap and fixture diagnosis

The existing compiler bootstrap completed compiler release compilation in 9m46s
but its enclosing command timed out after 600s during stage1 assembly/codegen
backend work. After confirming the original process exited, a no-deadline resume
completed successfully in 25.69s. The compiler-only target did not install stage1
std: the initial acceptance attempt failed E0463 before capture. Building
`python3 x.py build --stage 1 -j 2 library/std` completed successfully in 166.88s,
including waiting for another bootstrap owner of the same target. Neither the
original process nor that other owner was killed.

The initial rlib fixture failed raw archive SHA identity. A strengthened control
also failed capture-off versus capture-off warm archive identity, before capture.
An isolated three-run capture-off diagnostic found identical `lib.rmeta` and
identical object payload hashes across runs, but different CGU object filename
suffixes (`17ue9um`, `0ojwam5`, `08pq5di`). `lib.rmeta-link` changed with those
filenames. Its encoder stores `rust_object_files` names in
`rustc_codegen_ssa/src/back/rmeta_link.rs`. Raw archive hashes were:

```text
85cfa3d3a04a5ad70f7967cfb4437aae2a555d0821d9ed3b5da6324928efcd3f
1c3d5fc00fe2848d4406e25ec15efb10d8483aca95f1d2da5e095bf99f775f4a
19bca9f092e732433ef888210bcb551f4d5872b367888bb4382b7bd4d58eb812
```

That diagnostic used distinct output names, so it identifies member-name
sensitivity, not a controlled estimate of capture overhead. The same-output
harness control independently established warm rlib byte instability. The
subsequently updated executable fixture retains a raw byte identity assertion
and executes its output. It passed without a native producer source fix.
The harness edits were made by another active session; this report does not
claim ownership of them. Raw incremental rlib identity is not established by
the executable gate, nor is Cargo cache-key neutrality established.

## Other correctness gate

Coordinator-reported full client suite: run `20261014-094927167-2439152`,
479 passed, 3 ignored, 115.2s. `justrust show` identified local execution after a
recent remote failure. This is local correctness evidence, not hosted execution
or timing improvement evidence. Prior hosted query gates remain recorded in
`execution-graph-query-admission.md`. Bootstrap/direct fixtures have no justrust
run IDs; none are invented here.

## Next missing causal instrumentation

Prioritize semantic query-read and cache-hit dependencies, then Cargo unit to
compiler context binding, resource/wait internals and nonquery work. Admission
parent nesting is not a scheduling prerequisite. A complete graph and a credible
parallelism bound require independently validated coverage of those boundaries.
