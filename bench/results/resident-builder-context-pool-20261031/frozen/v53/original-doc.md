# Native builder-context pool candidate

## Scope and status

This is an opt-in candidate in the licensed Blaze-derived resident native engine, not the clean-room compiler. Only `toolchain/resident/crates/hir-ty-codegen/mod.rs` changes executable source. No compiled Cranelift `Context`, function, diagnostic, query result, or cache entry is pooled. Shim builders and their existing reuse are untouched.

Source-only inspection and formatting have been performed. Native compilation, tests, jobs, benchmarks, and commits were intentionally not run by this worker. The coordinator must execute correctness gates and varied matched-state measurements with positive activation before making any performance claim. Allocation reuse alone is not evidence of a speedup.

## Primary-source lifetime contract

Inspected local registry source:

`/home/jeremy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-frontend-0.136.2/src/frontend.rs`

* Lines 61-65 document automatic clearing of `FunctionBuilderContext` after a builder completes translation.
* Lines 68-80 implement private `clear`: SSA, block status, variables, safepoints, and safepoint alias-region state are reset.
* Lines 704-747 implement `finalize(mut self, frontend_config)`. It consumes the builder, performs debug block checks and optional safepoint processing, then calls `self.func_ctx.clear()`. Its comment explicitly states that allocated buffers are preserved.

Calling `clear` directly is neither available nor needed. A context is eligible only after `finalize` returns normally and the enclosing builder scope has ended. A fill error, a panic during fill or finalization, and an unfinished builder skip the explicit return call. There is deliberately no recycling Drop guard. Existing `fill(...)?` status propagation remains intact.

## Gate and storage

`RA_JIT_BUILDER_POOL=1` is the only accepted value. Missing, empty, `0`, `true`, whitespace, and other spellings disable it. A process-wide `LazyLock` latches the environment at first native build, matching native component flag conventions. Set the environment before starting the process. It is never reread for later functions.

When disabled, acquisition constructs the same fresh `FunctionBuilderContext` as before. Neither acquisition nor return consults TLS, allocates pool storage, or locks pool storage. The gate itself has one-time lazy initialization, not an unconditional process-start initializer.

When enabled, each thread has at most one idle context. Acquisition takes the Option while briefly borrowing the RefCell, and releases that borrow before constructing a builder or invoking caller code. Reentrant builds therefore obtain a separate fresh context when the outer build has checked out the slot. If an inner build fills the slot first, the outer context is dropped after successful finalization instead of increasing the bound. There is no bound on concurrent active builders or on the capacity retained inside the single idle arena.

Both TLS operations use `try_with`. Destroyed TLS means fresh allocation on take and discard on return. `try_borrow_mut` also fails open on a borrow conflict. No unsafe code is introduced. Native `build` still allocates a fresh compiled `Context` and preserves its signature/source-location setup and guarded-entry fixup.

## Added native fixtures

All added tests have the `builder_pool_` prefix in the existing test module. Original semantic tests are unchanged.

* Exact default-off flag parsing and disabled operation with zero reuse/return counters.
* Fresh-versus-reused real Cranelift functions across I32/I64 signatures, leaf functions, diamond control flow, and a backedge. Compare complete displayed IR and signatures, and verify each function using the target ISA flags.
* Positive cfg(test)-only reuse counters: eight sequential candidate fixtures require seven pool hits and eight returns.
* Exact error text preservation, panic discard, and unfinished-build discard, followed by a clean verified build.
* A debug-only panicking-finalize fixture that leaves a block incomplete. No return is permitted when finalize panics.
* A nested real-builder fixture proving no RefCell borrow spans caller work and the idle capacity remains one.

Fixtures use fresh worker threads to isolate TLS counters and direct boolean gate injection to avoid unsafe process-environment mutation. They exercise actual native Cranelift builders, not a shim or mocked arena. They do not replace the coordinator's full native semantic suite, process-level environment-gate checks, or varied end-user workloads.

## Required coordinator validation

1. Compile and run the added native tests, including the debug-only finalize-panic test. Treat compile/API errors as unresolved until actually checked.
2. Run the unchanged native semantic tests with the environment absent, with `0`, and with exact `1` in separate processes. Confirm positive reuse using the cfg(test) counters rather than merely observing a successful build.
3. Run representative varied native workflows and matched before/after states, inspecting regressions and correctness before considering promotion. Keep the default off pending those results.
4. Do not infer a general speedup from the pool implementation or from a single allocation-sensitive microcase.

## Frozen handoff

Evidence directory: `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/builder-context-pool-v52`.

It contains the full original, candidate, this document, the two-path patch, SHA256 manifest, source-check notes, and `FROZEN_READY`. The released ownership paths are exactly:

* `toolchain/resident/crates/hir-ty-codegen/mod.rs`
* `docs/resident-builder-context-pool.md`

No cached source or Cargo file is part of this candidate.
