# Live frontend ownership domain

## Status

The owned `stable` compiler now has experimental ownership and revision-safety
building blocks for live typed frontend retention. The explicit
`--jr-resident-owned` development mode connects common native owners to serial
requests, fresh contexts and revision-aware source reads. Its default behavior
still refuses incremental persistence and does not reuse typed query values.
The separate private `JUSTRUST_RUSTC_RESIDENT_TYPED=1` option now connects
post-green native `typeck_root` and `optimized_mir` lookups, with exact same-source
identity authorization and publication only after complete transport success.
`JUSTRUST_RUSTC_RESIDENT_TYPED=control` permits incremental persistence under the
same owners without enabling those lookups. Neither option selects managed hosted
routing. Native compiler checks and ownership/publication tests pass. The frozen
1446 compiler also passed an explicit MIR-demand acceptance workload locally and
on an authenticated production guest: each enabled worker recorded ten retained
native MIR hits across unchanged revisions, while the control stayed at zero.
Body edits, signature edits and failures produced no new hits. Type-checking hits
were zero. The original receipts are stored with exact authenticated JSON readback:
`bench/stable-resident-typed-20261116-1450-mir-native` and
`bench/stable-resident-typed-host-20261116-1451-native`. This validates only local
`no_core`, same-source reuse with exact diagnostics and a narrow constant-return
LLVM artifact oracle. It is not general changed-source reuse, real-project
Typeck reuse, a speedup, managed routing or default production activation.
The initial std-based test exposed historical macro hygiene reaching a fresh
crate store with an old numeric crate ID. Its failed original receipt remains
stored as `bench/stable-resident-owned-20261116-1334-native`. A revision-local
hygiene traversal boundary fixes that failure without resetting historical
contexts. The corrected frozen compiler passes eight same-PID source-edit,
error/recovery and nonce-checked executable transitions with exact fresh-compiler
diagnostics. That original is stored and verified as
`bench/stable-resident-owned-20261116-1346-native`. This is correctness evidence
for the scoped request loop, not typed-cache reuse or a speedup. The same frozen
compiler also passed the owned request-loop and guard harnesses on an authenticated
production build guest. Shipping-module dogfood passed there with three repetitions;
its untouched original is stored as
`bench/stable-resident-owned-host-20261116-1346-dogfood` and authenticated readback
matches the original JSON exactly, including timing floats. This is a private
candidate experiment, not selection of a new default production toolchain.
No full live engine or default promotion follows from this mode.
The previously measured `JUSTRUST_RUSTC_RESIDENT_FRONTEND` mode still retains
encoded snapshots and starts a fresh `GlobalCtxt` per request. Its results do
not measure the live design described here.

## Selective primitive MIR cross-edit experiment

The separate private `JUSTRUST_RUSTC_RESIDENT_TYPED=leaf` mode adds a deliberately
narrow cross-source path without changing `RevisionIdentity::authorize_reuse`.
Only an ordinary local, nongeneric, argument-free function whose optimized MIR
assigns a primitive scalar literal to the return place and returns is eligible.
The full local definition mapping, semantic invocation and body-owner mapping
must still agree. Normal dependency-green validation, diagnostic replay and
incremental hash verification remain in force. Typeck values never use this path.

An opaque proof binds the actual retained body allocation to currently active
native source-file allocations. It checks the complete reviewed MIR shape,
primitive types and pointer-free scalar constants, retained lint roots, and all
visited spans, including their endpoints, hygiene and exact same-item parents.
It rejects historical files, foreign parents, expansions, references, complex
constants, calls, generic items, inlining and unreviewed metadata. Inspection runs
outside the live-state lock, followed by snapshot/body/currentness revalidation.
Cleared borrow-checking metadata and primitive literal retag flags are accepted
because they retain no additional referenced allocation or definition universe.

The frozen 1527 candidate passed the real three-file acceptance workload locally
and on an authenticated production guest. Editing B produced one actual retained
MIR hit for unchanged A, and editing A produced one for unchanged B. Error recovery
produced one additional leaf hit. Each enabled worker finished with three distinct
`mir_leaf_hits` and six ordinary same-source MIR hits. The control remained zero.
Fresh-compiler diagnostics and both exported constant-return LLVM oracles matched
through edits, deliberate signature failures and same-PID recovery. The originals
are `bench/stable-resident-leaf-20261116-1527-native` and
`bench/stable-resident-leaf-host-20261116-1527-native`.

