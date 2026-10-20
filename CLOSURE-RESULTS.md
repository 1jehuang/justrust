# Closure result carrier contracts

Status: bounded source-check prerequisite implemented. The authentic compose-shaped
source is checked, lowered and ownership-validated in all three callable modes.
Reference-bearing result substitutions and native opaque composition remain
unsupported. This is not complete closure-result support.

## First bounded contract

A retained higher-order closure may expose one rigid root result family only
when its original tail is a checked required call with exact existing
parameter-equality result authority. Its parent-minted shape retains an immutable
receipt binding owner, provider, lexical site, original tail ExprId, required
member, ordered trait/method type-witness recipes and argument occurrences.
Recipes contain only canonical ParamIds, Unit and Tuple nodes. They retain no
Ty, TraitCall or Signature. Actual calls and signatures are reconstructed from
the authenticated defining-owner environment and ordered actual bindings.
The receipt is data, not independent callable or result-flow authority.

Every result consumer replays the required-call view in its actual caller world,
environment and phase. The ordinary closure shape/source factory and final
publication replay remain mandatory. Mapping a closure preserves the source
receipt unchanged. Consumption reconstructs each witness through the exact
ordered owner bindings and independently replays the actual call signature.

Existing restricted equality-result obligations survive through closure body,
call and instance admission. A concrete reference-bearing result substitution is
explicitly unsupported. Fixed/nested reference results remain unsupported until
canonical InputCarrier origins and per-mode RefContract rows are implemented.
A rigid parameter is never interpreted as carrier-empty.

The first source-flow slice admits only a tail required call and straight-line
value/call/tuple/field/borrow argument expressions. Early returns, conditional
returns, loops and unknown result producers reject explicitly. No default-empty
provider contract can authorize a result, and no union of arbitrary inputs may
stand in for actual result-flow proof.

## Symbolic Output equality in defining opaque returns

A genuine prelude `compose<A,B,C>(f: impl Fn(A)->B, g: impl Fn(B)->C)
-> impl Fn(A)->C` now checks its actual returned closure, not only a local
nonopaque closure value. Equality proof dispatch admits only the existing
`checked_closure_projection` case alongside closed goals. The canonical
normalizer retains the genuine caller world, environment and phase, replays
sealed owner/provider/site and ordered actuals, and compares its exact normalized
Output with declared C. General open projections still require declared equality
rows. No hidden representation or body read is added.

Focused LOCAL nightly-2026-10-08 evidence:
- `20261019-194407131-1647342`: genuine returning-opaque source consumer passed.
- Dedicated equality replay rejects a changed member/RHS, provider environment,
  schema phase and independently authenticated foreign-world context.
- `20261019-194731263-1661719`: mandatory graph integrity passed 14/14 with
  unchanged ratchets and stack limits.

This is source-check proof support only. Fixed/nested reference substitutions,
physical opaque composition and corpus 11 acceptance are not claimed.

## Captured-carrier opaque pack source contract (bounded support)

Public return eligibility is independent of the hidden body. The current
edition-2024 source registry captures its complete canonical generic environment:
raw schema opaque arguments must match the declaration's ordered formal ParamIds.
Eligibility replays the original result occurrence, bounds, captured regions and
exact ordered substitution before admitting a genuine formal ParamTail input.
It never infers permission from an arbitrary root Parameter, a guessed capture
list or the fact that a hidden operand happens to have a carrier. Named-region
rows retain their existing precedence. Nonzero reference prefixes remain outside
this first slice. Public eligibility alone cannot mint a defining pack proof.

The bounded source contract covers only a defining `OpaqueWrap` whose
canonical hidden closure has direct by-value captures of distinct defining-owner
parameter families. Two different fields of the same family are valid source,
but remain explicitly Unsupported in this bounded slice, not InvalidMetadata.
Ordered hidden locations are exact field paths ending in a root
`ParamTail`; each is mapped to the public identity's root `OpaqueTail` without
erasing the input family. Duplicate, reordered, missing, nested and fixed-reference
channels reject. Unknown or unsupported capture shapes never become empty.

`OpaquePackProof` must retain only its existing immutable publication identity,
original wrapper ExprId, exact hidden/public types and an immutable ordered list
of source locations plus destination location. Sealing and occurrence replay
independently reconstruct every row from the graph-read original publication and
its sealed closure capture metadata. A raw list or a bound proof cannot authorize
packing. New payloads must retain no publication, world, facts, CFG or budget.

