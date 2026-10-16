# Findings: where agent build time goes

## 2026-10-16: Fixed arrays preserve operand effects and indexed storage reads

Implemented bounded `[T; N]` values, left-to-right literals, once-only repeats,
contextual empty literals and decimal literal indexing. Every array is an owning
indirect value, even lengths zero and one. Empty backing has a zeroed eight-byte
sentinel in this private ABI. Repeat and large-copy code generation uses bounded
native loops rather than expanding one instruction sequence per copied word.
This is a code-size/correctness property, not a measured performance claim.

Stock oracles established that `[s; 0]` still consumes a non-Copy operand and
`let _ = a[0]` still requires initialized array storage. CFG Inspect marks that
non-consuming indexed-base read without moving the enclosing value or unrelated
siblings. Assignment snapshots RHS before inspecting the LHS, allowing RHS code
to initialize a previously uninitialized array. Non-Copy moves through any index
remain invalid, including nested fields. Distinct element move paths keep enum
selection proofs isolated, and whole/element writes invalidate appropriate facts.
Shared limits fail closed before expanded path allocation.

The exact historical `fn main() { let x = [1, 2]; }` fixture was promoted unchanged
from rejected to native execution. The independent 30-case array corpus contains
12 supported programs, 13 invalid programs and five legal-unsupported boundaries.
Coordinator stock-only verification passed all 30. Runtime usize indices, array
patterns, slices, general constants and later-use inference remain unsupported.

Final frozen hosted gates passed:
- check `20261016-000203898-1877035`;
- all 591 tests `20261016-000204707-1877151`;
- strict all-target lint `20261016-000206557-1877244`.

Three exported profiles are retained in
`data/runs/prototype-bootstrap/2026-10-arrays.jsonl`. Formatting and diff checks
passed, independent read-only review reported no remaining blocker. Tracked
M8a run `compiler-arrays-20261016-0703` at source revision `854415ba972f`
passed all 505 cases across eleven corpora: 310 accepted parity cases, 150 invalid
rejections and 45 explicit legal-unsupported boundaries. All raw JSONL receipts
are retained as `experiments/results/20261016-m8a-arrays-*.jsonl`. The native
96-worker example executed successfully and retained complete18-node capture,
23edges, zero failures and zero dropped events. Its observed total10,169,938ns
and engine2,858,755ns are contended/uncontrolled example observations, not cold
or comparative performance measurements. No full-Rust or speed claim follows.

## 2026-10-16: Payload enums preserve explicit selection and initialization facts

Implemented unit/tuple/braced enum variants, mixed struct/enum declaration DAG
validation, source-ordered constructor snapshots, top-level variant payload
patterns and singleton irrefutable destructuring. Typed Field/Downcast projections
carry nominal owner identity. Cached analysis retains a separate must-selected
variant proof: true edges establish it, joins intersect it and ancestor writes,
moves and storage changes invalidate it. Inactive initialization bits are logical
only and never authorize a physical payload read. Partial multivariant rematches
remain invalid, while singleton disjoint moves and wildcard no-read stay valid.

Native payload values use a checked private tag plus maximum-payload frame layout
and aggregate calling convention, including unit variants of mixed declarations.
Constructors zero all backing before writing tag and fields. Tests poison backing
and verify every inactive word, exercise recursive aggregate returns and check
that singleton typed switches never load an uninitialized/moved tag.

Independent review also caught an accepted-invalid imported variant alias/type
collision. The resolver now distinguishes all variants in the type namespace,
unit/tuple constructors in the value namespace and braced variants as type-only.
Stock-confirmed braced variant/function aliases and local bindings remain legal.
The original historical payload boundary fixture is promoted without source edits.
The 44-case independent payload corpus has 23 accepted, 15 invalid and six
explicit legal-unsupported cases, including deferred tuple-variant rest.

Final hosted gates: check `20261015-234538588-1745345`, test
`20261015-234539387-1745510` (**533 passed**), strict all-target clippy
`20261015-234541842-1745780`. Formatting and diff checks passed. Profiles are
in `data/runs/prototype-bootstrap/2026-10-payload.jsonl`. Agent JSON now reports
variant proof checks, refinements and invalidations, and retained CFG edges include
typed switches. This is not a performance, platform-layout or full Rust claim.

