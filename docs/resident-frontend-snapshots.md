# Native resident frontend snapshots

This is an opt-in architecture inside the owned `stable` compiler. It uses the
native `--jr-resident` request driver, not Blaze, the clean-room compiler, a Cargo
wrapper, or a separate compiler implementation. The ordinary compiler and
published client remain the defaults.

## Representation and ownership

`JUSTRUST_RUSTC_RESIDENT_FRONTEND=1` requests bulk frontend snapshot retention.
The environment variable alone cannot enable it in one-shot rustc: the native
resident driver must also permanently identify the process before its first
compiler session. Arguments, working directory and environment are frozen for
the lifetime of that worker.

The snapshot representation deliberately separates reusable compiler work from
session-owned references:

- The dependency graph survives as an owned Rust graph, including dependency
  nodes, result fingerprints, edges, and its stable-key lookup structures. A
  successful current-session graph encoder publishes the next snapshot.
- The complete cacheable query-result archive survives as owned bytes and owned
  lookup tables. This includes the compiler's cacheable type-checking and MIR
  results, not merely the nine boolean queries in the earlier experiment.
- Each request still creates a fresh compiler session. Query results are decoded
  into that session's arenas and interners when needed. Source-file caches,
  allocation decoding, and hygiene decoding are fresh for that request.

This is **not** a permanently live `GlobalCtxt`, nor does it retain arbitrary
borrowed `Ty<'tcx>` or MIR pointers across sessions. Parsing, lowering, uncached
queries, decoding, validation, codegen and linking can still cost time. Retaining
an encoded result is not proof that all frontend work, or most elapsed time,
has been eliminated. The experiment must measure those remaining costs.

## Revision validation

The normal incremental file header/toolchain checks, tracked-option checks, and
work-product reconciliation remain in place. A resident snapshot is scoped to
the incremental root and tracked options. Its backing artifact must match the
complete serialized bytes exactly. A same-size rewrite, a replaced file, or a
restored timestamp cannot select a different snapshot based only on metadata.

A matching archive is not authorization to reuse every query result. Normal
input hashing and dependency-green validation still decide which values are
reusable in the new source revision. Diagnostics are replayed normally, and
query decoding remaps stable identities to the current session. Red or missing
queries compute normally. Existing incremental hash verification remains in
place; acceptance runs also enable `-Zincremental-verify-ich`.

Successful serialization publishes the next candidate snapshot. Ordinary type
errors do not fabricate a successful new checkpoint. Missing, incompatible,
changed, oversized or evicted snapshots follow the existing disk/recompute path.
The disk cache remains durable and its format is not replaced by a private RAM
format that stock tools would encounter.

Retention is bounded, with independent graph/archive budgets and a small number
of namespaces. Those bounds concern retained snapshots, not a hard limit on the
compiler's total memory, active compilation, temporary encoding buffers, or
other process-owned resources.

## Native protocol evidence

Completion replies keep the original scalar-cache counters and add a `frontend`
object:

```json
{"graph_hits":0,"graph_misses":0,"graph_bytes":0,"archive_hits":0,"archive_misses":0,"archive_bytes":0,"value_loads":0,"typeck_loads":0,"mir_loads":0}
```

Hit/miss and successful query-load counters are cumulative. Misses also count
reuse opportunities rejected before lookup because the ordinary loader found
missing or incompatible incremental artifacts. Byte counters are
live retained storage and may decrease on replacement or eviction. Query loads
count successful green-result rehydration from a cross-session resident archive,
not same-session query hits or merely the existence of cached bytes. Off-mode
counters remain zero. Type-checking and MIR counters distinguish useful bulk
reuse from scalar-cache activity.

Use one frozen candidate compiler and compare fresh processes, resident with
bulk snapshots off, and resident with them on. Disable the original scalar cache
in both resident modes to isolate the bulk architecture. Rotate mode order and
check exact diagnostics, actual executable/test results, source and artifact
changes, item reordering, type-error recovery, worker identity, memory, and actual
bulk-hit counters. Report setup separately from hot edits.

For example, against an explicit frozen developer candidate:

```sh
python3 bench/stable_resident.py --compiler /absolute/candidate/bin/rustc \
  --output "$JCODE_SCRATCH_DIR/frontend-native" -n 5 --items 256 --frontend
python3 bench/stable_resident_dogfood.py --compiler /absolute/candidate/bin/rustc \
  --source "$PWD/src/remote_sync_queue.rs" \
  --output "$JCODE_SCRATCH_DIR/frontend-dogfood" -n 3 --frontend
```

These are commands to run, not claims that acceptance has passed. The frontend
harness requires actual graph/archive hits and successful type-checking or MIR
result loads, with all bulk counters zero in the off control.

## Production boundary

The existing proc-macro, target/backend, recording and job-admission restrictions
of the native experimental driver remain. Per-compilation proc-macro isolation
and a managed hosted worker lifecycle are separate requirements before a
production-default promotion. An authenticated guest experiment is not that
integration. Keep the published client installed; run unpublished candidates by
explicit path.

Native harness timings are scoped diagnostics, not a general speedup claim.
General or default-promotion claims require the matching session-weighted
`bench/suite.py` before/after comparison and relevant cold, hosted and correctness
gates. Generated evidence belongs in the account-private record store, not
hand-written benchmark logs.
