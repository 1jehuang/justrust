# Optimized native compiler frontier br48, 2026-10-30

**Focused descriptive timing only. No general speedup or default-promotion claim.** This run uses a plain optimized compiler build, not PGO or BOLT. Historical development-compiler sessions are not controlled before/after baselines for br48. No defaults change.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned experiments off | 1.028383 |
| Owned earlier three experiments on | 0.997915 |
| Owned native Cranelift | 1.057973 |
| Owned frontend threads=4 | 0.966622 |
| Owned native Cranelift plus threads=4 | 0.994295 |
| Blaze, pinned Cranelift backend | 0.099186 |
| Blaze, pinned LLVM backend | 0.109765 |

The smallest owned median is the four-thread stack at 0.966622 seconds, still far above either Blaze median in this filtered-test diagnostic. Five samples in one seeded-warm session do not establish significance or a session-weighted result. Comparing br48 with an earlier development build would mix different timing sessions and is not a measured dev-versus-release speedup. Separate native acceptance/profile timings are correctness and mechanism evidence, not speed claims.

## Plain optimized compiler provenance

The retained bootstrap configuration uses `profile = "compiler"`, ThinLTO, one compiler codegen unit, compiler incremental compilation disabled, debug assertions/logging disabled and line-table-only debug information. It builds LLVM and Cranelift backends, enables the profiler, and explicitly disables downloaded rustc. This is **plain release optimization, not PGO/BOLT**. The compiler build configuration is distinct from the benchmarked project's dev/test profiles, which still use opt-level 0, debug 0 and incremental true.

The exact frozen executable SHA-256 is `64689a080e88a1aece4307ee5012a59bc0e89d54d3ec208dd16cb3102c60156d`, and the driver SHA-256 is `961a7344a71a7b1997557fa4167cc44305daa341ea2aee920ff88ee1e709ea43`. Both match the build-freeze identity receipt, native acceptance before/after identity receipts, and runtime before/after identity maps. The native Cranelift backend is independently pinned to `97e114ae3dd6fcbbbfbc9553fe2b9ac48dee6a59949317e2474bbf004e26314d`. Compiler version bytes agree between the frozen build, native acceptance and runtime harness receipts.

The native source files `compiler/rustc_monomorphize/src/partitioning.rs` and `compiler/rustc_session/src/options.rs` match implementation commit `66e0f9273fd079a7a2334d8b89aafeeade9e8c76`. Their retained copies match the build's source-before hashes, and the source-after check reports both `OK`:

- `partitioning.rs`: `fd269dc9cb3dc92599839ca127f799921f613088fc50a9fbd05ebbba5a56134f`
- `options.rs`: `6301a8c55991317045183d0e1ae2673dbab48b13bf08f9b68686f3922e62ff6b`

The bundle retains the bootstrap config, complete build log and launch scripts/logs, build/freeze/acceptance zero-status receipts, source hash/check receipts, version/identity receipts and those two source files. No compiler, driver, fixture executable, sysroot, native dependency cache or target directory is retained. Replay verifies historical receipts and source bytes, not present-day binaries or an independently reproduced compiler build. Source identity here covers those two explicitly hashed native files, not an attested hash of the entire compiler tree.

## Complete native acceptance, separate from runtime timing

The frozen optimized compiler passed **96 cases across eight configurations, 672 main compile modes, plus four mechanism compiles/profiles**. Configurations cover codegen units 1/16, incremental on/off and opt-level 0/2. The retained complete `acceptance.json` includes exact base64 diagnostic/runtime bytes, commands, identities, negative/recovery fixtures, fresh and persistent caches, and no/yes/no transitions. The replay derives expected cases and schedules from the retained acceptance-runner sources and checks every mode, exact diagnostics/runtime outputs, flag state, verify-ICH and incremental cache isolation.

All four raw `.mm_profdata` files are retained and independently decoded offline using the bundled format-9 decoder, rather than trusting summarized marker counts:

| Profile | Shared traversal events | Cross-root revisit events |
| --- | ---: | ---: |
| Cold, option off | 0 | 0 |
| Cold, option on | 23 | 1 |
| Warm, option off | 0 | 0 |
| Warm, option on | 23 | 1 |

The positive counts prove the native shared-traversal branch and cross-root duplicate-DFS avoidance for the acceptance fixture. They do **not** prove a performance gain. The shared traversal option remains default-off. The br48 runtime backend/thread stacks do not enable it, so these acceptance mechanism counts are not runtime-stack activation claims.

