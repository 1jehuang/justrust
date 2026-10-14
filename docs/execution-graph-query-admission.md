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

## Validation status

The initial actual compiler bootstrap exposed missing `rustc_session` crate
dependencies in query implementation hooks and timed out. Standalone observer
unit tests and synthetic consumer tests do not substitute for rebuilt-compiler
acceptance. Before promotion, rebuild the actual owned compiler and run
`tests/test_execution_graph_query_native.py` with `JR_QUERY_RUSTC` selecting that
binary. The harness exercises serial and parallel admission accounting,
capture-on/off artifact identity, failed-interface receipts and an unwritable
sink. Full native acceptance is pending. The subsequent actual bootstrap reached
`rustc_interface` and exposed a generic `QuerySystem` Drop lifetime regression
(E0597/E0505). The corrective design moves allocator snapshot Drop into a
non-generic allocator owner instead of rearranging arena lifetimes.

Observed client gates, exported under `data/runs/query-admission-20261014/`:

| Gate | Run | Result |
| --- | --- | --- |
| Hosted check | `20261014-092735147-2043158` | passed |
| Full client suite before root-inventory followup | `20261014-092739452-2044277` | 472 passed, 3 ignored |
| Strict client lint | `20261014-092758327-2048967` | passed |
| Root-inventory hosted check | `20261014-093126015-2114162` | passed |
| Root-inventory consumer tests | `20261014-093153907-2119512` | 14 passed |
| Root-inventory strict client lint | `20261014-093200356-2120729` | passed |

The query-aware client was installed after both gate sequences. The standalone
observer receipt round-trip passed with two contexts, three admitted jobs and
one descriptive parent edge. This is synthetic interoperability evidence, not
rebuilt compiler acceptance. Bootstrap and direct compiler fixtures do not have
justrust build run IDs and are not attributed invented IDs.
