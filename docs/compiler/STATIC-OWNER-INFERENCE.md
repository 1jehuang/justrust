# Static associated owner inference

## Status

Implemented and validated as an isolated compiler slice. Recovered NonNull
sources and their original fixtures remain separately preserved and blocked by a
source-versus-graph receiver correspondence boundary. This slice does not claim
NonNull support or alter that receiver authority check.

## Frozen contract

A statically written nominal owner, such as `Packet::<u64>::wrap(42)`, constrains
the selected inherent declaration's canonical `associated.self_pattern` through
the same ordered schema parameter terms used for its arguments, return type and
bounds. This holds when the return is nested, for example `Option<Self>` or
`Option<(Self, U)>`, not only when its outermost nominal is `Self`.

Written owner constraints are independent of receiver presence. Static calls do
not skip a receiver formal. Dot calls retain their existing receiver adjustment
and skip behavior. Explicit method actuals remain mapped to the declaration's
method binder in order. Wrong owner or method argument types remain errors.
Unused invalid bodies remain rejected.

## IR and graph shape

No new IR, fact kind, lookup, whole-program scan, queue, budget or authority is
introduced. The existing node-local inference solver unifies the canonical owner
pattern before the selected signature's argument/result constraints. Ordinary
canonical checking and source correspondence still validate the final call.
The old outer-result-only constructor shortcut is replaced by this owner rule.
CHECK must demand no executable image, MONO_PLAN, OBJECT or executable optimizer.

## Acceptance gates

- `static_owner_inference::static_owner_nested_returns_match_stock`: stock and
  native CHECK/BUILD/run at workers 1 and 8, nested Option/Self, ordered explicit
  method arguments, and written-owner inference of a generic null pointer.
- `static_owner_inference_authority::static_owner_constraints_reject_wrong_arguments_and_unused_bad_items`:
  stock E0308 and native CHECK/BUILD rejection with no image, including unused bad
  code. These are source authority controls, not forged-fact coverage.
- Existing `method_owner_inference` regression was run unchanged. Its
  `disjoint_inherent_candidates` case fails with nominal inference no-progress on
  both this candidate (`20261020-012347236-3129991`) and exact published parent
  `519a7ff` (`20261020-012648869-3138822`). The baseline probe executed that one
  byte-identical case, with all production/Cargo bytes verified against the
  parent and a recorded source-triggered rebuild. This is a known failure, not a
  passing regression suite. The other two existing cases passed on the candidate.
- `static_owner_inference_graph` observes actual CHECK at workers 1 and 8.
- Mandatory unchanged `graph_integrity` gate must pass before publication.

The positive source gate passed stock and native CHECK/BUILD/run at workers 1 and
8 (`20261020-011618274-3109254`). All three invalid controls passed stock E0308
and native CHECK/BUILD rejection at workers 1 and 8
(`20261020-012039593-3121038`). Actual CHECK graph frontier passed
(`20261020-013035450-3147870`), `check --tests` passed
(`20261020-013105601-3149060`), and unchanged graph integrity passed 14/14
(`20261020-013141110-3154105`). All runs used pinned nightly 1d81, LOCAL placement,
PREBUILD0 and the existing target directory.

`clippy --lib` exited successfully (`20261020-013125485-3153144`) but reported
87 warnings, including pre-existing nested-if shapes in the edited solver file
and warnings in untouched source files. This is not a warning-free claim; a
baseline clippy warning-count comparison was not run. No warning suppression or
unrelated lint cleanup is included.

Eight authentic slim receipt rows, including both failure receipts, are retained
under `data/runs/static-owner-inference/`. No speed, full Rust, NonNull,
function-item or generalized nominal realization claim follows from this change.
