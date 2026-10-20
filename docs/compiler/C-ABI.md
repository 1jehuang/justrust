# Linux x86-64 external C ABI

Status: implementation slice of G6, not completion of G6 or full Rust. Edition
2024, Linux x86-64, LP64 System V AMD64. Private aggregate snapshots are NOT
external ABI storage. No compatibility claim follows from private Rust-to-Rust
calls or synthetic graph fixtures.

## Frozen contract

* Original `extern "C" fn` definitions and `unsafe extern "C" { fn ...; }`
  declarations. Foreign calls require an unsafe context. Explicit original
  `#[unsafe(no_mangle)]` exports a definition under its source identifier.
  Non-exported C definitions remain callable from Rust with a private entry.
* Scalar values: unit, bool, signed/unsigned integers through 128 bits, f32,
  f64, and thin raw pointers to canonical i64/u64/isize/usize/f64 pointees.
  Only this eight-byte scalar pointer set shares exact private/native storage
  reads and writes. Narrow, bool, f32, ZST, aggregate and nested-pointer pointees
  are rejected, recursively through aliases, aggregate fields and signatures.
  Existing private stores write eight bytes, so admitting narrow native pointers
  would be incorrect. This is not a claim of full C-pointer interoperability.
* Original `#[repr(C)]` structs, recursively containing admitted scalars,
  arrays and repr(C) structs. Canonical declaration identities authorize repr,
  not a display-name or structurally similar forged type. C field offsets,
  padding, array strides, alignment and size are calculated separately from
  the existing private eight-byte carrier representation.
* INTEGER/SSE eightbyte classification, mixed results, independent six GPR
  and eight SSE register banks, complete aggregate rollback on exhaustion,
  16-byte call stack alignment, aligned stack arguments, two-register results
  and hidden result pointers for MEMORY returns. Native wrappers preserve the
  SysV callee-saved register set even though the private emitter does not.
* `--native-object PATH` captures immutable ELF object or ar archive bytes
  before graph execution. `--native-library NAME` requests the system linker
  library. `--emit-object` writes a real ELF64 relocatable object with exported
  symbols. The private `main` symbol is separated from C `main`.
* ELF relocations are emitted directly: PLT32 calls and absolute 64-bit data
  and vtable references. No assembler, LLVM, rustc or Cargo implementation is
  embedded. `cc` is used only as the native system linker driver. Native C
  compilation and stock rustc are test oracles, never compilation fallback.
* CHECK validates even unused boundary declarations and bodies, demands no
  code generation or linker. BUILD publishes only after CHECK succeeds.
  Missing symbols, invalid native inputs, unsupported language and linker
  infrastructure failures never publish an executable.

## Graph and authority

Parser-owned immutable declaration metadata travels with the existing source
facts. Resolution checks its source signature authority, then seals canonical
resolved signatures. SIGNATURES validates boundaries, including unused ones.
Each OBJECT producer computes only its own bridge from checked closed types,
then publishes immutable machine code, export offsets and relocation records.
BUILD consumes existing object handles and performs final linking after CHECK.
No new whole-program node, mutable shared fact, global admission budget or
whole-program ABI side table is introduced. Local layout and bridge allocations
are metered by their source task's existing budget. Private backend layout
consumers retain their previous meaning. Imported declarations have no Rust SSA
body: their existing SSA producer publishes the typed unsupported-optimizer
result, so summaries cannot inline the parser's authenticated declaration carrier.
This is optimizer authority hardening, not rejection of the admitted source ABI.
Real Rust-defined C exports retain their real bodies.

## Acceptance gates

`tests/native_c_abi.rs` compares stock Rust + real C with this compiler at
workers 1 and 8. It exercises narrow signed scalars, bool, floats, integer and
SSE stack overflow, i128 alignment, small/mixed/large/nested/array aggregates,
MEMORY results, register rollback, native libc symbols and a C function-pointer
callback into exported Rust, C constructors and buffered C output at shutdown.
A real SysV main bridge uses the host CRT rather than bypassing native lifecycle.
A second executable has a real C main linked
against `--emit-object` output and calls the exported definitions directly.
The C oracle asserts sizes and offsets with `_Static_assert`. Adaptive and Final
modes also run real `native_add` imports and Rust-defined exports at workers 1
and 8. Object-mode graph acceptance demands the foreign SSA producer but retains
zero inlined declaration carriers; forged declaration renaming is rejected.

`native_c_abi_authority.rs` covers unused unsupported boundaries, invalid bodies
and unsafe call obligations. `native_c_abi_graph.rs` checks owner-local object
demand and CHECK-without-codegen at both worker counts. `graph_integrity`
remains mandatory and its ratchets must not be raised.

## Explicitly open G6 work

Variadics and `%al` protocol, C-unwind/unwinding, TLS, atomics and target memory
model, public Rust layout/niches, aggregate-pointer addressable storage,
repr(packed)/align/transparent, foreign statics, link/export_name attributes,
source `#[link]` metadata, generic/lifetime-generic/associated C declarations,
generated ABI attributes/declarations, vector/x87/SSEUP classes, platform ABI
variants, PIE/shared-library emission, symbol versioning and full ABI metadata
replay remain unsupported. These are explicit rejection boundaries, not private
ABI approximations. Native linking currently requires a functioning host `cc`
and Linux x86-64 system libraries. No speedup claim is made.

Specification: x86-64 psABI, Low Level System Information, parameter passing and
return values (`gitlab.com/x86-psABIs/x86-64-ABI`, `low-level-sys-info.tex`).