Tracked M8a run `compiler-payload-20261016-0646` on source `a050aed7e`
passed all **475 fixtures** across ten corpora: 298 accepted parity cases,
137 invalid rejections and 40 explicit legal-unsupported boundaries. Full raw
receipts are retained in `experiments/results/20261016-m8a-payload-*.jsonl`.
The real payload example executed successfully with 96 workers, 18 once-evaluated
nodes, seven checked downcasts, three variant refinements, zero failures or
dropped events and complete capture. Its 15,518,330 ns total is explicitly
contended/uncontrolled and is not a cold-build comparison or speed claim.

## 2026-10-16: Typed deferred initialization uses cached flow analysis

Added explicit `let [mut] name: Type;` declarations with fresh shadow identities.
Lowering emits StorageLive without a fabricated value or assignment. First whole
local writes use cached analysis 109 rather than a blanket frontend immutable
assignment rejection. Branch/match joins, missing initialization and immutable
history across moves/backedges use the same fixed-point analysis. Direct semantic
check helpers lower and analyze once, while graph body typing remains pure.

The exact historical wildcard/uninitialized match source is promoted from
legal-unsupported to accepted. The new corpus has 10 accepted cases, seven invalid
programs and three explicit unsupported inference/destructuring boundaries.
Graph tests prove once-only facts, retained failed analyses and no-read wildcard
semantics. Immutable reassignment still fails both checked backend conveniences
and check/build roots, including unused functions. Valid assignment after an
unconditional return is not misclassified. Empty Never matches remain valid.

Final hosted gates: check `20261015-232901274-1612259`, test
`20261015-232901788-1612499` (**465 passed**), strict all-target clippy
`20261015-232903155-1612244`. Formatting and diff checks passed. Exact redacted
profiles are in `data/runs/prototype-bootstrap/2026-10-deferred.jsonl`. This is
correctness evidence, not a speedup or full inference/borrow-checking claim.

Tracked M8a run `compiler-deferred-20261016-0632` on source `573b1dc6b`
passed all **431 fixtures** across nine corpora: 274 accepted parity cases,
122 invalid rejections and 35 legal-unsupported boundaries. Full raw receipts
are in `experiments/results/20261016-m8a-deferred-*.jsonl`. The real native
example ran successfully with 96 workers, 14 once-evaluated nodes, no failures
or dropped events and complete capture. Its 10,068,017 ns total is explicitly
contended/uncontrolled, not a cold measurement or performance claim.

## 2026-10-16: Fieldless enums and match preserve static ownership paths

Implemented nonempty unit-variant nominal enums and native match expressions over
enums, bool, unit and i32, including simple variant imports, catchall bindings,
arm-local scopes and finite exhaustiveness checking. Enums remain non-Copy even
though this first private representation is one word. `ENUMS.md` defines the
exact first-slice contract and explicitly deferred payload/guard/pattern features.

Variant tests lower to nonconsuming discriminant reads, not fabricated Copy
operations. Scrutinee places are addressed without eagerly moving them, while
rvalue effects occur once. Native emission consumes the existing cached CFG and
analysis 109 without a second lowering or analysis pass. JSON/retained accounting
now includes enum declarations.

Independent stock probes caught two subtle correctness obligations: singleton
variant matches need no read even after a move, and unreachable pattern arms
still affect ownership checks and post-match initialization joins. Constant-true
CFG switches preserve the latter semantic paths while execution selects only the
real arm. Later unreachable constructor tests retain their required tag reads.
Imported variant spellings in unsupported parameter-pattern positions no longer
silently become local bindings. All arm types remain checked, including dead
break types. The differential corpus preserves existing type-alias/grouped-use/
uninitialized-let sources as legal-unsupported boundaries, rather than counting
their rejection as language support.

