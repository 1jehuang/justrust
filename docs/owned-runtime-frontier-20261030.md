# Remaining owned edit critical path, 2026-10-30

This is an instrumented diagnostic, **not a benchmark speedup claim**. The six successful invocations retain the same pinned f97fcc2 source, compiler/driver and client identities in `state.json`. All four selected tests pass per invocation. The executed edit has the same source hash in both modes. All 63 source files are restored byte-for-byte and tool identities are unchanged.

Artifacts: `bench/results/owned-runtime-frontier-20261030/`, including runner snapshot, exact raw output journals, per-run native units/passes, metadata, summaries and SHA256 inventory. Six runs are exported under `data/runs/xps14-frontier-macro-{off,on}-20261030/`.

Both modes enable the four existing default-off owned query/lint experiments. They differ only in explicit `-Zcache-proc-macros=no/yes`. `JUSTRUST_PASSES=on` instruments native compiler phases. Each mode runs prepare, executed edit, restoration. These runs do not establish general proc-macro cache correctness, which requires a separate external-input differential gate.

| Executed edit diagnostic | Macro cache off | Macro cache on |
| --- | ---: | ---: |
| Harness wall seconds | 1.332673 | 1.351485 |
| Recorded compile seconds | 1.264783 | 1.286200 |
| Macro expansion ms | 291.104 | 161.620 |
| Resolve crate ms | 106.405 | 130.347 |
| Type checking ms | 62.693 | 64.280 |
| Mono graph walk ms | 69.410 | 68.250 |
| Partition and symbol checks ms | 58.022 | 61.160 |
| Persist result cache ms | 106.866 | 146.550 |
| Linker ms | 70.198 | 77.690 |

Off edit run: `20261030-035237630-1591346`. On edit run: `20261030-035258029-1595722`.

## Interpretation and next experiment

Actual compiler work, not the few milliseconds of test execution or client startup, remains the primary target. The observed macro expansion interval decreases, but total edit time does not improve in this single instrumented pair. Persisting larger query caches and other work can offset a local reduction. Noise is unknown, execution order is not randomized, and phase instrumentation changes the workload. No statistical, time-saved, default-promotion or Blaze-beating claim follows.

Native time-pass spans are nested and sometimes aggregated across calls. In particular `expand_crate` is inside `macro_expand_crate`, result-cache persistence is inside `serialize_dep_graph`, and mono collection/partition are inside `codegen_crate`. **Do not sum this table into an exclusive CPU or wall-time decomposition.**

Next native candidate: default-off deterministic declarative macro match/transcribe retention, preserving parsing, hygiene, definition/input invalidation, rule-usage diagnostics and error recovery. This is an implementation hypothesis, not an established optimization. Arbitrary procedural macro environment/file side effects cannot be assumed pure. The goal of beating Blaze remains unmet and requires a new matched correctness/timing gate plus the broader workload before default promotion.
