# Splitting PARSE and RESOLVE into per-file and per-item facts

Status: design, not implemented. Owner: zebra. Date: 2026-10-19.
Scope agreed with cricket's swarm: PARSE and RESOLVE only. SIGNATURES (`PlanCtx`,
owned by llama) and MONO_PLAN (bat and llama) are out of scope until their
owners agree on a successor design. No gate is loosened by this work.

## Why the nodes are whole-program today

`SOURCE` is already per file (`[0]`, `[0, i, ...]`, `[1]` prelude). Everything
after it is one join:

1. `PARSE` runs `SourceAssembler`, which reads every `SOURCE` unit and splices
   them into one `PreparedSource` text, then parses that text once.
2. Parsing builds one `TokenSession`, one `NameStoreBuilder` and one
   `ModuleArena`. Every handle (`AstIdent`, `SpanId`, `SyntaxContextId`,
   `ModuleId`, `NameKey`) carries the session/arena owner id, and about 40 sites
   reject handles from a different owner. A file cannot be parsed alone
   because its handles would have a different owner from its siblings'.
3. Textual `macro_rules!` scope crosses files: a macro defined before
   `mod child;` is visible inside `child.rs`. The parser threads one
   `SourceItemMacroDriver` through the whole spliced text.
4. `resolve(&ParsedProgram)` walks every item bucket, every import and every
   alias in one `Resolver` with program-wide memo tables, and publishes one
   `Program`.

## Target graph

| Kind | Key | Reads | Publishes |
|---|---|---|---|
| `SOURCE` (exists) | file ordinal | parent `SOURCE` | prepared file unit |
| `PARSE_FILE` | file ordinal | own `SOURCE`, parent `PARSE_FILE` output 1 for the entry macro scope | 0: file syntax (items, local names, spans); 1..n: textual macro scope at each `mod child;` site, published as soon as the parser passes that site |
| `MODULE_INDEX` | module path ordinal | the `PARSE_FILE` that contains the module | the module's declarations per namespace with visibility, plus its unresolved imports |
| `RESOLVE_IMPORT` | (module, import index) | `MODULE_INDEX` of each module along the path | the resolved target, or an explicit cycle/unresolved fact |
| `RESOLVE_ITEM` | item id | its own `MODULE_INDEX`, the `RESOLVE_IMPORT` facts its paths use | resolved item (types, bounds, paths) |

`PARSE` and `RESOLVE` disappear as root keys. Downstream consumers that need
one item read that item's facts. The temporary `Program` that SIGNATURES still
needs is assembled at the SIGNATURES boundary (SIGNATURES is already debt) so
the split never adds a new join.

## The three blockers and how each is removed

### 1. One name/module owner per compilation

Per-file sessions with crate-scoped resolution:

- Each `PARSE_FILE` owns its own `TokenSession`/`NameStore`. The owner id is
  derived deterministically from (snapshot, file ordinal), not from a global
  `AtomicU64`. This also removes one `static` plus atomics per identity module
  from `out_of_graph_state.baseline`.
- Constraint found while scoping: 39 authority test files (`*_authority.rs`)
  depend on two parses of the same text getting different owners, so that a
  handle from one rejects as foreign in the other. A pure (file ordinal) owner
  would collide. The owner must be (engine snapshot nonce, file ordinal), with
  the nonce issued once per `Engine` by the engine (allowed engine state) and
  passed through `Snapshot`. Direct non-graph API callers get a fresh nonce
  per call. That keeps foreign rejection and removes the per-module counters.
- Handles stay owner-checked. Cross-file comparison goes through canonical
  keys (`NameKey` atom spelling plus syntax context), which the resolver
  already uses for semantic equality. Owner checks move from "same session" to
  "owner is one of the files of this crate snapshot", validated against the
  explicit file list.
- `ModuleId` becomes (file owner, local index). The crate module tree is the
  union of per-file subtrees linked at `mod child;` sites, which `SOURCE`
  already identifies by ordinal.

### 2. Textual macro scope across files