Final hosted gates, nightly-2026-10-08 on the 96-core host:
- check `20261015-231940165-1535781`, passed.
- test `20261015-231941164-1535880`, **434 passed**, no failures.
- strict all-target clippy `20261015-231942944-1535725`, no warnings.
- `cargo fmt --check` and `git diff --check`, passed.
Profiles are retained in `data/runs/prototype-bootstrap/2026-10-match.jsonl`.
The new source corpus has 59 independently stock-classified fixtures: 26 accepted
parity cases, 20 invalid programs and 13 explicit legal-unsupported boundaries.
Tracked M8a run `compiler-match-20261016-0621` on source `c24c0cfb6` passed
all **411 fixtures** across eight corpora: 263 accepted parity cases, 115 invalid
rejections and 33 explicit legal-unsupported boundaries. Full source-bearing
receipts are in `experiments/results/20261016-m8a-match-*.jsonl`. The 96-worker
example compiled and executed with three functions, one struct and one enum;
36 CFG blocks, 41 CFG edges, 75 move paths and nine moves were retained. Graph
capture was complete with zero failures or dropped events. The raw timings are
contended/uncontrolled, not cold measurements or a speed claim.
These gates are not proof of full Rust support or a performance improvement.

## 2026-10-15: Irrefutable let patterns consume only selected places

Implemented nested tuple/struct let patterns, wildcard, per-binding mutability,
renames/shorthand, whole-pattern annotations and struct `..`. `PATTERNS.md`
records the exact API and ownership contract. The frontend validates the complete
pattern before installing new binders, while the initializer resolves in its old
scope. Explicit fields enforce visibility and type accessibility even for `_`.

Checked IR carries a bounded flat projection plan. Lowering addresses local and
temporary-rooted field chains without eagerly consuming the parent, materializes
non-place roots once, then consumes only selected binding paths. Wildcard/rest
add no read or move, including after prior whole/partial moves. Rvalue initializer
effects still happen. Divergent initializers never create impossible field loads.
Existing analysis 109 and the native backend need no new semantic instruction.

Review caught invalid trailing commas after struct rest and retained both cases
as negative fixtures. Resource review replaced per-binding parameter scans with
a cached set, bounded iterative pattern type equality, and shared-tuple Copy
classification that visits distinct handles rather than expanding a DAG. Checked
plan bounds are 65,536 bindings, depth 128 and one million comparison/projection
work units. These are explicit implementation limits, not language limits.

Final hosted gates (nightly-2026-10-08, 96-core host):
- check `20261015-230233651-1393741`, passed.
- test `20261015-230234847-1393876`, **356 passed**, no failures.
- strict all-target clippy `20261015-230236326-1393951`, no warnings.
- `cargo fmt --check` and `git diff --check`, passed.
Exact profiles are in `data/runs/prototype-bootstrap/2026-10-patterns.jsonl`.
The historical tuple-destructuring rejection is now a positive check/build and
execution-parity fixture with its original source preserved. Independent source
fixtures cover wildcard versus underscore bindings, ignored moved fields,
initializer effects/order, shadowing, privacy and selected-path move failures.

Charged M8a run `compiler-patterns-20261016-0604`, source revision `4ea263270`,
passed all **352 fixtures** across seven corpora: 237 accepted execution-parity
cases, 95 invalid rejections and 20 explicitly legal-unsupported boundaries.
The new pattern corpus contributes 31 accepted cases, 20 invalid rejections and
six boundaries. Unsupported boundaries are not claims of language support.
Complete source-bearing receipts are retained in
`experiments/results/20261016-m8a-patterns-*.jsonl`.
The 96-worker native example compiled and executed successfully: three functions,
two structs, 11 CFG blocks, 81 analysis move paths and six moves. Its graph had
18 nodes and 23 edges, with zero failures or dropped events and complete capture.
Recorded timings are contended/uncontrolled, not cold measurements or a speed claim.

Tuple rest, ref/ref mut, @, refutable/match patterns, destructuring assignment,
parameter patterns, let-else, general backward inference, borrowing and destruction
remain open. No full-Rust or performance claim follows from this milestone.

## 2026-10-15: Nominal structs and source-level partial moves

The clean-room compiler now accepts nonempty braced named structs, nominal type
annotations and import aliases, source-ordered construction, field privacy,
whole/partial moves, mutable reinitialization and aggregate calls/returns. Each
compilation prepares one immutable nominal environment. Identity uses a checked,
non-reused snapshot namespace and declaration index, not structural field equality
or an unversioned index that could escape its owner.

