# Native Rust compiler implementation

Status: executable Rust-subset implementation with bounded root-reference generic actuals, shared/mutable reference returns, noescape reference calls, local borrowing, genuine usize, runtime indexing, fixed arrays, nominal structs, payload enums and match,
2026-10-16.
The target is the entire Rust language and existing-project compatibility.
See `RUST-COMPATIBILITY.md` for the implementation roadmap and completion gates.

This extends the synthetic runtime prototype with actual Rust source input and
Linux x86-64 ELF executable output. Standard Rust tools build this compiler,
but compiling user input does not invoke rustc, Cargo, LLVM, a C compiler,
an assembler, a linker, or an interpreter. This is an initial Rust subset,
not a replacement capable of compiling arbitrary Rust projects or itself.

## External contract

`jr-rust SOURCE -o NEW_EXECUTABLE [--workers N] [--trace NEW_TRACE]`
loads a bounded UTF-8 source tree, retaining immutable expanded input once loading
finishes. Files are read sequentially, not as an atomic filesystem-wide snapshot.
`--check` validates without generating
an executable and does not require `-o`. Existing outputs and traces are never
overwritten. Invalid and unsupported programs fail explicitly. Output is not
published until every required check and code generation operation succeeds.

Supported integer primitives are `i8`, `i16`, `i32`, `i64`, `isize`, `u8`,
`u16`, `u32`, `u64` and `usize`, with exact type identity, typed literal patterns,
checked arithmetic and normalized private eight-byte scalar transport. Decimal,
binary, octal and hexadecimal literals accept underscores and exact suffixes.
Signed minimum literals support direct or parenthesized negation, not arbitrary
block folding. Integer-to-integer `as` casts cover all 100 pairs, preserving source
signedness before destination truncation/normalization without overflow traps.
Unsuffixed cast operands receive a soft destination hint through literals, unary
operators and block tails, not arithmetic or branch unification. Concrete source
types retain their identity. See `INTEGER-TYPES.md` and `INTEGER-CASTS.md`.
Eager `&`, `|` and `^` accept identical integer types or Boolean operands and
evaluate both operands once, left to right. Logical `&&`/`||` remain short-circuit.
Checked `<<` and `>>` accept independently typed integer counts and return the
LHS integer type. Negative or width-exceeding counts trap101, left shifts truncate
lost value bits, and signed right shifts preserve sign. A bounded typed static
count diagnostic rejects known invalid counts, sometimes earlier than stock
metadata-only checking. This is not general consteval or release wrapping.
Builtin statement compound assignments `+= -= *= /= %= &= |= ^= <<= >>=`
snapshot RHS first, then select the current destination/carrier/index, read the
initialized scalar and update the same address. Arithmetic uses identical integer
types, bitwise also supports Bool, and shifts retain independent integer counts.
Dynamic indices evaluate once with one checked address. Compound shifts share
the bounded static-count diagnostic budget. See `COMPOUND-ASSIGNMENTS.md`.
Overloaded trait assignments, compound expressions outside statement contexts,
128-bit integers, floats and noninteger casts remain unsupported. Parenthesized
cast target types are an explicit boundary.

