# Named record-rest match contract

Status: **implementation in progress, not accepted**. Full Rust remains open.
This extends AGGREGATE-MATCHES.md only for named record patterns with `..`.

## Admission and authority

A checked source record pattern may select some or none of its named fields and
omit the others with `..`. Exact nominal declaration and source StructShape::Record
remain mandatory. Validate every explicitly mentioned member, its visibility and
its complete recursively irrefutable subpattern. Duplicate or unknown selectors
reject even if their child is wildcard. Omitted private fields are not mentioned
or inspected as selected members. Construction privacy is unchanged.

Use genuine existing source binding identities and the checked Aggregate domain
with nonempty plain Field projections for actual bindings. Do not synthesize
wildcards, identifiers or binding plans for omitted fields. Empty binding plans
are valid only after exact aggregate and source-pattern validation.

Tuple/unit/enum rest, refutable aggregate leaves, guards, or-patterns and implicit
reference ergonomics remain explicitly unsupported. Existing separate scalar,
enum and unit matches retain their prior support. No new AST, IR or ABI is needed.

## Execution and resources

Evaluate the scrutinee once. Consume only explicitly selected non-Copy binding
projections, copy selected Copy fields, preserve omitted siblings and reject
whole-parent use after a partial move. Bindingless rest patterns do not move the
scrutinee. Keep arm-local cleanup and dead-arm validation unchanged.

Charge actual selected-pattern visits and binding projections under existing
limits. No expansion into all omitted fields and no cap increases are allowed.
Canonical source inventory/replay authority remains unchanged.

## Required acceptance

- Stock-first source corpus including cross-module omitted-private success and
  explicit-private failure, selected partial moves and omitted sibling reuse.
- Native workers 1/8 CHECK/BUILD, no compiler fallback, actual executable output,
  no failure artifacts, separate bootstrap debug and release profiles.
- Public source selector and owner/nominal authority controls with genuine handles.
- Complete unchanged hosted tests/checks/strict lint, formatting and diff checks.
- Retained exact failed and passing redacted run summaries.

No full-Rust, public layout, ABI or speed claim follows from this bounded slice.
