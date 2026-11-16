# Live frontend ownership domain

## Status

The owned `stable` compiler now has experimental ownership and revision-safety
building blocks for live typed frontend retention. The explicit
`--jr-resident-owned` development mode connects common native owners to serial
requests, fresh contexts and revision-aware source reads. It **does not connect
cross-revision typed query lookups**, incremental persistence or managed hosted
routing. Native compiler checks, scoped ownership tests and a runnable build pass.
The initial std-based test exposed historical macro hygiene reaching a fresh
crate store with an old numeric crate ID. Its failed original receipt remains
stored as `bench/stable-resident-owned-20261116-1334-native`. A revision-local
hygiene traversal boundary fixes that failure without resetting historical
contexts. The corrected frozen compiler passes eight same-PID source-edit,
error/recovery and nonce-checked executable transitions with exact fresh-compiler
diagnostics. That original is stored and verified as
`bench/stable-resident-owned-20261116-1346-native`. This is correctness evidence
for the scoped request loop, not typed-cache reuse or a speedup. No full live
engine or default promotion follows from this mode.
The previously measured `JUSTRUST_RUSTC_RESIDENT_FRONTEND` mode still retains
encoded snapshots and starts a fresh `GlobalCtxt` per request. Its results do
not measure the live design described here.

## Why a second cache is insufficient

A `Ty<'tcx>`, `TypeckResults<'tcx>` or MIR `Body<'tcx>` can refer to an interner,
an arena, `DefId`s, allocation IDs, symbols, hygiene contexts, spans and HIR
identities. Keeping a pointer after a request returns does not keep those
owners alive or make their numeric identities meaningful in the next request.
Matching a dependency fingerprint does not establish that ownership either.

The current request path recreates `SessionGlobals`, `Compiler`, `Session`,
`GlobalCtxt`, interners, arenas and query caches. The lifetime origin is
`rustc_interface::passes::create_and_enter_global_ctxt`. Its local owners are
deliberately destroyed at the end of one request. The ordinary path still owns
its type-interner resources alongside that same context. Consequently, adding a
process-global map of borrowed MIR or type-checking pointers would be unsound.

## Implemented building blocks

- `ty::with_resident_type_domain` owns native compiler and HIR arenas, native
  `CtxtInterners` and a native allocation-ID map for an entire higher-ranked
  callback. Actual `Ty`, `TypeckResults` and MIR objects can live under that scope.
  `GlobalCtxtResources` selects either fresh owners or borrowed domain owners at
  context construction. The context's hot interner/allocation fields are plain
  references, not per-query ownership-variant switches. A reference cannot escape
  the callback, including by returning an object that contains it. Reset means
  leaving and dropping the whole scope, not just clearing a map. Ordinary contexts
  still construct fresh resources, and their query states remain revision-local.
- The domain owns up to 64 native `OnceLock<GlobalCtxt>` cells, with context
  storage destroyed before its borrowed resources. The explicit
  `create_and_enter_global_ctxt_in_domain` entry shares the existing compilation
  pipeline and refuses exhausted storage before incremental preparation. The
  additional-owner factory can scope a component-owned compiler arena under the
  same lifetime without introducing a dependency from `rustc_middle` back to
  `rustc_interface`. The experimental owned request mode uses this entry, but
  no live query lookup is connected. The cell
  count is a structural bound, not an owning-arena byte or RSS budget.
- `rustc_interface::resident::with_resident_compiler_batch` constructs one serial
  worker, one registry, and scoped native compiler/session storage under the type
  domain. Its explicit per-revision entry finalizes diagnostics and rejects custom
  capture/admission/loading and incompatible global settings. The owned transport
  selects it explicitly. Incremental persistence is refused until source encoders
  have an active-revision file view. It has no owning-byte budget and does not
  authorize query reuse.
- `ty::resident_identity` checks bounded, exact, ordered crate and definition
  mappings, namespace and options. It rejects malformed identity universes and
  issues opaque authorization tied to the exact validated snapshots. The caller
  still has to construct complete identity material from real compiler state.
- `ty::resident_values::ResidentValues<'domain, 'identity>` retains actual borrowed
  type-checking and MIR objects, not encoded bytes. It stages a revision,
  publishes only on explicit success, and requires authorization before reading
  a previous published generation. It does not extend lifetimes. Entry and
  estimated reachable-byte budgets bound the cache's references, not the arena
  or the compiler's total RSS.