Explicit type-parameter functions, braced structs and payload enums use rigid
declaration-qualified parameters and canonical nominal applications. Fully
explicit ordered arguments remain supported, for example `id::<u64>(x)` and
`Wrap::<u8>{value:x}`. Calls and constructors can also omit all arguments when
bounded first-order evidence from known operands or a hard expected result
resolves them. Deferred integer leaves default only after all constraints,
while opaque operands cannot force numeric fallback. Partial type holes,
future-use inference and unresolved nested omitted applications remain explicit
boundaries. Every generic body, including unused declarations,
receives universal type and move analysis with parameters treated as opaque
values. Copy permission requires an authoritative canonical Copy obligation,
including declared bounds, rather than a parameter-layout assumption.
Demanded concrete instances substitute checked typed metadata,
then lower and analyze again before sealed native emission. Exact instance
recursion is deduplicated, while growing instantiations fail boundedly. Generic
signatures and bodies admit bounded shared/mutable root reference chains ending
in reference-free types or rigid parameters. Fully explicit root-reference type
actuals, such as `id::<&i32>(r)`, use a common Source and Recipe carrier proof,
not a Clone-only exception. Counted carrier layouts replace the earlier fixed
two-layer domain, subject to unchanged depth, region and analysis-state limits.
Internal tuple, array, struct and enum components transport distinct origins
through certified substitution after an admitted root-reference actual. This is
not permission for external aggregate/nominal reference actuals or explicit
source reference fields. Newly inferred structural-reference actuals remain
unsupported, while existing reference-free inference and rigid forwarding remain.
Positive local trait bounds use the canonical
obligation foundation below. Const generics and generic function values remain
unsupported. See `GENERIC-REFERENCES.md` for transported reference contracts,
borrowed receivers and genuine bounded two-phase loans. See
`NESTED-REFERENCES.md` for the preceding two-layer shared-reference foundation.
`GENERIC-CARRIERS.md` defines the extended root-reference domain, current-cell
semantics, entry-value effects, proof gates and preserved high-arity legacy path.
See `GENERICS.md` for the universal/concrete trust boundary and shared
planning limits, and `GENERIC-INFERENCE.md` for the private evidence grammar,
fallible no-fallback hints and exactly-once ordinary elaboration. This foundation
is not full Rust generic compatibility.

Local nominal inherent impls support associated functions and consuming `self`
or `mut self` methods, including generic impls and explicit method type arguments.
Canonical `Self` types, constructors, variants and qualified associated paths
share the existing checked-call ABI. Ordered impl/method binder environments,
simultaneous substitution and one authoritative callable enumeration preserve
universal validation of every unused associated body. Receiver selection matches
impl parameters against rigid actual types, while a separate bounded overlap
check rejects conflicting same-name items. Receiver and arguments are checked
once in source order. Borrowed receivers use bounded ordered builtin-dereference
and shared/mutable autoref candidates, matching the actual receiver formal before
selecting an implementation. Selected adjustments execute once, with genuine
mutable reborrows retaining the two-phase protocol, not general trait autoderef.
Omitted method type arguments remain unsupported. See `ASSOCIATED-ITEMS.md` and
`GENERIC-REFERENCES.md`.
The associated checkpoint passes1318 hosted tests and988 independently verified
cases across28 corpora, including an actually executed96-worker example. Raw
receipts are retained under `experiments/results/20261016-m8a-associated-items-*`.
These are bounded correctness gates, not full Rust or performance certification.

Local nongeneric traits support required static and owning-value methods,
including signature-only `Self` results, canonical inline `T: Trait + Other`
bounds, explicit UFCS, unique bound/scoped trait method lookup and conditional
generic nominal implementations. Every header is validated for conformance and
coherence before any body succeeds. Required signatures have no executable body
or native slot. Every direct call discharges its declaration prerequisites,
while abstract trait calls retain real trait/item identities until unique closed
implementation selection. The same exhaustive call-closing pass covers concrete
instances and reused nongeneric bodies, including dead metadata. Exact closed
facts and successful ownership analysis remain prerequisites for native emission.
The obligation engine uses one shared ledger, bounded memoization and
operation-local cycle detection. Required declaration `Self` is not silently
assumed Sized in executable code. Local traits merely named `Copy` grant no Copy
permission. Trusted canonical core/std identities now provide Sized, Clone, Copy
and Drop declarations, with real supertrait closure and default Sized bounds on
ordinary source parameters. Explicit nominal Copy implementations require Clone,
all owned fields to be Copy, and no canonical Drop implementation. Primitive,
product and shared-reference Copy evidence uses the same obligation engine.
Borrowed required receivers and projected reference returns use canonical input
envelopes and private implementation refinements, never invented opaque fields.
Canonical Clone recipes are genuine universally checked typed bodies. Tuple and
array recipes call each child's Clone implementation, including observable manual
Clone on Copy types. Shared-reference Clone loads the original inner carrier,
with world-certified declaration-bound topology transport rather than a fresh
contract derived from coincident concrete types. Every source body is checked,
and generic source bodies plus every admitted recipe pass universal lowering and
analysis. Demanded instances pass closed analysis before native publication.
Logical Copy/Clone membership alone does not authorize a receiver topology or
reference-valued executable instance: counted layout and the exact common
universal carrier proof remain required. A real source clone_from override runs
normally, while its unavailable canonical default fails before instance admission.
General default bodies, trait or trait-method generics, associated types, trait
objects, destructor execution and full standard-library integration remain
explicit boundaries. See `CANONICAL-COPY.md` for the catalog, topology, provider
and resource contract, and `TRAITS.md` for the underlying obligation foundation.

