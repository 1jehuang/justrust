# Cold whole-source diagnostics overlap, v54

This is a source-only, default-OFF experiment in the licensed Blaze-derived resident engine. It is not a clean-room implementation. No native build, test, workload or timing run was performed by this worker. The coordinator owns native correctness gates and timing. This document is not a promotion or speedup claim.

## Startup control and scope

Only the exact startup environment value `RA_JIT_COLD_CHECK_OVERLAP=1` enables speculation. Unset, `0`, `01`, `true` and every other value keep the serial schedule. The experiment applies only to a cold `build_unheld` with the existing mandatory `nothing_of_rustc && written.is_empty()` check. A served manifest, rustc patch startup and edited-source startup do not start a task.

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

Full input files are frozen under `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/cold-check-overlap-v54/fixtures`. The standalone Cargo package has individually selected bins:

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

These are real source fixtures, not mocked DB assertions, and have deliberately not been executed. They are outside production source ownership. Their README specifies original/OFF/ON comparisons and source backup/recovery. Compare complete diagnostics and source locations, status/refusal/fallback, executable presence and real outcomes against the original binary. Native support limitations must match the original, not be silently treated as passes. Cover lint caps and borrow refusal controls, a manifest hit and miss, and long-lived valid/error/recovery transitions for included and unused files. Every cold comparison needs a fresh private home. Require the positive marker for eligible exact-1 startup and its absence for every disabled spelling.

Join/fallback checks require an actual native error/include transition plus source lifetime audit. Failure injection for compile panic, diagnostic panic, unset and proc-macro priority belongs in the coordinator's native harness. This candidate adds no production panic knobs. Fixture source alone does not prove these paths or satisfy a passed native gate.

## Freeze and release boundary

The v54 scratch directory contains the full original and candidate `main.rs`, candidate documentation, native fixture files, unified patch and SHA-256 inventory. `RELEASE.md` explicitly hands this source candidate to the coordinator for native gates only. It does not authorize default promotion, claim measured savings, or assert tests passed. No build/test/native command or commit is part of this worker's evidence.