Model it as data flow, not shared mutable state. `PARSE_FILE[parent]` publishes
output `1 + i`, the immutable textual macro scope in effect at its i-th
`mod child;` declaration, as soon as parsing reaches that point.
`PARSE_FILE[child]` reads exactly that output. The child does not wait for the
rest of the parent. Exporting macros upward (`#[macro_use] mod`) stays an
explicit unsupported diagnostic until it gets its own fact.

### 3. Program-wide resolver memo tables

Memoized import and alias results become `RESOLVE_IMPORT` facts, so sharing
is through graph identity instead of `HashMap`s in one `Resolver`. Import
cycles (and later glob fixed points) use an explicit SCC protocol: a
`RESOLVE_IMPORT` that reaches an active import on its own path publishes a
cycle fact instead of waiting. This is the convergence protocol
IMPLEMENTATION.md G9 asks for.

## Diagnostics and ordering contract

Measured and pinned in `tests/source_error_order.rs` (workers 1 and 8, check
and build):

1. Every file is prepared (delimiter balance, child loading, size limits)
   before anything parses. Any preparation error beats every parser error,
   even a parser error earlier in the parent. A child's preparation error
   beats a parent parser error before `mod child;`.
2. Among preparation errors, a file's own error beats loading its children.
3. Parser errors follow assembled order: each child is spliced at its
   `mod child;` site, so a child's error precedes parent text after the
   declaration, a grandchild precedes later text at every ancestor level, and
   siblings follow declaration order rather than file name.

So the order is not min(file ordinal, offset). The per-file pipeline keeps it
explicitly: each error fact carries (phase, assembled position), where the
assembled position is the canonical child-ordinal path plus byte offset,
compared as the depth-first splice order (a child sorts at its parent's
declaration offset). CHECK reports the minimum by (phase, assembled position),
with preparation before parse. That ordering is structural over ordinals, never
a hash or a file-name sort.

## Increments, each independently mergeable

### Increment 1 owner derivation (for review, not activated)
Today eight process-global counters issue owner ids through `allocate_identity`:

| Domain | Site | Issued per |
|---|---|---|
| `SourceSnapshot` | token_session.rs `seal` | sealed source map |
| `TokenSession` (`SessionId`) | token_session.rs `TokenSession::new` | session |
| `ModuleArena` | module_identity.rs `ModuleArenaBuilder::new` | arena (source or catalog) |
| `ScopeArena` | semantic_scope.rs `for_names` | scope arena |
| `SourceItemMacroDriver` | source_item_macro.rs `new` | item macro driver |
| `LocalMacroDriver` | source_macro.rs `new` | function macro driver |
| `TokenSlice` (`SliceId`) | macro_cursor.rs `new` | owning token slice |
| `PreparedView` | source_lex.rs | prepared lex view |

Replacement: an `OwnerPath` value, structural and compared whole, never hashed
or truncated into authority:

```text
OwnerPath = Root(EngineAuthority) / Child(domain, ordinal) / ...
EngineAuthority = the engine's own graph id (engine state, allowed)
domain = one closed enum tag per row above
ordinal = position among siblings of that domain under the same parent,
          assigned by the parent while it is exclusively owned (&mut)
```

- Domain separation: the domain tag is part of every step, so a session and a
  module arena at the same position are never equal.
- Distinct issued owners never share a path. Every issue is a child of a
  unique parent, numbered by a counter inside that parent's `&mut` builder.
  Two sessions in one file are `File[k]/Session#0` and `File[k]/Session#1`.
  Expansion drivers and slices are children of the session that created them,
  so every expansion occurrence is a structural child.
- Standalone direct parsing (`frontend::parse`, unit tests, authority tests)
  creates a fresh `EngineAuthority` per call, issued by the same engine-id
  source, so two direct parses of the same text still reject each other's
  handles as foreign.
- Equality checks compare the full path, so cost scales with depth (bounded by
  the file/expansion depth limits already charged).
- No pointer identity fallback, no global cache. `allocate_identity` and the
  eight `static NEXT` items are deleted in the same commit that activates
  this, which shrinks `out_of_graph_state.baseline`.

Required regression tests before activation, each at workers 1 and 8 and
repeated runs: foreign-snapshot handles reject; duplicate occurrences from two
sibling sessions reject; swapped source maps (same text, different snapshot)
reject; concurrent direct parses of identical text get distinct roots.