Earlier failed cross-edit originals, including the strict span-parent rejection,
remain archived as `bench/stable-resident-leaf-20261116-{1516,1522}-native`.
Same-source typed acceptance, owned guards and three-repetition shipping-module
correctness dogfood also passed on the final candidate. These are bounded native
correctness experiments, not general MIR/Typeck reuse, full-client typed-cache
dogfood, a performance result, managed resident routing or default activation.
External crates and broad changed-source typed reuse remain unfinished. A general
speedup still requires representative, controlled suite measurements.

### First focused performance measurements

The frozen 1527 compiler was measured with `bench/suite.py stable-native-leaf`
using 128 primitive functions per module, eight alternating file edits and six
repetitions. Fresh-process, persistent-process control and live-leaf arms rotate
execution order. Warmup is excluded, leaving 48 measured compiles per arm.
Every accepted observation requires the exact diagnostic and exported literal
LLVM oracle, while enabled edit observations require actual retained leaf hits.

The two original local runs are stored and authenticated exact-JSON readback
verified as `bench/stable-resident-leaf-suite-20261116-1544-a` and
`bench/stable-resident-leaf-suite-20261116-1544-b`:

| Run | Fresh median | Process-only control | Live leaf | Leaf/control |
| --- | ---: | ---: | ---: | ---: |
| A | 40.281 ms | 20.402 ms | 92.676 ms | 4.543x |
| B | 42.498 ms | 17.580 ms | 108.390 ms | 6.166x |

`bench/suite.py compare` validates these identical-compiler replicate receipts.
They are not a before/after implementation change. The within-run on/off
contrasts show a large regression on this narrow workload despite real reuse.
Other machine activity is not excluded, and no statistical significance,
general-project conclusion or default promotion is claimed. Repeated complete
identity validation at each query lookup is the next optimization hypothesis,
not an established attribution. The original slow baselines remain unchanged.

The next candidate, `15a9a1f35`, memoizes full identity compatibility once per
actual private published/pending generation pair and identity budget. Replacing
the pending identity, aborting, publishing or evicting the previous generation
invalidates the memo. Rejections are memoized too. The public borrowed snapshot
authorization API is unchanged. Leaf inspection and its exact body/current-source
recheck remain mandatory, as do dependency-green and incremental hash checks.
No borrowed token is retained or lifetime extended by the memo.

The complete frozen 1555 sysroot has driver SHA256
`5b69dcb65e4e25971d5b57be0175d77c7b55a04511b4fefc9e6201ef120e9669`.
Driver checking, 69 resident native tests, two lifetime-escape doctests, strict
same-source/cross-edit acceptance, safety guards and three-repetition shipping
module correctness dogfood passed. Exact original acceptance records are
`bench/stable-resident-memo-20261116-1555-{leaf,typed,guard,dogfood}`.

Four further identical-workload local runs used old/new/new/old compiler order.
Each still rotates its three arms and has 48 measured samples per arm:

| Run | Fresh median | Process-only control | Live leaf | Leaf/control |
| --- | ---: | ---: | ---: | ---: |
| Before A | 38.250 ms | 14.351 ms | 133.361 ms | 9.293x |
| After A | 36.249 ms | 20.345 ms | 20.945 ms | 1.029x |
| After B | 38.507 ms | 20.057 ms | 15.318 ms | 0.764x |
| Before B | 42.700 ms | 21.665 ms | 104.929 ms | 4.843x |

Both `bench/suite.py compare --allow-compiler-change` comparisons passed exact
workload, host, semantic execution and artifact checks, with explicitly attested
different driver hashes. Originals are stored with authenticated exact-JSON
readback as `bench/stable-resident-memo-suite-20261116-1556-{before-a,after-a,after-b,before-b}`.
The large old-engine regression is substantially reduced, but retention itself
does not yet show a consistent advantage over the same-process control: After A
is 2.95% slower and After B is 23.63% faster. Control medians also varied. Machine
load observations remain in the receipts, and this is not a quiet-machine,
statistical, production-performance or representative-project speed claim.

On November 17, the same old/new/new/old experiment was repeated with a frozen
`d1baf2081` harness and `taskset -c 3`. New captures record the actual Linux boot
ID and effective affinity, and comparison refuses mismatched host dictionaries.
All four runs used the same boot and affinity `[3]`, with the same 128-item,
eight-edit, six-repetition workload and strict native hit/artifact oracles:

