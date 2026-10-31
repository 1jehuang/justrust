# Rejected cold whole-source diagnostics overlap, v54b

**REJECTED for shipping. Production `main.rs` restored byte-for-byte to its pre-experiment original and HEAD.** This is licensed Blaze-derived resident engine evidence, not clean-room work. The feature described below exists only in the frozen candidate, not shipping source.

## Native observations and decision

The coordinator executed native check/build and full release suites: **53 codegen and 360 resident tests passed**, with the whole3757 source inventory and production root lock unchanged. Thirty real native original/OFF/ON source observations passed complete diagnostic and source-location equivalence across all10 fixture cases. Exact-1 ON startup markers were present once per fixture worker, OFF and original zero. These observations do not prove injected failure paths or alternate policies.

All28 strict native runtime gates passed in four counterordered private runs, but performance was rejected. Percent change is `100 * (ON / OFF - 1)`, positive means slower:

| State | OFF then ON | Reverse ON then OFF |
| --- | ---: | ---: |
| cold-prepare | -11.4660% | +38.1424% |
| repeated-noop-1 | -1.2650% | +2.2637% |
| manifest-feature-add | -12.1500% | -2.4224% |
| type-error | -12.6505% | +4.5522% |
| type-recovery | +41.1482% | +4.4552% |
| assert-failure | +245.8553% | +9.9943% |
| assert-recovery | +178.6375% | +1.3354% |

There was no useful repeatable major-state improvement. Noise is **unknown**, including daemon/background CPU and OS cache state. There is no promotion, speedup, zero-overhead or phase-sum savings claim. This scoped native workload is not the comprehensive session-weighted synthetic acceptance gate.

The runtime worker-log snapshots are cumulative. Both ON sequences have marker counts `[1,1,1,1,0,0,0]`, and both OFF sequences all zero. Do not sum snapshot counts or infer fresh asynchronous work for every phase.

Portable raw evidence, full v54/v54b source lineages, native journals, exact original semantic oracles, validators and restoration receipts are retained in `bench/results/resident-cold-check-overlap-20261031/`. The evidence README documents offline replay and limitations. Source authoring and evidence archival launched no native jobs, while the coordinator performed the actual native gates above.

## Startup control and scope

Only the exact startup environment value `RA_JIT_COLD_CHECK_OVERLAP=1` enables speculation. A process-local `LazyLock<bool>` is forced as the first operation in `main`, before dispatch or threads, and requests read only that latched value. Later daemon request environment changes cannot toggle the experiment. Unset, `0`, `01`, `true` and every other value keep the serial schedule. The experiment applies only to a cold `build_unheld` with the existing mandatory `nothing_of_rustc && written.is_empty()` check. A served manifest, rustc patch startup and edited-source startup do not start a task.

A process-local atomic emits `cold-check-overlap: asynchronous diagnostics started` once, from inside the running asynchronous task, never from an eligibility decision. Disabled execution emits no marker. A marker establishes task startup, not reuse or successful compilation.

The observed v53 OFF manifest log had approximately 2.3 seconds of roots and 2.7 seconds of compilation followed by a mandatory 31-file check around 1.1 seconds. Those phase durations are not additive savings evidence. Contention, query sharing, speculation discard and cache effects require actual varied OFF/ON measurements.

## Exact diagnostic contract

`ColdCheck::plan` is the extracted original whole-source selection. It uses `listed.programs` when available. Otherwise it runs the original `args.crates` / `crates_to_build` selection and extends every selected crate's `transitive_deps`. It retains the original owned-source predicate, relevant-crate membership and module-map test, sorts the complete file IDs, and retains exact file paths. No reached-only, edited-only or count-limited check replaces it.

The task runs the unchanged `broken_in`, with the same crate set, file set, paths, checkout root and lint flag. Borrow-check environment policy, per-crate checking, diagnostics sorting/deduplication and precise source rendering remain there. `refusal` still runs synchronously at the original location. It therefore preserves its proc-macro, parse-error and diagnostic ordering rather than caching a speculative refusal.

