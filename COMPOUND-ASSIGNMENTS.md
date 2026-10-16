# Builtin compound assignment contract

Status: proposed exact contract, owner feasibility and external oracle required
before implementation GO. This is another bounded native milestone, not full
Rust completion. User-source compilation never delegates to rustc or another
compiler. Existing private x86_64 eight-byte scalar/aggregate ABI stays unchanged.

## Scope and external semantics

Admit statement-only builtin `+= -= *= /= %= &= |= ^= <<= >>=` on existing
supported places. No traits, overloaded operators, methods, custom AddAssign,
128-bit types, new indexing-through-reference capability, computed-call target,
or general assignment-expression support. Do not claim Rust repr(C) or FFI.
Arithmetic requires identical integer types. Bitwise requires identical integer
types or Bool. Shifts permit any independently typed integer count and return
the destination's exact integer type. Preserve all ten primitive identities.
Compound shifts never infer RHS from LHS or LHS from RHS.

Primitive order is grounded in local rustc_mir_build builder/expr/stmt.rs59-90
and six separately executed stock probes, not extrapolated from simple assignment:

1. Evaluate RHS exactly once into an owning scalar snapshot.
2. Evaluate destination place, current reference carrier and dynamic index once,
   after RHS effects. Perform existing prefix inspections and bounds checks.
3. Read the CURRENT initialized selected scalar after index effects.
4. Apply the existing checked arithmetic/bitwise/shift kernel.
5. Write the SAME selected address, without resolving carrier/index again.

`x += {x=10;1}` reads10 and writes11. A RHS retargeting `p` makes `*p += rhs`
select the new referent. An effectful index runs after RHS and before the old
value is read. There is no pre-RHS reservation, old-value snapshot or fake Copy
of a mutable reference. Overloaded trait calls have different semantics and
are excluded. Compound assignment always reads before writing. A deferred local
is invalid unless RHS initializes it before the old-value read. This does not
turn compound assignment into first initialization.

## Exact representation proposal

Frontend source and checked IR add:

- `Stmt::AssignOp { place, op: BinaryOp, value }`
- `Stmt::AssignIndexOp { array, index, op: BinaryOp, value }`

Source place/index forms retain existing source target representations. Checked
fields use `Place` and `ExprId`. Simple local targets normalize to root Place,
not another competing local-only compound form. Allowed operators are exactly
Add/Sub/Mul/Div/Rem/BitAnd/BitOr/BitXor/Shl/Shr. Reject comparisons, logical
And/Or and all forged operators even in dead checked IR.

CFG adds exactly one dynamic statement:

`Statement::IndexAssignOp { array: Place, index: Operand, value: Operand,
                           op: BinaryOp }`

No new Rvalue shape, reference origins, persistent captured pointers or public
constant evaluator. Static compound lowers RHS once, performs existing static
prefix inspections, then existing Assign with Binary Copy(destination) and RHS
snapshot. No target expressions execute between its old-value read and write.
Dynamic compound lowers RHS snapshot first, existing static prefix inspections,
then index snapshot once, then the new statement. Do not lower dynamic compound
as IndexCopy plus IndexAssign, which recomputes address/bounds and obscures the
single-selected-address contract.

Checked IR and CFG verify destination/array/element types, operator domain,
RHS identity or heterogeneous shift integer type, index exact Usize and canonical
constants before any indexing. Validate all nodes including unreachable ones.
Preserve Int semantic type with Never completion, while semantic Never endpoints
retain existing explicit unsupported boundary. Completion follows RHS then index;
never invent a runtime value on a nonreturning path.

## Native and analysis obligations

IndexAssignOp computes one checked address, preserves it across operand loading
and arithmetic/division/shift register clobbers, loads current element, invokes
the existing exact-width kernel and stores through the preserved address.
Only scalar Copy Int/Bool elements are admitted. No aggregate snapshot, raw
pointer escape or additional ABI layout. Bounds and overflow traps keep exact
existing checked runtime101 behavior. Left shift truncates value bits and only
bad counts trap; full count is checked before CL masking. Bool eager compounds
always execute RHS and preserve canonical0/1.

Ownership preflight meters every Place/operand/closed operator descriptor under
existing budgets. Transfer consumes RHS and index snapshots in their established
order, PhaseRead checks the full conservative array initialized/readable, then
PhaseWrite invokes existing index_write. Unknown-index writes kill whole-array
may-target proofs but do not resurrect initialization or invent a selected child.
Static Copy(destination) plus write preserves read-before-write, reference
permissions, active-loan checks and selected proof invalidation. Borrow use/defs
visit RHS/index and destination carrier exactly as needed, with no new origin
propagation. Authority comes from actual reference kind, not carrier mutability.
Never weaken all-body ANALYZE-before-OBJECT/BUILD trust gates.

