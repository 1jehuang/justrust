# Nominal field projections

Status: implemented for closed acyclic non-closure nominal applications, with
source, execution and authority gates recorded below. Full Rust and full Iterator
support are not claimed.

## Frozen contract

A struct or enum field may retain a canonical associated projection such as
`I::Item` only under that nominal declaration's exact binder and declared
predicates. The projection's source occurrence, trait declaration, associated
item, receiver, ordered type and region arguments, and snapshot must remain
intact. A projection occurrence is not evidence that its receiver implements
the trait.

Every nominal declaration must be well formed, including unused declarations.
`struct Bad<I> { item: <I as It>::Item }` must be rejected without a declared
`I: It` obligation. No fake callable may supply nominal declaration authority.

For executable closed instances, field projections must normalize through
sealed canonical selection and dependency replay. The result must be a distinct
canonical realized application when its field representation differs from the
raw source application. Never mutate the raw schema, reuse its application ID
with replacement fields, or erase unresolved carriers. Raw and closed residual
projection rejection remains mandatory.

World validation and projection normalization must occur outside the nominal
interner lock. Preserve the existing nominal reentry repair. No new side cache,
shared mutable fact, whole-program node, or graph-integrity ratchet increase is
permitted.

## Historical boundary observations

- Baseline fresh compiler rejects nested `Peek<I: It>` field projection with
  `projection without retained source declaration context`.
- A declaration-scoped source context and exact Struct/Enum retained field
  traversal pass the minimal unused valid declaration CHECK.
- That context-only draft is insufficient and must not ship: both closed struct
  and enum consumers hit the existing closed-field residual projection gate.
- The unchanged unused `Bad<I>` negative was accepted by that context-only draft. This was an
  outstanding correctness failure, not an unsupported-control success.

## Connected implementation and evidence (2026-10-20)

The connected implementation uses declaration-local output 138 and per-application
closed realization output 139. Source interner slots and graph output identities
are distinct domains. The latter requires an actual typed read, exact definition
allocation, declaration and ordered actuals, plus complete source-field and
normalization-dependency replay. No world, mutable interner, engine handle or
scheduler state is retained by the new proof payloads.

The unchanged nested `Peek<I: It> { item: Option<Option<I::Item>> }` consumer
first passes stock compilation/execution and native CHECK/BUILD/execution at
workers 1 and 8 in `20261019-203329022-1917591`, frozen tree
`2257ea41e457a077b7129a06ca1c44972bc65726`. CHECK emits no executable and demands
no codegen. The closed enum and three unused invalid declarations then pass
`20261019-204410003-1968781`, tree `b4994d31249613bf255d21a181bc4590086233b4`.
These are scoped closed nominal acceptances, not full Iterator activation or
full Rust compatibility.

Method selection realizes the entire declaration-owned call signature through
the same borrowed inputs before its unchanged strict receiver comparison. It
retains the original signature for final call publication, where the exact
original/mapped signature receipt is recorded and replayed. Recipe providers
never acquire fabricated source callable contexts.

Instance planning's existing implicit Drop type-substitution consumer receives
the current engine authority explicitly. A closed realized nominal can retain
its exact typed-read wrapper only after engine, caller world/budget, definition,
ordered closed actuals, fields and normalization inventory replay. Source-only
and capability-less contexts still reject graph identities. Identity carrier
mappings remain recorded; the canonical conflict invariant is unchanged.

Closed acyclic non-closure realizations are the current bounded scope. Open or
changed actuals, unsupported nominal-valued normalization frontiers, recursive
realizations, closure readiness and unproved reference-carrier transport must
not silently use an interner or incomplete layout. Authority controls observed
three real bypasses: a foreign-engine child under a current-engine parent,
a typed realization key with the wrong world, and removal of the complete
call-realization receipt inventory during real source publication replay.
The preserved controls reject all three after the bounded repairs in
`20261019-205829089-2031507` and `20261019-205852126-2036835` (seven realization
controls and one full-publication receipt control).

The current-parent integration retains the opaque hidden-witness protocol and
uses the ordinary authenticated opaque path, not nominal realization of opaque
terms. `20261019-211544341-2122510` checks all 337 test targets and
`20261019-211631210-2128430` replays the real call publication receipt after that
merge. Graph feedback exposed a raw source constructor payload paired with a
realized destination. Constructor ingress now authenticates the original source
application, canonical symbolic template, exact ordered actuals and complete
field-normalization receipts before constructing both the payload and expression
with the graph type. Existing graph inputs retain strict typed identity rather
than being remapped as raw source applications.

