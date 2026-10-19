# Immutable resource authority migration

## First stage: authority and paid storage

`PlanAuthority` is a sealed, immutable identity and clamped admission policy.
Cloning it does not retain a ledger or grant spending authority. Equal policies
from independently constructed budgets do not establish common authority.
`PlanBudget` remains runtime scratch and still owns its existing mutable ledger.
No limits, counters, or graph ceilings are raised by this stage.

`PlanReservation::retain` consumes one already admitted byte lease and returns
`RetainedStorage`, containing only an immutable authority token and byte count.
Promotion does not re-admit bytes, release bytes, or increase cumulative bytes.
For direct API accounting it moves one current scratch reservation to a retention.
Receipt clones and destruction have no accounting effects. Publication failure
may therefore conservatively leave a charge, never refund live retained storage.
Poisoned direct ledgers reject promotion rather than produce evidence.

Direct API retained charges remain admitted until the originating budget ledger
ends. A receipt may outlive that ledger without extending its lifetime. This is
admission accounting, not process RSS measurement or an allocator lifetime trace.
In graph execution, admission belongs to the originating node meter. Promotion
never refunds that meter, including after publication or dropping the runtime
budget. The engine owns the node and its published outputs. Receipts never retain
an engine node, meter handle, mutable ledger, closure, or reservation.

Tests cover exact full-budget admission, independent authority rejection, clone
and drop behavior, zero-byte receipts, arithmetic-overflow rejection, poisoned
promotion, runtime scratch release, ledger-drop ordering, and actual graph task
publication under the unchanged byte cap.

## Remaining stages, not delivered by this foundation

Published fact holders must replace retained `PlanReservation` fields with
receipts at their checked publication boundary. Published `PlanBudget` fields
must become `PlanAuthority`, while operations receive explicit task-local
`&PlanBudget` scratch and return typed resource failures. Mutable expansion and
invocation containers require their own immutable publication migration. The
reachable-state debt baseline must only shrink when paths actually disappear.
This API foundation alone does not remove any existing reachable-state debt.

Per-task work bounds must be derived from checked input complexity and the full
borrow/cleanup fixed-point dimensions. The measured repeated call-binding scan
was removed first. No larger fixed work cap is a substitute for that derivation.