The typed IR retains source field evaluation order. CFG lowering snapshots each
initializer before reordering operands to declaration order, and consumes only
the projected place. Scalar fields copy without moving their containing struct.
Non-Copy fields move their subtrees. Tuples containing structs are also non-Copy.
The existing cached analysis fact 109 gates both checking and code generation.
The graph does not repeat lowering or analysis in the emitter. Standalone checked
emission helpers now lower and analyze once before calling the trusted raw emitter.

Review fixed inferred private-type access through call results, field results,
intermediate assignment paths and divergent constructor values. It also caught
width being mistaken for declaration depth and exponential inhabitance traversal
of compact shared declaration DAGs. Depth is checked dependency-first, inhabitance
visits distinct definitions, nominal debug output is shallow/bounded, and field
lookup uses a per-definition index. Analyzer traversal remains budgeted by expanded
work before path allocation. These are correctness/resource fixes, not speed claims.

Final hosted gates on the 96-core host, nightly-2026-10-08:
- check: `20261015-225029653-1291328`, passed.
- test: `20261015-225030192-1291440`, **278 passed**, no failures.
- strict all-target clippy: `20261015-225031346-1291549`, passed without warnings.
- `cargo fmt --check` and `git diff --check` passed.

The exact run summaries are in `data/runs/prototype-bootstrap/2026-10-structs.jsonl`.
Tests cover checked convenience rejection, field moves and snapshots, recursive
aggregate ABI, privacy, nominal identity, DAG resource limits, and exactly-once
shared graph analysis for both roots. The obsolete tuple-era struct-declaration
rejection was promoted to a positive check/build regression, not silently deleted.

Empty/unit/tuple structs, derives, destructuring, references, loans, lifetimes,
destructors, enums, platform layout and full Rust remain open. This is not a
completed borrow checker or a general Rust performance comparison.

Tracked experiment `compiler-structs-20261016-0552` ran committed `0bc7d5900`
on the shared M8a host and passed **all 295 source fixtures**: 205 accepted
execution-parity cases, 75 invalid rejections and 15 explicitly unsupported legal
boundaries. The new nominal corpus contributed 19 accepted, 20 invalid and two
unsupported cases. Workers 1/8 cover native check/build in the focused corpora.
The 96-worker real struct example also ran successfully: 807 source bytes, three
functions, two structs, 12 CFG blocks, 11 CFG edges, 136 move paths and 12 analyzed
moves. Its work graph retained 18 nodes/23 edges with no failures or dropped events.
Full six-corpus receipts and the observed example accounting are committed under
`experiments/results/20261016-m8a-structs-*`. This is correctness evidence on a
contended/uncontrolled host, not a cold benchmark or a full-Rust speed claim.

## 2026-10-15: Cached initialization and move-path analysis gates native emission

Per-function operation 109 now consumes the immutable CFG from operation 108.
Check and object generation both require its successful analysis fact, so neither
root repeats the fixed point or bypasses a rejection. Completed analyses remain
available after success and failure, with original function indices on partial
failures. CLI JSON reports move paths, worklist pops and static move operations,
and explicit traces retain analysis reports and diagnostic provenance.

The dataflow model tracks must/may live storage, must/may initialized move paths,
aggregate shallow initialization and immutable assignment history. Whole moves,
partial moves, ancestor requirements, source-order operand consumption and new
storage lifetimes have separate transfer rules. Diagnostics are emitted only
from converged reachable states; structural checks include unreachable blocks.
This remains an internal foundation, not a borrow checker or new source syntax.
All currently admitted source types are still Copy.

Independent acceptance adds 25 tests, including a 32-program concrete-path oracle,
and six private resource/bitset regressions. Review found and bounded repeated
large-type comparisons before structural validation. Block/local/path/state,
operation/work and diagnostic limits fail closed. Exhaustion in the final
replay pass is tested separately from solver exhaustion. Bit operations are
checked across every interval through 131 bits, including empty ranges and word
boundaries. No speedup is claimed from adding this correctness gate.

Hosted check `20261015-223750587-1164403`, all 209 tests
`20261015-223820066-1171340` and strict all-target Clippy
`20261015-223824005-1172230` passed. Profiles are retained in
`data/runs/prototype-bootstrap/2026-10-analysis.jsonl`.