| Run | Fresh median | Process-only control | Live leaf | Leaf/control |
| --- | ---: | ---: | ---: | ---: |
| Before A | 28.642 ms | 14.457 ms | 81.650 ms | 5.648x |
| After A | 29.740 ms | 15.472 ms | 16.213 ms | 1.048x |
| After B | 28.526 ms | 17.352 ms | 17.024 ms | 0.981x |
| Before B | 30.113 ms | 15.420 ms | 91.352 ms | 5.924x |

Both strict `bench/suite.py compare --allow-compiler-change` pairs passed.
The exact originals are stored and authenticated readback verified as
`bench/stable-resident-memo-suite-20261117-0033-pinned-{before-a,after-a,after-b,before-b}`.
This repeat confirms removal of the large identity-validation regression, but
does not establish a useful extra retention win over the process-only control:
enabled medians are 4.79% slower and 1.89% faster, both below the 10% diagnostic
floor. A fixed core is not a guarantee of an otherwise idle machine.

A separate authenticated guest baseline passed with fresh/control/leaf medians
22.816/10.674/80.402 ms, archived with exact readback as
`bench/stable-resident-leaf-suite-20261116-1602-guest-before-a`.
The optimized guest upload then failed before hash verification or execution.
Subsequent normal SSH attempts encountered a stale hosted slot assignment,
a daemon-held provisioning lock and a DNS failure. The existing hosted recovery
coordinator owns that repair. No optimized guest timing, production activation,
or successful final guest dogfood is claimed for the memoized candidate.

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
  `rustc_interface`. The experimental owned request mode uses this entry. Typed
  lookup state is present only under the separate explicit typed option. The cell
  count is a structural bound, not an owning-arena byte or RSS budget.
- `rustc_interface::resident::with_resident_compiler_batch` constructs one serial
  worker, one registry, and scoped native compiler/session storage under the type
  domain. Its explicit per-revision entry finalizes diagnostics and rejects custom
  capture/admission/loading and incompatible global settings. The owned transport
  selects it explicitly. Incremental persistence remains refused by default.
  The private typed/control options permit persistence after active-source encoding
  checks. Whole-batch retirement occurs at 64 contexts or a 2 GiB boundary RSS
  check, including refusal when RSS cannot be obtained. This is not a hard peak
  allocation limit within a single compile. Retirement flushes the final response
  and leaves the owner scope, rather than merely clearing retained references.
- `ty::resident_identity` checks bounded, exact, ordered crate and definition
  mappings, namespace and options. It rejects malformed identity universes and
  issues opaque authorization tied to the exact validated snapshots. The caller
  still has to construct complete identity material from real compiler state.
- `ty::resident_values::ResidentValues<'domain, 'identity>` retains actual borrowed
  type-checking and MIR objects, not encoded bytes. It stages a revision,
  publishes only on explicit success, and requires authorization before reading
  a previous published generation. It does not extend lifetimes. Entry and
  estimated-byte budgets bound the cache's references, not the arena
  or the compiler's total RSS. Current query hooks charge the immediate native
  result size, not a traversal of all reachable allocations.

- `SessionGlobals::enter_resident_revision` exclusively borrows the existing
  scoped owner, preserving symbol/span/hygiene identities while replacing the
  per-revision metavariable map that cache encoding can freeze. It restricts
  `$crate` name updates to contexts created in the current revision so the new
  resolver does not traverse historical contexts through a fresh crate store.
  It requires all prior workers to have returned. It neither refreshes source
  files nor validates foreign crate or expansion identities for typed reuse. The
  experimental owned batch calls this entry. Typed reuse is separately gated by
  the explicit same-source identity and green dependency checks.
- `SourceMap::new_source_file_version` explicitly appends a native source version.
  Historical byte positions still resolve to their original source and diagnostic
  coordinates, while stable-ID lookup selects the latest version. The explicit
  RAII `begin_resident_source_revision` dispatches ordinary parser `load_file`
  calls through fresh first reads, exact raw-content reuse and bounded tracking.
  Repeated reads freeze within the scope, loading errors poison that revision,
  and reads outside an enabled scope fail closed. Default one-shot loading is
  unchanged. A fallible active-revision view preserves original all-history
  indices and excludes unloaded historical files from dep-info and local source
  identity hashing. Metadata and query-cache encoders now check active files while
  preserving historical indices and ordinary formats. Incremental persistence
  remains refused in default owned mode and is permitted only by the explicit
  private typed/control options. This is not a span-remapping policy and does not
  authorize changed-source reuse.
