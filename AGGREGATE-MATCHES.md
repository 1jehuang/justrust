# Irrefutable aggregate match contract

Status: **implementation in progress, not accepted**. Full Rust remains open.
This extends the checked unit and tuple-struct milestones, not general pattern
matrices, guards, match ergonomics or enum payload matching.

## Source admission

Admit match patterns over existing nominal structs and nonempty structural tuples
whose complete pattern tree consists only of genuine bindings, wildcards and
already-irrefutable nested tuple, record, unit or tuple-struct patterns. Validate
the whole tree before treating an arm as catchall or allocating its binding plan.
Exact source schema/declaration, nominal type, field names/positions, arity and
privacy remain authority. No enum or refutable child is discarded as wildcard.

Refutable leaves, enum children, guards, or-patterns, rest patterns and implicit
reference ergonomics remain explicit Unsupported in the new aggregate route.
Existing scalar/enum/unit match behavior remains separately supported. Existing
scoped bare-unit constructor candidate restrictions remain unchanged.

## Checked IR and execution

`MatchPattern::Aggregate { ty: Ty, bindings: Vec<PatternBinding> }` retains the
actual checked aggregate domain, including typing when the scrutinee is Never.
The domain is a nominal struct or nonempty structural tuple. A returning
scrutinee must have the same domain. Every destination is a valid fresh distinct
nonparameter local with the exact projected type. Projections are nonempty plain
Field chains only, bounded by existing count/depth/work limits, and cannot
overlap. No enum downcast, reference dereference or array index is admitted by
this pattern variant. All arms, including dead arms, are validated.

Substitution, body type identity, facts validation and clone/storage preflight
must retain the explicit domain and each binding projection. Original source
binding IDs, structural inventory paths and canonical body publication remain
unchanged. Aggregate constructors or arm bindings gain no callable or root-borrow
replay authority.

CFG lowering evaluates the scrutinee exactly once and binds only selected fields.
Wildcard/omitted bindingless children do not consume values. Copy fields copy,
non-Copy fields move, siblings remain available and whole-parent use after a
partial move rejects. Aggregate arms need no tag/discriminant read. Existing
scope cleanup, dead-arm checking and private aggregate transport remain intact.

## Required acceptance

- Independent stock-first source corpus and workers-1/8 native CHECK/BUILD with
  unavailable compiler fallback, real executable stdout/exit and no failure
  artifacts, in separate bootstrap debug and release profiles.
- Public malformed IR domain, local identity, projection class/type/bounds,
  overlap, freshness, depth/count and dead-arm controls.
- Once-only effects, nested patterns, scalar Copy reuse, wildcard nonmove,
  non-Copy partial moves, privacy/nominal mismatch and unused invalid bodies.
- Complete unchanged hosted checks/tests/strict lint, formatting and diff checks,
  with exact failed and successful redacted receipts retained.

No full-Rust, ABI/layout or speed claim follows from this bounded milestone.