Charged experiment `compiler-analysis-20261016-0540` tested exact revision
`9bc87da47194e2d989f51a57aad778a6cf088f0c` on the dedicated 96-core M8a.
All 254 fixtures passed: 186 accepted parity, 55 invalid rejections and 13
explicit legal-unsupported boundaries. The 96-worker tuple example ran correctly
with 35 CFG blocks, 39 CFG edges, 100 CFG locals, 192 analysis move paths and
39 worklist pops. Its source values remain Copy, so analysis_moves was zero.
Complete corpus receipts are `experiments/results/20261016-m8a-analysis-*.jsonl`.
An earlier attempt `compiler-analysis-20261016-0539` failed connecting to a stale
SSH host address before workload execution. Refreshing the authenticated remote
environment fixed it. These are correctness regressions, not speed measurements.

## 2026-10-15: Cached explicit control-flow facts replace structured native lowering

Revision `38da4ff44300d78a584219fa85ed58c9b57e72d1` introduces per-function
lowering operation 108 after semantic body operation 102. Check and build share
that exact validated CFG fact. Native object emission consumes its ordered
assignments, place snapshots and explicit branch/call/return edges. The previous
structured native emitter is removed, rather than maintained as a second path.
A direct checked-function API remains a convenience that lowers once.

CFG facts retain expression-ID provenance and shared post-compilation handles.
CLI accounting distinguishes CFG blocks/edges/locals from the scheduler's work
graph, and explicit traces include the actual retained CFG. No CFG is rebuilt
for tracing or the check root. This is not yet ownership, borrow or lifetime
validation: the current verifier checks structural/type invariants. Scope-marker
regressions explore reachable live-storage sets through branches and backedges;
review found and fixed conditional-RHS cleanup and double-dead while-exit edges.

Hosted check `20261015-165904225-945608`, all 178 tests
`20261015-165904998-945762`, and strict all-target Clippy
`20261015-165906475-945873` passed. Exported profiles are in
`data/runs/prototype-bootstrap/2026-10-cfg.jsonl`. An earlier plain check was local
because a hosted timeout had set the client's 10-minute fallback backoff. Final
gates used explicit `--remote` and verified hosted execution.

Charged M8a experiment `compiler-cfg-20261016-0001` passed all five existing
corpora against Rust 1.99.0 edition 2024: 64 generated scalar, 64 generated loop,
38 tuple, 28 Never and 60 module cases. Across 254 fixtures there were 186
accepted parity cases, 55 invalid rejections and 13 explicit legal-unsupported
boundaries. Boundaries are not semantic parity successes. Raw results are
`experiments/results/20261016-m8a-cfg-{scalar,loops,tuples,never,modules}.jsonl`.
The 96-worker tuple example also ran successfully, with retained control-flow
counts distinct from the dependency graph. These are correctness runs, not
representative performance or speedup measurements.

The first broad experiment `compiler-cfg-20261016-0000` correctly surfaced a
stale validation boundary: the module harness still required external modules
to reject, although that feature had already shipped. The compiler accepted and
executed the stock-valid input. Revision `8588a6c19ffb9d98997cd18b6139399fb4e2e318`
moves the same fixture into positive executable/check parity and the full rerun
passes. The initial failure is retained as
`experiments/results/20261016-m8a-cfg-modules-stale-boundary.jsonl`, not discarded
or reported as a compiler regression.

## 2026-10-15: Copy tuples and caller-owned aggregate snapshots

Revision `c37ba8c90973b07e8a2e88de448ff34205f82037` adds nested tuple types,
construction, projections, mutable local-rooted field assignment and aggregate
values through calls, returns and control-flow joins. Structural Copy semantics
are limited to existing scalar/unit/tuple fields. Named structs are not silently
made Copy. Checked types use cheap shared tuple field arrays, not source rechecks.

Each aggregate expression owns a frame snapshot. Tuple arguments pass pointers
to these snapshots and are copied into callee locals before body execution.
Tuple returns copy into a hidden caller destination, returning that destination
rather than a dangling callee-frame pointer. All current leaf slots occupy eight
bytes in this private ABI, not the platform Rust ABI. Aggregate/frame sizes have
a checked 1 MiB limit. Runtime stack exhaustion remains an explicit gap.