- `SourceMap::resident_raw_source_files` exposes bounded, frozen, pre-normalization
  loader bytes without rereading disk. The compiler identity builder requires
  complete one-to-one coverage of active source objects, captures actual local
  definition paths and numeric IDs, and rejects external crates or unfrozen inputs.
  A real three-context native test proves identical file revisions can authorize
  identity reuse and a changed file cannot. This completed-analysis snapshot must
  not be called from a type-checking lookup: doing so would force analysis and
  create a query cycle rather than establish pre-query authorization.
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

1. The explicit owned request batch now runs under one scoped serial compiler
   executor with a common worker registry and type-interner universe. Keep that
   ownership invariant while connecting retained typed objects to query lookups.
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

## Strict Typeck demand diagnostic (2026-11-17)

`bench/stable_resident_typeck.py` requires a positive actual `typeck_hits`
delta on successful unchanged repeats. MIR hits cannot satisfy this oracle.
Against the frozen memoized compiler from `20261116-1555`, this acceptance
**fails**: unchanged repeats record zero Typeck hits while MIR hits increase
to 15. Exact diagnostics, primitive LLVM output, disabled controls and error
recovery still pass. The original failed receipt is stored as
`bench/stable-typeck-acceptance-20261117-0047-final-original`, with authenticated
exact JSON readback verified. The dedicated harness has 26 passing mock tests.

The observed unchanged repeats have zero Typeck misses as well as zero hits.
That is consistent with normal green `ensure_ok` checks avoiding materializing
the Typeck result, rather than evidence that a retained lookup is succeeding.
This is a diagnostic inference, not a demonstrated explanation for every query.
Forcing unnecessary Typeck work merely to increase a hit counter would not
demonstrate a faster edit loop.

The native query telemetry now distinguishes revision-level identity begin and
completion rejections from query misses, and counts successful publications and
aborted revisions. Disabled mode does not increment these revision counters.
Driver typecheck, 69 resident middle tests, two lifetime-escape doctests and eight
metadata tests passed after this additive change. These counters are not present
in the earlier frozen timing compiler, and do not change its measured results.

`CStore::capture_external_metadata` now provides a bounded owned prerequisite
snapshot of the actual loaded metadata bytes and encoded-to-current crate-number
maps, including transitive crates. It does not reopen paths or substitute hashes
for bytes. Proc macros and unsupported numeric maps reject capture. This is not
permission to reuse external types: imported source translations, hygiene,
allocation decoding, complete invocation inputs and other session-local decoder
state still require validation. The existing external-crate rejection remains
in force. Neither this prerequisite nor the strict Typeck diagnostic completes
general real-project frontend residency.

The actual loaded-`std` acceptance also passes: two fresh contexts capture equal
owned metadata snapshots, repeated captures match, all three budgets reject,
and recapture after rejection still matches. The test explicitly requires the
external-crate reuse gate to keep rejecting. It selects the matching bootstrap
sysroot rather than the unit executable's default location. After that test-only
source change, matching std was rebuilt normally. No metadata version override
or incompatible-artifact acceptance was used.

The frozen `stable-resident-metadata-20261117-0057-sysroot` candidate's driver is
`d351240a07c8e4db4cb9422393787b0c1382ed7bf34b0f42cd91597de46efa96`.
Its actual same-source native-query, safety-guard and shipping-module `n=3`
correctness regressions pass. Exact original records are stored and authenticated
JSON readback verified as `bench/stable-resident-metadata-20261117-0057-{typed,guard,dogfood}`.
The enabled query fixture reports nine published and two aborted revisions;
control revision counters remain zero. These are local correctness runs, not
production performance or full-client typed-reuse acceptance.

The shared query harness now explicitly reports `observed_hit_kinds`, so its
either-kind acceptance cannot be mistaken for Typeck-specific acceptance.
The fresh report `bench/stable-resident-metadata-20261117-0100-observed-kinds`
passes and names only `mir`; its exact original JSON readback is verified.
The separate strict Typeck receipt remains failed and unchanged.

A subsequent natural-demand diagnostic adds ordinary path-statement,
unused-result and must-use-call lint consumers without changing compiler query
execution. `bench/stable-resident-typeck-demand-20261117-0351` preserves all
20 cases and the strict failed verdict: fresh/resident diagnostics and outputs
match, disabled controls stay zero, and actual Typeck hits remain zero. MIR
reuse cannot satisfy that gate. The untouched original receipt passed exact
authenticated JSON readback. These additional source fixtures do not establish
a natural Typeck reuse opportunity or justify forcing extra query work.

## Exact imported source-universe prerequisite

