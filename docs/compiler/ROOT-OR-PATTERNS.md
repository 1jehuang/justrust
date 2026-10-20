# Original root-arm OR patterns: private M1

This is a bounded source-language increment, not full Rust pattern support or a
promotion of `tests/fixtures/full_programs/09_vec_stack.rs`. That unchanged program
still requires library/generic/iterator support beyond this slice.

## Admission

Original ordinary match arms and the existing authenticated original builtin
`matches!` client may use root alternatives over bool, integer, char, unit,
wildcard and tuples of those leaves. Tuple alternatives are separate conjunction
rows. Nested OR, binders, guards, references, non-Copy/nominal domains, ranges,
`@`, generated match owners and generated `matches!` owners are not admitted.
Other pattern clients retain their old grammar. In particular, the stock-valid
function-parameter fixture retains the preexisting native Syntax parser boundary.
It is not classified as invalid Rust, and no parameter grammar is broadened.

Duplicate or subsumed alternatives are valid Rust with warning semantics. This
increment reports explicit Unsupported after supported semantic obligations have
been checked. It does not implement unreachable-pattern warning or lint parity.
Existing non-OR arm policy and semantic errors are preserved.

## Graph and authority

No kinds, global joins, resource ceilings, mutable published values, or new
source authority are added. Existing per-callable source/BODY/TEMPLATE_BODY,
lowering and analysis graph facts remain the operation owners. CHECK still
validates unused source bodies and does not produce OBJECT.

Typed `MatchPattern::Alternatives` owns a domain and ordered flat conjunction
rows. Each test has an exact checked tuple projection and bool/int/char terminal.
Every row is checked, including dead or impossible rows. Empty conjunctions are
locally well typed but are **not source authority**. The existing private
canonical checked-body Arc seal authenticates row count/order/tests/projections,
scrutinee and shared arm-body identity. A mutated or foreign body Arc cannot
reuse that publication, including mutation of a row to an empty conjunction.

CFG evaluates the scrutinee once, tries rows left to right, cleans each row's
comparison temporaries on success and failure, and enters one shared arm body on
the first success. Existing ownership analysis remains mandatory.

## Accounting

Parser rows reuse the existing requested-capacity prepayment and actual-surplus
admission before writes. Pending outer headers use BudgetVec; nested decision
and projection scratch retains its existing producer leases. Publication pays
outer row headers, inner test capacities and projection capacities through the
existing retained-vector producer APIs. Clone/substitution preflight accounts
actual nested capacities, including empty row headers. Typed-row validation
meters row headers, tests, projections and duplicate-path comparisons, with one
cumulative existing work bound across the whole alternative set.

Requested capacity is prepaid, not a claim that unknown allocator surplus was
physically prepaid. Surplus must be admitted before writes/publication. Scratch
and retained outputs coexist under their respective paid storage. No new Arc
conversion or independent budget is introduced by flat rows. Existing canonical
body publication owns its wrapper accounting. Resource rejection stays sticky
and cannot be converted into exhaustiveness, usefulness or source authority.

Review follow-up: both typed alternative validators reject at entry when the
caller ledger already failed, preserving its exact first failure without another
charge. The original OR span check pays one Work unit and uses `render_span` for
exact snapshot/range/UTF-8 authentication, not diagnostic line-prefix scanning.
Regression controls cover prior Bytes and Work failures with spare Work, retries,
and constant parser work across a 128 KiB same-line whitespace prefix.

These are scoped accounting claims, not complete CFG accounting. Inherited
`decision_pattern_test` helpers still allocate comparison vectors and clone a
projected place without paying the second projection capacity. Existing
block/statement pushes pay element bytes rather than geometric backing capacity;
scope headers and live-local backing pushes also need admission. Adjacent
cleanup collections reachable from decision lowering need separate inventory.
Those helpers predate OR and remain separate design/review work, not repaired by
the two-slice follow-up. No fully paid CFG claim follows from the tests here.

## Evidence boundaries

The frozen stock matrix has 32 original cases plus separately approved I9.
I8 is preserved as the lexical `|=` negative. I9 uses exact `0 | =>` and rejects
a trailing OR separator in both stock profiles. The exact original source32
manifest hash is `8a1d32ebe4105e5dd74c9e361c76db11f1b6c15762a1323b54ab060f8993b658`.
I9 source hash is `957d6aa82e2c357054e56dd697df1318030487a6c49584a00ec18eb21d0f3c44`.

Native workflow tests embed those exact bytes, keep stock-invalid and policy
exclusions separate, and run CHECK/BUILD with workers 1 and 8. Independent
runtime counters cover scrutinee once/body once and fallback once. Source-seal,
malformed later-row, shared CFG body and resource tests are separate controls.
No full suite, main publication, compiler installation, speed claim or corpus
promotion follows from private scoped gates.
