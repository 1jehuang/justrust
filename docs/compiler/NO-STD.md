# Crate-scoped no_std admission

Status: implementation in progress, not an acceptance receipt (2026-10-19).

## Frozen contract

Original effective `#![no_std]` belongs only to an assembly-issued crate root.
Admission includes library targets, dependencies and hosted test harnesses.
A bare root executable still rejects as Unsupported until no_std startup and
panic-handler semantics exist, using retained root ordinal and target-kind facts,
not path or spelling guesses. The generated test entry uses compiler-owned
runtime paths, independent of user `std` aliases or shadowing, without giving
source tests a std prelude or admitting std-only names and macros.
It changes that crate's namespace and prelude, not its importer's policy.
Nested inline and external modules inherit their own defining crate policy.
A mixed ordinary executable and no_std dependency remains mixed after mounting.
Malformed, duplicate and misplaced forms reject. Generated metadata and caller
mutation of public AST/preparation audit records cannot issue crate policy.
Explicit opt-in `extern crate std` is a distinct unsupported boundary until its
namespace semantics are implemented, not an invalid-Rust diagnostic.

The admitted core prelude excludes heap/String/std-only names. Explicit supported
`core::` paths remain source-library resolution. Implicit `std::`, std-only
prelude types and std I/O macros are unavailable. Unsupported core APIs reject,
never map to unrelated std implementations. The qualifier is never deleted from
cached crate source, and const/unsafe semantics retain their existing authority.

## Graph shape

Per-file source preparation publishes final immutable original crate metadata.
Assembly preserves existing canonical source crate roots. Native parsing and
resolution consume the source-owned policy through their existing graph facts.
No global policy map, mutable fact, side slot, new barrier or whole-program node.
CHECK must not demand codegen. Registry source authentication remains a separate
project-input concern.

The standalone-owner whitelist previously admitted `(src/compiler/frontend.rs,
lex)`. That production entry is removed: `parse_unresolved` now uses canonical
`parse_prepared`, preserving original-source policy and the snapshot-issued token
session rather than minting a fresh owner from flattened text. Only that obsolete
whitelist tuple is deleted. No ceiling or other baseline is raised.

## Acceptance gates

- Original unchanged utf8-width 0.1.8 CHECK, BUILD and run prints `3\n`, workers 1/8.
- Ordinary and no_std mixed crates, nested modules, explicit supported core paths.
- Missing std/heap prelude and I/O macros reject, including unused declarations.
- Malformed/duplicate/misplaced/generated attributes and forged policy identities.
- Hosted no_std test CHECK/BUILD/run, panic/ignore/filter behavior, and unchanged
  Adler2 2.0.1 upstream tests at workers 1/8 (`no_std`, `no_std_graph`, and
  `experiments/adler2_repo_probe.py`). Bare no_std executables remain rejected.
- Graph integrity unchanged or tighter, no ratchet increase.

No gate is claimed passed until measured on the exact private tree.