The focused constructor ingress control includes genuine source/current-graph
positives and wrong source application, foreign snapshot, changed actuals,
wrong declaration and same-world foreign-engine graph rejection. It passes in
`20261019-214110409-2256269`. All 14 graph-integrity tests pass without ratchet or
stack-limit changes in `20261019-214138782-2258342`. The unchanged nested struct
consumer again passes stock/native CHECK, BUILD and execution at workers 1 and 8
in `20261019-214525958-2255968`, on frozen tree
`4cae2fd2449532bdf82593ddd5e5b11d29333ae2`. These gates retain the bounded scope
above. They do not establish reference-carrier, recursive, closure or full
standard-library compatibility.

## Closed call substitution and final integration

Generic BODY instantiation uses the same current-engine borrowed typed inputs for
callee metadata and complete call signatures, including explicit nominal type
arguments. It does not canonicalize colliding queue entries by semantic equality.
The original canonical call signature and the graph-realized signature are
retained separately. Both genuine Call subjects, exact ordered actuals, all
source/typed-capability correspondences and the full reference contract are
validated before the mapped signature adopts the canonical signature's contract
allocation. The mapped Call subject is validated again. The existing strict
receipt contract-identity and call-payload guards remain unchanged.

Closed receipts bind the original template call occurrence, substituted selection,
resolved callee, original/mapped signatures, engine authority and complete ordered
nominal-conversion rows. Replay independently traverses both the authenticated
original template calls and current calls. TraitItem-to-Direct selection and all
ordered owner/method substitutions are replayed, not inferred from a copied
current inventory or generated-shape equality alone.

Actual typed BODY consumers derive whether a substitution proof is required from
the genuine original schema arity, closed target actuals and Source/Recipe
provider role. Removing the whole proof, or both its engine and call inventory,
cannot select a legacy route. The preserved real-consumer omission control passes
in `20261019-230040527-2656238`. Another real bypass altered a retained nominal's
source application while leaving its source definition intact. Replay now binds
the complete source application ID to that independently authenticated definition;
the preserved mutation control passes in `20261019-230236201-2663145`.

The changed-contract negative rejects an invalid mapped contract before canonical
allocation adoption, alongside source-application mutation rejection, in
`20261019-231235265-2701689`. The unchanged forwarded generic Hash consumer then
passes stock/native CHECK, BUILD and execution at workers 1 and 8, including its
missing-bound rejection, in `20261019-231256541-2703029`, tree
`b978209c670252805edee5f869e1ae0379afba94`.

A genuine two-call generic BODY fixture independently rejects omitted, empty,
missing, extra, reordered and wrong-expression receipt inventories, changed
callee/actuals/original selection/contract and foreign-engine receipt rows through
the actual BODY consumer and original-template replay. This passes in
`20261019-232217144-2733864`. Checking all 341 test targets and all 14 unchanged
graph-integrity tests then pass in `20261019-232315842-2737020` and
`20261019-232409929-2742857`, tree
`9b4e2c80a1485be854230adbf52a1e0a52ebb3ec`, without ratchet or stack changes.

Closed-call nominal conversion rows remain local to their call receipts, rather
than being merged into the enclosing body's nominal substitution map. These
represent different substitution contexts. The body map still rejects conflicting
targets (`20261020-031145817-3606040`), and each call receipt still independently
replays its exact engine, selection, signature and conversion inventory. A real
BUILD of the unchanged basic format fixture supplies nonidentity conversion rows
for the authority regression: replacing a converted identity with its source
identity or omitting the conversion inventory is rejected by the actual BODY
consumer (`20261020-031211231-3608364`). The existing full receipt mutation test
also passes (`20261020-031146421-3604869`). The unchanged fixture matches live
stock rustc stdout and stderr after native CHECK/BUILD at workers 1 and 8, alongside
all 14 unchanged graph-integrity gates (`20261020-031050478-3603331`). This fixes
the baseline format BUILD's `conflicting nominal substitution` rejection without
relaxing body substitutions or call authority. It does not complete G0-G12.

Temporary diagnostic dumps were removed. A bounded two-boolean no-progress
message preserves the rejection guard and avoids a new unused-field warning;
its minimal compile check passes in `20261019-232942365-2756546`. Baseline warnings
remain; warning-free linting is not claimed.

The later embedded-function authority parent was carried by exact three-way
merge, preserving its parser, resolver, registration and synthetic-function
metadata. On tree `477aa574efcb11269271bf9690cc8d9237e8265e`, all 341 test targets
check in `20261019-233202311-2762780`, and the genuine embedded helper plus
resolved-mutation authority control passes in `20261019-233242152-2767358`.
The preceding graph and Hash receipts retain their stated frozen-tree provenance;
they were not repeated or misrepresented as this later parent's full acceptance.
All cited successful summaries are retained under
`data/runs/nominal-projections/2026-10.jsonl`.

## Associated factory receiver integration

A chained associated-call result hint replays the complete selected Call
signature through the same borrowed nominal inputs used for method formals,
before extracting its result type. Missing typed reads retain the exact Blocked
inventory and propagate an error, never a raw result hint or `None` fallback.
Strict receiver comparison and final original/mapped source-call receipts remain
unchanged. No new graph node, shared state, type-equivalence exception or separate
authority protocol is introduced.

