# Whole-result acceptance and requirement traceability, 2026-10-14

Published implementation: `d3a5319b9`. The entire mapped acceptance set below
was rerun after source publication, not inferred from code inspection or only
from tests run before the last edits. No implementation source changed during
this audit. Native tests used the rebuilt owned stage1 compiler, matching std
and proc_macro, owned Cargo, and installed justrust client.

## Observed improvement, and its boundary

The earlier admission-only schema described which jobs existed and their
parents. It did not supply the wait inventory, resolved read-job edges, pool
continuation graph, or independently reconciled compiler-launch authority now
observed below. The current native workflow supplies actual dependency facts:
57/1561 complete wait lifecycles at 4/8 threads, 5870 resolved publishing-job to
consumer-job edges in a small incremental build, and 16 fork/join continuation
edges. It also detects missing receipts using independent inventories. This is
a demonstrated increase in mapping capability, not a measured build speedup.

Full source-to-test mapping is NOT satisfied. The whole-pipeline flag remains
false and CPU work/span remain null. Consequently the result does not yet
establish the graph's theoretical parallelism ceiling or prove an architectural
optimization. Large read captures still overflow, and scope/resource grants,
query-to-task binding, nonquery/codegen/link/subprocess work, cross-capture
clock binding and previous-session/feed provenance remain incomplete.

## Requirement to check to observed public result

| Requirement / changed output | Check rerun over published result | Concrete observation |
| --- | --- | --- |
| Client compiles and preserves existing behavior | hosted `justrust check`, full `justrust test`, strict all-target Clippy | check/lint exit 0, 540 tests pass, 3 ignored, no warnings |
| Old query schemas and strict new schemas | full client suite, including query/read/continuation/compiler consumers | v1/v2 compatibility and v3 routing exercised; duplicate/unknown fields, invalid lifecycle/endpoints, false full/CPU/span claims rejected by passing fixtures |
| Real serial versus parallel query waits | native wait suite, 1/4/8 frontend threads | 0/57/1561 waits; every parallel registered wait has notified, condvar-returned and resumed stages; `control_causal_edges` equals resumed count and `wait_structural_complete=true` |
| Separate descriptive and control dependencies | installed `graph --json` on actual receipts | small build reports 2208 descriptive parents separately from 8 control edges, not relabeled as semantic result dependencies |
| Native read/result association | native small incremental semantic-read test and installed graph | 13575 observed reads, 12342 tracked reads, 5870 resolved job edges, 3872 unbound reads, 0 lost events; `records_complete=true`, `semantic_complete=false`; executable prints `42` |
| Actual fork/join continuation output | parallel native fixture plus installed graph | separate `rustc-thread-pool-join` receipt has 15 nodes, 16 edges, structural completeness true, no issues; no full-pipeline claim |
| Pool lifecycle, panic, bounds and allocator safety | direct owned pool API suite | 85 pass, 26 ignored, including cap and u64 saturation regression |
| Native descriptor transport and expected-launch inventories | cargo-util `jr_` and Cargo `jr_compiler_launch` tests | 16 and 9 tests pass, respectively, including transport and independent registry controls |
| Parent/compiler/query authority binding | native binding suite, installed `graph --compiler-manifest ... --json` | dirty build: expected/observed 1 unit, 1 launch, 7 journals, 1 entry and 1 query, structural completeness true |
| Recording does not invalidate fresh artifacts | same native binding fixture without edits | Cargo reports Fresh; authority reports 0 expected/observed launches, 3 journals, 0 entries/queries, structural completeness true |
| Missing data cannot silently pass | native fixture removes entry, query, journal or initial authority declaration | missing entry/query/journal each produce `structural_complete=false`; removing initial declaration makes CLI exit 1; restoring it returns completeness true |
| Failure outcomes are preserved | native type-error, alias-cycle and failed Cargo build fixtures | Cargo failure remains failure and graph incomplete; our audit observed 1 cycle and 1 poison notification. Independent rerun exposed parallel E0391 orientation instability, investigated below rather than claimed byte-stable |
| Fail-open recording sinks | native binding and admission bad-file/private-permission sink fixtures | successful compilation remains successful; unsupported/missing recording is not labeled a complete graph |
| Proc macros cannot observe private recording argv | real proc-macro build, capture off/on | genuine native binding entry exists, private argument absent both ways, entire macro-visible argv and generated output identical, executable SHA256 identical |
| Executable identity and execution | equally warmed native off/on/off tests at 1/4/8 threads | same executable hash within fixture and same executed output `shared-query-bytes=4096; independent-sum=2097152` |
| Bounded capture reports loss honestly | native large contention fixture | semantic reads overflow and remain incomplete, while independently complete wait inventory stays separately true |
| No unsupported ceiling/full graph result | every native query/pool/authority public assessment | `full_pipeline_complete=false`, `cpu_work_ns=null`, `execution_span_ns=null`; opaque boundaries remain visible |
| Publication and unrelated work preservation | Git HEAD/origin verification and scoped status audit | implementation and acceptance evidence pushed; unrelated xps14, .wrangler, a/b and unknown-run files preserved |

