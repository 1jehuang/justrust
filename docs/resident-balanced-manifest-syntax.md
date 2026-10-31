# Balanced manifest syntax scheduling v82

This is a rejected, executed experiment against accepted base `c8829bae0`.
It is not an acceptance or reinterpretation of the rejected v81 experiment.
The hypothesis is narrower than parallel parsing in general: contiguous
**equal-count chunks can leave unequal parser work on one worker**. V82 assigns
one manifest at a time through an atomic work index, so a worker that finishes
one small manifest may claim the next while another parses a larger manifest.
Independent native and runtime gates passed. The targeted comparison was
0.051771s OFF versus 0.052116s ON, **0.67% slower**, with unknown noise.
The candidate was rejected and restored, not shipped or promoted.

## Candidate and compatibility boundary

The only production changes are in `blaze-manifest/src/manifest.rs` and
`blaze-metadata/src/lib.rs`. The v81 split between owned syntax and sequential
semantic normalization is retained. Relative to frozen v81, metadata is bytewise
unchanged and only the manifest batch scheduler changes. The exact original
accepted normalization body and original sequential syntax prefix remain intact.
A copied, independent accepted `read_manifest` oracle in the fixture module
checks full native structures, exported metadata, diagnostics and error chains,
warning order, workspace state, and target discovery, not only selected fields.

The opt-in is still **exactly** `RA_JIT_PARALLEL_MANIFEST_PARSE=1`, cached once
per client process, with all other values OFF. The same fixed raw stderr marker
contract is deliberately reused so ROOT can run the unchanged private frozen v81
factor harness with a distinct **v82 executable SHA/source identity**:

```text
ra-jit: parallel-manifest-parse-v81 batch
```

The newline-terminated marker occurs at most once per client process and only
after a real bounded batch has executed successfully. No marker is emitted by
flag eligibility alone, a version query, a tiny/nonregular batch, or a failed
spawn/join batch that falls back. It proves activation, not useful concurrent
work on every thread, cache hits, phase work, or a performance benefit.

## Dynamic scheduling and ordering

Only eligible fetched registry manifests enter the fast path, at least eight
regular `Cargo.toml` paths, on Linux x86_64. Source replacement eligibility,
remote registry filtering, sequential fallback, fresh file reads, nonblocking
and no-follow file opens, and opened-handle regular-file checks are unchanged.
Worker count is bounded to 2 through 4, including on a one-core host, matching
the original factor. No pool, cache, process-global syntax, or new dependency
is introduced.

Each scoped worker claims a unique input index with `AtomicUsize::fetch_add`
and reads/parses only that file's owned bytes and TOML syntax. Relaxed ordering
is sufficient for unique index allocation, while scoped-thread spawn/join
provides synchronization for returned values. Each thread appends indexed
results to its own vector. There is no per-manifest result mutex or per-edge
logging. The caller joins all started workers, gathers by original input index,
and only then performs source/config resolution and full semantic lowering
sequentially in input order. `Context`, workspace/config `RefCell` state,
warning emission, and target discovery never cross the worker boundary.
Normal read/UTF-8/TOML errors stay ordered result values, not scheduling errors.
Per-file unsupported/open cases still use the existing fresh sequential path.

Thread creation failure and unwinding worker panic drain **every started
handle** before declining the batch. This prevents an implicit scoped-thread
join from re-panicking after an early return. The caller then uses its original
sequential fallback. This is not panic-abort recovery, a filesystem snapshot,
or a promise about concurrently mutated filesystem contents. Ordinary parse
errors do not take this exceptional fallback and retain normal diagnostic order.

## Authored native fixtures, not executed by source worker

Both `tests/parallel_manifest_syntax/mod.rs` modules retain the frozen v81
real-filesystem equivalence fixtures. Added v82 fixtures cover:

- Exact-once file claims, original-order gather, rotated inputs, and 2/3/4 workers.
- A controlled slow first file that must not trap its adjacent input in a fixed
  chunk. This uses a condition variable with a bounded five-second failure wait,
  not a timing performance assertion, and reads actual fixture files.
- Uneven real TOML sizes, multiple syntax/UTF-8 errors, input-order error choice,
  fresh repair, renamed packages and newly discovered target files.
- Normal parse errors retained as values rather than scheduling fallback.
- Unwinding worker failure with all remaining real file reads completed before
  fallback, followed by unchanged sequential parsing.

Native compilation, Send/Sync proof, all fixture execution, original semantic
runtime failures/recoveries, actual marker/current nonce checks, and controlled
ABBA timings were ROOT's independent gates, now passed as recorded below. The source worker executes only
read-only Python static checks, never builds, native tests, probes or jobs.

## Evidence boundary

The distinct private source freeze is
`ROOT/balanced-manifest-syntax-v82`, with full accepted originals, candidates,
patch, source-only checks, SHA receipts, USAGE, RELEASE and STOPEDITS.
The original v81 freeze and its rejected runtime evidence remain unchanged.
Live `bench/resident_suite.py` and its documentation remain accepted HEAD.
ROOT's private adapter, not a newly shipped mode, performs any later comparison.
All twenty measurements and all first touches are retained in the portable
[evidence archive](../bench/results/resident-balanced-manifest-syntax-20261031/README.md).
ROOT independently passed 33 manifest, 7 metadata, 53 codegen and 373 resident
named tests, 8 activation rows, 28 strict rows, and 54 ABBA calls.
Root rustfmt parsing succeeded but --check returned 1 for formatting-only
differences, without modifying frozen bytes. Native source restoration was
verified at 2026-10-31T17:45:29Z, not asserted about later owners' edits.
This remains a noop/clientplanning combined manifest+lock factor experiment,
with unknown noise and no separate-file attribution, significance, general
agent speed, default promotion, or session-weighted performance claim.
