# Rejected native nondebug mark memoization, 2026-10-30

**REJECTED. No promotion.** This experiment concerns the licensed Blaze-derived
resident, not the clean-room compiler or shipping justrust client. Root's real
native correctness/provider-entry tests passed, but the matched scoped runtime
comparison showed no benefit. Both edited production paths were restored to
exact HEAD bytes before root started its independent cold-diagnostic change.

## Actual decision and measurements

| Matched diagnostic | Frozen v28, RSB20 | Candidate v29b, RSA26 |
| --- | ---: | ---: |
| Engine median, five body edits | 86.112926 ms | 91.814848 ms |
| Reference median, same five edits | 122.752362 ms | 109.605910 ms |

The retained `bench/suite.py compare` receipt reports raw **+6.62%** and
reference-normalized **+19.41%**. These are descriptive scoped observations with
unknown noise. They are not statistical significance, proof of a causal
slowdown, the comprehensive session-weighted workload, or default-promotion
support. Positive memo reuse does not imply faster total validation. The
prospective optimization failed its benefit gate and was rejected.

Root's native v29b gates actually passed:

- Four new direct-oracle/provider-counter tests, run
  `20261030-161531907-1577564`.
- All 50 codegen tests, run `20261030-161551972-1579958`.
- All 359 resident tests, run `20261030-161552243-1580059`.

No positive assertion was weakened to allow zero execution. The four tests
compare independent original computations with tracked marks, count actual
provider entries, require positive cold and changed-input executions and zero
same-revision repeat executions, and cover body/signature/restoration edits,
both debug keys, actual runtime caller-location requests, and a real
function-pointer shim's declaration location. The raw named test results and
exact compiled source are retained in the evidence bundle. Passing those
fixtures is scoped correctness/reuse evidence, not universal semantic coverage
or a speedup claim.

## Candidate evolution and exact semantics

The initial v29 proposal added explicit root/instance LRU capacities of
4,096/32,768. Root rejected that cache-policy change before building it. Its
full original/candidate source, documentation, patch, hashes and honest
source-only readiness remain preserved separately.

Final v29b retained the original `#[salsa::tracked(returns(copy))]` attributes
with no explicit LRU or retention override. It added `debug` to the existing
root/instance query keys and routed both modes through them. Providers retained
the original compiled-query failure check, serial query and mark computation.
The unchanged `Mark::of_function` and `Mark::new` still hashed every runtime
`Location`/`Declaration` dependency, plus debug rows when requested. No mark,
touched/home/cache-authority check or serial dependency was skipped.

Test-only dependencies on `hir`, `ide-db` and `test-fixture` enabled real
`RootDatabase::with_single_file` fixtures, `hir::Semantics` function resolution,
root instances and existing stored environments. The independent oracle copied
the original direct compilation/serial reads and complete hashing without
calling the candidate mark queries or mark constructors. Provider counters
were thread-local and `cfg(test)` only. Root added only those three already
existing dependency names to the codegen package's lock entry; all 592 package
records and versions otherwise remained unchanged.

Potential costs included the new tuple-key interner for root marks and the
validation graph itself. This experiment does not attribute the observed
total-time result to a particular cost or infer provider avoidance merely from
an equal returned mark or compiled-query hit.

## Restoration and retained evidence

`toolchain/resident/crates/hir-ty-codegen/patch.rs` and its `Cargo.toml` were
restored from preserved original bytes at 23:30:56 UTC on October 30, 2026.
Candidate byte comparisons passed before restoration, original byte comparisons
passed afterward, and a scoped `git diff --exit-code HEAD` was clean. Root
restored `Cargo.lock` separately. No resident source from this candidate was
promoted. Later cold-diagnostic source work belongs to root and is independent.

The portable bundle is
`bench/results/resident-nondebug-mark-20261030/`. It includes all v29 and v29b
full-source/readiness snapshots, original and rejected manifests/lock patch,
actual raw build/native journals, all RSB20/RSA26 timing/error/recovery outputs,
source/toolchain/binary identity receipts, exact source patch replay,
restoration receipt, licenses and the unchanged comparison output.

The standalone Python 3.11+ validator never builds, executes a native tool,
contacts a service, dereferences historical absolute paths, or reads the live
checkout. The mutation controls rehash their changed artifact hierarchies to
exercise semantic constraints rather than only checksum errors. Relocation
runs only the copied validator with an empty PATH and no checkout dependency.
Offline replay is not a new native build or runtime workload. Actual offline
validation passed for all 197 inventoried files. All 23 controls passed,
including coherently re-signed semantic mutations and detached empty-PATH
relocation. The exact results are retained in `audit.json` and
`offline-tests.log` inside the bundle.

An inherited cold-matrix precise-diagnostic ordering gap is separate from this
mark-query change. This rejection does not misattribute that upstream gap to
the candidate's passing correctness fixtures or claim a cold-support result.
