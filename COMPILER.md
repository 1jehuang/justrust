# Native Rust compiler implementation

Status: executable Rust-subset implementation with inline and external modules,
2026-10-15.
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

Initial supported syntax: free functions with `i32`, `bool`, unit and nested
Copy tuple parameters/results, tuple literals and numeric field projections,
local bindings and shadowing, `mut`, whole-value and local-rooted field assignment,
integer
arithmetic, comparisons, short-circuit Boolean operators, blocks, `if`/`else`,
`while`, unlabelled `loop` expressions, `break` with supported value types,
`continue`, `return`, calls including recursion, and `println!("literal")`.
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
generics, structs/enums, references/borrowing, strings as values, closures, async,
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
- Validate declarations and publish the callable signatures.
- Check and lower each function body independently against those signatures,
  publishing an immutable typed-function fact (operation 102).
- Emit each function from that exact checked fact after its own body check,
  without repeating checking or waiting for other bodies.
- Link emitted functions and resolve relocations into one executable.
- A check root joins all required semantic checks but demands no code generation.
- A build root joins emitted objects, then executable publication.

Recursive calls depend on declarations, not completion of the callee body.
All independent function prerequisites are demanded before awaiting results.
Within one graph, concurrent check and build requests share parsing, resolution,
declarations and body checks. Output facts contain typed-arena handles because
the prototype runtime currently stores u64 slots. Arenas remain owned by the
compilation snapshot and are not persistent cache keys.

Checked functions now retain resolved local and expression IDs, concrete
inferred types, validated call signatures and explicit normal-completion versus
divergence information. Every shadowed binding has its own LocalId. The native
emitter consumes these facts directly, without source-name lookup or inference.
This structured typed IR is groundwork for general types and ownership analysis,
not a completed layout, CFG, borrow checker or public cross-version IR format.
In the checked IR, semantic `Ty::Never` is distinct from runtime non-completion.
Explicit unreachable tails retain their actual type and are still checked. Pure
never operands to nonlogical binary operators currently receive an explicit
unsupported diagnostic. This deliberately includes some legal stock comparisons
pending integer/operator inference, rather than accepting invalid programs.
`break`/`continue` targets are resolved loop IDs. Native abrupt edges restore the
exact temporary-stack depth for the target, preserving enclosing expression
values and discarding abandoned operands/arguments. A direct break/continue in
its own while condition is rejected, while a nested loop has its own target.


Source-tree loading and text expansion currently form one coarse operation.
String-input library calls retain their original parse-first graph.
The parser consumes expanded source as one operation. Name resolution,
declaration validation and executable linking are also initially single
operations. These are explicit implementation limitations, not architectural
claims that all
Rust parsing or linking must be serialized. Per-function checking/codegen is a
starting granularity, not proof of optimal units. No cross-build cache exists.

## Aggregate implementation boundary

Nested tuples contain only the currently supported Copy types. Reads produce
by-value snapshots, including tuple fields, arguments and control-flow joins.
Every tuple-typed expression has its own frame buffer, so later mutation of a
local cannot change an already evaluated argument. Mutable field assignment
requires a mutable local root and evaluates its complete RHS before writing.
General destructuring, tuple comparisons, references and named structs are not
included in this slice. In particular, a named struct is not made Copy merely
because its fields are scalar.

The private ABI passes one 8-byte word per argument, containing a scalar or a
pointer to a caller-owned tuple snapshot. Callees copy tuple arguments before
executing their bodies. Tuple-returning calls add a final hidden caller-buffer
pointer, and all return paths copy into that buffer. Nested tuples currently use
recursive 8-byte leaf slots, including unit dummy slots. This is an internal
representation, not a Rust platform-layout or FFI promise. `size_of` and general
memory inspection are not supported. Concrete frame limits and implementation
validation are recorded with the aggregate milestone in `FINDINGS.md`.

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