## Runtime gates, isolation and exports

Seven stacks each pass prepare, deliberate E0308 rejection with its injected marker, type recovery, intentional single-test failure, and test recovery before five timed edits. All **70 rows and 35 timed edits** are present and all timed rows are accepted. Successful runs contain exactly four expected `summary::tests::` tests and 144 filtered out. Each timed edit has exactly its own unique `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The intentional test failure affects only `shares_overlapping_time`. Stack order rotates across replicates and the identical edit bytes are used across stacks.

All five owned stacks use the same frozen optimized compiler and official Cargo identity, with separate HOME, target and XDG roots. Proc-macro and dependency/local/peer/remote/prebuilt caches, buildd and toolchain installation remain disabled. Native backend stacks explicitly add `-Zcodegen-backend=cranelift`, thread stacks add `-Zthreads=4`, and both are composed only for the combined stack. The earlier three experimental flags remain `no` except in the separate `all3on` control. Blaze backend/optimization settings are explicitly pinned. Source before/after maps and restored original bytes agree, as do complete tool and seed identity maps. Both Blaze stores are seeded from the same recorded br30 content. Contamination is recorded as unknown, not proof of a quiet machine.

The new `bench/results/native-release-br48-20261030/` directory retains root JSON, original edited source, historical launcher/exit/logs, all flat stdout/stderr, all fifty raw owned journals and private recorder directories, complete optimized build/acceptance evidence and four raw profiles. Raw journals, not condensed console rendering or slim exports, are the runtime authority. Targets/caches/binaries/sockets are excluded. Absolute paths inside receipts are historical strings, not portable file dependencies.

All **50 owned calls** were exported exclusively from the five private br48 recording homes, ten per home, into bundle-local `exports/`:

- `xps14-runtime-br48-owned-off-20261030/2026-10.jsonl`
- `xps14-runtime-br48-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `xps14-runtime-br48-owned-clif-20261030/2026-10.jsonl`
- `xps14-runtime-br48-owned-threads4-20261030/2026-10.jsonl`
- `xps14-runtime-br48-owned-clif-threads4-20261030/2026-10.jsonl`

No global export or existing `data/runs` directory was modified. The validator checks all fifty export IDs/statuses against recorder state and exact row provenance.

## Portable offline replay and mutations

```sh
python3 bench/results/native-release-br48-20261030/validate.py
python3 bench/results/native-release-br48-20261030/test_validate.py
```

Replay requires only Python 3.11+ and this bundle. It uses no native executables, network, private files or repository dependencies, even when the directory is copied elsewhere. Use normal Python, not `python -O`. `validate.py` checks the complete sealed inventory and SHA-256 hashes, runtime results/gates/markers, injected source hashes, private isolation and exact profiles/flags, source/tool/seed receipts, 70 row keys and rotated 35-edit order, recomputed medians, all 50 recorder states/exports, optimized build configuration/source/status receipts, all 672 native acceptance modes, and all four raw decoded profiles. `native_audit.py` performs the build/native-acceptance portion. Retained runner modules are imported for pure offline validation only, never executed as capture commands.

The mutation suite checks detached-directory golden replay and explicitly forbids subprocess creation during the complete in-process offline audit. It rejects inventory mutations, raw journal stale/duplicate markers and broken error/recovery gates even after raw hashes and recorder copies are updated, wrong backend/thread flags, identities and exports. Native mutations cover config/source/status changes, failed or incomplete acceptance, altered diagnostics/runtime output, missing verify-ICH, missing profiles, forged marker summaries and raw marker tampering with an updated profile hash. Thus the raw decoder, not just hash agreement, must reject forged mechanism evidence.

`seal.py` generates the inventory once and refuses to overwrite it. `retain.py`, historical build/acceptance shell scripts, and `launch-runtime48.sh` are provenance utilities, **not replay commands**. No native workloads were run for this retention task. Only the new results directory and this new document were changed.

## Limits

Seeded-warm filtered tests do not establish full-suite, cold-start, check/build or comprehensive session-weighted performance. Native correctness/mechanism acceptance has its own fixture/configuration scope and is separate from runtime timing. No statistical significance, PGO/BOLT claim, default promotion or controlled historical dev-versus-release claim follows from these receipts.
