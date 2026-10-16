# Local references and borrowing, staged contract

Status: B1 local-reference implementation passes 701 hosted tests, strict
all-target lint and formatting. Resource-limit review is complete. The tracked
thirteen-corpus native comparison passed all 572 cases, including 35 borrowing
fixtures. Full Rust language and borrow-checker completion remain open.

## B1: genuine nonescaping local references

Support standalone local `&T` and `&mut T` reference values to existing,
initialized, nonreference sized local/parameter places. Shared references are
Copy, mutable references are non-Copy, independent of the referent type. Permit
shared copies, mutable handle moves, explicit `&*r` / `&mut *r` reborrows,
dereference Copy reads, and mutable-reference replacement of initialized values.
An immutable reference binding can still write through its mutable capability.
A mutable binding of a shared reference cannot write through that reference.
Moving non-Copy referents through any dereference is rejected.

References are standalone locals/temporaries only in B1, not parameters, results,
aggregate fields, arrays, globals or reference referents. Reject these declaration
boundaries even when unused. Borrowing rvalues, dynamic indexed places, implicit
autoderef, methods, reference patterns, coercion-based reborrows, raw pointers,
casts, pointer comparisons and escape remain explicit later work. Ordinary
struct/tuple fields may be borrowed separately. Static array-index borrows
conservatively overlap the entire array. Dynamic indexing through reference
bases is explicitly deferred too; Copy arrays can first be snapshotted into an
owned local. One dereference followed by static fields/indices is supported,
but downcasts after dereference and enum-payload dereference pattern matching
are deferred until canonical target variant authorization is proved.

## Semantic and CFG checkpoint

The shared types are `BorrowKind::{Shared,Mut}`,
`Ty::Ref { kind, referent: Arc<Ty> }`, `Projection::Deref`, checked borrow and
dereference expressions, and CFG `LoanId` plus immutable per-function loan sites
containing kind, source place and expression origin. Each borrow site is unique.
`Rvalue::Borrow(LoanId)` has no runtime loan table. Frozen metadata is
`cfg::LoanSite { kind: BorrowKind, place: Place, origin: Option<ExprId> }`,
`LoanId(pub usize)` and `Function.loan_sites: Vec<LoanSite>`. Source sites carry
the actual expression origin. Each source borrow expression has one site and
one lowered borrow instruction; repeated synthetic materialization rejects.

Deref does not index a static child in the move-path tree. Resolve its reference
carrier through forward reference-origin sets, canonicalize every possible
referent target, and check authority and initialization on the actual target.
Do not eagerly move an `&mut` handle merely to dereference or reborrow it.
A pure ignored dereference (`let _ = *r` or wildcard-only match) introduces no
carrier or referent read, including for an uninitialized or moved carrier.
An actual selected read or borrow issuance still checks initialized carrier
storage and canonical provenance. This differs from indexed wildcards, which
inspect array readiness and bounds.
Reference reassignment changes that carrier, not aliases or child loan origins.

## Required loan analysis

Compute bounded forward reference origins and backward carrier liveness to a
fixed point before deterministic diagnostics. Active loans include the origins
of live carriers and parent ancestors needed by live reborrows. Borrow issuance
requires permission and initialized live storage even when its result is unused.
A shared loan freezes overlapping mutation, moves and storage end. An exclusive
loan freezes overlapping outside reads as well. Reborrow authority is tracked
separately from conflicting outside access. A child can outlive a parent carrier,
but cannot outlive the referent storage or its parent loan authority.

Real last-use behavior is required, not guessed lexical EndLoan markers. Direct
owner mutation after the last reference use can succeed. Branch origin sets
union conservatively, liveness follows actual CFG successors, and ambiguous
authority must fail closed or receive an explicit unsupported diagnostic.
A loan issued within a loop SCC and carried live across a backedge receives an
explicit unsupported diagnostic. Parent-provenance cycles also fail closed. Loans
issued outside the loop and ordinary per-iteration last-use borrows are admitted.
Ambiguous writes require every candidate initialized and authorized, preserve
initialization bits, and invalidate selection proofs for every possible target.
Unknown-index writes overlap all borrowed elements and invalidate contained
variant proofs. Existing initialization, variant proof and resource checks remain
mandatory for both check and native emission.

Bounds: 4096 loan sites, one million retained origin/target/parent memberships, existing
128 MiB retained-state and 32-million work limits, 128 projection depth, 16384
blocks and 65536 locals. Exhaustion is incomplete, never a passing analysis.
No exponential path enumeration or unbounded retained proof dump.

## Native addressing and later stages

Thin references use eight-byte target addresses, not aggregate by-value ABI.
Address generation must distinguish frame storage from a validated runtime
reference plus projections. Never encode a runtime pointer as an rbp offset,
copy an aggregate instead of borrowing its storage, or publish host addresses.
Both graph and standalone checked emission require the exact successful full
initialization-and-loan analysis before trusted code generation.

B2 adds direct noescape function reference parameters, abstract formal referent
roots, caller conflict checks and genuine expected-site reborrows. B3 adds
reference returns/escape with signature regions, outlives and provenance
substitution. Reference-containing aggregates, temporary lifetime extension,
Drop, method two-phase borrowing, slices and general Rust lifetimes follow.
B1 is not advertised as a complete Rust borrow checker.

## Acceptance gates

External stock cases must cover shared aliases and last use, exclusive owner
conflicts, immutable mutable-reference carriers, mutable-handle moves, shared
and mutable reborrows, child/carrier/storage lifetime distinctions, disjoint
struct fields versus array overlap, branch and loop origin flow, dangling
storage, unknown-index mutation conflicts and non-Copy dereference rejection.
Private malformed CFG must reject missing provenance, Copy of mutable refs and
fabricated references. Execute scalar i32/usize and Copy aggregate dereference
transport at workers 1 and 8, with graph facts retained exactly once. Existing
corpora remain regression gates. No full compatibility or speed claim follows
from this bounded stage.
