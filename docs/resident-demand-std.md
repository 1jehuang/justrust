# Experimental demand-compiled standard library

This is an opt-in experiment in the licensed Blaze-derived resident engine, not the clean-room compiler or shipping client. It is disabled by default. Start a fresh private engine namespace with exactly `RA_JIT_DEMAND_STD=1` to enable it. Other values retain the existing eager standard-library path.

## Native boundary

The opt-in skips the separate eager standard-library object and adds the existing `library_unit` selection to the program's cold dependency inventory when there is no prebuilt object. The normal resolver, on-demand instance compilation, diagnostics, refusal checks, code generation and linker remain responsible for the result. The authoritative library selection excludes the unused panic runtime and preserves existing library-selection rules. This is not evidence that every Rust program or platform is supported.

The startup setting must reach the daemon and workers. Existing manifest configuration hashing includes this `RA_JIT_` variable, separating cached artifacts between modes. Use distinct private namespaces for comparisons. Do not reuse a running daemon with a different setting.

## Failures retained before the working candidate

- v31 skipped the eager object without extending the dependency inventory. Its enabled cold runtime gate failed with unresolved standard-library and allocator symbols.
- v32 included all transitive language crates. Its enabled gate failed with duplicate `__rust_panic_cleanup`, because the graph contains alternative panic runtimes.
- v33 uses the existing `library_unit` selection. Check, build and all 359 resident unit tests passed. A fresh enabled cold build linked and executed the four expected summary tests. The original disabled control also passed. These are bounded correctness observations, not universal compatibility certification.

## Five-session state matrix

The frozen v33 executable was tested against the pinned source-built reference using the same pinned toolchain and 63-file workload. `rsm05` enabled demand mode only for the candidate. `rsm08` used the same candidate executable with demand mode disabled. Both retained 140 workload calls, restored source bytes, and unchanged source/tool/toolchain identity receipts.

Each matrix admitted 130 calls. Five reference type-error calls lacked the required fresh diagnostic, so their five candidate counterparts remain excluded from cross-engine comparison. An independent replay of the unchanged oracle admits the five enabled candidate diagnostics intrinsically. This does not change the original exclusions or establish a passing complete matrix.

Selected medians in seconds:

| State | Enabled reference | Enabled candidate | Disabled reference | Disabled candidate |
|---|---:|---:|---:|---:|
| Cold prepare | 23.613375 | 16.227106 | 25.583267 | 23.995366 |
| Summary body edit | 0.207819 | 0.188457 | 0.214590 | 0.213632 |
| Ordinary function edit | 0.123657 | 0.128230 | 0.118590 | 0.127401 |
| Signature invalidation | 0.246882 | 0.196612 | 0.207840 | 0.231032 |
| Manifest feature addition | 5.910214 | 7.722203 | 5.986772 | 5.925158 |
| Type-error recovery | 5.605044 | 7.092142 | 5.421375 | 5.331577 |
| All library tests | 0.840957 | 0.870163 | 0.917855 | 0.807980 |

The cold result is promising, but manifest changes and type-error recovery regress. These separate sessions are descriptive observations with uncontrolled background CPU and OS caches, not a paired significance test or a comprehensive weighted-workload gate. Resource cleanup occurred between sessions and is part of the provenance. No general faster-than-Blaze claim or default promotion is justified.

Other launches remain negative evidence: `rsm03` was an aborted default-mode launch, `rsm04` rejected nonpristine input before workload execution, `rsm06` used an unsupported reference command, and `rsm07` timed out under memory pressure without final restoration. They are not successful comparison runs. Durable backups restored interrupted inputs, and private namespace cleanup preserved stores and original receipts.

## Remaining work

Keep the flag opt-in. Portable failure and runtime receipts are retained in `bench/results/resident-demand-std-v33-20261031/` at evidence commit `8f8ad157f`. Root independently replayed its validator and all 29 semantic mutation/relocation tests. The full Python benchmark harness regression passed 733 tests. These checks validate the retained observations, not a general performance conclusion. Measure additional workloads and cold correctness cases before expanding support. A useful next hypothesis is a reusable demand-built library artifact, which could retain cold savings without repeatedly rebuilding library instances after manifest changes. It requires its own correct dependency boundary, cache keys and measured gates, not a timing-based assumption.
