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

## Validation checkpoint

Foundation commit `d1c2bce26`, integrated with origin `1d3f991ce` in
`af847941b`, passed explicit hosted check, all 1,369 library tests, all seven
graph integrity tests, and strict all-target clippy. Hosted run receipts:
`20261018-201512793-2274395`, `20261018-201519226-2278862`,
`20261018-201531641-2286525`, `20261018-201551250-2272342`.
`justrust show` confirms `where remote on hosted` for graph acceptance.
Earlier default-command runs used the client's remote-failure cooldown local
fallback. Those receipts remain recorded as local evidence, not hosted evidence.
The explicit hosted retry succeeded. `cargo fmt --check` also passed.

## Ownership proof receipt checkpoint

`VerifiedTemplateCfg` and `ConcreteCfg` now retain immutable `RetainedStorage`
rather than live `PlanReservation` handles. `bind_proof` promotes its paid lease
only after admission, and uses checked addition for its conservative wrapper,
analysis, lease, and allocator-overhead estimate. Overflow returns an incomplete
`ResourceLimit` analysis instead of wrapping to a smaller byte charge. The
existing larger lease estimate is deliberately preserved during this migration.
A direct publication regression proves exact retained bytes, no refund on proof
or receipt clone/drop, no duplicate cumulative charge, and overflow rejection.

Hosted check, all 1,370 library tests, 39 integration tests across ownership,
accounting, borrowing_graph and graph_integrity, and strict all-target clippy
passed. Receipts: `20261018-201913209-2456507`,
`20261018-201919225-2461386`, `20261018-201923864-2464724`,
`20261018-201942082-2476365`. An initial integration command named nonexistent
`ownership_graph`/`ownership_accounting` targets and failed at target selection.
The corrected command uses the actual `ownership` and `accounting` targets.
No graph ceiling or debt baseline is changed. Other published lease paths remain.

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