Review status (scratch commits 75e6521c4, 6081dfe90, not pushed): the
foundation is in `src/compiler/owner_path.rs` and `Context::owner_root`.
First review found reusable root/file factories, allocation before payment,
and a wrapping graph-id counter; all three are fixed. Roots are single-mint
and not `Clone`, files are issued rather than positional, steps are paid
before allocation, and graph ids fail closed at exhaustion.

Resolved (scratch 46bf8c9e6 + a95b5df48): step storage is sealed into hare's
immutable `RetainedBytes` receipt before the path is shared.
`owner_path_reaches_no_mutable_state` in `tests/graph_integrity.rs` enforces
it. It failed before the swap and passes after. Remaining before activation:
coordinator review of mint semantics, the hog constructor hunk, and the main
freeze.

### Activation representation (found while scoping the 8 constructors)

Handles are `Copy` and carry an 8-byte owner: `AstIdent {owner: SessionId,
index: u32}` (size pinned at 16 bytes), `ModuleId`, scope handles, `SpanId`
and others. About 65 sites read or compare these owners. An `OwnerPath` (two
`Arc`s, variable depth) cannot live inside a `Copy` handle without breaking
the pinned layouts, adding refcount traffic to every identifier comparison,
and making hot handles non-`Copy`.

Constraint: a handle is validated against the container that checks it
(`ident.owner == names.session.id()`), and the handle itself carries no root.
So the 8-byte owner tag must be unique across every live root, not only within
one. Per-root tags are unsound: two identical direct parses would issue equal
tags, and a grafted identifier would pass. `tests/owner_authority.rs`
(`identical_direct_parses_issue_distinct_owners`) catches exactly this.

Options under review (no code yet):

1. Split the handle owner into `EngineAuthority` plus a per-engine local tag.
   `AstIdent` is 8 + 4 bytes padded to 16, so `{authority: NonZeroU64,
   local: u32, index: u32}` keeps the pinned 16-byte layout. Engine authority
   separates roots. The local tag must be unique within one engine across all
   of its producer nodes, which needs an engine-issued per-producer range.
   That is the remaining design question.
2. Keep the 8-byte owner, but allocate it from engine-owned state (the engine
   already issues fail-closed node ids) instead of per-module statics. This is
   the smallest change and stays globally unique. It moves the counters into
   the engine, which is allowed engine state, rather than deriving them
   structurally from `OwnerPath`. `OwnerPath` remains the published authority
   of record next to it.

Option 2 removes the eight compiler statics with no handle layout change, and
fits GRAPH-INTEGRITY.md (engine internals are allowed). It needs coordinator
approval because it reinterprets "structural derivation" as "engine-issued,
recorded alongside the structural path".

#### Option 2 exact API and accounting (for approval)

Engine (`src/engine.rs`):

- `Shared` gains one field, `next_owner_tag: AtomicU64`, starting at 1. This
  is engine-internal state; GRAPH-INTEGRITY.md allows engine internals, and
  the compiler-code scan does not cover `src/engine.rs`.
- The tag space is shared by every engine and by standalone work, so it stays
  globally unique across live roots. One process-wide engine-module counter
  replaces the eight compiler statics. A per-`Shared` counter alone would let
  two engines collide.
- `pub struct OwnerTag(NonZeroU64)`: `Copy`, 8 bytes, no public constructor,
  no conversion from `OwnerPath` or from integers. Only the engine mints one.
- Minting is reachable only through `OwnerIssuer::issue`. The issuer holds a
  private engine `TagMint` capability, taken from the `OwnerRoot` it consumed
  (`Context::owner_root` or `OwnerRoot::standalone`). `TagMint` is neither
  `Clone` nor public.
- Allocation goes through `allocate_graph_id`: checked CAS, fails closed at
  exhaustion, never wraps.

Compiler (`src/compiler/owner_path.rs`):

- `OwnerIssuer::issue(domain) -> Result<Issued, OwnerError>`, where
  `Issued { issuer: OwnerIssuer, tag: OwnerTag }`. The child path and its tag
  are created together, in one paid step.
