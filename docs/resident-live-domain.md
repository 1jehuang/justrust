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

## Native Cargo routing boundary

The current resident protocol freezes compiler arguments, cwd and environment
for the lifetime of each worker. It is not yet a managed per-crate Cargo pool.
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