Initial supported syntax: free functions with these integers, `bool`, unit, nested tuples
and nonempty braced named structs or enums and fixed arrays as parameters/results, aggregate literals and
numeric/named field projections,
local bindings and shadowing, typed deferred local initialization,
irrefutable tuple/struct let patterns, wildcard,
per-binding `mut`, struct-pattern `..`, whole-value and local-rooted field assignment,
integer arithmetic, comparisons, short-circuit Boolean operators, blocks, `if`/`else`,
`while`, unlabelled `loop` expressions, `break` with supported value types,
`continue`, `return`, enum/scalar `match`, calls including recursion,
and `println!("literal")`. Match supports variant/literal, wildcard and whole-value
binding arms, with exhaustive bool/unit/enum coverage and bounded integer-domain coverage or a catchall. Guards,
or/range and nested refutable patterns remain following work. Unit, tuple and
braced enum variants support source-ordered construction, selected payload
patterns and single-variant irrefutable let patterns. Tuple-variant rest remains
unsupported. See `ENUMS.md` and `PAYLOAD-ENUMS.md`.
Fixed arrays `[T; N]`, array literals, repeat initializers and in-bounds decimal
literal indexing are supported up to 4096 elements, subject to independent
resource limits. Repeats evaluate their operand once even at length zero; indexed
wildcards still require initialized array storage. Non-Copy moves through an
index reject, including nested fields. Contextual empty literals are supported,
while backward inference, slices and array patterns remain following work.
Runtime indices use genuine unsigned 64-bit `usize`, with full-width bounds checks
and owning Copy-element snapshots. Direct ignored non-Copy dynamic indexing uses
readiness and bounds checks without consuming an element. Writes snapshot the RHS
before LHS effects and invalidate all contained enum selection proofs. Chained
dynamic place bases and non-Copy dynamic extraction remain unsupported. Every
array uses the private indirect aggregate ABI, including zero-length arrays.
See `ARRAYS.md` and `USIZE.md`.
Standalone shared/mutable reference parameters support direct calls, recursive
mutation of original caller storage and expected-site carrier reborrows. Formal
referents have separate invocation-lived canonical roots. Mutable calls invalidate
all candidate referent selection proofs without inventing initialization. Every
body must pass analysis before executable publication. Standalone checked-function
emission rejects reference calls without whole-program signature/noescape evidence.
Shared reference results may point to shared or mutable parameter referents,
while mutable results derive only from mutable inputs. Either kind may accept
unrelated inputs of either capability. Results may name typed static tuple,
struct and constant-array subpaths under normalized lifetime contracts. Omitted input lifetimes receive fresh rigid declaration IDs. An omitted
output lifetime on free/static/value functions requires exactly one input
reference lifetime position, even when multiple input positions explicitly share
the same name. Borrowed methods instead prioritize the receiver lifetime. Explicit output bounds
remain rigid, and unrelated anonymous inputs do not gain inferred outlives edges. Complete declaration-derived InputPath candidates, rigid body outlives
obligations and separate transitive lifetime dependencies gate returned pointers.
Each suffix stays correlated with its original argument and source authority.
Private-field getters do not grant callers direct private access. Mutable results
are consumed unique handles, not copied capabilities, and keep overlapping parent
authority frozen until their last use. Input kinds remain genuine capabilities,
while all region-related inputs constrain output lifetimes independently of target
eligibility. See `MIXED-REFERENCE-RETURNS.md`. Enum payload paths, dynamic borrowed
indices, trait-based autoderef, bare reference indexing and general elision
and general source-declared reference-containing aggregates remain unsupported.
Certified internal generic products are covered by the carrier domain below,
not by a general reference-field or aggregate-type-actual permission.
Builtin `r.field` and `r.0` insert one real Deref before field lookup in reads,
assignments and borrows. Privacy and pointee capability are checked after peeling
the reference, without moving its carrier or snapshotting its aggregate. Genuine
reference Call-result field reads use the existing result temporary. Assignment
through a genuine reference Call result snapshots the RHS first, evaluates the
carrier once and uses the same checked dereference assignment. Borrowing Call
rvalues, runtime indexing through references and enum
downcasts through Deref remain unsupported.
See `BORROWING.md`, `REFERENCE-CALLS.md`, `REFERENCE-RETURNS.md`,
`MUTABLE-REFERENCE-RETURNS.md`, `PROJECTED-REFERENCE-RETURNS.md` and
`LIFETIME-ELISION.md` and `REFERENCE-FIELDS.md`.
Typed declarations `let [mut] name: Type;` use cached flow analysis for first
initialization and immutable assignment history. A narrow untyped deferred
reference declaration can acquire its type from its first independently typed
reference assignment in the declaring lexical scope. Backward inference,
underconstrained deferred values and deferred destructuring remain unsupported.
See `DEFERRED.md` and `GENERIC-REFERENCES.md`.
`main` takes no arguments and returns unit. Arithmetic has overflow checks,
matching Rust debug behavior. Runtime arithmetic failure exits 101. This first
runtime does not yet reproduce Rust panic text or unwind behavior.