- Containers (`TokenSession`, `NameStore` via session, `ModuleArena`,
  `ScopeArena`, both macro drivers, `TokenSlice`, `PreparedView`) store
  `(tag, OwnerPath)`. Handles keep 8 bytes and compare `tag`; layouts are
  unchanged (`AstIdent` stays 16 bytes).
- Published facts carry `OwnerPath` as the authority of record. The tag is a
  lookup accelerator that is never published alone.

Accounting:

- The tag is 8 bytes stored inside an already paid container, so no new
  retained storage. Issuing charges `Work(1)` for the tag plus the existing
  path charge, both before allocation.
- No new budget handle: `TagMint` holds no budget, and published paths still
  carry only `RetainedBytes`.

Removal in the activating commit: the eight `static NEXT`/`allocate_identity`
sites listed above, then `GRAPH_INTEGRITY_SHRINK=1` to record the smaller
baseline. Gates before the commit: `tests/owner_authority.rs` (identical
direct parses with grafted handles reject, swapped stores reject, concurrent
roots distinct), new cases where two engines and two sibling sessions issue
the same domain sequence and a cross-graft rejects, and
`owner_path_reaches_no_mutable_state`, plus workers 1/8 targeted suites only
while the main CPU window is reserved.

#### Activation blast radius (measured on 677ea96ab)

Foundation landed in scratch (ef3a5e285): `OwnerTag`, sealed `TagMint`, and
`OwnerIssuer::issue -> Issued {issuer, tag}`. Tags are minted only after
payment, and the ordinal commits last. Gates pass: owner_path 14/14
(sibling, cross-root, 16x8 concurrent and engine/standalone tags distinct; a
failed issue consumes no ordinal; tag exhaustion fails closed),
owner_authority, graph_integrity with the reachability gate, and clippy.

Swapping the 8 statics for issued tags changes these constructors and their
callers (production and tests):

| Constructor | Files calling it |
|---|---|
| `TokenSession::new` | 21 (frontend, ir, cfg, ownership, semantic_*, macro_*, lexer, source_*) |
| `ModuleArenaBuilder::for_source/for_catalog` | 13 (frontend, traits, traits/catalog, ir, cfg, backend, ownership, ...) |
| `ScopeArenaBuilder::for_names` | 10 |
| `SourceItemMacroDriver::new`, `LocalMacroDriver::new` | 2 each (frontend + own module) |
| `SourceSnapshot` seal, `TokenSlice`, `PreparedView`, token tree stream | internal to their module |

Lowest-churn sequencing: thread the issued tag by deriving it from the
already-owned parent. Sessions get theirs from the snapshot owner, arenas and
drivers from the `NameStoreBuilder`'s session. Only `SourceSnapshot` creation
(one issuer per input) and the catalog arena (one per `PlanCtx`) take an
issuer argument. That limits the public signature change to the snapshot
constructors and `for_catalog`, and the other sites change only internally.
This plan still needs a coordinated merge order with the owners of
`frontend.rs`, `traits.rs`, `ir.rs` and `cfg.rs` before any edit.

Exact map (checked 2026-10-19 on 677ea96ab): every production constructor
site is in `frontend.rs`. The sites in `backend.rs`, `cfg.rs`, `ir.rs`,
`traits.rs`, `traits/catalog.rs` and `type_check.rs` are all test fixtures.

- `TokenSession::new`: `ProgramBuilder::new`, `native_input`,
  `validate_inactive_syntax`. This needs an explicit issuer argument.
  `SourceSnapshot` is an `Arc` cloned at about 30 sites, so deriving a tag
  from it would put a mutable issuer inside a shared immutable value.
- `ModuleArenaBuilder::for_source`, `ScopeArenaBuilder::for_names`, and both
  macro drivers derive their tag from the session they already hold
  exclusively, with no signature change.
- Owner map from the coordinator: dromedary owns the frontend.rs production
  hunks (ACK pending). Dragon's `associated_types.rs` is not touched.

#### Review of activation step 1 (scratch 76eef8201): two defects

