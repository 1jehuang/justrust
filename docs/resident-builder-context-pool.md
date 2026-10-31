# Rejected native builder-context pool, 2026-10-31

**REJECTED for shipping. Production `mod.rs` restored exactly. No speed, zero-overhead, or default-promotion claim.** This is licensed Blaze-derived resident work, not clean-room work. Only evidence and documentation remain in the checkout.

## What was tested

v52 proposed a bounded per-thread pool of Cranelift `FunctionBuilderContext` for real native functions. It did not pool the compiled `cranelift_codegen::Context` or change shim reuse. The proposed environment control accepted exact `RA_JIT_BUILDER_POOL=1`, latched at first native build, and otherwise used a fresh builder context. This describes the archived candidate, not an installed feature.

Registry primary source `cranelift-frontend-0.136.2/src/frontend.rs` documents automatic context clearing at lines 61-65. Its private `clear` resets SSA, block status, variables, safepoints and alias-region state. `finalize(mut self, ...)` at lines 704-747 consumes the builder and clears its context only after block checks and optional safepoint processing, preserving allocated buffers. The candidate explicitly returned a context only after finalize returned normally and the builder scope ended. Errors, panics and incomplete builders discarded it. The pool retained at most one idle arena per thread, held no RefCell borrow across caller work, and used `try_with` and `try_borrow_mut` to fail open. Arena byte capacity was not bounded.

v53 was a marker-only repair. Its fixed `resident: builder-context-pool reused` stderr line was attempted at most once per process, only after an actual cached take, outside the TLS borrow, with its I/O result ignored. A pool-local flag avoided repeated global `Once` checks on later reuse by that thread. The gate, build, discard path and native fixtures were unchanged from v52.

Native fixtures compared fresh and reused real builders across differing I32/I64 signatures, leaf/diamond/backedge IR and verifier results. They checked positive reuse counters, exact error status, fill-panic and incomplete-builder discard, and nested usage. Original semantic tests were preserved byte-for-byte. The separate debug-only finalize-panic fixture **was not executed by the release-mode native gates**. No debug pass is claimed.

## Actual native and runtime results

The coordinator's native check/build and full release suites passed for both v52 and v53: **57 codegen tests and 360 resident tests**, zero failed/ignored/filtered. Complete raw journals, source3757/lock checks and native binary identities are retained. Those gates are fixture evidence, not proof for arbitrary Rust programs.

All **28 actual strict runtime calls** passed on the same v53 binary, with four private cases in order OFF, ON, ON-repeat, OFF-repeat. Each case ran cold prepare, repeated no-op, manifest-feature addition, type error, type recovery, assertion failure and assertion recovery. The original invocation-specific runtime/type/assertion oracles were used, not a success-exit proxy.

The successful-reuse marker appeared exactly once in each captured ON worker snapshot and zero times in each OFF snapshot. These are cumulative worker-log snapshots. They demonstrate actual reuse somewhere in the ON worker, not fresh reuse during every phase. Do not sum snapshot markers or attribute assertion timings to fresh per-phase work.

Percent changes below are `100 * (ON / OFF - 1)`, positive means slower:

| State | First OFF then ON | Reverse ON then OFF |
| --- | ---: | ---: |
| cold-prepare | +37.2901% | -1.4956% |
| repeated-noop-1 | -18.2826% | +4.8085% |
| manifest-feature-add | -0.6164% | +9.6261% |
| type-error | +0.2089% | +3.2514% |
| type-recovery | +0.3153% | -2.9040% |
| assert-failure | -5.9322% | -9.1732% |
| assert-recovery | +8.4700% | -16.3483% |

Major states did not show a repeatable useful broad gain. Noise is **unknown**. Waited-child CPU does not account for resident daemon work, other processes or OS cache effects. The assertion-failure pair alone does not justify shipping. These scoped counterordered pairs are not the comprehensive session-weighted synthetic acceptance gate. Allocation intuition is not a performance result. The coordinator rejected both candidates and requested source restoration.

## Source restoration and retained evidence

Before restoration, exact live v53 source and doc were preserved. `toolchain/resident/crates/hir-ty-codegen/mod.rs` was restored from the full v52 original. At `2026-10-31T06:11:49Z`, byte comparison to the original and `git diff --exit-code HEAD` both passed. Source ownership was released at `06:12:01Z`.

Restored source SHA256:

`ec08cb7a35fa6c4bcdfbf3514e7fceb367a84eb3958817ef9f803703100a7b14`

The portable bundle is `bench/results/resident-builder-context-pool-20261031/`. It retains full v52/v53 originals, candidates, docs, patches and frozen hashes, exact live-before-restoration bytes, native check/build/test logs and full journals, 3757-source/lock checks, all28 raw outputs and worker snapshots, input backups, exact pinned63 workload, complete4020 toolchain identity receipts, launchers, actual probe, original oracle modules, runtime analysis, rejection and restoration receipts. No engine binary or private cache is embedded.

Replay with Python's standard library, from any working directory:

```sh
python3 -B /path/to/resident-builder-context-pool-20261031/replay.py
python3 -B /path/to/resident-builder-context-pool-20261031/test_replay.py
```

Replay verifies identities, reconstructs each exact edited input, reparses the actual raw output, and invokes unchanged original semantic oracles. It also executes the archived original analysis with only its `__file__` bound to the relocated raw directory. It never dereferences historical checkout, scratch, engine or toolchain paths. Twenty-four coherent negative controls update raw receipts, parsed results, indexes and checksums as needed, and still require semantic rejection. Isolated relocation uses empty HOME/PATH, isolated Python and an audit hook forbidding external application reads and native/network actions.

This is historical offline replay, not native re-execution. `SHA256SUMS.bundle` is transport integrity, not provenance authentication or cryptographic signing. Re-sealed mutations do not model an adversary rewriting the validator. The evidence lane launched no native jobs and made no production feature change. The archived proposed defaults and pool behavior are not installed shipping behavior.