`CStore::capture_external_sources` pairs the exact metadata snapshot with every
already-imported external source mapping. Limits bound metadata, mapping count,
file count and declared source bytes before retaining source allocations. Missing
imports, proc macros, malformed ownership/index/endpoints and exceeded budgets
reject capture. Capture does not force imports or reopen source paths.

Equality requires the actual translated `Arc<SourceFile>` allocation and its
original/current numeric endpoints, owner and metadata index. Stable IDs, paths,
content hashes and equal metadata bytes are not substitutes for allocation
identity. This snapshot is not a reuse authorization.

The actual loaded-std test demonstrates the distinction: repeated captures within
one context match, but two fresh contexts have equal metadata snapshots and
unequal imported source universes. The test explicitly imports sources as setup,
checks incomplete pre-import rejection and both source budgets, and verifies that
external typed reuse still rejects. Ten native metadata tests and this native
cross-context acceptance pass. Matching std was rebuilt normally after an initial
incompatible-sysroot rejection, without a version override.

The native decoder now owns its imported-source map and `HygieneDecodeContext`
in one `Arc`-owned `ExternalDecoderIdentityState`. Both actual span importing and
syntax-context decoding use this bundle. Every `CrateMetadata` still constructs
fresh storage, so ordinary semantics and external rejection remain unchanged.
Storage tests prove fresh bundles are independent and a cloned bundle preserves
both source entries and the actual hygiene decoder identity. The complete native
interface suite (21 tests), metadata suite (15 tests plus two doctests), and
loaded-std source-universe acceptance pass. This supplies an ownership unit, not
cross-context binding or authorization.

An explicit, default-off native decoder owner now binds that bundle across
contexts before `CrateMetadata::new` decodes anything. Publication requires bounded
complete metadata/source snapshots and untouched foreign source-text caches
(`AbsentOk` only). Selection requires actual same `SourceMap` ownership, exact
translation settings, exact metadata bytes and numeric maps, and actual previously
validated decoder owners for every loaded dependency. No source imports are
forced by publication or selection.

Reused imported files are atomically reactivated at their original sparse source
indices. Cross-map, shadowed, local, inactive and poisoned inputs reject activation.
The decoder owner is scoped to a fresh `SessionGlobals` and drops on unwind. Nested
masking is rejected, so compilation cannot silently run outside a retained owner.

Rejection after publication requires retiring the whole owner, not silently
constructing fresh decoder storage: native `decode_expn_id` also caches foreign
expansions globally by `(CrateNum, ExpnIndex)`. Clearing the per-crate cache would
not refresh that global state. Publication failure latches retirement and cannot
be undone by publishing again.

The real loaded-std test now also compiles a `std::matches!` consumer twice with
this owner explicitly enabled. Constructor hit counters are positive and exact
imported-source snapshots match across the two fresh contexts. Disabled controls
still allocate different source universes. A forced publication-budget failure
makes the next compile refuse the retired owner, as required. External typed-query
identity still rejects in both modes. All 70 span tests, 23 metadata tests plus two
doctests, and 21 interface tests pass against matching standard libraries.

This is actual source/hygiene decoder reuse, not external typed-query reuse. It is
not wired into the resident transport or Cargo defaults. Full external typed
ownership and query identity, ordinary demand-driven bounded
publication, mutation/error fallback acceptance and representative performance
remain separate gates. Globally interning sources by stable ID alone is still not
a valid replacement, and no speed improvement follows from this native test.

## Native allocation decoder ownership

The same native decoder bundle now owns `AllocDecodingState`, initialized once
from the exact selected metadata's allocation index. Actual metadata decoder
sessions use that storage, rather than constructing a fresh per-context allocation
cache. Retention is bound to the actual `AllocMap` universe as well as the existing
exact metadata, dependency, source and hygiene checks.

Every new allocation map receives an opaque process-local monotonic identity.
Shared domain resource views preserve it; independent maps do not. The identity
is not a raw address, cannot be recycled by allocator address reuse, and is not a
serialized artifact key or sufficient reuse authorization. Published decoder state
cannot be selected or republished with a different allocation universe. A mismatch
retires the whole scoped owner before metadata decoding, even when source-map
ownership and metadata bytes are unchanged.

The native loaded-std test demands `std::env::consts::OS` through real constant
evaluation, checks its decoded allocation bytes against the host OS name, and
keeps the result across two fresh compiler contexts. Enabled decoding preserves
both the actual `AllocId` and native allocation pointer. The disabled control uses
the same allocation map but decodes distinct allocation IDs. A third context with
an independent allocation map is refused, and publication-budget retirement still
refuses a third compile. These are actual decoded external allocations, not only
synthetic bundle pointer tests.