Inline modules now support nested `mod name { ... }`, ordinary `pub` visibility,
qualified function paths (`crate`, `self`, `super` and relative paths), and simple
module-level `use path;` / `use path as alias;` imports of modules and functions.
Module/type and function/value namespaces remain separate. Resolution validates
unused imports and call privacy, and canonical function names reach the native
emitter without external compilation. The entry point currently must be a
function declared as `main` in the root, not a function imported under that name.

External `mod name;` loads `name.rs` or `name/mod.rs`, including nested file and
inline-module directory rules. Missing, ambiguous, cyclic and oversized source
trees fail explicitly, including unused modules. Limits are 1,024 file occurrences,
64 module levels and 8 MiB of input and expanded source. These are prototype
resource limits, not Rust language limits. `#[path]` is not supported.
Canonical paths detect active inclusion cycles, not enforce a filesystem sandbox.

Not supported: crates/manifests, grouped or
glob imports, public re-exports, restricted `pub(...)`, block-local items/imports,
standard-library APIs beyond the documented print form, general macros, traits,
generic inference/bounds beyond the bounded slice above, empty enums, explicit discriminants, empty/unit/tuple structs,
derives, struct update/shorthand,
general reference results and escaping references beyond bounded static input paths,
general lifetimes, strings as values, closures, async,
FFI, labels/labeled blocks, `for` loops, while expressions in general value
positions, or other targets.
This list does not turn unsupported syntax into permission to accept programs
with incorrect semantics. Diagnostics and acceptance tests must police the
implemented boundary. Semantic coverage must be expanded explicitly.

## Graph operations

- For filesystem inputs, load and expand the source tree once (operation 107),
  retaining source-file/original-byte accounting and failure events.
- Parse immutable source into a retained unresolved module/declaration arena.
- Resolve paths, imports and visibility once, publishing canonical item identities.
- Validate declarations, reject recursive by-value types and publish one immutable
  nominal type environment with callable signatures (operation 101).
- Check every source body and available registered recipe against authoritative
  signatures (operation 110). Generic source bodies and every recipe, including
  zero-arity recipes, lower universally (111) and pass the same move/dataflow
  solver (112), including unused declarations.
- Every universal body has its own admission fact at kind 115. It reads that
  body's exact universal proof (112) plus only the universal callees needed for
  reference-actual admission, then validates the rule without a whole-program
  join. Nongeneric Source bodies remain closed-only and supply no early body
  summary. Closed analysis uses direct callee proof reads, not a global table,
  so kind 115 depends on neither closed analysis (109) nor the final concrete
  barrier (114), avoiding a caller/body-summary dependency cycle.
