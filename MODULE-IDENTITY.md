# Checked module authority

## Status

This is the implementation contract for the in-progress owned-name migration.
The arena, typed IR interfaces and catalog adapters are saved, but the complete
frontend migration and coordinated acceptance gates have not passed yet. This
document does not claim new Rust syntax support, general hygiene, source macro
expansion, cross-crate compatibility or full Rust support.

## Ownership and roots

`ModuleId` is an opaque arena-qualified handle, independent of declaration
snapshot IDs. Source modules exist before a new `PlanCtx` declaration snapshot,
and one immutable source program can be checked in multiple planning contexts.
The source program and its catalog retain the same `Arc<ModuleArena>`.
Definitions store checked scalar module handles, not separate arena copies.

A source arena is constructed against the exact owned name store and shared
expansion budget. It contains a real source root and a distinct canonical core
root. A standalone catalog arena has only typed catalog roots and modules.
Requesting a source root from a catalog-only arena fails explicitly. There is no
sentinel module, inferred owner from the first type, or fabricated source name.

Source children are interned by checked parent and semantic name key. Original
identifier occurrences remain available for diagnostics and structural identity
serialization, but printable paths are not lookup authority. Parent references
are flat, append-only and owner-checked. Arena creation introduces no new module
depth or canonical path cap. Existing source-loading depth limits remain separate.

## Privacy and metadata admission

IR visibility is `Public` or `Restricted(ModuleId)`. A restricted audience is
allowed exactly when the requester is a descendant of its checked scope in the
same root. Requester identity comes from the selected checked declaration and
callable context, not an unchecked caller-provided string.

All module and restriction handles are validated even for public declarations
and fields. A restriction must name an ancestor of its defining module. A
restricted reexport cannot widen its original audience. Public/public decisions
still check the arena budget and preserve its first latched failure.

Source `pub(crate)`, private, `pub(super)` and `pub(in ...)` restrictions resolve
to checked source modules. User syntax and semantic errors remain distinct from
foreign-owner contract failures. Catalog declaration modules use closed typed
catalog descriptors. Catalog spelling does not grant source or intrinsic
publication authority.

## Names, planning and persistent identity

The universal AST validator checks names, module ownership and their shared
budget association before declaration or body authority is published, including
empty programs, unused bodies and public metadata. Canonical source declaration
paths must agree with their explicit source module. Plan-backed typing retains
exact immutable owner associations and validates its supplied active body without
repeatedly walking the entire program for every callable.

Closed builtin name keys do not fabricate source `AstIdent` occurrences. Trait
references distinguish source paths from typed builtin traits. Persistent
instance keys must encode checked structural source/core module and declaration
descriptors, never `ModuleId`, session IDs, allocation order or rendered diagnostic
paths. Source-order and alpha-renaming invariance remain required gates.

## Resource and acceptance gates

Arena nodes, child indexes, rebuild peaks and returned storage retain actual
capacity leases. Traversal work is charged before bulk scans and allocations.
Owner failures, limits, overflow, allocation and infrastructure failures preserve
their typed causes. Temporary storage is released on failure without refunding
monotonic work.

Acceptance requires the independent public module corpus, stock-first semantic
resolution corpus, foreign and unused AST mutation controls, catalog/schema and
replay owner checks, unchanged source fixtures and resource witnesses, and a
clean full hosted test/check/strict-lint/format run. These gates remain open until
concrete receipts are recorded with the integrated implementation.
