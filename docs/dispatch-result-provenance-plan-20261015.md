# Next slice: exact nonincremental dispatch result availability

Status: design frozen at the semantic boundary, implementation not yet accepted. This follows the accepted cache-hit mapping in `cache-call-provenance-20261015.md`, not a completed execution DAG.

## Why continue

The cold retained capture has 18,306 non-feed requests with no first-cache-hit fact. Existing `try_execute_query` already returns the selected value and optional result index, but `execute_query_non_incr_inner` discards the index. Keeping the exact index at the actual lexical request terminal can explain another missing relation without inferring request parents, provider execution, or dependencies from adjacent recorder events.

This is an observed availability relation. It is not a semantic prerequisite edge or an architecture-independent parallelism limit. It cannot by itself fix opaque CPU attribution or compute a critical path.

## Minimal native protocol

Preserve the old `QueryVTable::execute_query_fn` signature and behavior. Add a separate optional `observed_non_incr_execute_query_fn` returning the existing `(Value, Option<DepNodeIndex>)` tuple. Populate it only for an explicit diagnostic feature in nonincremental mode. Its wrapper calls the existing `try_execute_query::<_, false>` algorithm. Do not rewrite cache locking, scheduling, waiting, provider invocation or cycle handling for this slice.

The three lexical callers in `rustc_middle/src/query/calls.rs` retain their actual `QueryReadRequest`. A matching request observer identity and saved receiver context must be validated before choosing the observed thunk. Do not discover the consumer by querying TLS after execution. Preserve `ensure_result`'s normal conversion: record a resolution only after conversion returns normally. Panic or cancellation remains an interrupted request without resolution.

The old function-pointer ABI is retained, but adding a vtable field, code and a selection branch is not free by assertion. Measure default-disabled and opt-in costs separately.

## Strict capture grammar

- Existing constructors continue producing query root 4 or 5 and nested read schema 1, byte-for-byte for fixed input.
- An explicit dispatch-resolution constructor produces root 6, requiring nested read schema 2.
- Root 6 has a mandatory nullable `execution_clock` field. Without timing it is null. With timing it must use the existing exact reciprocal bridge to pool 3. No new clock inference.
- Read schema 2 adds a mandatory nullable `resolution` field to every original row. No new event rows, request IDs or allocator budgets are introduced.
- Only a normally finished nonincremental dispatch request can carry `{"source":"resolved_dispatch","dep_node_index":null}` or the same object with an actual `u32` index.
- The source label deliberately does not say fresh provider, cache race, waited, blocked or cycle. A null index means unresolved dispatch, not an inferred cycle.
- First-cache-hit requests, incremental dispatch, feed calls, green ensures and interrupted requests have null resolution for this first slice.

Native observed branches can return an index after a cache recheck, after an owned nonincremental computation, or after obtaining a waited-for cached result. Their branch labels remain unknown in this protocol. Forced calls without a lexical request stay on the old path.

## Consumer authority

Expose a separate explicit `--dispatch-result-provenance` analysis, combinable with existing large-capture and cache-call options. Keep manifest modes separate. Legacy strict collector APIs reject root 6 rather than accidentally promote it through resource-attachment discovery. The new collector parses original typed bytes, validates read-schema-2 metadata, then explicitly projects unchanged event rows for legacy base checks. No serialization roundtrip, filtered event ordinals or generic JSON rewrite.

Keep dispatch-result and cache-call assessments separate. The latter may use the validated unchanged-row projection when both options are requested. Never add these counts to semantic dep-read coverage or union their publishing-job pairs into an executable job graph.

A certified dispatch relation needs:

1. An actual successful `Get/Value` or `Ensure/Ensured` lexical request and exactly one terminal.
2. A terminal `resolved_dispatch` object with a nonnull actual result index, and no first-cache-hit fact for that request.
3. Matching capture/context namespace and actual admitted producer and consumer endpoints.
4. A unique publishing result for that index strictly before the terminal in the original single event stream.
5. Complete relevant source and admission authority. Missing or lost evidence cannot certify uniqueness.

Publication may occur after request start. A producer's completed marker need not precede terminal consumption. Same-job event relations can be valid, but must not become a whole-job self-loop. Missing ownership, absent or future publication, null index, incompatible route, loss or duplicate capture authority stays explicitly unresolved or invalid according to the strict grammar. Do not select the latest ambiguous publisher.

## Measurement gate before default-path acceptance

The immutable accepted compiler is the baseline, not the system compiler. Compare A-disabled to B-disabled separately from B-enabled. Use separate compiler-target/incremental/cache namespaces and record compiler, driver and helper identities. A shared upstream `rustc -vV` string is not sufficient identification of these owned compiler builds.

The comprehensive gate remains matched `bench/suite.py run --synth 1 -n 1` with all 40 points and identical seed, generator/weights, snapshots and controlled execution mode, followed by `suite.py compare`. The harness must attest the actual compiler selected in each run and reject silent stock fallback. Current audit found two blockers: the accepted prefix has only `rustc`, while `jr-install` requires rustdoc and clippy helpers, and the synthetic benchmark does not yet attest per-run compiler/driver hashes. Resolve these before claiming a default-path nonregression. Do not substitute the focused itoa fixture for this gate.

Supplement with repeated matched cold/warm itoa compilations at one and four frontend threads. Retain compilation wall/CPU separately from serialization and graph analysis. Where usable, retired instructions help distinguish work from scheduling noise. An inconclusive noisy comparison is not a nonregression pass. No speedup is claimed by this design.

## Correctness acceptance

- Native recorder unit tests: old fixed-input serialized bytes unchanged, explicit root/read version matrix, mandatory nullable fields, normal terminal with Some/None index, unwind null, event cap and independent loss accounting.
- Generated query thunk compile and short-backtrace regression checks. Original scheduling algorithm remains shared.
- Actual lexical nested/interleaved requests prove each terminal receives its own selected index. Do not construct edges from admission-parent nesting.
- An actual cold nonincremental request binds a prior result while incremental semantic reads remain disabled. A warm first-cache hit retains null dispatch metadata and its existing cache-call relation.
- Foreign context, future publication, absent index, duplicate authority, malformed route, source/admission loss and interrupted execution cannot certify a result relation.
- Untouched locked itoa source, ordinary compiler semantics, descriptor-only controls consumed before compiler entry, identical compiler argv/environment for OFF/ON, and existing within-candidate OFF/ON/OFF artifact/output neutrality checks.
- Replay old root-4 and root-5 raw captures and accepted default assessments exactly, without changing originals.

Neither branch stress without a witness nor itoa scheduling that happens not to exercise a path proves that path correct. This slice makes no branch-specific route claim, so it does not require inventing route labels simply to satisfy a test.