- Seal a bounded deduplicated instance frontier seeded by every nongeneric
  source function, with recipe instances admitted only on demand.
  Concrete bodies (102) reuse nongeneric facts or instantiate verified
  typed templates, never source text. Calls and nominal applications use resolved
  declaration identities and ordered type arguments, not display names.
- Lower each concrete checked instance once into an explicit control-flow graph
  (operation 108), with ordered value snapshots, branches, loop edges and calls.
  Publishing the CFG does not imply it passed initialization analysis.
- Analyze each CFG once (operation 109), validating structure and running bounded
  definite-initialization, storage-lifetime, move-path, loan-origin and bounded
  signature-region dataflow to fixed points.
  Closed analysis can use selected-body effect refinements only after exact
  target, template, substitution and facts validation against its callees'
  proofs. Missing summaries retain conservative declaration envelopes, not a
  fabricated family proof.
  Both check and emission require this successful immutable analysis fact.
- Join all concrete analyses in one cached publication barrier (114) before
  requesting any native object. Unused invalid nongeneric bodies therefore
  cannot race an unrelated object's publication.
- Emit each instance only from a privately constructed `ConcreteCfg` tied to that
  exact immutable closed CFG and its successful analysis, resolving call symbols
  exclusively through the sealed plan. Parameter-bearing types have no layout.
- Link emitted functions and resolve relocations into one executable.
- A check root joins all required semantic checks but demands no code generation.
- A build root joins emitted objects, then executable publication.

Recursive calls depend on declarations, not completion of the callee body.
All independent function prerequisites are demanded before awaiting results.
Within one graph, concurrent check and build requests share parsing, resolution,
declarations, body checks, per-function CFG lowering and initialization analysis.
Check demands the same successful analysis facts as native codegen, without
demanding objects or linking.
Output facts contain typed-arena handles because
the prototype runtime currently stores u64 slots. Arenas remain owned by the
compilation snapshot and are not persistent cache keys.

Checked functions retain resolved local and expression IDs, concrete or rigid
declaration-parameter types, validated call signatures and explicit normal-completion versus
divergence information. Every shadowed binding has its own LocalId. The native
emitter consumes these facts directly, without source-name lookup or inference.
This structured typed IR feeds explicit CFG lowering before native generation.
CFG blocks contain ordered assignments to owning local/temporary places and
explicit conditional, call, return and unreachable terminators. Short-circuit
operators become branches, and loop exits become ordinary edges with resolved
destinations. Source-expression IDs remain attached to operations for provenance.
The CFG is a prerequisite for ownership analysis, not a completed borrow checker,
platform layout or public cross-version IR format.
In the checked IR, semantic `Ty::Never` is distinct from runtime non-completion.
Explicit unreachable tails retain their actual type and are still checked. Pure
never operands to nonlogical binary operators currently receive an explicit
unsupported diagnostic. This deliberately includes some legal stock comparisons
pending integer/operator inference, rather than accepting invalid programs.
`break`/`continue` targets are resolved loop IDs. CFG edges preserve enclosing
expression snapshots and end exited storage lifetimes. A direct break/continue in
its own while condition is rejected, while a nested loop has its own target.


Source-tree loading and text expansion currently form one coarse operation.
String-input library calls retain their original parse-first graph.
The parser consumes expanded source as one operation. Name resolution,
declaration validation and executable linking are also initially single
operations. These are explicit implementation limitations, not architectural
claims that all
Rust parsing or linking must be serialized. Per-function checking/codegen is a
starting granularity, not proof of optimal units. No cross-build cache exists.

## Initialization and move-analysis boundary

Analysis tracks per-local storage state and per-place initialization, including
aggregate shallow-initialization markers. Whole moves invalidate an entire value,
while a field move leaves its initialized ancestors intact. This permits mutable
field reinitialization after a partial move without permitting field writes into
an entirely uninitialized or whole-moved aggregate. Immutable initialization
history survives moves and resets only at a new storage lifetime.