Ownership first evaluates the actual operand once, including its move effects.
It requires exact complete correspondence between actual operand carrier rows and
the sealed source list. It then unions each source row's real reference origins
and loans into the one public destination row. An omission is invalid even if
that row currently has no loans. No empty result may replace symbolic families.

Closed realization independently replays the source occurrence and ordered owner
substitution against exact published hidden representation. A physical pack may
be erased only after all instantiated input/destination channels are independently
proven empty and operand/representation types match. Fixed or nested reference
substitutions stay explicitly unsupported in this slice. Existing public opaque
region/capture authority and result-flow checks remain mandatory.

Acceptance requires a real compose-shaped defining-source lower/ownership path,
forged row order/omission/family/publication/occurrence controls, real origin/loan
transport, and rejection of fixed/nested reference substitutions. This bounded
source contract claims neither corpus 11 success nor native captured opaque execution.

Verified LOCAL nightly-2026-10-08 receipts:
- `20261019-202859549-1893797`: genuine compose returning `impl Fn(A)->C`
  passes engine CHECK and ownership at workers 1 and 8, without representation,
  dispatch, mono or codegen demand.
- `20261019-203842753-1945175`: declaration capture-envelope authority control.
- `20261019-203938536-1948622`: pack publication/occurrence/ordered-row forgeries.
- `20261019-203940260-1948749`: closed fixed/nested reference-channel refusal.
- `20261019-203940327-1948787`: genuine repeated-family fields remain explicitly
  Unsupported rather than being classified as forged metadata.
- `20261019-203953392-1949673`: combined-parent graph integrity, 14/14 passed,
  unchanged stack limits and ratchets.
- `20261019-205409850-2013894`: final combined graph integrity on parent
  `265356c2bb220cafce8642c4ae68526da2c0fb28`, 14/14 passed after its dual
  local/task budget-accounting repair; no earlier consumer or authority replay.

## Graph and payload requirements

No new graph operation, global inventory, mutable fact or shared budget.
Receipts contain immutable canonical IDs and finite source type-witness recipes
only. Witness construction/reconstruction charges work, retained bytes and depth
to the existing task budget; source-flow worklists use the exact input edge
inventory for allocation accounting.
No CheckedFunction, CFG, publication, world, facts, SchemaCx or PlanBudget is
reachable from the new receipt. Existing canonical publication and closure seals
are borrowed/replayed, not copied into receipts. CHECK must remain free of
representation, dispatch, mono and codegen demand.

## Acceptance evidence and remaining gates

The focused compiler tests under `frontend/closures/result_contract_tests.rs`
prepare the real prelude and check the actual retained higher-order source. They
replay its sealed source receipt and carrier view, lower its CFG and run ordinary
ownership validation for Fn, FnMut and FnOnce. This is compiler-interface evidence,
not an end-to-end engine worker-1/8 or native execution claim.

Verified on the frozen implementation:
- All three callable modes check/lower/ownership-validate the compose-shaped body.
- Forged owner/provider/site/tail/member/ordered actuals/arguments/result reject.
- Missing or replaced source-call receipts and copied shape seals reject.
- Changed caller world, environment, schema phase and receiver mode reject.
- Direct and nested reference-bearing result substitutions reject.
- Inconsistent reordered owner actuals reject.
- Full graph-integrity gate passes with unchanged ratchets and stack limits.

Authentic LOCAL nightly-2026-10-08 receipts:
- `20261019-175918386-1433107`: all test targets type-check after bounded
  witness/source-edge accounting.
- `20261019-175938401-1437177`: positive all-mode consumer passed; the first
  authority attempt failed only because the test requested schema facts for a
  synthetic callable rather than the original required trait member.
- `20261019-180049624-1441754`: corrected dedicated authority controls passed.
- `20261019-180351238-1453660`: current-parent graph integrity, 14/14 passed.

The existing final source/publication proof remains mandatory. A complete engine
CHECK worker-1/8 and stock/native higher-order return campaign is not claimed by
these focused tests.

No native opaque composition, reference-returning closure, or full corpus-11
claim follows from the source slice.
