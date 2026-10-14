# Native query admission coverage

This diagnostic extends native build observation into the owned compiler. It does
not claim that the whole Rust pipeline is a single fully observed graph.

## Scope and independent accounting

The producer is `rustc-query-admission`, scope
`query-job-admission-lifecycle`. The standalone, untracked diagnostic option is
`-Zjr-query-graph=<private-directory>`. It is not automatically injected through
Cargo, and this slice makes no Cargo cache-key neutrality claim.

The root observer accounts for contexts independently from serialized context
records. Within each context, the native job allocator supplies the expected
contiguous job range independently from event serialization. A complete receipt
requires all expected contexts, all admitted jobs, exactly one completed or
poisoned terminal per admitted job, valid local parent identities, no lost
events, and a returned interface after compiler and scoped worker-pool teardown.
An absent receipt remains unknown. A killed process cannot publish a successful
seal merely because some earlier events exist.

The consumer checks original bytes with typed schemas, bounded input, contiguous
context and job inventories, event ordering and per-job timestamps. Admission
parent edges describe nesting only. They are not semantic read dependencies or
proof of scheduling prerequisites. Foreign-context TLS parent identities mark
loss rather than fabricate a same-context relation.

## Explicit boundaries

Fast cache hits, admission-race cache hits, green ensure paths without jobs,
feeds, direct paths, nonquery work, cache-read dependencies, resource and wait
internals, Cargo parent binding, and driver/process outcome remain opaque.
`full_pipeline_complete` is always false. CPU work and execution span are null.
Neither a thread ceiling nor a parallelism speedup follows from these receipts.

## Operational contract

Capture is opt-in and failure to create or publish a receipt must not suppress
the ordinary compilation. Linux publication uses a private anchored directory,
private temporary files and atomic no-replace publication. Directory ancestors
are caller-trusted. Diagnostic serialization and filesystem synchronization can
add latency, and no bounded I/O latency or production performance claim is made.

## Validation

The initial actual compiler bootstrap exposed missing `rustc_session` crate
dependencies in query implementation hooks and timed out. The next build exposed
a generic `QuerySystem` Drop lifetime regression (E0597/E0505). Both were fixed:
allocator snapshot Drop now belongs to a non-generic allocator owner without
rearranging arena lifetimes. The final stage1 compiler, matching std and libtest
were rebuilt successfully using the owned bootstrap.

Actual native acceptance passed on October 14, 2026:

- `tests/test_execution_graph_query_native.py` against the rebuilt stage1 binary:
  serial and two-thread captures, independent context/job accounting, warm
  capture-off/off/on/off executable SHA-256 identity, executed output correctness,
  intentional type failure remaining incomplete, and an unwritable sink failing open.
- Native compilation of the production observer module with two compiler threads:
  **47,517 admitted jobs**, **45,130 descriptive parent edges**, no assessment
  issues, scoped structural completeness true and full pipeline completeness false.
  All **16 observer tests** passed using the matching rebuilt test library.
- Assessment preserved in
  `bench/results/query-admission-20261014/native-source-assessment.json`.

The native workload is the actual observer source compiled as a standalone test
module. Its test configuration aliases the atomic to the matching host std type.
It is not a full rustc test suite, hosted integration test or fleet promotion.
The rebuilt artifacts remain diagnostic stage1 artifacts, not promoted defaults.

Observed client gates, exported under `data/runs/query-admission-20261014/`:

| Gate | Run | Result |
| --- | --- | --- |
| Hosted check | `20261014-092735147-2043158` | passed |
| Full client suite before root-inventory followup | `20261014-092739452-2044277` | 472 passed, 3 ignored |
| Strict client lint | `20261014-092758327-2048967` | passed |
| Root-inventory hosted check | `20261014-093126015-2114162` | passed |
| Root-inventory consumer tests | `20261014-093153907-2119512` | 14 passed |
| Root-inventory strict client lint | `20261014-093200356-2120729` | passed |
| Final full client suite | `20261014-093727237-2236830` | 476 passed, 3 ignored |

The query-aware client was installed after both gate sequences. The standalone
observer receipt round-trip passed with two contexts, three admitted jobs and
one descriptive parent edge. That round-trip alone is synthetic interoperability
evidence, not rebuilt compiler acceptance. Bootstrap and direct compiler fixtures
do not have justrust build run IDs and are not attributed invented IDs.

### Artifact comparison limitation

Incremental rlib container SHA-256 changed even on repeated capture-off builds.
The owned compiler generates a per-invocation temporary suffix for CGU filenames,
and its private `lib.rmeta-link` records those names. Archive timestamps are zero.
A separate diagnostic found identical `lib.rmeta` and all object payloads while
filenames and `lib.rmeta-link` differed. No universal rlib byte-neutrality claim
is made and no private metadata was normalized away to claim one. The acceptance
harness instead requires exact executable SHA-256 identity after first proving
that the capture-off warm executable baseline itself is stable.