Joins and backedges use a worklist fixed point. Diagnostics are produced from
converged reachable states, with block, statement, expression-origin and place
provenance. Structural validation includes unreachable blocks. Resource exhaustion
fails closed rather than publishing a successful analysis of partial work.
Source-level named structs and enums are non-Copy unless an explicit canonical
Copy implementation satisfies its field and supertrait obligations. Scalar-only
representation and enum discriminant inspection do not grant Copy permission.
Wildcard-only and singleton matches do not require a place read. Unreachable
match arms still participate in ownership checks and post-match joins through
constant-true CFG switches whose imaginary successor is analysis-reachable but
never executed. Tuples
are Copy exactly when every component is Copy. Source consumption lowers to
Copy or Move of the projected place, never an accidental move of its parent.
Local nonescaping references now add bounded forward provenance and backward
carrier liveness to the same analysis gate. Loan targets and parent authority are
frozen at issuance, so reassigning a carrier does not retarget existing children.
Shared aliases, mutable-handle moves, explicit reborrows and last-use permission
checks operate on canonical storage places. An immutable binding holding a mutable
reference may write its referent. Thin native addresses are emitted only after
the exact CFG passes initialization and loan checks. Parameters and bounded
whole-parameter results use the follow-on contracts above. General source-declared
reference-containing aggregates, temporary lifetime extension and general Rust
regions remain unsupported. See `BORROWING.md` for B1 boundaries and B2/B3 work.

### Generic carrier values, current cells and effects

Source and Recipe use the same checked IR/CFG, ownership Plan, worklist and joins.
`CarrierLocation` is a typed value lens, distinct from a canonical memory-place
projection. Its `Fixed` channels identify reference positions, while an
owner-qualified `ParamTail` denotes the latent component family of a rigid type
parameter. Distinct input occurrences retain distinct entry origins even when
their types or eventual concrete actuals coincide. Family origins are symbolic
relations, not concrete loans, targets, layouts or Copy permission. Concrete
reference origins and symbolic families occupy separate typed domains in the
same solver. Every Copy exemption requires the exact canonical `must_copy` fact.

World-owned `CarrierContractView` binds declaration schemas and topology to the
exact world, registry snapshot, ordered actuals, environment, phase and shared
planning ledger. It supplements fixed `RefContract` authority rather than
rederiving return candidates from coincident specialized types. Header topology
is not executable family proof. `VerifiedTemplateCfg` retains successful
universal carrier/effect analysis, and the exhaustive checked-body substitution
walk retains a private `BodySubstitutionProof` for nominal/component mappings,
including body-local and dead metadata. Reference-valued instances require this
exact common capability, fresh lowering with concrete Copy/Move decisions and
closed analysis before the existing `ConcreteCfg` native seal. An older
reference-free value proof alone cannot grant reference-family permission.

Addressable carrier storage uses current cells keyed by canonical place and
channel, not a physical reference-depth slot. A typed dereference follows handle
targets, then loads the current inner cell, so writes through one alias are
observed by later parent or sibling reloads. Existing issued loans still retain
their original targets and parent authority. Reborrows through mutable chains
retain all required intermediate unique-parent and lifetime dependencies, while
an independent shared-reference value load does not permanently inherit a
transient outer handle. Unknown aliases require a finite complete original typed
candidate-cell cover or an explicit Unsupported result.

Call returns and carrier writes name entry input-value snapshots. Analysis
captures inputs, destination OLD contents and target lenses before consumption,
computes effects from that same state and applies poststate simultaneously.
OLD remains an allowed write alternative in conservative envelopes. Recursion,
mutual recursion and abstract calls initially use canonical declaration
envelopes. A private `CarrierEffectProof` records may effects and MustOverwrite
across all normal exits, with joins intersecting definite-write facts. A call
strong replacement requires a singleton Exact unique mutable target and sealed
selected-body MustOverwrite, not merely an `&mut` formal. Direct stores establish
their definite updates locally. The 115 table refines closed analysis only and
never supplies missing universal family capability or skips all-body checks.

Original caller named lifetimes remain rigid, including unused names pruned from
normalized contracts. Each callee use has fresh occurrence bindings, even at
recursive calls to the same declaration. Source-stage call evidence preserves
firm explicit reference expectations and the original Move/reborrow decision.
Family region constraints use witnessed pointwise component correspondence,
with both directions required for mutable invariance, not type equality or an
origin union. Call targets and native identities remain lifetime-erased.
Instantiation does not rerun coercion from a final concrete type. Unused bodies
and dead calls still validate new carrier metadata and capability requirements,
without manufacturing reachable origins or loan permission.

