# Source-aware runtime bounds checks

Status: implemented bounded native indexing slice, 2026-10-19. Full Rust
compatibility and unwinding remain incomplete. No performance claim is made.

## Frozen contract

Supported builtin array and slice indexing evaluates the receiver place once,
then the index once. Slice descriptor contents are read after the index, so an
index expression that reassigns the slice carrier selects the updated descriptor.
Assignments retain RHS-first evaluation. Nested checks happen before later index
effects. Shared and mutable borrows and ignored non-Copy elements must not move,
load, clone or drop the selected element merely to test its bounds.

A genuine source out-of-bounds operation aborts with SIGABRT and stock-compatible
source path, line, column and full unsigned index/length text under the supported
panic-abort policy. The runtime PID is normalized only by differential tests.
Known invalid fixed-array constant projections still fail CHECK and BUILD,
including unused bodies. This does not add unwinding, general Rust panic hooks,
range panic diagnostics, borrowed non-Copy extraction, raw-pointer index borrowing,
or arrays containing custom Drop owners where those forms remain unsupported.

## Typed graph and CFG shape

Original AST Index and DynamicIndex occurrences retain optional token-session
SpanId values. Checked per-function RuntimeSites contain immutable ExprId to
RuntimeLocation rows, sealed by canonical source replay. No global registry or
mutable fact is introduced. Existing per-function graph nodes own these facts.

`BoundsCheck { array: Place, index: ExprId } -> Unit` is an explicit nonmoving
operation. It lowers to source-authenticated CFG IndexInspect, not an element
value read. Every saved-index read has its own expression identity, so probing
and the subsequent write do not materialize one temporary twice. Indexed borrows
save each index then check it before later projection effects. Literal slice
borrows retain the original AST until typing can distinguish runtime slice
checks from compile-time fixed-array projections.

Source runtime locations attach only to the exact lowered operation. Synthetic
address-materialization rechecks, descriptor extent faults and infrastructure
failures remain separate legacy diagnostic-101 traps. A diagnostic origin alone
cannot authorize a source panic. RawSlice constructor authority, exact operand
ExprIds, origin receipts and no-call lowering proofs remain unchanged.

The backend branches to a cold formatter with unsigned index and length already
snapshotted in registers. The formatter does not reread Rust storage. CHECK does
not request code generation. Successful BUILD remains gated on all required
checks and unchanged graph-integrity ratchets.

## Acceptance gates

- `tests/runtime_bounds.rs`: stock abort differential across off/adaptive/final,
  workers 1 and 8, exact normalized stderr, stdout/effect order and SIGABRT.
  Includes direct array/slice, shared/mutable non-Copy place borrowing, writes,
  constant slice writes/compounds, ignored values, receiver mutation, nested
  outer/inner failures and suppression of later index effects. Each case has a
  static libtest name so bounded hosted batches use normal Cargo test filters.
  The associated-constant slice-index case protects integration with canonical
  scalar constant graph demands.
- `tests/runtime_bounds_authority.rs`: ordinary no-move storage and normal-drop
  controls and CHECK/BUILD rejection of unused compile-time array OOB.
- `frontend/runtime_sites.rs` canonical-publication unit tests: forged
  place/index/type/runtime-site/CFG-origin rejection, including reused saved-index
  identities and probe/final-use origin swaps. These use source publication seals,
  not the intentionally source-agnostic low-level CFG template API.
- `tests/runtime_bounds_graph.rs`: per-function deduplication, successful
  initialization/loan analyses, no executable and no object nodes at CHECK.
- Existing array, indexed-borrow, nested-index, typed/mutable slice, indexed-format,
  raw/range authority and runtime-arithmetic regression suites remain required.
- `justrust test -j1 --test graph_integrity`: all 14 unchanged gates.

Validation receipts and exact parent/candidate/source/CLI hashes accompany the
integration report. Hosted infrastructure failures are not semantic failures or
passing cells. Local validation during the verified hosted ENOSPC outage is
explicitly identified and does not create a new export or target.

## 2026-10-19 validation receipts

Authentic slim run summaries are retained in
`data/runs/xps14/2026-10-19-source-bounds.jsonl`.

- Frozen parent `4c66c1d20ec0a7fb0c59624a0ddda1272afcbb93`, candidate
  `be2d462a297957090f1f0cd3276f1202cf1c64f8`: local focused 66 stock
  differential cells plus no-move/drop, unused constant-array rejection and graph
  controls passed in `20261019-134708788-361613`, during the coordinator-confirmed
  hosted ENOSPC outage. Source-manifest SHA256
  `de612e3c858debde1a6724280b6d248fa45340809ace6ba0554c38aaf87f7bac`,
  fresh CLI `132e9d0b048e18c81d424fb2b52938f36b60214c0153bb210ef95ea288a650f0`.
- Parent `20a05b8b15de4a16c90f990e98546ac1ff6021d5`: the hosted monolithic
  matrix completed cases receiver through format_constant_slice, 84 cells, before
  the caller's 600-second tool cap interrupted the ignored case. This is NOT a
  terminal-green full matrix. The custom environment filter was not transported.
  After replacing only harness entrypoints with static test names, the remaining
  ignored/array/slice 18 cells passed in `20261019-142256204-595416`.
  Both batches used identical compiler source: C-locale sorted relative-path
  manifest SHA256 `92a29d314a83015e1382229d4ccdc915f20f5649bee770fe6392895ddfdfe4f5`,
  actual hosted CLI `32f844e4f873bfca842b2347bdce9fe61faf4511415b127ada251d1d4906432d`.
  Thus the original 102 cells passed across bounded batches, not one green suite.
  Canonical bounds authority, graph 14/14, raw/range/format authority, synthetic
  compound trap controls and 13 scoped nonmoving/indexed/nested/mutable-slice
  regressions also passed; their individual run receipts are retained.
- Final compiler integration parent
  `19285dd2ddbdf29d16632a19eac860f27ecb05ad`, tested candidate
  `ca463eb09c177884da57355d207db21922ba3885`: hosted CHECK of all test targets,
  associated-constant-in-slice-index six-cell differential, canonical bounds
  authority and unchanged graph 14/14 passed in `20261019-143645620-644884`,
  `20261019-143721647-646803`, `20261019-143824453-650054` and
  `20261019-143901185-652039`. Source-manifest SHA256
  `39e5a38ca5c5604e1d06f0c84f8e04c3385f2084c4bfbe54a93c5e62a78d3b8e`,
  actual hosted fresh CLI
  `2fe278f05b0726a8750538cdadad3c58499bf0e1f2ddc135982f987420c91252`.

The broad legacy typed-slice native campaign, whole repository tests/lints and
published arithmetic 78-cell matrix were not rerun for this change. Existing
unrelated compiler warnings were not repaired or hidden. No graph baseline,
stack size or supported Rust compatibility claim was increased.
