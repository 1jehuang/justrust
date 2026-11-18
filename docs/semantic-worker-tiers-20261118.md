# Native semantic worker memory tiers, 2026-11-18

## What this implements

This is a continuation of the disk-AST experiment, not another AST loading
optimization. The native compiler can retain dependency graphs and encoded
semantic query archives between requests. Green queries can decode Typeck and
MIR values into the next request's fresh compiler context. The new lifecycle
can leave those archives hot, compress inactive archives, or drop the owner
cache and use normal incremental files again.

This is **not** a permanently live `GlobalCtxt`, an in-place mutable query
universe, or a heap snapshot. Parsing, expansion and creation of fresh compiler
contexts still happen. Typed references are not serialized by copying pointers
and are not borrowed from a previous context. The ordinary incremental header,
options, dependency graph and exact artifact-byte checks remain authoritative.

The implementation is in:

- `rustc_middle/src/query/on_disk_cache/resident.rs`: bounded compressed archive
  entries, owned indexes, exact-byte/header checks before promotion, and safe
  hot/compressed/cold transitions.
- `rustc_middle/src/dep_graph/resident.rs`: release unshared graph entries at an
  idle boundary. Active graph references remain charged and are not invalidated.
- `rustc_driver_impl/src/resident.rs`: strict idle control frames, handled only
  between completed legacy resident requests. Controls share increasing request
  IDs but do not change frozen arguments, environment or compilation results.
- `bench/semantic_worker_tiers.py`: actual process memory samples, real tier
  ownership gates, green query load checks, and fresh diagnostics/runtime oracles.

Compression uses miniz level 1. Decompression has an explicit output limit and
must reproduce the exact independently validated artifact bytes before native
indexes can be used. Corruption or incompatible data falls back to the ordinary
disk loader. Trusted writer snapshots can compress before their first reader;
a successful validated read binds their header offset. Cold cache eviction does
not delete incremental files. These budgets bound retained owner caches, not
all compiler memory or compression's transient allocation peak.

## Hosted correctness gates

All Rust/compiler work ran on the normal authenticated production guest
`ip-172-31-47-214`, with 16 allocated CPUs recorded in `/run/pool/share`.
No privileged snapshot tooling or separately provisioned marker is required.
The frozen candidate is:

```
/home/ubuntu/.justrust/experiments/semantic-worker-20261118-1255/compiler
launcher sha256: 8b913cea6cfdb1627d89441724568785936df59f6ad4512447ce47647f21f079
driver sha256:   4f97e174fabe6d904d3a9e9d55a7c3a98bb79034ed5ed5f05ed45be53993ac97
source hash:     51d931e4e7d9559ba34498750d69116849acff30fe612b361966473c7d010cce
```

The driver check passed. Native tests passed: 83 resident-filtered middle tests,
2 middle doctests, and 37 driver tests with 1 ignored. The matching stage1
compiler and complete library build passed. The existing `/run/pool/share`
guard and pure Python harness tests also passed.

The real-module workload privately copies `src/remote_sync_queue.rs`, executes
all five original tests plus a fresh runtime nonce probe, and checks exact
fresh-compiler diagnostics/status. It includes unchanged work, body edits,
reverts, deliberate assertion failure/recovery, type error/recovery and
signature edit/recovery. The original supplied source remains unchanged.

The first two diagnostic receipts were deliberately retained as failures:
`semantic-worker-hosted-20261118-1302-diagnostic` and
`semantic-worker-hosted-20261118-1302-diagnostic-real`. Their gate incorrectly
expected unchanged builds to demand Typeck/MIR values. Native evidence showed
that unchanged builds can validate green nodes without loading those particular
values. The corrected gate requires actual archive/query loads on unchanged
wake, and **both** Typeck and MIR load deltas on the real body edit. Compressed
mode intentionally evicts decoded dependency graphs, so a graph hit is required
for the hot mode, not for the graph-evicting compressed mode. Original receipts
were not edited or relabeled.

## First balanced real-module measurement

Run `semantic-worker-hosted-20261118-1304-real-a-lossless` contains the exact
2,551,663-byte receipt, authenticated reconstructed SHA256
`f89c171cc0ff4a174ed0369fd5e3b60180059adda66bf0ea460ee3a380423d27`.
Five repetitions rotate execution order across five modes. There are 250
compiler requests, including deliberate type errors, with fresh runtime
consumers for successful compilations. Typed-load, compression reduction,
cold owner-release, exact diagnostic and full named-test gates passed.

Aggregate medians across this workload:

| Mode | Compiler wall ms | Live idle RSS KiB | Live idle PSS KiB | Private KiB | Transition ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| fresh process, normal incremental | 124.229 | not retained | not retained | not retained | n/a |
| persistent process, frontend cache off | 103.571 | 107040 | 33090 | 9016 | 0.155 |
| semantic archives hot | 109.134 | 117660 | 43651 | 19596 | 0.145 |
| semantic archives compressed | 109.572 | 112288 | 38335 | 14392 | 1.946 |
| semantic owner caches dropped, process alive | 108.731 | 112024 | 38131 | 14240 | 0.109 |