The first affine domain permits at most one returned owned affine component with
no affine writes, or one written owned affine cell with no affine results.
Components proven Copy by exact canonical facts are exempt from affine counting,
but still undergo all lifetime, alias and effect checks. Owned-component counting
stops at reference-value boundaries, rather than counting nested pointee views as
extra transferred handles. Affine writes need one singleton Exact destination.
Wider affine result/write correlation,
multi-target affine writes, newly inferred structural-reference actuals, unknown
cell aliases without a complete typed cover and recursive precision needing
pre-barrier MustOverwrite remain Unsupported. Internal nominal value transport
does not authorize a derived nested executable actual such as `Wrap<&U>`:
instance facts do not yet accept certified call-lineage authority for that case.
Explicit nominal lifetime/reference-field declarations and external
reference-containing aggregate actuals remain outside this stage.

All paths, families, cells, snapshots, effect relations, proof retention and
concrete expansions charge the same `PlanBudget`, including symbolic work with
no concrete loans. Limits remain 4096 combined concrete origins, 1,000,000
memberships, 64 normalized regions per canonical contract when constraints are
required, shared depth 128 and 128 MiB combined analysis state. The dynamic
combined occurrence graph is separately metered against unchanged total-state
limits, not a 64-region total across caller and callee bindings. Exactly qualified
legacy erased subjects with no induced constraints keep zero normalized region
rows while retaining their full typed carrier and alias analysis. New reference
topology must create genuine fixed/family constraints or fail, not borrow that
erased bypass or merge
distinct origins to fit the cap. Resource exhaustion fails closed and is not
reclassified as a semantic negative or Unsupported precision boundary. See
`GENERIC-CARRIERS.md` for the detailed bounded contract. This is not full Rust
generic-reference support or a compilation-performance claim.

## Aggregate implementation boundary

Named nonempty braced structs have nominal identity scoped to one immutable
compilation environment. Distinct declarations never become interchangeable
because their fields match. Named types, simple import aliases and field privacy
are checked, including inferred private call results. By-value declaration cycles
are rejected before immutable handles are constructed. Initializers evaluate in
source order before operands are arranged in declaration order.

Aggregate reads produce independent by-value snapshots. Non-Copy consumption
moves only the selected place. Scalar field reads leave the containing struct
available, partial moves preserve siblings, and mutable field reinitialization
restores the selected subtree. Joins and loops must retain definite initialization.
Assignments evaluate their full RHS before writing and require mutable roots
except for a first whole-local initialization or a write through an authorized
mutable reference. Borrowing an owned aggregate does not copy it; Copy fields can
be read through references without moving the containing value. Aggregate
comparisons, general source-declared reference-containing aggregates, derives
and destructors remain unsupported. Certified internal generic products under
admitted root-reference actuals use the carrier substitution proof above.
Let patterns support nested tuples/structs, renamed or shorthand fields, wildcard
and struct `..`, with whole-pattern annotations and independent mutable binders.
Tuple-rest patterns, ref/ref mut, @, or/refutable patterns, parameter patterns,
let-else and destructuring assignment remain outside this slice. See `PATTERNS.md`.
Ignored place fields do not become reads or moves. RHS effects still execute once,
and all new binders enter scope only after the initializer and pattern checks.
Unannotated tuple patterns requiring later backward inference from a pure-Never
initializer receive an explicit unsupported diagnostic.

The private ABI passes one 8-byte word per argument, containing a scalar or a
pointer to a caller-owned aggregate snapshot. Callees copy aggregate arguments
before executing their bodies. Aggregate-returning calls add a final hidden
caller-buffer pointer, and all return paths copy into that buffer. Tuples and
structs currently use recursive 8-byte leaf slots, including unit dummy slots.
This is an internal representation, not a Rust platform-layout or FFI promise.
`size_of` and general memory inspection are not supported. Frame size is bounded
to 1 MiB. The trusted raw CFG emitter requires successful analysis of the exact
unchanged CFG. Checked standalone convenience APIs lower and analyze once before
calling it. The graph consumes its cached analysis without repeating those checks.