Initial hosted gates passed check `20261015-165033688-859052`, 163 tests
`20261015-165034280-859178`, strict Clippy `20261015-165035940-859253`.
Six additional frontend regressions then passed the final 169-test gate
`20261015-165140490-874198`, check `20261015-165139822-874099` and strict
Clippy `20261015-165142156-874598`. All six summaries are exported in
`data/runs/prototype-bootstrap/2026-10-tuples.jsonl`.

Charged M8a experiment `compiler-tuples-20261015-2351` tested the exact committed
revision against Rust 1.99.0 edition 2024: 24 accepted parity cases, 10 invalid
rejections and 4 legal-unsupported boundaries passed at workers 1/8 through
build and check. The separate 28-case Never corpus also passed unchanged.
Coverage includes argument snapshots before mutation, nested projections,
recursive/mutual aggregate returns, caller-buffer lifetime, abrupt control
transfers, unit-field side effects and a 20-field tuple. The four-function
903-byte example also compiled at 96 workers and printed all three expected
success lines. Receipts are `experiments/results/20261015-m8a-tuples-differential.jsonl`
and `experiments/results/20261015-m8a-tuples-never-regression.jsonl`.
These are bounded correctness checks, not representative compiler benchmarks
or proof of full Rust support. Destructuring, tuple comparisons, non-Copy data,
ownership and borrowing remain open.

## 2026-10-15: Loop values and semantic Never typing, validated on M8a

Revision `c39c6402acc55653ef5f7bccb9c4e55c53311156` implements unlabelled
loop expressions, break values, continue and while exits with resolved loop IDs.
Native abrupt edges restore the target's exact temporary depth, preserving outer
pending operands and arguments. Explicit semicolons discard block-like values,
while omitted-semicolon expression statements retain their unit requirement.

Stock-oracle probes exposed a correctness issue in the earlier scalar checker:
runtime divergence had erased the semantic type of unreachable tails. The checker
and typed IR now track semantic type and completion independently. For example,
`1 + { return 3; }` is rejected, a typed dead tail in
`1 + { return 3; 0 }` remains i32, and a boolean dead tail cannot satisfy an i32
return type. Full integer/operator inference is not implemented: pure-Never
nonlogical binary operands are conservatively unsupported, including some legal
comparisons. These are tracked boundaries, not falsely counted as Rust errors.

Final hosted gates passed: check `20261015-164244849-795645`, all 148 tests
`20261015-164245371-795676`, warning-free all-target Clippy
`20261015-164246569-795821`. Exported summaries are in
`data/runs/prototype-bootstrap/2026-10-loops.jsonl`.

Charged experiment `compiler-loops-20261015-2346` ran this exact revision on the
96-core m8a.24xlarge and compared native output against Rust 1.99.0 edition 2024:
64 seeded loop programs matched output and status with workers rotating 1/8/96.
The separate 28-case Never corpus passed 10 accepted parity cases, 15 invalid
rejections and 3 explicit legal-unsupported comparison boundaries, with native
build/check paths at workers 1/8. Raw records, compiler hashes and oracle identity
are retained in `experiments/results/20261015-m8a-loops-differential.jsonl` and
`experiments/results/20261015-m8a-never-differential.jsonl`.
These are correctness experiments, not compiler latency benchmarks or speedup
claims. Labels, for loops, general while expressions and full Rust remain open.

## 2026-10-15: Checked scalar function bodies become immutable graph facts

Revision `2d51558feaea8d4f92b07dc3afb63b2bc3bb7eaa` changes body operation 102
from a check-only success token plus retained syntax into a published checked
function arena. The same semantic traversal produces concrete scalar types,
unique local IDs, typed expression IDs, validated call signatures and explicit
divergence/normal-completion information. Object operation 103 consumes that exact
checked function without rechecking, source-name lookup or inferred backend types.
Check/build still share every body node once and keep independent function work
exposed before waiting. Names remain temporary call-symbol identities.

Hosted check `20261015-163401512-724532`, full test
`20261015-163402100-724713` (123 passed), and all-target warnings-denied clippy
`20261015-163403475-725036` passed. Summaries are in
`data/runs/prototype-bootstrap/2026-10-typed-ir.jsonl`. Seven direct typed-IR
acceptance tests cover shadowed locals, initializer binding identity, inferred
scalar types, canonical calls, divergence, failed unused bodies and native
execution under pending binary/call temporaries. Existing frontend, native
backend, multi-file and scheduler tests remain passing. A read-only independent
review checked parameter copies, temporary-stack returns and Never placeholders.

