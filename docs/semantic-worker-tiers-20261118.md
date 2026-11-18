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