After compilation the exact original plan is computed again. Reuse requires equal crate membership, sorted file IDs and file-to-path mapping. A narrower or broader plan is not accepted. An early planning error suppresses speculation and is reconsidered only at the original mandatory-check location. A task panic is explicitly joined and discarded, then the complete original check runs at that location. Existing `broken_in` nested-thread panic behavior is unchanged.

The existing `cold.unset` and proc-macro failure gates remain before mandatory refusal reporting. Precise whole-source errors still precede `cold.refused_reached`. Linking, executable production and subsequent mutable memory reporting remain after the mandatory check. Compile errors remain compile errors even if speculative checking also fails.

## Database lifetime boundary

`RootDatabase::clone` in the vendored `ide-db` clones Salsa storage and shared file/crate maps. This is the same actual snapshot mechanism already used by `broken_in`, not a newly asserted independent database or permission for concurrent writes. `build_unheld` and the diagnostic task receive immutable database access. The only new overlap boundary surrounds the first cold compilation.

The task owns its cloned database. A lexical `std::thread::scope` encloses compilation and the explicit `join`. A normal compile `Err` is propagated only after that join. Compilation panic unwinds through the scope, which joins outstanding tasks. The returned diagnostic payload contains no database. Nested `broken_in` workers finish within its own scope before the outer task finishes. No worker is detached, retained for edits, or allowed to survive into mutable database work.

In particular, `WatchedFile::include_sources(&mut files, &mut tree, &mut db)` is outside that scope. Every cloned database is dropped before that call, including on error/panic exits. If it returns anything other than `Loaded::Nothing`, speculative diagnostics are discarded even if the file-ID set looks unchanged. The existing `known.touch`, `listed.touch` and second compilation run, followed by a fresh complete original mandatory check, remain intact. A second-build error cannot reuse old diagnostics.

This lifetime audit is not proof that every shared query, macro-server side effect or codegen interaction is safe under the additional overlap. Native gates must establish the supported behavior before release. No unsafe code, DB mutation workaround or synchronization shim was introduced.

## Native fixture handoff

Full input files are frozen under `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/cold-check-overlap-v54b/fixtures`. The standalone Cargo package has individually selected bins:

| Bin | Coverage |
| --- | --- |
| `valid` | Successful native baseline |
| `unused_error` | E0308 in an uncalled function |
| `reached_error` | Precise E0308 before generic reached refusal |
| `denied_lint` | Denied non-snake-case lint in unused code |
| `borrow_error` | Moved-value error in unused code, default and opt-out policy |
| `include_valid` | Body include loads valid source, then native success |
| `include_error` | Body include contains a type error |
| `include_module` | Included item contains unused erroneous code |
| `macro_error` | Declarative macro creates unused erroneous code |
| `syntax_error` | Parse refusal priority |

The source author supplied these real fixtures without execution, then the coordinator collected30 actual native original/OFF/ON observations and compared full diagnostic blocks without message/location normalization. They passed the exact10-case contract described above. Their original README retains the proposed wider control matrix, which is not a record that all proposed controls ran. Compare complete diagnostics and source locations, status/refusal/fallback, executable presence and real outcomes against the original binary. Native support limitations must match the original, not be silently treated as passes. Cover lint caps and borrow refusal controls, a manifest hit and miss, and long-lived valid/error/recovery transitions for included and unused files. Every cold comparison needs a fresh private home. Require the positive marker for eligible exact-1 startup and its absence for every disabled spelling.

No injected panic, unset or proc-macro failure tests, alternate lint-cap/borrow-refusal controls, or long-lived include recovery controls were executed in the native fixture batch. The scoped lifetime audit and successful includes do not substitute for those missing tests. No production panic knobs were added.

## Freeze and release boundary

The v54b scratch directory and portable evidence bundle preserve the preliminary v54 candidate separately, full originals/candidates, candidate documentation, native fixture inputs, unified patches, SHA-256 inventories and historical explicit release receipts. Historical source readiness did not authorize promotion. After rejecting performance, the exact live candidate source/doc were preserved, and only `main.rs` was restored via `write` from the v54b original. `cmp` and `git diff --exit-code HEAD -- main.rs` passed. Restored SHA-256: `3ef8143a8f6294335168f9a987d3266ce347f06209e1e3c3f209b72d9c6447fd`. Production source ownership was released immediately.
