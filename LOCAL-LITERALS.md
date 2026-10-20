# Scoped local numeric literal constraints

Local unsuffixed integer and float literals can now receive type evidence from
later uses before the ordinary expression checker applies i32/f64 fallback.
This is a scoped scalar increment, not general Rust type inference.

## Mechanism and integrity

`frontend/local_literals.rs` collects lexical equality constraints in local
scratch inside the current body-check graph computation. Union-find classes
retain unknown, integer, float, or rigid numeric domains. Existing parameters,
annotations, suffixed literals and nongeneric call signatures are rigid.
Conflicting integer widths, float widths or literal classes are rejected.
There are no new graph kinds, shared state, result slots, external compiler
inputs, or integrity-baseline changes.

Supported evidence paths include scalar let aliases, arithmetic and comparison
peers, assignments and compound assignments, explicit returns and expected block
tails, nongeneric function arguments/results, nested blocks/if/while, and simple
range-for bindings. Lexical declaration/reference keys preserve hygiene, old
initializer frontiers and shadowing. Shift operand widths remain independent.

Only resolved numeric classes supply initializer expectations. An unresolved
integer/float class supplies no hint, so the existing checker can still obtain
stronger field/generic-call peer evidence before its normal fallback. Every
initializer, literal range, operation, call, return, assignment and generated IR
still passes ordinary checking. A hint never constitutes acceptance or fallback
from an error.

## Acceptance

`tests/local_literals.rs` has 28 stock-differential sources, including the exact
unchanged `03_collatz.rs` and its frozen stdout, later returns/assignments,
transitive aliases, nested uses, integer/float defaulting, explicit/suffixed
rigidity, conflicting constraints, integer range/sign errors, and field/generic
peer regression controls. The stock oracle is external only. Run locally when
hosted workers do not provide stock rustc.

The full-program corpus promotes `03_collatz` only after native check/build/run
and byte-for-byte stdout comparison. It prints:

```text
longest chain under 10000 starts at 6171 with 261 steps
collatz(27) = 111
```

Final validation on composed `ae799cafd` plus this change:

- `cargo fmt --check` and hosted `justrust check`: pass
  (`20261018-193224598-443343`).
- Hosted `justrust test --lib --test graph_integrity --test loops --test compiler
  --test full_programs --test local_literals`: 1,404 tests pass, including all
  five unchanged integrity gates (`20261018-193232382-449668`).
- Hosted `justrust clippy --all-targets -- -D warnings`: warning-free
  (`20261018-193253292-439049`).
- Local `justrust test --local --test local_literals --test format_scalars`:
  all three tests pass, with real stock rustc for all 28 inference sources and
  the existing formatting oracle (`20261018-193156673-419201`). Local execution
  explicitly exercises the installed stock oracle independently of hosted tool
  availability.

For the initial increment, these four task-owned exported receipts were appended
to `data/runs/`.
These are correctness gates under contention, not speed measurements.

## Bidirectional iterable item increment

The next concrete blocker was unchanged `05_gcd_fib.rs`: the `lcm` call requires
its unsuffixed range binder to be u64, and the `fib` call requires the items of
its written numeric array to be u32. Previously the iterable was defaulted
before the body was checked, despite sufficient later evidence.

The same body-local solver now retains an item class for every for statement.
Simple ranges equate their endpoint classes with the binder. Direct array
literals equate all element classes, and repeat literals use their written
initializer class. Numeric constraints from the body resolve the item class
before checking bounds or array elements. Array shape/length comes only from
the actual written literal. No iterable or literal is rewritten or evaluated
by the solver. Normal checking still validates every suffix, range/sign error,
call, pattern, borrow, move and element. Bounds/elements retain source order and
repeat initializers execute exactly once, including length zero.

`tests/iterable_literals.rs` adds 27 stock-differential cases including unchanged
GCD/Fibonacci exact frozen stdout, range/array/repeat item evidence, float array
items, loop/initializer shadowing, evaluated-once ordering, constrained empty
arrays, fixed annotation/suffix conflicts, conflicting uses and overflow. An
independent stock-negative ambiguity control confirms unconstrained empty array
iteration still rejects without publishing an executable. Its legacy native
error remains an explicit unsupported empty-array-inference boundary rather
than guessing an element type.