## Parsing, static counts and resources

Lex longest compound tokens, including three-character <<= and >>=, without
stealing logical &&/|| or borrow &. Recognize only in assignment statement
context. Preserve quoted-token rejection, source privacy, normalized Deref,
computed-call LHS rejection, scalar hints and existing cast/postfix/statement
boundaries. Do not change expression operator precedence or admit general
compound expressions by accident.

Compound shift count statements MUST join the same per-function private
CountEvaluator memo/work/depth/allocation budget used by ordinary shifts.
Inspect statement RHS against destination width, including dead statements.
Known negative or >=width count rejects native CHECK/BUILD. Existing pure forms,
Dynamic opaque effects, unsupported static-shaped boundaries and completion-first
Never handling remain unchanged. CFG rejects direct invalid Constant::Int counts
independently; no CFG constprop. No fresh1M budget per statement, no unmetered
statement recursion, no retained environment or body fixedpoint.

## Frozen candidate oracle and acceptance gates

Scratch artifact `/home/jeremy/.jcode/scratch/compound-oracle/corpus.json`,
SHA256 `f5bdf3fcfdb06d3d4771f16b0e5db3342e660bc9bce6295036968475965b8b2a`:
exact32,16positive/8declared runtime101/6invalid/2legal-unsupported. Ten per-type
fixtures exercise all ten operators and scalar/reference/aggregate transport.
Six focused positives cover actual order, current carrier, index once, aliasing,
Bool eager effects and RHS initialization before compound read. Eight traps
retain source-bound exact101/stdout and independent stock panic reasons. No
stock metadata phase overrides in this corpus. Stock compile/metadata/runtime
passed independently in root task289263md69 as well as author verification.
No source changes or reclassifications are authorized by this document alone.

Required focused owner gates include malformed/dead operators and endpoints,
single index/address, RHS/current-carrier retarget, old-value read after index
mutation, initialized-read rejection/allowed RHS initialization, immutable/shared
writes, active loans and conservative array proof kills. Include compound shift
static-count metering and huge/negative dynamic counts. Whole hosted test/lint/
format gates, exact32 stock/native+four graph tests, actual real native example
and tracked25corpus receipt follow-through are required before delivery claims.
Do not promote historical boundaries without unchanged-source native acceptance.

## Ownership and GO protocol

butterfly owns frontend.rs source/parser/checker/evaluator and publishes new
source enum variants first. frog owns ir.rs/cfg.rs types/validation/lowering and
publishes checked+CFG variants first. deer owns borrow.rs/ownership.rs visitors,
phase checks and conservative proof effects. badger owns backend.rs single
address scalar update. bear owns NEW tests/compound_assignments.rs, duck NEW
experiments/compound_assignments_correctness.py. Root owns four graph tests,
example, docs, independent receipts and commits. Reviewers stay read-only.
Publish production+fixture COMPILE-READY BEFORE bounded focused tests, then wait
integrated-ready before builds. No implementation GO is implied by this draft.

## Implemented validation checkpoint

The approved protocol is implemented with exact32 stock/native CLI fixtures,
four all-body graph tests and1191 hosted tests passing. Final check/test/lint
IDs are20261016-040609216-3829885,20261016-040610524-3830109 and
20261016-040612566-3829785, retained as three exact redacted profiles in
data/runs/prototype-bootstrap/2026-10-compound-assignments.jsonl. Formatting and
both independent actual-code reviews pass. The1451-byte real example executes
with stock/native stdout parity, including RHS initialization, carrier retarget,
index once, full-width shifts and dynamic division address preservation.
Exactly two historical compound boundaries were promoted only after unchanged
source native execution, preserving stdout/value7. Tracked25corpus run
compiler-compound-assignments-20261016-1108 on4048173fb passes900cases:
544accepted parity,250invalid rejected,35declared trap101 and71unsupported.
Independent parsing verifies832normal worker comparisons,70declared trap runs,
128generated compile/runtime exit sets, source/process hashes and complete
successful graphs. Twenty-six raw corpus/example receipts are retained under
experiments/results/20261016-m8a-compound-assignments-*. The real example
actually executed at96workers, not merely built. Full Rust and comprehensive
performance acceptance remain open, with no experimental installation.
