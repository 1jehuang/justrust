# Immutable retained expansion-byte receipts

Status: isolated bridge implemented. Existing published expansion-budget
holders have not been migrated by this checkpoint. OwnerPath adoption and its
reachable-fact acceptance belong to the coordinated PARSE/RESOLVE split lane.

## Frozen publication contract

`ExpansionBudget::authority()` returns an immutable `ExpansionAuthority`.
It contains only `Arc<ExpansionLimits>`, with no state, ledger, engine meter,
closure or spending API. Pointer identity, not equal limits, establishes
common admission authority. The authority is created once in the existing
runtime ledger. It adds no second ledger or global identity counter.

`ByteLease::retain(self) -> Result<RetainedBytes, BudgetFailure>` consumes a
healthy `Storage::Retained` lease. A temporary lease must first be explicitly
promoted using the existing `promote_retained` operation. Misuse rejects with
a sticky authority failure. The caller must admit actual allocation capacity
and relocation peak before allocation; sealing is not an allocation exemption.
The existing `reserve`/`resize`/array-overflow admission remains unchanged.

Sealing removes one live scratch-reservation count, but does not release or
re-admit bytes, change peak/cumulative bytes, or increment the already recorded
retention count. The consumed lease disables its releasing destructor exactly
once. `RetainedBytes` holds only immutable authority and the admitted byte count.
It is Send + Sync. Clone and drop have no accounting effects. Unsealed scratch
leases still refund live bytes on drop, including after sticky failure.

Direct API retained admission lasts until the originating budget ledger ends,
not until the receipt drops. A receipt may outlive that ledger without keeping
it alive. This is conservative admission accounting, not allocator lifetime or
RSS tracking. This bridge does not move expansion admission onto engine task
meters or remove the remaining existing expansion/invocation budget carriers.

The niche-optimized optional runtime budget keeps `ByteLease` at 24 bytes on
x86_64. No budget limit, graph gate or recorded debt baseline was increased.

## Exact stable-client hosted validation

- `20261018-210905777-3484771`: check passed.
- `20261018-210911720-3479046`: all 28 expansion-budget unit tests passed,
  including five new receipt tests. `justrust show` confirms remote on hosted.
- `20261018-211012136-3510465`: final all-28 rerun passed after adding the
  compact lease layout assertion and producer pre-admission documentation.
- `20261018-210936314-3497831`: unchanged graph-integrity suite, six passed,
  one failed at the already recorded 1,165-function associated-normalization
  Work32M boundary. No new static/reachable state violation was reported.
- `20261018-210954765-3500376`: strict all-target clippy passed, also covering
  the separately recovered immutable instance receipt checkpoint.
- `cargo fmt --check` and `git diff --check` passed.

New acceptance covers exact shared-cap admission, independent equal-policy
rejection, immutable receipt clone/drop, zero bytes, weak-ledger destruction,
explicit temporary promotion, temporary publication rejection, checked array
overflow, sticky allocation and poisoned-lock failure, scratch refunds after
failure, and actual graph fact publication after the runtime ledger has died.

The larger cache-composed graph gate is still blocked. No main edit/build,
installation, push, compiler compatibility claim or timing speedup claim.

## Downstream OwnerPath adoption reported by its owner

The coordinated `cr-split` lane adopted this bridge as `46bf8c9e6`, then
published immutable step-storage receipts in `a95b5df48` and prepaid immutable
root-storage receipts in `aaa4244af`. Zebra reports that the OwnerPath
reachable-state test changed from a live ByteLease/ExpansionBudget/Mutex path
to passing and was un-ignored. These are downstream scratch checkpoints, not
an activation or acceptance claim about the separate cache-composed tree here.

Exact hosted receipts supplied by zebra:
- `20261018-211953364-3874008`: graph_integrity, owner_authority, engine and
  source_error_order integration suites, all 41 tests passed.
- `20261018-211912565-3849986`: strict clippy passed.
- `20261018-211909292-3848206`: engine library tests passed.

These results were executed and reported by the owning lane, not rerun here.
The coordinator also received the checkpoint and receipt inventory.