The IR remains structured and scalar-only. It does not yet implement general
types/layouts, places/projections, ownership, borrow checking or full Rust.
The optional structural validator checks reference bounds/arity, not semantic
soundness, and is not called on the normal compilation path. No speedup is claimed
for this representation change without a matched benchmark comparison.

## 2026-10-15: External source trees compile through an observed loading node

Revision `8907a8b0af275f13816e8d4db5b91a53d933101b` supports `mod name;`
from `name.rs` and `name/mod.rs`, nested file/inline directory rules, and module
privacy without invoking another compiler. Source operation 107 captures a bounded
expanded snapshot before parsing, with original byte/file counts and failures.
Loading is sequential and coarse, not an atomic filesystem snapshot or a final
per-file dependency architecture. Expanded positions are not original source maps.

Hosted check `20261015-162923136-686611`, full test
`20261015-162923692-686710` (106 passed), and warnings-denied all-targets clippy
`20261015-162924892-686868` passed. Relevant summaries are retained in
`data/runs/prototype-bootstrap/2026-10-external.jsonl`. Twelve external-module
acceptance tests include missing/ambiguous files, privacy, unused modules, nested
paths, lexical boundaries, failed-source graph capture and trace/input aliasing.
Trace creation is deferred so writing a trace cannot satisfy a missing module.
Nonregular files, active inclusion cycles, invalid UTF-8 and resource-limit
violations are rejected. These limits are not full Rust language support.

Charged M8a experiment `compiler-external-20261015-2331` built the committed
compiler and the four-file, 356-byte example. All six compile/check samples at
1/8/96 workers completed with complete capture and zero failures/dropped events.
All three native programs matched stock Rust 1.99.0 stdout, stderr and exit status.
Raw provenance and samples: `experiments/results/20261015
... (output truncated)
## 2026-10-16: explicit generic templates and sealed native instances

Implemented GENERICS.md's explicit, reference-free Sized parameter foundation.
Every generic declaration, including unused bodies, receives universal typing
and the existing ownership solver with rigid opaque non-Copy parameters.
Capture-free checked-IR substitution remaps all nominal applications, variants,
places, patterns and call targets. Each demanded concrete instance is lowered
again and analyzed before a private exact-CFG native capability can be issued.
The graph checks every declaration before planning the bounded native frontier.
CHECK requests no OBJECT, exact recursive instances deduplicate, and growing
instantiation fails with a generic resource-limit diagnostic. Existing
nongeneric reference behavior remains under the same regression gates.

Final hosted gates, retained in data/runs/prototype-bootstrap/2026-10-generics.jsonl:
- check: 20261016-044705895-4155350, passed.
- test: 20261016-044707552-4155543, 1,235 passed, zero failed.
- clippy --all-targets -- -D warnings: 20261016-044710461-4155321, passed.
- cargo fmt --check and git diff --check passed on the same frozen candidate.

The independently stock-classified exact 28-case artifact has SHA256
b792c18770c85af3754d32ac6298a0a5277a868ce314448e212dc25f411c4d97.
It contains 16 positives, eight invalid programs, three supported-language
boundaries and one expanding-instantiation resource case. The latter retains
stock metadata success versus stock BUILD recursion-limit rejection, without
claiming invalid Rust or attempting runtime execution. All 28 native fixtures
and four graph gates pass with bounded workers 1/8. The real
examples/compiler/generics.rs emits and executes an independent native ELF with
eight workers and prints `generic instances compiled correctly`.

Actual-patch reviews caught and closed retained relocation-symbol accounting,
nominal schema/name/capacity accounting, and nested dead Enum constant metadata
closedness gaps. Exact same-application forged foreign-parameter metadata now
fails both concrete sealing and universal validation. Existing deep-parser and
120-cast resource regressions remain admitted/rejected at their original caps,
without stack-size overrides. All owners and both read-only reviewers cleared
the corrected source. These are source-level budget checks, not measured peak
memory proof. Independent whole-corpus receipts on the implementation commit
are still pending. No install replaced stable justrust, no source compilation
was delegated, and no speedup or full Rust completion is claimed.