- `SessionGlobals::enter_resident_revision` exclusively borrows the existing
  scoped owner, preserving symbol/span/hygiene identities while replacing the
  per-revision metavariable map that cache encoding can freeze. It restricts
  `$crate` name updates to contexts created in the current revision so the new
  resolver does not traverse historical contexts through a fresh crate store.
  It requires all prior workers to have returned. It neither refreshes source
  files nor validates foreign crate or expansion identities for typed reuse. The
  experimental owned batch calls this entry, but does not reuse old typed query results.
- `SourceMap::new_source_file_version` explicitly appends a native source version.
  Historical byte positions still resolve to their original source and diagnostic
  coordinates, while stable-ID lookup selects the latest version. The explicit
  RAII `begin_resident_source_revision` dispatches ordinary parser `load_file`
  calls through fresh first reads, exact raw-content reuse and bounded tracking.
  Repeated reads freeze within the scope, loading errors poison that revision,
  and reads outside an enabled scope fail closed. Default one-shot loading is
  unchanged. A fallible active-revision view preserves original all-history
  indices and excludes unloaded historical files from dep-info and local source
  identity hashing. Metadata and query-cache span encoding still use historical
  indices, so incremental persistence remains refused. This is not a
  span-remapping policy and does not authorize reusing old results.
- `IncrCompSessionPaths` separates the immutable directory view used by contexts
  and codegen from `IncrCompSession`, which still owns the real filesystem locks.
  `GlobalCtxt` owns a cloned path snapshot, so keeping a context does not borrow
  the lock owner. The ordinary executor still holds locks through cache writes,
  codegen joining and finalization. A retained snapshot grants no right to read
  a directory after the executor has released its lock or renamed that directory.
- The native interface now separates owned `Compiler` construction from borrowed
  execution and diagnostic finalization. An enclosing scoped owner can choose
  storage for the compiler while construction, execution and teardown remain
  inside the normal worker/global and frontend-admission scope. This extraction
  does not repeat an invocation or connect the resident driver to such an owner.

These use native compiler types. An actual self-contained two-revision compiler
analysis test retains project-produced `TypeckResults` under the domain, reads
both results after the second context, and checks common primitive interning.
It does not transplant either result into the second query cache or validate
standard-library metadata compatibility. The initial identity gate rejects changed
source or body revisions. It must not be loosened to accept edits merely to
produce a hit counter. A later selective reuse path needs the normal green
validation plus complete owner/source identity validation.

## Required integration before measuring the requested engine

1. Move the request batch under one correctly scoped compiler executor. The
   same worker registry and shared type-interner universe must outlive all
   retained typed objects. Hoisting an arena alone is insufficient.
2. Give revision-local sessions, diagnostics, query states, definitions and
   HIR inputs explicit ownership under that executor. Release incremental
   directory locks and finish diagnostics normally even while type owners are
   retained. Retaining an entire old session with permanent allocations is not
   an acceptable substitute for that lifecycle.
3. Establish a coherent symbol/hygiene/source universe. Source changes must
   be read freshly. Old spans must either retain their precise source version
   with correct diagnostic remapping or force recomputation. Crate/definition
   remapping, item insertion/reordering, native allocation IDs and proc-macro
   isolation require explicit handling or conservative reset.
4. Connect live query lookups only after green validation and normal diagnostic
   replay. Require compatible numeric/stable identities for everything reachable
   from a retained result. Preserve incremental hash verification. Red results
   compute in the current revision and failed revisions cannot publish.
5. Bound the owning batch, not just the cache map. Eviction of a reference does
   not reclaim an arena containing it. A domain retirement/reset must finish
   outstanding users and release all owners together, including on failure.
6. Demonstrate actual project-produced type-checking/MIR pointer reuse across
   changed-source revisions, exact diagnostics and runtime results, explicit
   failure recovery and bounds. Counters for archive loads or synthetic unit
   allocations do not satisfy this gate.

Then freeze one exact compiler, measure the same fresh/process-only/live modes
with isolated rotating workloads and production-guest dogfood, and retain the
original generated evidence. General speed claims or managed-hosted default
promotion additionally require the comprehensive session-weighted suite and
managed worker integration. The published client and ordinary compiler path
remain unchanged during development.
