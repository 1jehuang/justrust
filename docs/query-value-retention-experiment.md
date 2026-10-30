# Small query-value retention correctness experiment

This is an opt-in compiler correctness experiment, not a speedup result or a
change to the default retention policy. The native tracked flag
`-Zjr-cache-small-query-values` retains the already serializable values for
`hir_owner_parent_q`, `constness`, and `coroutine_kind`. It does not authorize
broader value retention or bypass dependency tracking.

## Run

Use the owned compiler containing that flag. No Cargo invocation is involved:

```sh
python3 bench/reuse-query-accept.py \
  --rustc toolchain/rustc/build/host/stage1/bin/rustc \
  --output "$HOME/query-retention-acceptance-$(date +%s)"
python3 -m unittest discover -s bench -p test_reuse_query_accept.py -v
```

`--compiler` is an alias for `--rustc`. The output must be a new directory with
an existing parent. The harness refuses to overwrite anything, creates its root
with mode 0700, and the CLI sets umask 077. Sources are rewritten normally, not
through compiler internals. Both modes use the same compiler and flags except
for the experimental flag. Every compilation enables
`-Zincremental-verify-ich`. Each mode has separate main-crate and dependency
incremental directories, retained and warmed throughout the ordered sequence.
No cache is shared between modes.

## Acceptance contract

The 22 cases cover initial and unchanged builds, body edits and reverts,
const/nonconst transitions, async/ordinary transitions, async closures, nested
owners, item addition/removal/reordering, macro definition edits and reverts,
lint and type errors followed by recovery, and a cross-crate signature change,
intentional signature mismatch, and recovery. The dependency is compiled before
the main crate on every step. Nineteen cases must compile and print the expected
runtime result. Three must fail with ordinary compiler exit status 1. Crashes,
signals and unexpected successes fail acceptance.

For each pair the harness compares compiler exit statuses, stdout, and stderr.
The only normalization is replacement of that mode's absolute source directory
with `<SOURCE>`. It does not filter warnings, error codes, line numbers, crate
hashes, whitespace, or other diagnostic text. Runtime exit statuses and both
output streams must agree, and output must match the expected result. A failure
prints complete captured errors and exits nonzero. A partial `acceptance.json`
is retained even if identity checks or a later case fail.

Artifact SHA-256 values are provenance only. They are deliberately **not** an
acceptance equality test: the tracked opt-in flag intentionally changes the
crate hash, so byte equality would be an invalid contract.

## Captured result (2026-10-30 UTC)

`bench/results/query-value-retention-20261030/native-stage1/acceptance.json`
passed all 22 cases: 88 compiler invocations, including six expected failing main
compilations across the three error cases and two modes. Compiler executable
SHA-256 was
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
The captured rustc-driver SHA-256 was
`502401b14a878674fddf862e4fe7c7dca0948da8a7b8e8aefd3543ffd57a0eb7`.
These hashes, rather than the upstream version string alone, identify the tested
owned compiler. After the coordinator confirmed the final compiler and tools
build had settled, a fresh independent capture at
`bench/results/query-value-retention-20261030/native-final-stage1/acceptance.json`
also passed all 22 cases with exactly the same executable and driver hashes.
A subsequent compiler rebuild requires a new capture directory.

The companion `old-compiler-negative/acceptance.json` is an intentional failure:
the old stage2 compiler rejected the unsupported flag, the harness exited 1,
and the full error was retained. It is not a passing acceptance result. Six
Python unit tests also passed. None of these observations is a speedup claim.

## Evidence and limits

`acceptance.json` contains compiler `-vV`, compiler executable SHA-256, sysroot
query, SHA-256 and resolved path for each rustc-driver library in the sysroot's
`lib`, exact commands, original unnormalized diagnostics, exact per-case Rust
sources, runtime results, planned and completed case counts, and the verdict.
Keep the private capture if detailed forensic evidence is needed. Only the JSON
report needs to be committed, not incremental directories or compiled binaries.

Unit tests validate harness behavior using synthetic responses. They are not a
substitute for the real compiler run. Passing the 22-case corpus establishes
only this finite incremental correctness check. It does not prove query values
were actually deserialized, exhaustive language correctness, compatibility of
flag-on caches with flag-off caches, or any performance benefit. The deliberately
separate cache directories do not test toggling the tracked flag in one cache.
Any future performance/default-promotion claim still requires the comprehensive
session-weighted benchmark and relevant broader correctness gates.
