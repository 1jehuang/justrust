# Native prelude library

`libraries/prelude/` is justrust's own library, written in ordinary Rust (not
Rust's std). It provides `std::`/`core::` paths and the implicit prelude.
The source library contains `Option<T>` and `Result<T, E>`, their constructors,
borrowing predicates (`is_some`, `is_none`, `is_ok`, `is_err`), and consuming
`unwrap_or`. Methods are ordinary generic Rust impl bodies, not compiler
method-name exceptions. `unwrap` and other panicking convenience APIs are not
provided until their panic and trait-bound contracts can be implemented
correctly. The explicit bootstrap `native_heap` module adds Rust-source
allocation and pointer helpers over a primitive Linux syscall ABI, documented
in NATIVE-HEAP.md. This is not full std/core support. Iterator traits and the
rest of std/core remain follow-up work.

## Loading (graph inputs)

- Every prelude file is a real per-file `SOURCE` graph input (`FileUnit`).
  Ordinal `[1]` is `libraries/prelude/root.rs`, which declares
  `mod __justrust_std;`. Its children (`[1, i, ...]`) are the library files,
  read through `PreludeReader`, a read-only reader over the embedded table in
  `src/compiler/source_prelude.rs` (no filesystem capability).
- The assembler splices the prelude after the user crate. Prelude files get
  their own `FileId`s; they do not count in `source_bytes`/`source_files` and
  prelude items do not count in `functions`/`structs`/`enums` telemetry.
- `SourceOptions::prelude` (default true) controls attachment.
  `prepare_file`/`prepare_text` are low-level user-only preparation APIs and
  never attach it. Graph compilation uses `SourceAssembler::new`, while
  `SourceAssembler::user_only` preserves the low-level source contract.

## Resolution

- `std::a::b` and `core::a::b` resolve to `__justrust_std::a::b` only when
  no user item or import named `std`/`core` is in scope.
- An unresolved bare first path segment falls back to
  `__justrust_std::prelude::v1` (`libraries/prelude/prelude.rs`). User items and
  imports are tried first, so they shadow prelude names. Glob-import syntax
  remains outside the compiler's current supported surface.
- Inside prelude files `crate::` names the library root.

## Checking and codegen

Prelude bodies are type checked universally like user code (the graph
caches them as ordinary per-declaration nodes). Nongeneric prelude bodies are
only instantiated when reachable from user roots; generic ones are
instantiated on demand as usual.

## Source trait prerequisites

The canonical catalog currently owns `Sized`, `Copy`, `Clone`, and `Drop`.
It does not own `Deref`, `DerefMut`, `Iterator`, or `IntoIterator`, and this
library does not mint weaker substitutes for those contracts.
`tests/prelude_traits.rs` checks exact ordinary source contract fixtures with
stock rustc and records these current native boundaries in check/build modes
with one and eight workers:

- `Iterator::next(&mut self) -> Option<Self::Item>` needs general associated
  projection normalization inside canonical nominal applications. Even a
  concrete source implementor currently fails the residual-projection guard.
- `Deref::Target: ?Sized` needs general relaxed-bound semantics. Replacing it
  with an implicitly `Sized` target would be a different contract.
- `IntoIterator::IntoIter: Iterator<Item = Self::Item>` needs source trait
  bounds and associated equalities on associated declarations. Dropping the
  bound would admit invalid implementations.

Projection-bearing source methods can use ordinary shared or mutable receivers
when the receiver type is projection-free. Their value results use existing
associated-assignment proof and source-region authority. Reference-bearing
results and projected-reference parameters remain unsupported; accepting a
borrowed receiver does not manufacture a returned-reference origin contract.
Stock/native fixtures cover shared calls, mutable reborrows, and the unchanged
reference-result boundary.

The fixtures are stock acceptance evidence and native limitations, not claims
of Iterator, dereference operators, adapters, or `for` desugaring support.

## Adding a module

1. Write `libraries/prelude/NAME.rs`, add `pub mod NAME;` to `lib.rs`.
2. Add one line to `PRELUDE_FILES` in `src/compiler/source_prelude.rs` with
   virtual path `__justrust_std/NAME.rs`.
3. Re-export prelude names from `prelude.rs` `pub mod v1`.
