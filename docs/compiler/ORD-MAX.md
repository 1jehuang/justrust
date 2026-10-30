# Ord::max source default

Status: implemented and validated for direct source method/UFCS calls only.
First-class function-item values remain unsupported.

## Frozen slice

Add the ordinary source-provided `Ord::max(self, other: Self) -> Self` method
with an explicit `where Self: Sized` bound. Its body returns `self` exactly when
`other < self`, and returns `other` otherwise. Equal values therefore select the
second argument, including distinguishable equal non-Copy values. Existing trait
selection, impl overrides, method predicates, ownership and code generation remain
authoritative. The declaration is not an inherent primitive method, intrinsic,
constant, spelling-based fallback or specially lowered library operation.

Direct method calls, `i32::max(a, b)`, alias-qualified calls, explicit
`<i32 as Ord>::max(a, b)` and generic `T: Ord` calls are the intended first slice.
Function-item values such as `let f = i32::max` are NOT part of this milestone.
They require the separate FIRST-CLASS-FUNCTION-ITEMS.md contract and cannot be
implemented by synthesizing an unchecked closure or changing constant lookup.
`Ord::min` and `Ord::clamp` now also use ordinary source defaults with
`Self: Sized`. `min` chooses the receiver on equality. `clamp` first asserts
that its inclusive bounds are ordered, then returns the lower bound, upper
bound or original receiver. Canonical user overrides still take priority.
Explicit function pointers and full comparison-library compatibility remain
open. The existing Ord acceptance suite covers all three defaults.

## Source and graph shape

Only libraries/prelude/cmp.rs changes production behavior. No compiler node kind,
key, typed IR variant, mutable fact payload or whole-program scan is introduced.
The existing declaration/default-method source producers, canonical selected
trait item and exact ordered owner arguments determine the body. CHECK must still
validate all unused default and user bodies without demanding code generation.

## Acceptance gates before publication

- Stock/native CHECK without artifacts and BUILD/run at workers 1 and 8 for
  primitive, alias, generic, method and fully qualified calls.
- Distinguishable equal non-Copy values select the second argument. A user Ord
  override must be selected rather than the default or a hard-coded comparator.
- Missing Ord membership, mismatched arguments, invalid unused bodies and moved
  values reject. Same-spelled unrelated traits must not authorize the Ord member.
- Exact declaration/default signature replay, a retained `Self: Sized` predicate
  assertion on the genuine default facts, and foreign member/world or changed
  argument/result signature rejection use existing canonical APIs. The predicate
  assertion is not a forged missing-`Sized` replay rejection.
- Mandatory graph integrity passes unchanged on the combined current parent.

No corpus-11, function-item, performance or full-Rust claim follows from this
source default.

## Recorded validation

All runs used the installed stable justrust client with command-scoped
`nightly-2026-10-08` (stock oracle rustc `1d81eb4ad`), LOCAL placement because of
an independently verified hosted disk outage. No graph limits or fixtures changed.
Production/library checkpoint `10947a0cb05d75dd655432af3a3a47fc5f3f589c` passed
check--tests, the three positive stock/native tests and the invalid membership,
argument and unused-body controls, then graph integrity 14/14 with no filtering.
Test-only checkpoint `8588d99aa68414e9b0767da270d876d9e16c8d65` retained the
exact production/library bytes and added moved-after-max/unrelated-spelling
source controls plus genuine selected-default signature and member/world replay.
Both added filters passed. Stock independently reported E0382 and E0277 for the
new invalid cases. Existing green production/source/graph gates were not replayed.

Eight authentic slim summaries are preserved in
`data/runs/xps14/2026-10-19-ord-max.jsonl`:

| Gate | Run ID | Outcome |
|---|---|---|
| check--tests | 20261019-222251112-2440686 | exit 0 |
| method/UFCS/alias/generic | 20261019-222324268-2445785 | 1/1, stock/native workers 1/8 |
| non-Copy equal values choose second | 20261019-222546968-2455065 | 1/1, stock/native workers 1/8 |
| canonical user override | 20261019-222650780-2459196 | 1/1, stock/native workers 1/8 |
| missing membership/type/unused body | 20261019-222808489-2464238 | 1/1, invalid CHECK/BUILD workers 1/8 |
| graph integrity | 20261019-223146166-2482343 | 14/14, none filtered |
| moved value/unrelated member | 20261019-224750668-2587436 | 1/1, invalid CHECK/BUILD workers 1/8 |
| genuine signature/Sized/member/world | 20261019-225049248-2607178 | 1/1 |

No new owned warnings were observed. Existing CLI unused-mut and frontend/IR
dead-code warnings remain; this is not a global warning-free or full-suite claim.