Native validation passes all 21 interface tests, 24 metadata tests and two
metadata doctests, plus 70 middle resident tests and two lifetime doctests. Matching
standard libraries were rebuilt normally after compiler source identity changes.
The expected retired-owner diagnostics are caught by the negative acceptance
checks, not suppressed or converted into successful compilations.

The additional real dependency-mutation test builds an rlib with the matching
bootstrap compiler, decodes its exported string allocation, imports its sources
and publishes the owner. It then builds changed string/signature metadata into a
separate artifact and atomically replaces the configured `--extern` path, preserving
the old mmap-backed inode. The next compile in the same owner is refused before
constant decoding. A fresh owner successfully decodes the replacement string.
All 22 interface tests pass, including that actual replacement/retirement/fresh-owner
oracle. The test compiler's full diagnostics are retained in failure output; no
version override or mismatch suppression is used.

This closes native allocation-decoder ownership and scoped metadata-mutation
refusal prerequisites only. External typed-query admission remains rejected, and
default compiler/client selection is unchanged. No speed improvement or broad
external typed reuse follows from these tests. Demand-driven bounded publication,
complete typed identity, managed mutation/error fallback acceptance and
representative performance remain separate gates.

## Native Cargo routing boundary

**Update, 2026-11-17:** explicit std-only native Cargo and actual justrust client
routing now pass three-repeat hosted edit/error/recovery acceptance. See
[Native resident Cargo integration](resident-cargo-integration.md) for exact
candidate hashes, receipts and remaining limitations. The boundary analysis
below records the prerequisites that motivated that implementation, not a claim
that routing is still entirely unimplemented. No default promotion follows.

The original resident protocol freezes compiler arguments, cwd and environment
for the lifetime of each worker. It is not by itself a managed per-crate Cargo pool.
Native routing must live in `jr-cargo`, with a separate ordinary-toolchain path.

A worker cannot be safely keyed only by the text of its command line and reused
across Cargo invocations. `rustc_data_structures::jobserver` initializes its
inherited client in a process-global `OnceLock`. `BuildRunner::new` creates
Cargo's build jobserver after the existing `jr_daemon::park` snapshot. Therefore
two children of that parked Cargo server do not automatically share the same
jobserver lifetime. Keeping the old compiler alive can retain the previous
build's descriptors and its assumption that Cargo holds an implicit token.

Before enabling cross-Cargo routing, the native protocol needs explicit
request-scoped jobserver and implicit-token ownership, cancellation and client
liveness, completion/retirement handoff, and exact invocation/environment and
toolchain keys. Merely dropping jobserver variables, reusing descriptor numbers,
or bypassing token accounting is not a valid integration. Unsupported requests
must take the ordinary path without replaying an already executed failing
compilation or losing diagnostics. No Cargo routing or production default has
been enabled by this inspection.

### Guarded request jobserver ownership prerequisite

`rustc_data_structures::jobserver::resident::RequestLease` now provides a
separate, opt-in ownership primitive for a supplied native jobserver client.
It reserves serial admission before acquiring an actual base token, without
holding the admission mutex while acquisition waits. It never reconstructs a
client from inherited environment descriptor numbers and never replaces the
ordinary process-global client. Token acquisition may block, so the eventual
dispatcher must arrange explicit capacity rather than treating Cargo's existing
implicit token as a second available token.

Request clients and acquired tokens retain their owner. Clean completion,
errors and unwinding release the base token and permit another request. Ending
a request with outstanding tracked ownership permanently closes admission;
late acquisitions fail, and the base token stays held until that ownership is
destroyed. Clearing a request map cannot bypass this boundary. The API exposes
no conversion to raw clients or raw acquired tokens.

LLVM codegen helpers and linker token paths now use a request-aware
`CompilerClient` facade. Without an explicitly selected request it preserves the
ordinary client and helper behavior. Selected requests retain ownership through
helper callbacks and queued tokens. Helper destruction observes callback
retirement for up to one second after the underlying platform's shutdown
operation; that operation can itself block. Unretired callbacks or outstanding
tokens prevent clean request completion. A resident linker uses one thread when
nonblocking token acquisition is unsupported, rather than inventing parallel
capacity. The ordinary linker's historical fallback remains unchanged.

