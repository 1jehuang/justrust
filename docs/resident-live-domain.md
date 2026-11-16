# Live frontend ownership domain

## Status

The owned `stable` compiler now has experimental ownership and revision-safety
building blocks for live typed frontend retention. They are **not connected to
the resident request driver or query execution**. No live cross-edit engine,
production integration, speedup, or default promotion follows from these APIs.
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
deliberately destroyed at the end of one request. Type interning is currently
owned by that same context. Consequently, adding a process-global map of
borrowed MIR or type-checking pointers would be unsound.

## Implemented building blocks

- `ty::with_resident_type_domain` owns a native compiler arena and native
  `CtxtInterners` for an entire higher-ranked callback. Actual `Ty`,
  `TypeckResults` and MIR objects can live under that scope. A reference cannot
  escape the callback, including by returning an object that contains it.
  Reset means leaving and dropping the whole scope, not just clearing a map.
- `ty::resident_identity` checks bounded, exact, ordered crate and definition
  mappings, namespace and options. It rejects malformed identity universes and
  issues opaque authorization tied to the exact validated snapshots. The caller
  still has to construct complete identity material from real compiler state.
- `ty::resident_values::ResidentValues<'domain>` retains actual borrowed
  type-checking and MIR objects, not encoded bytes. It stages a revision,
  publishes only on explicit success, and requires authorization before reading
  a previous published generation. It does not extend lifetimes. Entry and
  estimated reachable-byte budgets bound the cache's references, not the arena
  or the compiler's total RSS.

These are native compiler types and native ownership tests, not successful
multi-revision project compiles. The initial identity gate rejects changed
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