## Running the compiler

Build and test with the installed stable client:

```sh
justrust check
justrust test
justrust clippy --all-targets -- -D warnings
justrust build --release --bin jr-rust
```

Use the resulting binary on the machine holding that build artifact:

```sh
/path/to/jr-rust examples/compiler/hello.rs -o /new/path/hello --workers 1
/new/path/hello
/path/to/jr-rust examples/compiler/arithmetic.rs --check --workers 8
/path/to/jr-rust examples/compiler/modules.rs -o /new/path/modules --workers 8
/new/path/modules
/path/to/jr-rust examples/compiler/external/main.rs -o /new/path/external --workers 8
/new/path/external
```

`enums` counts retained enum declarations alongside `functions` and `structs`.
`cfg_blocks`, `cfg_edges` and `cfg_locals` count explicit control-flow work,
separately from scheduling dependency `nodes` and `edges`. Counts include retained
blocks and generated temporary locals, not just source declarations.
`analysis_worklist_pops`, `analysis_move_paths` and `analysis_moves` describe
initialization-analysis work. These are not elapsed-time or speedup claims.
Successful traces retain the immutable analysis reports as well as the CFGs.
The compiler prints a JSON accounting summary, not the program's output.
Execute the emitted ELF separately. `--trace NEW_TRACE` writes the full Debug
report, including failure events on rejected compilations. Trace creation occurs
after compilation, so a trace path cannot supply a previously missing module.
Syntax positions currently refer to expanded text, not original multi-file source
positions. Loading errors identify the source path. Semantic diagnostics identify
the function,
not the exact expression span. There is no stable structured diagnostic format
yet. Native arithmetic/write failures terminate with 101 without unwinding,
Rust panic messages, or stdio buffering. Stack exhaustion is not yet guarded
by the generated runtime. The compiler does not implement stock rustc's lint
passes, including constant arithmetic lint propagation. Explicit semicolons discard
non-unit block/if/loop values. Omitted-semicolon block-like statements require unit.
The private function-call ABI is not an FFI ABI. The ELF is fixed-address,
not PIE, and requests separate non-writable executable code and read-only data.

Small-source and 258-function measurements, along with raw receipts, are in
`FINDINGS.md`. On the first M8a run all 66 tests passed, 64 generated execution
comparisons matched stock Rust, and all 36 repeated check/build samples passed.
These establish this bounded milestone, not full Rust conformance.

```sh
python3 experiments/compiler_correctness.py --binary /path/to/jr-rust \
  --rustc /path/to/reference/rustc --output /new/path/differential.jsonl
python3 experiments/compiler.py --binary /path/to/jr-rust \
  --revision "$(git rev-parse HEAD)" --reference-rustc /path/to/reference/rustc \
  --output /new/path/matrix.jsonl
```

The stock compiler is used only by these external validation tools. Compiler
implementation modules never spawn it. Use the charged hosted experiment wrapper
for remote runs, with source revision, guest timeout and lifecycle receipts.
The initial frontend rejects syntax/AST nesting beyond 128, and the private
backend supports at most 4,096 function arguments. Neither is a Rust language
limit. Numeric syntax is currently unsuffixed decimal i32, and `!` accepts bool
only. Runtime signals, stack exhaustion and stdio buffering do not yet match
all Rust standard-runtime behavior.

## Acceptance and measurement

Execute generated binaries and compare stdout and exit status with expected
results. Validate recursion, branches, loops, scopes, operand ordering,
short-circuiting, arithmetic traps, invalid programs, overwrite protection and
multiple worker counts. Use the bootstrap compiler separately as a semantic
reference, never as an implementation path. Retain all diagnostics on failure.

Measure actual source-to-ELF compilation and separately process execution,
including source size, functions, graph nodes/edges, accounting coverage,
worker count and toolchain/host provenance. Keep repeated samples and mark the
shared hosted machine contended/uncontrolled. A small Rust-subset fixture does
not establish performance on normal Rust projects.