Final composition `520819783` plus this increment passes hosted check,
1,412 selected library/compiler/loop/inference/corpus/prelude tests, all five
unchanged graph gates, strict all-target clippy, formatting and diff checks.
Local stock inference/iterable/formatting suites pass all five tests. Exact
commands and exported task-owned receipts are retained in
`data/runs/inference-validation/`. Local oracle execution guarantees the
installed external stock compiler is exercised. Stable installations remain
untouched. After composing concurrent heap/dyn/format work, 1,437 hosted tests
passed, including a mandatory stock oracle using PATH `rustc`. Its exact hosted
path/version was not recorded, so no hardcoded system path is assumed. Final
engine-guard composition then passed 28 engine/graph/inference/corpus tests and
strict clippy. The local oracle was verified as `/usr/bin/rustc` 1.98.1
(`48a229cea`, 2026-09-01), LLVM 22.1.8.

The corpus now has three passing programs. This is narrowly expected-type
propagation into known literal shapes, not a general structured solver:
nonliteral array locals, tuple/nominal iteration patterns, reference/slice item
inference and generic/method-call evidence remain separate work.

## Boundaries

### Source-constraint error provenance

Contradictory scalar equalities reject with a typed `Unproved` cause carrying
both conflicting domains and `does not match`, transported through the existing
canonical source capability. A source type error is not `InvalidMetadata`.
No classifier string matching, new side slot or relaxed validation is added.
Five stock-negative unused-body controls cover fixed widths, annotations,
float/integer arithmetic, assignments and return types. Each must reject in
check and build with workers 1/8, complete graph capture and no CLI executable.
The existing wide-integer unused-value gate continues validating overflow,
unsigned negation, suffix conflicts and invalid shifts before code generation.
The local solver has no individual expression span inventory, so this change
does not invent source spans or metadata authority.

This does not add general structured inference, future-use reference inference,
method/generic-call constraint solving, match-pattern inference,
or new iterator forms. Unsupported expression forms contribute no
speculative evidence and remain the ordinary checker's responsibility. A valid
Rust program depending on an unimplemented evidence path can still be rejected.
Full Rust support and the remainder of the corpus remain open. No speed claim is
made.

## Loop and labeled-block result constraints

The body-local solver retains one result term per loop or labeled block.
Break operands constrain their lexically selected target, so later numeric
uses can constrain nested break values and tuple projections before fallback.
Labeled blocks also constrain their written tail. While, while-let and for
targets have unit results, rather than contributing to an enclosing loop.
Nearest matching labels win, and closure return-context depth prevents an
inner closure's transfers from supplying evidence to an enclosing loop.

Loops without a value-producing break contribute the never type. Continue
contributes no result constraint, and a break whose operand diverges cannot
make its target produce a value. This is syntactic constraint collection, not
constant-condition reachability analysis: unreachable written value breaks
still participate in type checking. Missing local target context provides no
authority. Ordinary checking remains responsible for invalid labels, crossing
closure boundaries, illegal value breaks, and unlabeled transfers through
labeled blocks, including in unused functions.

The loop-specific stock differential matrix in `tests/local_literals.rs` has
14 valid and eight invalid sources. It checks native CHECK and BUILD at workers
1 and 8 and compares finite executable stdout, stderr and exit status with
stock. Negative cases additionally require complete observed graph capture and
exclude metadata, resource-limit and work-budget failures. Infinite loops are
validated in unused functions or unexecuted branches, never run by the oracle.
Direct solver tests additionally inspect the retained never type, later u64
evidence, lexical label selection and closure boundaries.
No graph operation, shared state, integrity exemption or compiler fallback is
introduced. The 22-source matrix passes on hosted in
`20261020-044246688-3961792`; direct solver controls pass in
`20261020-044135850-3958210`. The integrated loop/array snapshot passed all
14 graph-integrity gates in `20261020-043905042-3955130` before the next
shared-slice carrier workstream began. Full Rust compatibility remains open.
