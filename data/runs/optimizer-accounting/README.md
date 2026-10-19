# Optimizer accounting acceptance

Final source snapshot: `e6c85210024b6a63ef130716a1d9f5678c177326`.
The reviewed source-only replay `c7c62ed2caad9fcbcbde2ff4364373a65a227bbd`
was merged onto the published owner baseline plus documentation. The replay
contains only the five accounting commits, with old private checkpoints kept:

| Reviewed private commit | Current-base replay |
| --- | --- |
| 67e190636 | a6c71e6ea |
| 7f9ebcbe1 | e5da64ad5 |
| ef5890d91 | 67809c695 |
| b9da8bc9f | e97350ee7 |
| d55692677 | c7c62ed2c |

The earlier 11fc graph patch is excluded. No compiler_cli, optimizer default,
graph_integrity gate, WORK_GROWTH ceiling or fact-debt baseline is changed by
this replay. Optimizer default remains **off**, work growth ceiling remains
1.81, required 1208/known-failure 1209 remain, and reachable mutable debt stays
62 entries with six carriers. No compiler installation or speed/default
promotion is implied. Corpus remains 3/20.

## Bounded implementation and reviews

SSA/LOCAL_OPT/SUMMARY/IPO_OPT facts carry immutable storage receipts, including
own publication envelopes. Shared input bodies are not recharged as new body
allocations. Scratch frontiers, lowering, canonicalization, verification,
fixed-point optimization, inlining and machine emission use caller-owned
work/admission. Operand traversal is allocation-free. Machine estimates are
initial paid estimates, not new source caps: outputs can grow through paid
fallible transactional replacement.

Authority and sticky state are checked before buffer mutation, including
zero/no-op paths. Live payloads stay coupled to their leases. Recoverable seal
and absorb failures return admission for destruction-before-refund cleanup.
Call-symbol and panic-payload payments are separate, each owned by its own
buffer before final merges. Tests force all four merge boundaries and observe
no refund before panic payload destruction. Independent Hare/coordinator
inspection closed the earlier cross-buffer cleanup defect; the algorithm
review closed setup/child-push work, kept-symbol/parameter copy work and
literal liveness visits. See the durable audit and follow-up review documents
in `data/runs/graph-recovery/` for exact earlier defects and review scope.

PlanReservation::shrink is decrease-only. The other shared IR addition is a
cfg(test) ledger-poison hook, not a production API. Poisoned nonmetered retain
conservatively leaves admission charged; it is not described as a refund.

Caveats remain explicit:

- Legacy stack-emitter PanicAsm code growth is separate existing debt.
- Call-symbol admission is conservatively duplicated in part of the path.
- Conditional liveness copies are conservatively charged even if skipped.
- Chunk bookkeeping is separately paid, not included in Lease::bytes().
- Diagnostic error-message Strings on failure paths are allocation debt.
- Requested capacity is prepaid; allocator-rounded surplus is admitted after
  allocation before initialization/publication, not prepaid physical peak.
- Historical cumulative/work/peak/sticky telemetry can advance on failed
  attempts even when live bytes and reservations are refunded.

## Exact exported evidence

`2026-10.jsonl` contains exactly 20 unmodified selected records from the stable
slim path-redacted exporter. Four records pin b9, four pin d556, five pin the
current-base c7 replay, and seven pin central acceptance. Private scoped green
is not substituted for central full acceptance.

Central IDs all have prefix `20261019-`:

| Suffix | Result |
| --- | --- |
| 003427373-3801205 | Forced hosted all-target check, exit 0. |
| 003438443-3801651 | Command-only failure, exit 1: two cargo test filters were passed before `--`. No test ran. Preserved, not relabeled as a compiler/test failure. |
| 003451733-3802343 | Corrected complete hosted library, 1508/1508. |
| 003513950-3803253 | Current graph_integrity/optimizer/compiler/cfg/deferred_graph, 58/58. |
| 003532210-3804012 | Strict hosted all-target lint, exit 0. |
| 003554624-3805017 | Frozen full suite, **3627/3627**, exit 0, 299.188 seconds including compilation. |
| 004053903-4011044 | Post-full forced hosted strict all-target lint, exit 0. |

The c7 replay additionally passed all 1508 library tests and 738 selected
current graph/authority/integration tests, plus 48 resource tests and strict
all-target lint. Exact records and separate snapshot IDs are retained in JSONL.
`cargo fmt --check` and `git diff --check` passed centrally. Full acceptance
used local CARGO_BUILD_JOBS=4 because stock-oracle tests require absolute local
toolchain paths. Other checks/lints were forced hosted. Contended acceptance
wall times are not benchmark results.

## Fresh real CLI

After the full suite, the freshly built binary checked, built and ran the same
three-file crate used by owner acceptance. With JR_OPT unset, both default mode
and explicit `--opt adaptive` / `--opt final` passed at workers 1 and 8. All six
runs printed `multi ok\n` with empty runtime stderr. All six invalid-program
checks returned exit 1 with a nonempty typed diagnostic. This verifies CLI
composition, not that every function in the sample was optimized; the full
suite separately exercises optimized execution and graph demand.

Binary SHA256:
`a72672d2263afa3920e01dca66fbf3506626ab555754b40ca6e45a40e8988daf`.
Exact source bytes and source hashes are the three-file/invalid fixture in
`data/runs/owner-session-integration/README.md`; they were copied unchanged.
No source-support, full-Rust, ABI, corpus or performance claim follows.