For the body edit specifically, median compiler wall times were 115.706 ms
fresh, 94.446 ms control, 102.339 ms hot, 103.280 ms compressed and 102.819 ms
cold-owner. These are narrow module measurements, not full-client or general
Rust speed claims. Runtime tests are correctness consumers and are not included
in compiler wall times. First compilation and deliberate failures are included
in the aggregate, so inspect per-case values for individual edit-loop claims.

PSS apportions shared mappings among the concurrently alive workers. RSS includes
shared compiler/library pages and should not be multiplied as private fleet
cost. Private memory and PSS are sampled from live `smaps_rollup`, not peak RSS.
This module does not establish a RAM budget for a large real project.

The compressed owner tier reduced median idle PSS by 5,316 KiB relative to hot,
with about 1.95 ms spent making the transition. Owner release does not guarantee
that the allocator immediately returns every page to the operating system.
The cold-owner mode above **keeps its process alive** and is not the long-idle
zero-worker-RAM tier. An actual terminate-and-restart disk tier is a separate
acceptance path.

## Actual terminated-worker disk tier

The follow-up harness in `e997469ee` adds `frontend-disk`. After each completed
request it closes and reaps that worker, proves its old PID is absent, and
restarts a new worker against the same ordinary incremental directory. Each
generation has separate raw logs. Wake time starts before process creation and
ends after the compilation reply, so it includes dynamic loading and compiler
startup, not merely the constructor's short `Popen` interval.

The unchanged original receipt is authenticated in
`semantic-worker-hosted-20261118-1314-disk-a-lossless`: 5,761,923 bytes,
SHA256 `2be699c3b6300f9d568829fd718a65aa38da09673e0973ca31ab75080e00a398`.
The preceding one-repetition diagnostic also passed and is retained as
`semantic-worker-hosted-20261118-1314-disk-diagnostic-lossless`.
Five balanced repetitions cover six modes and 300 compiler requests. All
diagnostic, named-test, runtime nonce, typed-load and memory-tier gates passed.
All 45 between-request disk transitions proved successful process exit, old
PID absence, a different restart PID and identical durable artifact contents
during terminated idle. Compiler and supplied source identities stayed exact.

| Mode | Wake wall ms | Idle PSS KiB | Idle private KiB | Transition ms |
| --- | ---: | ---: | ---: | ---: |
| persistent control, archive off | 94.907 | 28702 | 9124 | 0.139 |
| semantic hot | 100.149 | 38845 | 19232 | 0.140 |
| semantic compressed | 100.536 | 33622 | 14120 | 1.914 |
| semantic owners dropped, process alive | 100.581 | 33814 | 14332 | 0.105 |
| worker terminated, disk-backed restart | 113.023 | 0 | 0 | 28.080 |

These wake medians exclude the initial compilation and include restart startup
for disk mode. They are not the aggregate compiler-only medians in the earlier
table. The disk transition interval includes the harness's content-hash evidence
collection as well as shutdown, so 28.080 ms is not an isolated production
shutdown cost. The terminated worker has zero process memory by verified
absence, not by reading `smaps` after exit. Its disk artifacts remain: median
3,101,835 bytes across 101 files. Kernel file cache and disk storage are not zero.
This measures restart with the host page cache uncontrolled, not a physical
cold-storage read or a restored compiler heap.

Within this six-mode run, compression saves 5,223 KiB PSS relative to hot.
Shared-mapping apportionment differs from the five-mode run, so compare memory
within a run rather than treating the PSS difference between runs as a change.
The disk-backed worker wakes about 12.9 ms later than the hot semantic worker.
This supports a hot/compressed/terminated lifecycle as a memory policy, but
does not show a speed advantage over the faster archive-off persistent control.

## Interpretation and remaining boundary

The semantic archive path genuinely reused query values, including Typeck and
MIR on an ordinary edit. It nevertheless did not beat the frontend-off
persistent control in this measurement, and did not approach 10x. Compression
is useful for retention density, not a demonstrated compile-latency win here.
Do not enable archive retention globally on the strength of the fresh-process
comparison while ignoring the faster persistent control.

This establishes a safe backing-store and ownership lifecycle. It does not
solve fine-grained frontend invalidation before macro expansion. A deeper live
semantic worker must own stable expanded/HIR inputs and track their dependencies
without reusing stale spans, crate identities, diagnostics, build-script or
proc-macro effects. The existing native archive decoder is a correctness path
for restoring encoded values, not proof that those live input/identity problems
have already been solved.

The concrete next boundary is `resolver_for_lowering_raw` in
`rustc_interface/src/passes.rs`. Its product is not an AST alone: it contains
the expanded crate, `ResolverAstLowering` and `ResolverGlobalCtxt`, after crate
store freeze. Reusing only the AST while rebuilding or mixing those resolver
outputs cannot establish semantic equivalence. A replacement needs stable
per-owner expanded/HIR inputs, complete macro/configuration/environment/file
dependencies, replayable diagnostics, and green proofs bound to the retained
type universe. The current `RevisionIdentity::authorize_reuse` deliberately
rejects changed source/body revisions; removing that gate is not an
implementation of owner-level invalidation.

Accordingly, the hosted memory lifecycle is validated, but a general live
post-expansion semantic engine is still unimplemented. No production-wide
frontend-cache switch was enabled: both balanced runs favored the persistent
archive-off control. Keep normal incremental restoration as the correctness
path, and evaluate a stable-input semantic engine separately before replacing
it or claiming a 10x edit-loop gain.
