# First native Rust compiler milestone

Status: first executable Rust-subset implementation, 2026-10-15.

This extends the synthetic runtime prototype with actual Rust source input and
Linux x86-64 ELF executable output. Standard Rust tools build this compiler,
but compiling user input does not invoke rustc, Cargo, LLVM, a C compiler,
an assembler, a linker, or an interpreter. This is an initial Rust subset,
not a replacement capable of compiling arbitrary Rust projects or itself.

## External contract

`jr-rust SOURCE -o NEW_EXECUTABLE [--workers N] [--trace NEW_TRACE]`
compiles one immutable UTF-8 source file. `--check` validates without generating
an executable and does not require `-o`. Existing outputs and traces are never
overwritten. Invalid and unsupported programs fail explicitly. Output is not
published until every required check and code generation operation succeeds.

Initial supported syntax: free functions with `i32`, `bool`, and unit parameters
and results, local bindings and shadowing, `mut` and assignment, integer
arithmetic, comparisons, short-circuit Boolean operators, blocks, `if`/`else`,
`while`, `return`, calls including recursion, and `println!("literal")`.
`main` takes no arguments and returns unit. Arithmetic has overflow checks,
matching Rust debug behavior. Runtime arithmetic failure exits 101. This first
runtime does not yet reproduce Rust panic text or unwind behavior.

Not supported: crates/manifests, modules/imports, standard-library APIs beyond
the documented print form, general macros, traits, generics, structs/enums,
references/borrowing, strings as values, closures, async, FFI, or other targets.
This list does not turn unsupported syntax into permission to accept programs
with incorrect semantics. Diagnostics and acceptance tests must police the
implemented boundary. Semantic coverage must be expanded explicitly.

## Graph operations

- Parse immutable source into a retained program arena.
- Validate declarations and publish the callable signatures.
- Check each function body independently against those signatures.
- Emit each function after its own body check, without waiting for other bodies.
- Link emitted functions and resolve relocations into one executable.
- A check root joins all required semantic checks but demands no code generation.
- A build root joins emitted objects, then executable publication.

Recursive calls depend on declarations, not completion of the callee body.
All independent function prerequisites are demanded before awaiting results.
Within one graph, concurrent check and build requests share parsing,
declarations and body checks. Output facts contain typed-arena handles because
the prototype runtime currently stores u64 slots. Arenas remain owned by the
compilation snapshot and are not persistent cache keys.

The first parser consumes the source file as one operation. Declaration
validation and executable linking are also initially single operations. These
are explicit implementation limitations, not architectural claims that all
Rust parsing or linking must be serialized. Per-function checking/codegen is a
starting granularity, not proof of optimal units. No cross-build cache exists.

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
```

The compiler prints a JSON accounting summary, not the program's output.
Execute the emitted ELF separately. `--trace NEW_TRACE` writes the full Debug
report, including failure events on rejected compilations. Syntax diagnostics
have line/column, while semantic diagnostics currently identify the function,
not the exact expression span. There is no stable structured diagnostic format
yet. Native arithmetic/write failures terminate with 101 without unwinding,
Rust panic messages, or stdio buffering. Stack exhaustion is not yet guarded
by the generated runtime. The compiler does not implement stock rustc's lint
passes, including constant arithmetic lint propagation. Non-unit block/if
expressions used as discarded statements are currently conservatively rejected.
The private function-call ABI is not an FFI ABI. The ELF is fixed-address,
not PIE, and requests separate non-writable executable code and read-only data.

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
