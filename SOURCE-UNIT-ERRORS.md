# Lease-free source-unit diagnostic publication

Status: bounded structural slice, scoped hosted validation passed privately on
`b85a0b6dd`. Frozen source and evidence commits await independent review.
This does not complete per-item parsing, Rust compatibility, or budget debt removal.

## Frozen contract

`SOURCE` still uses the same per-file ordinal keys and publishes `UnitResult`
through the existing typed engine handle. `UnitResult` now contains concrete
`FileUnit` / `UnitFailure` values whose diagnostic field is `SourceUnitError`.
The latter owns only kind, optional source span, message and optional location.
No budget, lease, interner, source map, mutable authority, fallback variant or
side table is reachable through its fields.

Direct source preparation uses `PendingUnitResult`, `PendingFileUnit` and
`PendingUnitFailure`. General `SourceError` is unchanged: an expanded diagnostic
may own a refundable `ByteLease`, destroyed after its message. This slice does
not replace that lease with a permanently admitted retained receipt.

`SourceUnitError::try_from(SourceError)` checks for a diagnostic lease before
moving anything. `Some` returns the original error, with message, location and
lease intact. `None` moves the existing owned fields without allocating or
charging/refunding anything. `publish_unit` applies this boundary both to
terminal file errors and errors that prevented file admission. Unexpected leased
errors fail the producer instead of publishing a mutable fact. The conversion
is not a claim that the original source diagnostic allocation was prepaid.

## Producer and consumer proof

Root files, embedded prelude files, child files and in-memory text enter the
same existing preparation routines. `Loader::error` and `SourceError::from_token`
construct unleased diagnostics. Original source lexical validation runs the raw
scanner, not the generated/native diagnostic adapter. Inactive-syntax failures
are rebuilt as original source diagnostics. Expanded diagnostic leases are
created later during native parsing, after the assembler has consumed source
units, so they remain runtime errors rather than `SOURCE` facts.

The parent-child preparation helper borrows the published parent's child list
and ancestry directly. It does not copy the parent or introduce a new cache,
index, key, graph barrier or global join. Publication moves existing buffers.
The assembler moves unique facts or copies the same shared payload fields as
before, then uses one common depth-first assembly path. Parent/child ordering,
anchoring, local-file span remapping, source registration, IO diagnostics and
location rendering remain unchanged. A published error can be moved back into
a runtime error without a lease. Shared diagnostic copying remains the existing
unmetered source diagnostic allocation behavior, not new accounting evidence.

## Gates

- Dedicated unit regressions cover all eight error kinds, Unicode paths and
  messages, source span/location preservation, message/path allocation identity,
  rejection of retained and temporary leases without refund or retention,
  eventual original refund, both publication error positions, anchoring, and
  unique/shared assembly equivalence.
- Independent-file versus sequential preparation fixtures now actually publish
  the immutable `UnitResult` before assembly.
- Source error ordering exercises CHECK and BUILD at workers 1 and 8.
- Existing expanded diagnostic actual-capacity transfer and exhaustion tests
  must continue to pass, preserving general refundable error ownership.
- Configuration, prelude, original lexing and source preparation gates remain.
- The unchanged graph scanner must observe only the two `UnitResult` carrier
  entries disappearing. `UnitResult` remains a discovered typed fact root.
  No lexical ceiling, budget, program-size floor or other debt entry changes.
- The existing positive 1208-function program gate and strict all-target lint
  must pass. No performance or full compatibility claim follows.

## Observed structural delta

Forced-hosted unchanged graph run `20261019-010607632-111472` passed 13 of 14
checks, including the existing program-size gate. Its only failure requested
recording exactly `UnitResult ExpansionBudget Mutex` and
`UnitResult InvocationBudget Mutex` removals. The inventory therefore shrinks
from 62 to 60 entries, while the six broader carrier types remain. The scanner's
same-name `State` merge is conservative; this slice does not rename types to
exploit that approximation. All other mutable fact paths remain explicit debt.

Final frozen source/evidence hashes and completed validation receipts are recorded
separately in `data/runs/graph-recovery/` after the scoped gates complete.
