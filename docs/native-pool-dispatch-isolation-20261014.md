# Native pool dispatch isolation, 2026-10-14

## Reproduced defect

The logical-task `CURRENT` slot is separate from rustc's implicit query TLV.
Helping workers dispatched unrelated scope/spawn jobs without hiding the
helping invocation's slot. An opaque helper could therefore inherit the root
or child ID. A join inside that helper incorrectly emitted fork edges from
the helping root and replaced its suspended active slot with a continuation.
Observer equality is not an ownership proof.

Three deterministic real one-thread pool controls failed on the baseline:
`20261014-114100301-42923`, 0 passed, 3 failed, exit 101. The regression retained
actual root/child identities and the false fork/continuation witness. This was
an existing lineage defect, not only a hypothetical obstacle to new bindings.

## Correction

For registries with an attached execution observer, `WorkerThread::execute`
now holds a non-Send RAII dispatch guard around the actual job dispatch.
The guard temporarily removes the exact outer `Option<Active>` slot and
restores it on return or unwind. It neither clones/mutates the suspended active
cell nor emits lifecycle events. In particular it preserves `Some(Active(None))`
for a suspended fork rather than collapsing it to a missing outer guard.

Opaque helper work is unbound. Instrumented join closures explicitly enter
their own children, including the stolen path. The inline join path already
enters its branch inside the closure and is not wrapped in an additional
mask that would destroy the original fork-resume slot.

The disabled dispatcher adds no observer TLS access. It does add the
observer-presence check, so this is not a zero-overhead assertion. Scope work
still has no recorded scheduler/resource causality and remains opaque.

## Requirement to observed check

| Requirement | Actual check | Observed result |
| --- | --- | --- |
| No inherited ownership for opaque scope help | deterministic helper regression | helper point is unbound and nested join no longer forks or mutates the helping root |
| Exact nested/panic restoration | nested helpers, original panic payload, direct guard unwind | caller root/child restored, original panic propagation preserved, mask emits no events |
| Scheduler dispatch coverage | FIFO, CountLatch without explicit yield, synchronous broadcast | all new controls pass through the isolated native dispatch boundary |
| Legitimate join identity | real stolen B, inline branch, suspended fork `Active(None)` | explicit branch identity and fork-resume slot preserved |
| Registry boundary | observed-to-observed and observed-to-unobserved install controls | caller restored and no numeric identity copied into a foreign registry |
| Full pool regression gate | `20261014-114616930-102856` | 95 passed, 0 failed, 26 ignored |
| Strict pool lint | `20261014-114616920-102851` | all-target Clippy with `-D warnings` exits 0 |
| Client compatibility | `20261014-114953812-142040`, `20261014-114956202-142423`, `20261014-115008201-145073` | check, 540 tests and strict lint pass; root format check passes |
| Real compiler integration | frozen compiler/std/proc_macro rebuild and task `773938x4fq` | admission, four wait/read/pool tests, genuine Cargo/compiler binding and proc-macro argv/output/artifact neutrality all pass |

The native gate used the combined frozen dispatch and bounded-read recorder
candidate. It is not an isolated dispatch performance experiment. The source
hashes were unchanged throughout the rebuild and seven native tests. The
accepted driver SHA256 is
`270af337d3643eda83e3f14660bd7e6ea35fc6bd170c088a70aeb9121758f90e`.
The unchanged launcher hash alone would not identify this rebuilt implementation.

All six cited justrust summaries, including the failing baseline, are retained
in `data/runs/query-read-cap-20261014/regression-runs.jsonl`. Independent
read-only review found no blocker in the attached-native-registry scope.

## Explicit limits

Manually entering a raw `TaskContext` on an unobserved worker can still expose
that context to opaque helpers, because the disabled gate intentionally skips
TLS masking. Ordinary native capture attaches the observer to the registry.
Inline branches whose recorder allocation/entry fails are not a universal
ownership guarantee. Their receipts must remain incomplete, and future point
binding must not promote them to a valid complete map.

This correction prevents demonstrated false lineage. It does not implement
query-to-task binding, scope scheduling, resource edges, CPU work/span, or the
full source-to-test graph. No speedup or default-promotion claim is made.
