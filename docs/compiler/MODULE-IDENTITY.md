# Checked module authority

## Status

The actual root-context AST, resolver, checker and retained source replay now
use owned semantic identifiers, structured paths and checked module identities.
The integrated migration passed the complete hosted acceptance gates below.
This does not claim new Rust syntax support, general hygiene, source macro
expansion, cross-crate compatibility or full Rust support. Raw, Unicode and
non-root/expanded semantic identifier admission remains explicitly unsupported.

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
clean full hosted test/check/strict-lint/format run.

### Acceptance receipts

The integrated production implementation passed all 1,248 public integration
tests across 96 binaries in hosted run `20261017-082305399-1037109`. This includes
the 13 public module tests, nine semantic-resolution tests, stock-first resolution
witnesses and native CHECK/BUILD at one and eight workers. The final frozen
implementation passed 2,020 tests across 100 binaries with zero failed, ignored
or filtered tests and no retry output in `20261017-083329969-1083515`.
Warning-free all-target check `20261017-083329203-1083407`, strict all-target
clippy `20261017-083343328-1084433`, `cargo fmt --check` and `git diff --check`
passed in the same coordinated gate. Original 65,537-use, 4,096-parameter and
ordinary-stack depth witnesses retain their source and resource limits.

The retained history in `data/runs/xps14/2026-10-17-module-identity.jsonl` includes
failed intermediate compiles and actual compatibility failures. In particular,
run `20261017-081908819-1019222` exposed an invalid public reexport of a private
`self` module. The resolver now obtains non-root module-anchor visibility from
its checked declaration instead of synthesizing public visibility. Subsequent
public integration acceptance includes the unchanged reexport corpus.

Unit run `20261017-082444532-1044071` also caught a real persistent-key collision
between default methods belonging to different source traits. Structural instance
serialization now includes the checked terminal trait owner; its wire domain is
v3, rejecting reuse of old aliased keys. Existing trait-owner inequality,
source-order and impl alpha-renaming controls pass unchanged. Snapshot-local
numeric handles and printable paths are not persistent identity.

Fixture construction now uses one explicit `ProgramBuilder` and canonical
declaration paths. Historical diagnostic expectations gained the explicit
`crate::` anchor. The obsolete test expecting per-clone visibility String heap
was replaced with an exact shared-module-Arc/last-owner accounting test, because
visibility now stores an inline checked `ModuleId`. No dead String, increased
cap or weakened source fixture was retained to imitate the former representation.
