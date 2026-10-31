# Rejected nested listing scheduler v70/v71 evidence

**Rejected for performance. Original source restored, no promotion, no speed claim.**
The corrected v71 passed 53 codegen and 366 resident native tests, including both
actual scheduler fixtures, and all 28 strict runtime rows. Counterordered pairs
did not show a useful replicated scoped latency improvement. Noise is unknown.
No causal regression claim is made. See `docs/resident-listing-scheduler.md` for
call-path audit, scheduling tradeoff, exact phase timing table and restoration.

## Evidence retained

- `observations.json`: four actual seven-phase runtime cases, exact selected
  original stdout and diagnostic witness lines, statuses, parsed receipts,
  fresh nonces, safe controls/environment, cumulative activation snapshots,
  per-row input inventories, private original byte sizes and SHA256 provenance.
- `inventories/`: full relative native-source3757, workload-source63 and pinned
  toolchain4020 inventories. Runtime identities agree before/after all cases.
  Native source and root Cargo.lock checks passed without lock edits.
- `native/codegen-results.json` and `resident-results.json`: all 53 and 366
  actual named passing lines and complete summaries. Actual run IDs are
  `20261031-060411424-1373945` and `20261031-060411827-1374092`.
  Resident results include the exact flag/unwind/isolation fixture and fresh
  real RootDatabase ordered-output/routing differential fixture. Those tests
  exercise actual production routing counters under manually scoped guards,
  not outer-pool scheduling or native worker-count measurements.
- `native/failed-v70-check.log`: full original check transcript, byte-exact and
  independently fixed-digest bound. E0308 and exit 101 are retained, not relabeled
  a passing gate. That approved transcript contains no private paths. Its generic
  standard telemetry notice is retained as part of the full transcript, not
  telemetry data. All private runtime/daemon logs remain excluded.
- `frozen/baseline`, `failed-v70`, `candidate`, `live-before-restoration`:
  full byte-exact Rust source snapshots. Selected proposal documents, patches,
  SHA/release/check receipts are sanitized historical text derivatives. v71
  fixed exactly one fixture FileId conversion. Assertions and production bytes
  were unchanged. Exact baseline-to-v71 and v70-to-v71 source patch replays are
  checked independently. Original OFF lower/exported bodies are checked bytewise.
  The full failed v70 source and v71 repair remain available after restoration.
- `frozen/v71/rustfmt-*`: actual static parser/check outputs and exits. Parsing
  succeeded, formatting check returned 1. No unrelated formatting was applied.
  Original source-uncompiled freeze receipts are historical, not statements that
  root skipped the later actual native gates.
- `analysis/`: root's actual successful strict raw/state rejection reanalysis
  output and sanitized historical analysis source. Exact raw pair seconds and
  arithmetic are recomputed. This is scoped evidence, not the comprehensive
  session-weighted synthetic benchmark or a default-promotion result.
- `oracles/`: historical hash-bound original pure parser/contract/oracle sources,
  actual runtime driver and actual native gate script retained as text. Replay
  extracts only pure functions by AST, never importing execution drivers.
- `checks/restoration.json`: actual preservation comparisons and write-only
  original restoration receipt at `2026-10-31T13:11:54Z`. Original cmp and scoped
  empty HEAD diff passed, then source ownership was released to root. This
  point-in-time receipt does not describe subsequent independent source changes.
- Cleanup: all four exact completed private namespaces were stopped with exit 0
  after measurement, preserving stores. Only safe receipt fields are exported.

The actual marker `listing: v70 flat nested branch activated` appears once in
all seven ON worker-log snapshots and zero times in all OFF snapshots. It was
once per process, not once per phase. It proves actual branch activation only,
not fresh phase work, avoided worker/thread counts, body counts or latency
attribution. No speedup or causal slowdown is inferred from this marker.

## Portable offline replay

```sh
python3 -B /path/to/bundle/validate.py --manifest
python3 -B /path/to/bundle/test_offline.py
```

Only Python's standard library and packaged files are needed. No Rust, Cargo,
justrust, engine, daemon, toolchain, Git or network job runs. All 28 raw rows
are reparsed with original hash-bound pure functions. JSON tuple/list conversion
is handled by reparsing, not by weakening the original oracle's equality. An
independent exact named/count/status contract checks the same selected evidence.
Full source patch context, bounded fixture repair, native named sets,
inventories, arithmetic, nonclaims, rejection and restoration are also checked.

`test_offline.py` has eight tests including 41 semantic negative mutations,
coherently wrong raw-plus-parsed test names, exact patch-context rejection,
original tuple/list behavior, and rehashed native/failure-log tampering. A
coherent elapsed-time plus matching root-arithmetic rewrite is rejected by the
fixed historical observation commitment. The relocation test runs Python `-I -B`
from an unrelated directory with empty HOME/PATH and requires HOME unchanged.
These are integrity/replay controls, not new native acceptance/performance tests.

`SHA256.json` inventories the package. Independent fixed digests bind historical
observations, full native selected journals, full failure transcript, full Rust
snapshots and original pure oracle sources. They are integrity commitments, not
external signatures or proof of authenticity against an attacker rewriting the
verifier itself.

## Fixed privacy allowlists

`export_private.py` regenerates the package only from explicitly named private
artifacts. Replay never needs private paths. Eight fixed non-path environment
values and five startup controls are selected; twelve path-bearing environment
fields per row are omitted. Full stdout must match an exact test/witness/summary
line grammar. Only exact type-error and assertion witness lines are selected
from stderr. Activation lines are selected by their exact literal from worker
snapshots, whose full raw originals receive hash/byte provenance only.

Selected path-bearing source/script/document constants are replaced with
labeled placeholders. Approved full Rust snapshots preserve existing synthetic
fixture literals such as `/home/one/.cargo`, not the user's home. Excluded:
ambient environment, full raw private state/worker logs, home/cache roots,
credentials, telemetry data, executables, targets and unrelated files. All
consumed private originals are checked unchanged after export. Only exact
historical unified-diff files are exempted from whitespace checks through the
bundle-local `.gitattributes`; full Rust snapshots retain normal checks.
