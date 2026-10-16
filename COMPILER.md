# Native Rust compiler implementation

Status: executable Rust-subset implementation with bounded shared/mutable reference returns, noescape reference calls, local borrowing, genuine usize, runtime indexing, fixed arrays, nominal structs, payload enums and match,
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

Initial supported syntax: free functions with `i32`, `usize`, `bool`, unit, nested tuples
and nonempty braced named structs or enums and fixed arrays as parameters/results, aggregate literals and
numeric/named field projections,
local bindings and shadowing, typed deferred local initialization,
irrefutable tuple/struct let patterns, wildcard,
per-binding `mut`, struct-pattern `..`, whole-value and local-rooted field assignment,
integer arithmetic, comparisons, short-circuit Boolean operators, blocks, `if`/`else`,
`while`, unlabelled `loop` expressions, `break` with supported value types,
`continue`, `return`, enum/scalar `match`, calls including recursion,
and `println!("literal")`. Match supports variant/literal, wildcard and whole-value
binding arms, with exhaustive bool/unit/enum coverage and an i32 catchall. Guards,
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
output lifetime requires exactly one input reference lifetime position, even when
multiple input positions explicitly share the same name. Explicit output bounds
remain rigid, and unrelated anonymous inputs do not gain inferred outlives edges. Complete declaration-derived InputPath candidates, rigid body outlives
obligations and separate transitive lifetime dependencies gate returned pointers.
Each suffix stays correlated with its original argument and source authority.
Private-field getters do not grant callers direct private access. Mutable results
are consumed unique handles, not copied capabilities, and keep overlapping parent
authority frozen until their last use. Input kinds remain genuine capabilities,
while all region-related inputs constrain output lifetimes independently of target
eligibility. See `MIXED-REFERENCE-RETURNS.md`. Enum payload paths, dynamic borrowed
indices, trait-based autoderef, bare reference indexing, method/general elision
and reference-containing aggregates remain unsupported.
Builtin `r.field` and `r.0` insert one real Deref before field lookup in reads,
assignments and borrows. Privacy and pointee capability are checked after peeling
the reference, without moving its carrier or snapshotting its aggregate. Genuine
reference Call-result field reads use the existing result temporary. Computed
Call LHS, borrowing Call rvalues, runtime indexing through references and enum
downcasts through Deref remain unsupported.
See `BORROWING.md`, `REFERENCE-CALLS.md`, `REFERENCE-RETURNS.md`,
`MUTABLE-REFERENCE-RETURNS.md`, `PROJECTED-REFERENCE-RETURNS.md` and
`LIFETIME-ELISION.md` and `REFERENCE-FIELDS.md`.
Typed declarations `let [mut] name: Type;` use cached flow analysis for first
initialization and immutable assignment history. Untyped/backward-inferred and
deferred destructuring declarations remain unsupported. See `DEFERRED.md`.
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
generics, empty enums, explicit discriminants, empty/unit/tuple structs,
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
- Check and lower each function body independently against those signatures,
  publishing an immutable typed-function fact (operation 102).
- Lower each checked function once into an explicit control-flow graph
  (operation 108), with ordered value snapshots, branches, loop edges and calls.
  Publishing the CFG does not imply it passed initialization analysis.
- Analyze each CFG once (operation 109), validating structure and running bounded
  definite-initialization, storage-lifetime, move-path, loan-origin and bounded
  signature-region dataflow to fixed points.
  Both check and emission require this successful immutable analysis fact.
- Emit each function from that exact cached CFG fact without repeating lowering
  or waiting for other bodies.
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

Checked functions now retain resolved local and expression IDs, concrete
inferred types, validated call signatures and explicit normal-completion versus
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
Source-level named structs and enums are non-Copy, including scalar-only
representations. Enum discriminant inspection does not grant Copy permission.
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
whole-parameter results use the follow-on contracts above. Reference-containing
aggregates, temporary lifetime extension and general Rust
regions remain unsupported. See `BORROWING.md` for B1 boundaries and B2/B3 work.

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
comparisons, reference-containing aggregates, derives and destructors remain unsupported.
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
