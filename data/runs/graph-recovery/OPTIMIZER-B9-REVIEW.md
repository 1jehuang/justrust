# Optimizer b9 accounting re-review

Frozen private checkpoint: `b9da8bc9f63dd03aa945991f8d545e93db388673`, parent
`ef5890d918141f4d522d6b744aa15fbe98edead8`. This checkpoint is not imported.
Main remains optimizer-default-off. This is source inspection, not independent
execution. Owner-reported hosted gates do not establish the error-path contract.

## Remaining blocking defect

At `ssa/machine.rs:1056`, the `symbols` lease is absorbed into `calls`.
That lease pays both call-symbol strings and panic payloads owned by `strings`.
If the following `code.absorb_from(meter, calls)` fails (1064-1072), cleanup
currently drops `calls` before `strings`. This refunds the absorbed panic
payload admission while the payload is still alive. Both Hare and the
coordinator independently identified this exact branch. The owner acknowledged
it and preserved b9 unchanged.

A separate private repair is approved: split call-symbol and panic-payload
payments into leases owned by the respective buffers, retain payload coupling
through every fallible merge, and add observed-drop tests forcing the second
and third merges to fail with a nonempty panic payload. End-state zero-byte
sweeps alone do not prove destruction-before-refund order.

## Closed by inspection within the reviewed scope

- `Lease::try_seal` returns the intact lease on authority, sticky-state and
  work precommit errors. `Owned::seal` destroys the value before the returned
  lease. `Owned::build`, `adopt` and `combine` explicitly preserve that order.
- `try_absorb` returns the incoming lease unchanged before moving payment on
  failure. Consuming production seal calls are envelope-only; payload-bearing
  transitions use recoverable forms.
- Poisoned `PlanReservation::retain` remains a conservative no-refund case:
  the nonmetered failure is a poisoned ledger lock, and Drop cannot acquire
  that same poisoned lock. Nothing clears poison. The metered retain path is
  infallible. The approved shared-file addition is cfg(test) only and poisons
  the private ledger for observation. Tests do not inject poison mid-walk.
- CallTarget equality charges initial and child stack pushes as well as visits.
  Shared-declaration test fixtures now exercise structural comparisons rather
  than rejecting every pair solely on distinct declarations.
- Kept-callee symbol bytes and signature parameter copies charge work.
- Liveness charges a conservative six word-row traversals per block per round,
  with successor unions charged separately. Conditional copies are charged
  even when skipped. This is over-admission, not exact executed work.

The independent algorithm reviewer found no remaining implementation blocker
in these three algorithm-work fixes. Test attribution remains weak for isolated
child-stack pushes, parameter-count scaling and liveness-only work. Focused
controls were requested for the next bounded repair.

## Documentation correction

Rollback statements must not promise unchanged `cumulative_bytes` after a
successful temporary reservation followed by allocation/surplus failure.
Cumulative bytes are historical telemetry, alongside work, peak and sticky
failure state. Only owned bytes and active reservations are candidates for
refund-neutral comparison. No exact physical allocator-peak claim is made.

## Owner-reported hosted evidence

All commands were forced hosted in the isolated optimizer worktree on b9's
final source before its commit. These records have not yet been imported as
central acceptance evidence:

- `20261019-002251549-3765031`: compiler library, 1420 passed.
- `20261019-002254967-3765245`: optimizer, graph_integrity, compiler, cfg and
  deferred_graph, 54 passed.
- `20261019-002305682-3765922`: strict library/tests lint, exit 0.
- `20261019-002237346-3764479`: supporting SSA/resource filter, 45 passed.

No full suite, default promotion, cap increase, graph allowance change or
speed claim is authorized by these results. Legacy stack-emitter panic growth,
conservative symbol double admission and diagnostic error-string allocation
remain explicitly separate debt.