This is still not compiler routing. Parallel frontend proxies remain on the
ordinary path, and owned resident batches still require a serial frontend.
Request-scoped cancellation, pipe/descriptor transfer, diagnostic forwarding,
hosted admission and exact invocation/toolchain identity remain separate required
integrations. No production compiler default changes follow from adding the
ownership primitive or migrating codegen token acquisition.

## Bounded arithmetic retention and ordinary-work control

The frozen `stable-resident-arithmetic-20261117-0124-sysroot` driver
`7f99d546fbfea59000f1f90b02ca1b027151d831bb25560d24c19efdf8c9fdd4`
passes the bounded one-block primitive argument/temporary arithmetic proof,
70 native resident middle tests, and literal, same-source, guard and shipping
module regressions. Actual cross-unrelated-file edits retain eligible arithmetic
MIR while fresh C consumers verify the independently calculated outputs. This
still excludes general Typeck, external crates, arbitrary control flow and
managed Cargo routing.

The positive arithmetic acceptance requests object, LLVM IR and MIR outputs.
The separate ordinary object-only control compiles and executes correctly but
fails strict unchanged-reuse acceptance with zero retained-query hits. Its
original failed receipt and nested leaf receipt are preserved as
`bench/stable-resident-arithmetic-20261117-0137-object-demand` and
`bench/stable-resident-arithmetic-20261117-0137-object-demand-leaf-original`.
No hit requirement was weakened and no extra query demand was added to make
this ordinary-work control pass.

Two fixed-CPU diagnostic repetitions, each `n3/items64/edits6`, retain the forced
object/LLVM/MIR demand and rotate fresh, process-only and retained arms. Medians:

| Original record suffix | Fresh | Process-only | Retained arithmetic | Retained/process-only |
| --- | ---: | ---: | ---: | ---: |
| `0141-a` | 22.435 ms | 12.886 ms | 13.271 ms | 1.0299x |
| `0141-b` | 22.821 ms | 12.957 ms | 13.144 ms | 1.0145x |

The original records are
`bench/stable-resident-arithmetic-suite-20261117-0141-{a,b}`. Exact authenticated
JSON readback, including floats, matches both unchanged originals and the
object-only failure evidence. `bench/suite.py compare` reports fresh +1.72%,
process-only +0.55% and retained -0.96% between repetitions, all below its 10%
floor. Retention is 1.45–2.99% slower than process-only here, not an additional
speedup. These are forced-output diagnostic timings, not ordinary Cargo,
real-project, hosted, comprehensive synthetic or production-default evidence.

## Ordinary-output clean token retention

An independent lexer experiment now retains successful clean token streams
inside the existing native serial batch's shared symbol/source-map lifetime.
`JUSTRUST_RUSTC_RESIDENT_LEXER=1` explicitly enables it; the default and `0`
remain disabled. It does not require typed-query reuse, forced MIR output or
external-crate identity authorization. It is not an AST/HIR cache: each request
still parses, creates fresh attribute IDs, performs gates and runs diagnostics.

The cache key requires the exact retained SourceMap and SourceFile Arc identities,
edition and stripping mode, and never admits override spans. Initial frozen-0200
ASCII eligibility rejects comments, attributes, strings, lifetimes, raw/forced
identifiers, Unicode and unsupported numeric forms. Only successful token-tree
construction enters the cache. Its otherwise skipped session effect, identifier
SymbolGallery insertion, is replayed in lexical order while preserving existing
first occurrences. Token mutations use the existing copy-on-write representation.
The bounds are 128 entries, 8 MiB estimated retained bytes, 256 KiB input per
file and 65,536 token trees. Owner retirement, disabled mode and unwind cleanup
remain separate from compiler result correctness.

Native driver typecheck and all 72 parser tests pass, including exact token and
gallery replay, preexisting gallery entries, map/file/edition/strip isolation,
malformed-token exclusion, source limits, eviction and unwind cleanup. The
matching frozen `stable-resident-lexer-20261117-0200-sysroot` driver is
`1b73420f6f480f61d52ff28ca48402df236dc72f9e0d76fad3a19bb8f1645555`.
Ordinary object-only acceptance passes all ten transitions per arm, including
unrelated/relevant edits, delimiter error/recovery and unsupported-syntax
fallback. Enabled mode records eight actual lexer hits; disabled mode records
zero in every lexer field. Fresh C consumers validate unique-nonce outputs.
This is a real ordinary-output hit result, not yet evidence of a speedup.

