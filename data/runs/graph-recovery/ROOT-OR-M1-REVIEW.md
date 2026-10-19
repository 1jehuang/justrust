# Root-arm OR M1 independent review

Frozen private source: `bd912d00363e0f5e77ed7f3d3bff0089986da7bd`.
Private evidence: `7a9dda14cd42924e1dc996f51634409269b59fd7`.
Neither checkpoint is imported by this note. Reviews were read-only source
inspection, not independent test execution. Main's published owner acceptance
remains 3594/3594. Corpus remains 3/20, including no promotion of program 09.

## Bounded source authority and correctness

Independent review found no confirmed correctness/authority blocker in the
original binder-free root-arm scalar/tuple slice. The two parser clients are
ordinary match arms and the existing authenticated original builtin matches.
Separator occurrences must be Original and expansion-free. Captured macro
spelling is reissued with expansion provenance, so copying original spelling
does not grant original owner authority. Non-arm/nested/binding/guard/reference/
nominal exclusions and the preexisting parameter Syntax boundary remain.

Every supported row is collected before coverage; rows update the existing
exact matrix in order. Popped pending rows are reversed back to source order.
Dead arm bodies are checked and joined before deferred redundant-alternative
Unsupported. Resource errors must not become usefulness/exhaustiveness errors.
The canonical checked-body Arc seal authenticates row count/order/tests,
projections, body and scrutinee. An empty conjunction is locally typed, not
source authority. CFG obtains one scrutinee and one shared arm body, tries rows
in order and cleans comparison temporaries on both row exits.

Private evidence preserves initial failures and final scoped outcomes. Detailed
per-binary scope is documented by the owner's README and test sources; exported
records have aggregate args and do not independently reconstruct full commands.
No full-suite/native corpus promotion is inferred from stock probes or scoped
private gates.

## Two confirmed slice resource blockers

1. `ir.rs:6214` validate_alternative_domain and the row validator do not reject
   a previously failed caller at entry. PlanBudget can accept later cheap work
   after a byte failure, so these new helpers can return Ok under a failed
   caller with spare Work. The current tiny-Work test does not isolate this.
   A separate private repair is approved: preserve the exact first failure at
   entry without fake charges or changing shared budget semantics. Test prior
   Bytes and Work failures and retries for both helpers.
2. `frontend.rs:10399-10403` calls `sources.location(original)` only to validate
   provenance, but location counts characters in the entire line prefix.
   That repeated scan is unmetered. A bounded replacement is approved: charge
   one existing Work unit and call `sources.render_span(original)`.
   SourceSnapshot::span authenticates snapshot identity, file index, byte
   range and UTF-8 boundaries without a prefix scan. Keep exact authority and
   add a long-line work control, rather than removing validation.

Existing MAX_PATTERN_PROJECTION_DEPTH/WORK, MAX_PATTERN_BINDINGS and
decision_work constants are reused, not new constants. Plain local limit
strings classify as Resource before Unsupported through FactsError transport;
they do not themselves latch the caller's first-resource-failure ledger.
The parser's original(...,512) returns on its first metered original occurrence
after the explicit Original/no-expansion guard, not a new 512-hop source cap.

Pending/parser-storage/retained-vector producers passed bounded inspection
under requested-capacity prepayment plus actual-surplus admission before fill,
with the inspected destruction-before-refund order. This is not an exact
physical allocator-peak claim. Missing tests include separate surplus seams
and comprehensive partial-publication/row-cleanup fault cases.

## Inherited CFG accounting debt, not newly introduced implementations

These existing helpers are exercised repeatedly by the new OR rows. They must
not be represented as fully paid merely because the row containers are paid:

- `cfg.rs:2800-2815`: decision_pattern_test uses per-test `vec!` comparison
  storage without dedicated capacity admission.
- `cfg.rs:2848`: `inspected.clone()` allocates copied projection backing.
  decision_place at 2872-2907 pays only the original projection.
- `cfg.rs:3206-3215`: block admits one PendingBlock's element bytes then calls
  blocks.push, whose minimum/geometric capacity may exceed the admitted count.
- `cfg.rs:3218-3225`: statement has the same element-only growth issue.
- `cfg.rs:2766` grows scope headers; live at 3245-3247 grows LocalId backing
  without dedicated capacity admission.

These are requested-capacity gaps, not unknowable allocator-internal physical
rounding. A bounded design for allocation-free comparisons, paid projection
copies and fallible accounted growth was requested separately. No broad CFG
rewrite, new cap, shared API or allowance increase is approved by this note.
The source remains private while repairs and their validation are reviewed.

## Scoped repair review, October 19, 2026

Exact separate private repair:
`20c00b60c263cfa961f54ec9eecd1834539d82d5`. Independent read-only
inspection closes both blockers above, without importing OR support into main:

- `ir.rs:6215-6217` and `6256-6258` reject a prefailed ledger before validation
  or further charges. `has_resource_failure` is read-only and also rejects
  poison. Tests exercise prior Bytes and oversized rejected Work failures,
  both entry points twice, exact original failure and unchanged work/bytes.
- `frontend.rs:10427-10435` pays one Work unit before `render_span`, before
  row allocation. The replacement authenticates exact snapshot/file/range
  and UTF-8 boundaries through bounded indexed lookup and stack-only span
  construction. No prefix traversal or successful payload allocation remains.
- The long-prefix Work-counter equality test is only a metering control: the
  old unmetered prefix scan would also pass it. Source inspection, not that
  test alone, establishes removal of the scan. Normal error String allocation
  remains existing diagnostic debt.

Inherited CFG repairs are a separate private implementation, not accepted by
this review. Owner-reported hosted runs on its uncommitted snapshot passed
seven focused controls (`20261019-010004684-85883`) and 65 OR/authority/unchanged
all-graph controls (`20261019-010013668-86677`), including required 1208.
Existing CFG regression run `20261019-010033657-88030` was **red: 131/132**.
The remaining test's old element-only admission expectation differed by 312
bytes after new container admission. That explanation is not yet acceptance:
any test repair must derive actual capacity admissions and preserve two full
panic copies plus the third one-byte-short failure before payload cloning,
with exact first-failure and sticky retry checks. No charges or graph gates
may be reduced to make the fixture green.

Current main baseline is optimizer acceptance `b85a0b6dd`, full 3627/3627.
That baseline does not include any OR checkpoint. Current-base replay, exact
owner-cap constructor adaptation, independent CFG failure-path review and
central acceptance remain pending. Corpus remains 3/20.