1. Bare tag replay. `TokenSession::with_owner(.., OwnerTag)` accepts a `Copy`
   tag, so a caller could build two sessions with one tag, possibly over
   unrelated snapshots, and grafts between them would pass. The agreed
   contract was a non-`Copy` issued pair. Fix: `with_owner` consumes an
   `Issued` (path plus tag, not `Clone`). The session stores the `OwnerPath`
   next to the tag, so one issuance yields exactly one session. A test
   asserts that `Issued` cannot be reused (a compile-fail doctest) and that
   the session exposes its path.
2. Standalone accounting bypass. `standalone_tag` charged the new root to a
   private default ledger, so an exhausted or zero caller budget could still
   mint a root and session. Fix: every standalone root and path is paid
   through the caller's budget, before allocation. Session-owned owner
   storage is a refundable `ByteLease` while the session is live (so drop
   still returns the caller to zero, which keeps the existing accounting
   tests honest). It becomes an immutable `RetainedBytes` receipt only when
   the store is frozen for publication. Tests: an exhausted caller budget
   rejects before any root or session allocation; a zero-byte limit rejects
   at admission.

No accounting exclusions, no widened debt.

#### Frozen store reachability: exact binding design

Gate: `published_stores_reach_no_builder_owner_types` (scratch 45c54b60c,
currently ignored) must pass un-ignored for `NameStore`, `ParsedProgram`,
`Program` and `OwnerPath`.

Types:

- `TokenSession`: has no owner field, enum, path or mint. It keeps only its
  8-byte `SessionId` (from the issued tag).
- `SessionOwnerCap`: not `Clone` or `Copy`, no public constructor. It holds
  `{ session: SessionId, path: BuilderOwnedPath }`. Only
  `TokenSession::with_owner(snapshot, budget, Issued)` and
  `TokenSession::standalone` create it, and they return
  `(TokenSession, SessionOwnerCap)` bound to that session's id.
- `NameStoreBuilder::new(session, cap, edition)` checks
  `cap.session == session.id()` and rejects a foreign or swapped cap with
  `TokenError::ForeignHandle`, even when budgets and snapshots are equal. The
  builder keeps `cap.path`. `TokenSession` itself never holds it.
- `NameStoreBuilder::freeze(self)` consumes the builder path into
  `NameStore { owner: OwnerPath }`. The published store holds only the
  immutable `OwnerPath` receipt.
- Session-only fixtures (token-level tests that never build a store) drop the
  cap, which refunds its lease. Without a cap they cannot construct a
  `NameStoreBuilder`, so they cannot publish a store and duplicate same-owner
  stores cannot reappear.

Tests: the un-ignored type gate; swapped pairs (two sessions, crossed caps)
rejected; a cap reused for a second store doesn't compile (compile_fail);
exhausted or poisoned callers mint nothing; and the existing replay and
accounting regressions.

## Coordination
- Kind ids reserved by cricket: 121 `PARSE_FILE`, 122 `MODULE_INDEX`,
  123 `RESOLVE_IMPORT`, 124 `RESOLVE_ITEM`. 114 stays retired. 116/117 are
  lexical/capture, 118-120 are reserved for const work.
- Owner ids must be collision-free by construction: the canonical
  child-ordinal ancestry path under a per-engine snapshot nonce. Never a hash
  used as authority.
- Every `ExpansionBudget`/`PlanBudget` handle reachable from a `NameStore`,
  `ModuleArena` or session is dropped or frozen before a file fact is
  published. `fact_reachable_state.debt` must not grow.
- Increment 1 lands only after the full `*_authority.rs` suites pass on it.

- The parse entry and `Program` hunk is shared with koala (captures),
  dromedary (const collections) and hog (item macro parser). Changes to
  `parse_syntax_unit`, `ProgramDeclarations` and `SourceItemMacroDriver` are
  announced before landing.
- `PlanCtx`, `instances.rs` and MONO_PLAN are not touched.
- Const items (dromedary, kinds 118-120) add `DeclKind::Const` and new
  `ProgramDeclarations` collections. Proposed order: Const lands first, then
  this lane rebases its signature-only owner threading onto it.
  `MODULE_INDEX` lists Const as a value-namespace declaration with no
  callable index. This lane never edits `DeclKind`, `ProgramDeclarations`
  fields or `Expr`.
