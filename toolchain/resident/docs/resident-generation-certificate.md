# Rejected native generation certificate experiment, 2026-10-31

**Disposition: rejected, not shipped.** The v60 through v64 production candidate was restored out of all nine Rust files. The accepted native correctness implementation at `69669f0c7b28721fc3130eb801311e7e7afca962` is preserved. No default, experimental activation, compiler policy, installation or release is changed by this evidence-only record.

## Why it was rejected

The experiment did establish one real completed changed-generation retained load, but that is not enough for promotion. Content hashing and fresh resolution were expensive, the observed same-binary OFF/ON diagnostic workload showed no useful win, and complete effective ABI/configuration safety was not established. In particular, the configuration root query used by the candidate returned the raw root configuration value before environment-key merging. Separate target, rustflags and plan receipts covered some effects, not a proof of every environment-only configuration effect. This gap was documented, not relaxed or repaired during these iterations.

The following are single observed wall times in seconds, not a controlled benchmark or statistically significant regression estimate:

| Seven-state phase | v64 OFF | Same binary ON |
|---|---:|---:|
| cold prepare | 30.496294 | 37.965017 |
| repeated no-op | 0.038374 | 7.041847 |
| manifest feature add | 8.886862 | 20.334796 |
| type error | 9.262103 | 20.495901 |
| type recovery | 11.071772 | 23.860282 |
| assertion failure | 2.895534 | 5.904080 |
| assertion recovery | 0.581097 | 4.929700 |

All fourteen rows passed their strict correctness oracles, including the intentional failures. Noise was classified **unknown**. Other-process CPU activity was substantial, daemon CPU is not waited-child CPU, execution was sequential rather than randomized, and this is not `bench/suite.py`'s comprehensive synthetic gate. No speedup, broad performance conclusion, or default promotion is claimed.

## Implemented then removed

The rejected implementation was real source, not an unchanged-source eligibility claim. Its exact opt-in was `RA_JIT_WHOLE_RELOAD=1`. Default and non-whole invocations bypassed the extra certificate work. It freshly resolved the actual compiler command, direct compiler and runtime dependency bytes, sysroot contents before and after queries, effective native target/cfg/rustflags, configuration inputs, plan/profile properties, actual native driver/linker selection, and engine contents. Receipts were carried to a worker and checked before retained loading and process-global sysroot reuse. Unknown and mismatched receipts used the existing cold restart rather than rejecting otherwise valid builds. The v58 source/script/native invalidation behavior was retained.

A changed-generation successful-load marker was emitted only after a retained worker completed its load. An unsuccessful retained load tried the original cold path once with coherent command/config/environment selection. Existing source diagnostics and failure status were preserved. These are descriptions of the rejected candidate, not features present in the restored production sources, and they do not assert that its identity boundary was complete.

## Actual compiler and source corrections

- **v60:** real compiler check failed with two `E0599` errors because the native library-input helper used `Option::context` without importing `anyhow::Context`. A source-only freeze was not a compiler pass.
- **v61:** the missing trait import and coherent cold-fallback checkout were repaired. The next real compiler check failed with `E0282` because the loader's `AsRef` path conversion was ambiguous.
- **v62:** explicit `AsRef<Path>` conversion compiled and built. Actual native gates passed 53 codegen tests and 374 resident tests. Native script A/B/A correctness passed, but no completed retention marker existed and PIDs changed.
- **v63:** fixed-code, opt-in-only resolver stages and parent reason booleans made the failure attributable without exposing raw environment/path/error metadata. Compiler/build and 53/374 native tests passed. All three client certificates were `unknown stage=plan`, while workers resolved known receipts. This was not evidence of eligible retention.
- **v64:** a shared `WorkerConfig::resolved_selection(root)` aligned the client certificate with actual worker arguments. Empty checkout-relative `asked_in` previously reached metadata as an explicit empty manifest and failed because it resolved to a directory. The worker already expanded that representation to the absolute checkout root. Relative member paths and absolute paths were covered by a strict argument-parity/original-immutability source fixture. Compiler/build and 53/375 native tests passed without reducing assertions or counts.

## What the native observations do prove

For v64 ON's `manifest-feature-add` row, the preceding status and completed request status both named worker PID **151379**. The new daemon events had `generation_equal=false` and exactly one `whole-generation retained-load success pid=151379 generation=...` marker. The exact four expected tests completed with the fresh request nonce. This establishes completed changed-generation retention for that request, not merely eligibility and not universal safety or a speedup.

The independent manifest-configuration failure fixture has nine rows: original/OFF/ON, each valid, manifest-error and recovery. ON's failing request emitted exactly one `retained load failed, restarting cold: error[E0308]...` event and one new cold worker event. It returned status 101 with the unique source-error literal, did not run the program, and subsequent recovery completed with a fresh nonce and a different worker. This is actual same-request fallback evidence, not only an internal unit fixture.

Native build-script A/B/A archive selection also returned exact values 11/22/11 with fresh nonces. In v64 these requests kept a single worker but were same-generation requests, not the changed-generation success claim above. The portable package retains v62 and v63 observations so these cases cannot be retrospectively mislabeled as completed changed-generation retention.

## Restoration and preservation

Before restoration, full live bytes of nine candidate Rust files plus this document were preserved in the private v64 `live-before-restoration` snapshot and verified against its immutable candidate inventory. Each Rust file was restored via exact writes from immutable v60 `base-originals`. At `2026-10-31T10:18:22Z`, all nine `cmp` checks against those originals and all nine scoped `git diff --exit-code HEAD` checks passed. The source-restoration acknowledgment was released before evidence packaging. Unrelated working changes were preserved.

All private iteration snapshots, compiler failures, raw native diagnostics, PID/status snapshots, fallback evidence and source hashes remain in the original immutable experiment records. Completed v64 diagnostic processes were stopped by the root coordinator after measurement, preserving stores. This worker ran no native builds, tests, resolver probes, lock resolution, installations or releases.

## Portable evidence and limits

See `bench/results/resident-generation-certificate-20261031/README.md`. That self-contained package contains explicit-allowlist observations, raw-input digests, iteration source inventories, strict offline validators and adversarial mutation/relocation tests. It contains no ambient environment map, private configuration payload or raw absolute-path log dump. Raw-input digests identify private originals but do not make absent raw files independently reconstructible. Offline validation verifies the selected recorded evidence and parser strictness, not a fresh native run or missing ABI/config safety.

Any future attempt needs a new implementation identity, complete native effective-input coverage, relevant correctness tests, and measured before/after comprehensive workload results. No part of this record authorizes reactivating the rejected candidate.