The explicit `Holder::<u32>` fixture covers both enum and struct factory results,
without depending on inferred static owners or the recovered NonNull fixture.
Stock/native graph CHECK produces no executable and demands no codegen at workers
1 and 8, followed by matching BUILD and execution. On frozen tree `b758c40e`
(parent `519a7ff4`), the positive passes in `20261020-013112547-3150488`.
Unused wrong-argument and missing-member programs are rejected by stock and native
CHECK/BUILD at workers 1 and 8 (`20261020-013623345-3171333`,
`20261020-013731049-3176221`). The genuine caller-facts unit retains the exact
missing demand and rejects callback metadata with a wrong declaration or changed
ordered actual (`20261020-013604879-3170360`). These adversarial metadata callbacks
are not fabricated graph capabilities and do not independently prove current-
engine provenance; successful graph callbacks still use the existing typed-read
capability boundary.

That frozen parent passes all-test-target checking, library clippy and all 14
graph integrity tests (`20261020-013835708-3178487`,
`20261020-013853837-3182240`, `20261020-013907002-3182685`). Clippy exits successfully
with existing warnings, not warning-free. Owned Rust files were formatted without
bundling unrelated baseline formatting changes.

After exact peer static-owner inference carry onto `59f91c4b`, combined tree
`0adee9e2` passes checking of all 345 compiled test targets and the same explicit
positive stock/native CHECK/BUILD/run at workers 1 and 8
(`20261020-014327809-3192995`, `20261020-014347945-3196974`). The prior authority,
clippy and graph receipts retain their `519a7ff4` provenance, rather than claiming
those gates were repeated on the combined parent. The nine authentic summaries
are retained in `data/runs/associated-result-receivers/2026-10.jsonl`.
This is scoped receiver representation integration, not acceptance of the full
NonNull input, all inferred owner forms, full Iterator or full Rust.

## Native constructor regression repair (2026-10-20)

Constructor ordered actuals now replay source-to-graph realization recursively
before exact identity comparison. Existing graph actuals retain their typed
identity, including nested foreign-engine rejection. Generic pattern applications
and method receiver candidates use the same ingress instead of comparing raw
source slots against graph signatures. Declaration, argument order, source schema
and complete realization receipt checks remain intact.

Closed call canonicalization validates the original selection/signature before
adopting the selected implementation's proven reference contract for nominal
representation replay. This preserves refined return pointers without accepting
a forged incoming contract or weakening signature equality.

The three unchanged native generic/trait regression programs pass stock/native
CHECK, BUILD and execution at workers 1 and 8 in
`20261020-035817872-3802212`. Stock alone uses `-Aunconditional_panic` for the
fixtures' existing `1/0` failure sentinels. All nine nominal realization controls
pass in `20261020-035825323-3802520`; expanded constructor nested-identity controls
pass in `20261020-035841880-3803328`, and original-call forged carrier controls in
`20261020-035844372-3803067`. All 14 graph-integrity gates pass unchanged in
`20261020-035829386-3802664`. These are scoped regressions, not full-suite or full
Rust acceptance. An earlier broader shared-tree run still exposed unrelated
native-adapter/scalar failures and two accepted unused-invalid projection cases;
those remain separate follow-up work, not passing evidence for this repair.

## Exact iterator call-result identities (2026-10-20)

Closed call arguments compare canonical declarations and ordered type arguments
only after both values pass the current validated fact authority. Foreign
snapshots, distinct declarations, changed reference kinds and reordered arguments
remain rejected. Generated `for` loops retain the checked `into_iter` result type
for iterator storage and the checked `next` result enum for match patterns and
downcast owners, rather than retaining their pre-realization source slots. The
existing strict pattern and downcast validators are unchanged.

All nine existing iterator-loop tests pass in `20261020-063714390-4138779`,
including unchanged stock/native CHECK, BUILD and run cases at workers 1 and 8
and source-replay rejection controls. The closed-call authority controls pass in
`20261020-063908438-4141399`, unchanged graph integrity passes all 14 gates in
`20261020-063758586-4138697`, and strict all-target clippy passes in
`20261020-063952048-4142401`. This repairs the bounded iterator regression suite,
not the entire iterator library or full Rust compatibility.

## Required acceptance gates

- Stock/native CHECK, BUILD, and run at workers 1 and 8 for nested struct and
  enum field projections, including a real Peekable-style consumer.
- Reject unused missing membership, unsatisfied assignments, foreign source
  owner/module/snapshot, wrong associated item and ordered arguments.
- Exact raw-to-realized field substitution and normalization dependency replay,
  with omission and transplant rejection controls.
- Preserve carrier origins through reference-bearing associated fields. Such
  cases remain unsupported until exact carrier transport is validated.
- Graph integrity and relevant nominal/projection authority regressions.
