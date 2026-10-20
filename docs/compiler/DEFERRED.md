# Typed deferred local initialization

This slice exposes the existing cached definite-initialization analysis through
ordinary Rust source declarations: `let [mut] name: SupportedType;`.
Only plain bindings with explicit concrete types are included. Untyped declarations
requiring backward inference and deferred destructuring patterns remain explicit
unsupported boundaries until their inference/pattern work is implemented.

The source AST adds `Stmt::Declare { name, ty, mutable }`. Name resolution checks
the annotation and introduces a fresh local at the declaration, including when it
shadows a prior name. Checked IR adds `Stmt::Declare { local: LocalId }`, referring
to that local's resolved type and mutability. CFG lowering emits StorageLive and
registers scope cleanup, but emits no value or assignment. No zero initialization
or fabricated unit/enum value is allowed.

The frontend checks whole-local assignment types but does not reject an immutable
assignment before flow analysis can determine whether it is the first write.
Analysis 109 is the single authority for initialization, immutable assignment
history and joins/backedges. Existing initialized immutable locals still reject
reassignment. Whole mutable reinitialization after a move remains legal. Field
writes retain mutable-root and initialized-ancestor requirements. Backend emission
requires that same successful cached analysis, without repeating it.

Reads require definite initialization on every incoming semantic path. Diverging
paths do not need to initialize a value before they exit. StorageLive begins a new
lifetime and resets history, which is different from a move or a loop backedge.
Wildcard-only and singleton enum matches retain their existing no-read semantics.
Variant tests of uninitialized multivariant enums remain errors.

Retain the exact historical wildcard/uninitialized match source when promoting it
from legal-unsupported to accepted. Verify real check/build/runtime behavior,
invalid reads/reassignments, branch/match joins, loops, shadowing, aggregate roots
and failure-analysis retention against independent stock classifications. No claim
of full inference, borrowing, destruction or full Rust follows from this slice.