The authority's `unreconciled_receipt_files=1` is retained in the raw public
observations. Selected compiler-authority structural completeness is not a
claim that every file or every pipeline component is reconciled.

Counts are observations, not constants: wait counts changed from 331/1253 in
an earlier run to 57/1561 in this run because actual scheduling changed. The
gate requires real nonzero parallel waits and consistent inventories, not a
hard-coded count. The semantic small-fixture edge count remained 5870.

## Independent diagnostic instability control

An independent post-publication native rerun (task `4808279fpw`) passed admission
and three of four wait/read tests, but failed the exact parallel alias-cycle
diagnostic comparison: the unrecorded build named `First` first, while capture
named `Second` first. This failure is retained, not silently normalized away.

Forty-eight real controls (task `6321585myz`) distinguish baseline instability
from a capture-only change. At one frontend thread, all 16 capture-off and all
8 capture-on runs emitted the identical `First` orientation and diagnostic hash.
At eight threads, capture-off alone produced `First` 9 times and `Second` 7 times,
with two diagnostic hashes. Capture-on produced `First` 6 times and `Second`
2 times, using the same two hashes. Exact parallel byte stability is therefore
not a property of this baseline fixture. These counts do not establish equality
of scheduling distributions or prove that observation has no scheduling cost.

The corrected acceptance gate passed all four native wait/read/pool tests
(task `7155938w95`, 26.3 seconds): serial exact diagnostic bytes for both type
and cycle errors, parallel exact bytes for the type error, and complete E0391
cycle semantics with both aliases and unsuccessful compilation for the parallel
cycle. No runtime instrumentation changed for this gate correction. Exact
parallel cycle byte stability remains explicitly unsupported. Independent
post-publication binding/macro and hosted regression reruns also passed, with
seven exported summaries in `data/runs/graph-post-publication-20261014`.

## Exact rerun evidence

All six hosted run summaries are committed in
`data/runs/causal-map-20261014/whole-result-audit.jsonl`:

| Check | Exported run |
| --- | --- |
| root check | 20261014-112540467-4051066 |
| root full tests | 20261014-112542067-4051503 |
| root strict lint | 20261014-112553304-4053758 |
| owned pool API | 20261014-112556455-4054257 |
| cargo-util transport | 20261014-112600601-4054824 |
| Cargo launch registry | 20261014-112602360-4051065 |

Actual native reruns all exited 0:

- wait/read/pool: 4 tests, task 365357qeoh
- compiler-authority binding: 1 test, task 3653573w66
- proc-macro neutrality: 1 test, task 365358fz2t
- admission/identity/fail-open: 1 test, task 365358hthi

Path-redacted actual query/pool/authority public JSON and negative controls are
committed in `data/runs/causal-map-20261014/requirement-observations.json`.
Full source fixtures, commands, outputs and raw receipts remain under the
session scratch `mapping-final-audit` directory. These are real compiler and
installed-client workflows, not copied-source mocks. The four scoped overhead
sample files remain explicitly ineligible for speedup/default-promotion claims.