Exact original account-private readback is verified for
`bench/stable-resident-lexer-20261117-0203-native` and the lexer-enabled native
guard and shipping-module `n=3` regressions
`bench/stable-resident-lexer-20261117-0204-{guard,dogfood}`. The earlier harness
flag-selection failure is preserved separately. Default compiler/client behavior
and production installation are unchanged. General frontend retention, broad
Rust-source coverage, managed Cargo routing and representative performance
acceptance remain unfinished.

### Ordinary rlib timing diagnostic

The suite now exposes `stable-native-lexer`, using ordinary rlib output and
incremental banks isolated per arm. Every produced library is linked into a
fresh Rust consumer that checks all generated functions with unique nonces.
Only producer invocation time is measured. Native typed `control` enables the
incremental active-source lifecycle, but all typed retention counters must
remain zero. This is not forced MIR/LLVM-output timing.

Two CPU-3-affinity captures in one unchanged parent environment, using the
same frozen 0200 compiler, boot and workload (`n=3`, 64 items per module,
six edits), passed correctness and strict suite comparison. Medians were:

| Capture | Fresh | Resident control | Lexer enabled | Lexer/control |
| --- | ---: | ---: | ---: | ---: |
| 0225-fixed-a | 31.495 ms | 18.575 ms | 23.536 ms | 1.2671x |
| 0225-fixed-b | 29.174 ms | 17.538 ms | 17.309 ms | 0.9870x |

These repetitions do not establish an added lexer-cache speedup. The apparent
26.45% lexer-time reduction between identical-compiler repetitions is variation,
not an optimization result. The machine was not globally idle. The command
`bench/suite.py compare` accepted the matching inputs and printed both within-run
ratios; no general, hosted, Cargo or production-default performance claim follows.

Exact original JSON readback is verified for
`bench/stable-resident-lexer-suite-20261117-0225-fixed-{a,b}`. Earlier captures
`0222-{a,b}` passed correctness but strict comparison refused their differing
environment fingerprints. Those originals, the failed `0217-diagnostic`
incremental-control configuration and the corrected passing `0219-diagnostic`
are separately stored and verified unchanged. No comparison gate or failed
receipt was rewritten to make the result pass.

### Non-doc comments and ordinary attributes

The frozen `stable-resident-jobserver-20261117-0231-sysroot` extends eligibility
to ASCII non-doc line comments, terminated non-doc block comments (including
nested comments), and ordinary `#` attribute tokens. Doc comments, shebangs,
frontmatter, strings, raw identifiers, Unicode and other previously excluded
side-effectful forms still reject admission. Attribute parsing and validation
remain fresh. The replay regression verifies that comment text never enters
the identifier gallery and that cached tokens preserve exact gallery contents.

This candidate was produced by the coordinated shared native gate, not a second
competing compiler build. Driver/codegen checks, all 75 parser tests and two
doctests passed (11 doctests ignored), followed by a complete matching compiler
and standard-library build. Driver SHA256 is
`832fd82045f9b25fbf4a60727c8e41ed263049e72c4afb60f96bb3f4167abcf5`.

Strict ordinary-object acceptance passes 23 transitions per arm, including
comment/attribute unchanged repeats, edits, errors/recovery and rejected syntax.
The enabled arm ends with 25 lexer hits, 42 misses and 29 rejections, retaining
12 entries with 5,799 estimated bytes. Disabled lexer counters and every typed
retention counter remain zero. Exact fresh diagnostics and unique-nonce C
consumer outputs match throughout. Lexer-enabled native guards and real
shipping-module dogfood (`n=3`) also pass with source/compiler identities intact.

The generated originals are stored as
`bench/stable-resident-lexer-common-20261117-0236-{native,guard,dogfood}`,
with authenticated exact JSON readback verified against each unchanged original.
This is expanded syntax correctness coverage only. It establishes neither a
speedup nor general AST/HIR/Typeck retention, managed Cargo routing or production
default readiness; the ordinary rlib timing limitations above still apply.

The identical frozen 0231 compiler was subsequently staged in a new private
directory on the authenticated production guest, without installation or slot
provisioning. Launcher, driver, harness and shipping-source hashes matched the
local originals before execution. The guest passed the same 46 common-syntax
transitions, lexer-enabled native guards and shipping-module `n=3` dogfood.
Original raw streams and generated receipts were transferred unchanged through
the normal authenticated SSH path. Exact account-private JSON readback is
verified for
`bench/stable-resident-lexer-common-host-20261117-0243-{native,guard,dogfood}`.
This is guest correctness evidence, not hosted timing or managed Cargo routing
acceptance. The default published client and compiler selection were not changed
by this experiment.
