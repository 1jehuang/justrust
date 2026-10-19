# Frozen optimizer accounting re-review: 7f9ebcbe1

Read-only source review on 2026-10-19 UTC. Reviewed checkpoint `7f9ebcbe1`, parent `67e190636`, against the A-H audit in `OPTIMIZER-ACCOUNTING-REPAIR-REVIEW.md`. Coordinator permitted this private audit commit only. No compiler source edits, builds, test execution, main writes, pushes, installs, cleanup, or stash changes were performed for this review. Owner-reported tests are not independent validation here.

## Activation blocker: Buf authority must precede mutation

`src/compiler/ssa/meter.rs:467-485`, `Buf::grow`:

- Lines 470-472 return through the capacity fast path without checking the supplied Meter's authority or sticky failure. Even a no-op can accept a foreign or poisoned meter and mutate the logical limit.
- Lines 477-480 admit temporary bytes through the supplied meter and reallocate the existing buffer in place.
- Only line 483 calls `self.lease.resize(meter, actual)`, which checks the original lease authority.
- A foreign-meter call can therefore enlarge the buffer before returning an authority error. The foreign temporary reservation then drops while the original healthy ledger still pays only the old allocation. The recoverable buffer can subsequently be sealed with the original meter and an underpaid lease.

Required repair: check the buffer lease's exact authority and the meter's sticky state before every branch, allocation, or limit mutation. Apply the same entry check to `extend_growing` at line 496, including paths that do not grow. Prefer a transactional replacement whose actual capacity is fully admitted before changing the old buffer.

Required regressions: foreign and poisoned no-op/within-capacity calls, forced reallocation with a foreign meter, unchanged pointer/capacity/limit/receipt on error, and subsequent original-authority sealing. Both ledgers must retain correct accounting on failure.

## New bookkeeping work remains unmetered

`src/compiler/ssa/meter.rs`:

- `Lease::bytes`, lines 193-200, scans all extra chunks on every call.
- Shrink in `resize`, lines 240-257, repeatedly searches backwards through the chunk list and can revisit chunks quadratically.
- `absorb`, around line 290, loops through chunks and may allocate/copy bookkeeping through `push`.
- `seal`, around line 314, loops through every reservation without charging those visits.

Thread work admission through chunk traversal/copy operations and avoid repeated reverse searches where possible. These are newly introduced accounting-helper loops, not exemptions justified by existing SSA algorithms. Broader algorithm review remains nautilus's separate lane. No budget or debt ceiling increase is proposed.

## Safe overcharge is not exact physical peak

`Lease::push`, lines 203-220, retains the old book reservation, admits `old_bytes + requested_new_bytes` at line 209, reallocates at line 211, and admits steady actual capacity at line 214 before dropping the temporary reservation and replacing the old book. This conservatively charges approximately twice the old plus twice the new bookkeeping storage during reconciliation, rather than the actual old-plus-new overlap.

`Buf::grow`, lines 477-483, similarly reserves old-plus-new while the old lease is already admitted, then grows the original lease before releasing the temporary reservation. This can charge approximately old-plus-twice-new.

This is conservative rejection risk, distinct from the authority underpayment blocker. It does not justify an exact/no-phantom-peak claim. A bounded repair should reserve the full new replacement while the already-admitted old allocation remains live, reconcile actual capacity, and transfer ownership transactionally. Do not raise allowances to accommodate duplicate admission. Exact inequalities depend on actual allocator capacity and retained bookkeeping chunks.

## Resolved by inspection, not independently tested

| Original finding | Frozen repair observed | Remaining boundary |
| --- | --- | --- |
| A nested lowered capacity | `ssa/store.rs:125` now requests `callee_bytes(..., true)` | Broad algorithms remain separately audited |
| D authority laundering through Lease | `resize`, `admit`, `absorb`, and `seal` check exact authority before zero/no-op paths | `Buf::grow` bypass above remains blocking |
| E sticky failure on Lease entry | Meter live checks and Lease entry checks added | Buf fast path still bypasses them |
| F unsupported fact and summary envelope | Producer pays unsupported SsaFact envelope and SummaryFact receipt/envelope | No engine-publication payment claim needed |
| G object/receipt wrappers and symbols | `machine::object_bytes:1082-1101` counts Object/Arc and Receipt/Arc envelopes, table capacities and nested strings, with admission before published wrappers. Call symbols use paid cloning around line 895 | Symbol admission is conservatively duplicated, not underpaid |
| H panic provenance | Generic callback removed from optimized machine path. Private `MachinePanic` uses bounded paid output and prepays diagnostic bytes at lines 1060-1067 before moving them into the paid strings collection | Legacy STACK `PanicAsm` remains explicitly separate debt, not repaired by this checkpoint |

Optimized `Asm` code/trap writes now use bounded buffers. The final output adopts object storage before Arc publication. Immutable `Receipt` contains `PlanAuthority` and a byte value, not a mutable ledger backreference. Retaining chunks then dropping immutable storage receipts preserves the existing conservative ledger-end admission contract.

## Shared PlanReservation::shrink

`src/compiler/ir.rs:1833-1856` is decrease-only by inspection: oversized requests fail, zero does not mutate, the existing engine-meter handle or local usage ledger releases only the requested amount, telemetry is updated, and the reservation's remaining byte count decreases. Later drop releases the remainder. `resource_accounting.rs:52` updates existing-reservation byte telemetry without creating an additional reservation.

Add direct tests for partial shrink, full shrink, oversized shrink, zero, drop after partial shrink, and retain/seal after shrink, covering both task-local and graph-meter-backed reservations. The wrapper-level partial shrink test does not establish all these paths. This review executed none of them.

## Disposition

Checkpoint remains private and blocked on authority-before-mutation repair and bounded helper work accounting. Conservative duplicate peak admission must be labeled honestly and repaired without allowance increases. A/F/G/H and the Lease-level D/E repairs are recognized as concrete improvements, not a full optimizer accounting acceptance result. No speed, full-suite, debt-removal, or independently executed test claim follows from this audit.
