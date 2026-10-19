# Recovered instance receipt migration, held checkpoint

The original `stash@{1}` remains intact. The standalone receipt patch is
recovered on top of inline-root checkpoint `79055965a`, not on frozen main.
`InstanceIdentity` and instance `Plan` now retain immutable `RetainedStorage`
instead of live leases. Publication pays the actual receipt-vector capacity,
then consumes previously paid leases. Failed vector admission releases the
unpromoted scratch. A foreign receipt cannot establish same-owner authority.
`Plan` still retains `PlanBudget`; this checkpoint does not claim its full
reachable mutable-state removal.

Exact hosted validation with the installed stable client:
- `20261018-210527752-3445412`: initial mechanical patch recovery failed check
  with a mismatched delimiter. Shifted unified-patch context duplicated two
  lines and removed `matches!`; those application artifacts were corrected
  without changing the preserved migration's logic.
- `20261018-210604611-3447989`: corrected check passed.
- `20261018-210606649-3448257`: two `instance_storage` tests passed.
- `20261018-210613536-3448577`: rejected-receipt-vector test passed.
- `20261018-210614148-3448807`: graph integrity six passed, one failed at the
  same 1,165-function `SchemaAdapter::declaration_info` Work32M boundary as
  before this migration. No gate or budget was changed.
- `cargo fmt --check` and `git diff --check` passed.

No push, install, main edit, timing speed claim or complete migration claim.
