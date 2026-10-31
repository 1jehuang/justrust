# Experimental demand-std v33 evidence, 2026-10-31

This is a separate evidence bundle, not a default promotion or a general faster-than-Blaze result. v33 remains a **default-OFF experiment**, demonstrated only on the working tested library subset here. Other projects, native libraries, targets, build scripts and proc macros are not generally established by these observations. The retained native implementation is imported, licensed Blaze-derived code, not the clean-room compiler. The evidence worker made no native engine calls, builds, or workload executions. The coordinator executed the original runs. Portable validation replays their retained bytes only.

## Frozen inputs and interpretation

- Workload: exact `f97fcc2` source63, all 63 file/blob identities retained under `input/`.
- Harness: commit `9e6f21120`, exact imported helpers under `provenance/`. The matrix harness was not changed for these observations.
- Reference: actual pinned native `ra-jit`, SHA256 `fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263`.
- Later native source publication: `9f3bab61e`, main Git blob `ce0f6796092661bd2bfc51fca8c77b23e266c8da`. This links the frozen source, not a claim that an earlier executable embeds a later source/doc commit. Root-owned contract: `docs/resident-demand-std.md`.
- v33 candidate: SHA256 `89c9445128b4ece98e9b99b0d399ac102bc68185d52ff96098d944f6f4627a20`, frozen main SHA256 `f25af99f976a585766adeb75ee8d21a1983113cfdf3ae46d9c05536975d1ad76`.
- rsm05 explicitly passed `--engine-demand-std`. Only candidate children received `RA_JIT_DEMAND_STD=1`, including startup/version calls. Reference children did not. The flag is opt-in and experimental, not the default.
- The same exact pinned nightly TC identities are checked before/after. Private stores are disjoint per stack/session and reused across the ordered phases. Rotation, phase input hashes, exact current-source nonces, durable backups including Cargo.toml, raw bounds, and original restoration results remain unchanged.

## Original rsm05 opt-in and rsm08 default-off matrices

In each of rsm05 and rsm08, all 140 workload calls completed. rsm08 uses the same exact v33 binary with the flag OFF and the actual pinned reference. Each is a complete observation set but a **failed cross-engine error gate**, not a globally passing matrix. Source restoration, source/tool/TC identity, and raw receipt immutability gates passed. **130 original cross-engine observations admitted, 10 excluded**. All five reference type-error calls returned 101 but generic `HasErrors`, without current nonce/E0308. They remain rejected. Candidate calls were excluded by the original reference-availability guard, not rewritten after the fact.

`intrinsic-typeerror.json` is an explicitly separate offline derivative. Each run's five candidate error raws independently passes the **unchanged** harness nonce/E0308/no-runtime oracle. Original `accepted:false`, reference-unavailable reasons, and excluded error medians remain intact. This does not turn either run into a completely passing cross-engine matrix.

Median elapsed seconds, five samples for each admitted stack/state:

| State | rsm05 reference | rsm05 v33 ON | rsm08 reference | rsm08 v33 OFF |
| --- | ---: | ---: | ---: | ---: |
| Cold prepare | 23.613375 | 16.227106 | 25.583267 | 23.995366 |
| Repeated noop 1 | 0.035397 | 0.035244 | 0.033168 | 0.033613 |
| Repeated noop 2 | 0.032222 | 0.031727 | 0.033068 | 0.033833 |
| Summary test-body edit | 0.207819 | 0.188457 | 0.214590 | 0.213632 |
| Summary edit recovery | 0.127774 | 0.129184 | 0.131330 | 0.133055 |
| Ordinary shim edit | 0.123657 | 0.128230 | 0.118590 | 0.127401 |
| Signature base | 0.109323 | 0.121767 | 0.105497 | 0.110378 |
| Signature invalidation | 0.246882 | 0.196612 | 0.207840 | 0.231032 |
| Manifest feature add | 5.910214 | 7.722203 | 5.986772 | 5.925158 |
| Type error | excluded | excluded | excluded | excluded |
| Type recovery | 5.605044 | 7.092142 | 5.421375 | 5.331577 |
| Assertion failure | 1.600754 | 1.655827 | 1.638185 | 1.620474 |
| Assertion recovery | 0.383961 | 0.384706 | 0.368833 | 0.401288 |
| All lib | 0.840957 | 0.870163 | 0.917855 | 0.807980 |

The cold result is scoped, actual elapsed time, not a tool-only timing. Fresh private tool stores are not flushed dependency disk caches or OS page caches. Manifest and type recovery regress, and many warm states are not better. rsm08 adds a same-v33-binary default-off control. Its cold candidate median is 23.995366s versus opt-in 16.227106s, but manifest regeneration is 5.925158s OFF versus 7.722203s ON and type recovery is 5.331577s OFF versus 7.092142s ON. The serial runs have different reference timings and cache/noise contexts. This supports a bounded descriptive tradeoff, not demand-only causal attribution or significance. Comparing either candidate directly with the original reference alone does not isolate demand-std from prior candidate changes. No weighted aggregate, significance, comprehensive synthetic-workload result, or general/default-promotion claim is made.

The larger all-lib contract is exactly 148 names/statuses: 147 passed, 0 failed, 1 ignored, 0 filtered. Only the independently pinned `#[ignore]` test `split_index::tests::attach_cost` is allowed. Earlier separate original negative admission and corrected offline admission remain in the historical matrix bundle, not silently rewritten here.

## Earlier prototypes and native correctness

`proof/v31`, `proof/v32`, and `proof/v33` retain original/candidate main files, exact source patches and licensed provenance. Each has native check/build/default full359 journals with source/lock before-and-after identity manifests and raw libtest names, not a summary-only assertion.

All three default native gates passed. Their opt-in cold/link probes differ:

- v31 off passes four tests, on fails with unresolved allocator/standard-library link symbols. It is a failed opt-in prototype, not a failed default build or an admitted timing.
- v32 off passes four tests, on fails with duplicate `__rust_panic_cleanup`. Its shorter failing elapsed time is not a speedup.
- v33 off and on both pass the exact four-test cold/link gate. These single calls and worker profiles are correctness/diagnostic receipts, not a statistical performance result.

Full off/on stdout/stderr and worker profiles are retained. All nine cited native gate IDs also have existing-run, path-redacted `justrust export` summaries under `slim/`, linked back to the full raw gate journals. The coordinator's separate actual 733-test Python bench regression log is retained at `regression/root733.log`, not confused with the native full359 gates or earlier 728-test log. The older probe helper **did not serialize** `RA_JIT_DEMAND_STD` or `RA_JIT_STATS` in its environment receipts. The retained script explicitly sets those controls and the state labels identify off/on. That is script-and-mode provenance, not an invented environment receipt. The newer rsm05 matrix serializes its candidate `RA_JIT_DEMAND_STD=1` exactly. rsm08 metadata and every version/workload receipt independently establish the flag is absent for both stacks.

## Distinct failed/aborted launches

- **rsm03:** accidentally launched without the opt-in flag, then parent cancelled. One incomplete reference cold call is retained. No finally/restoration/exit summary exists. Three durable original backups are retained. Coordinator reported copying those backups at `2026-10-31T00:21:41Z`. That is reported operational context, not a fabricated historical finally attestation.
- **rsm04:** pristine source admission failed before workload calls, specifically `src/shim.rs` did not match f97fcc2. The original launch log is retained. There was no output directory or timing result.
- **rsm06:** both command positions pointed at jr-engine with unsupported reference identity/style, no opt-in flag. All 140 rows remain rejected. This is an unsupported-command control, never valid demand-std attribution.
- **rsm07:** proper reference/default-off attempt timed out at the outer 600-second tool bound. Its 128 durable rows include later signal-15 workspace failures under reported memory pressure. No final restoration or summary was written. Its incomplete timings are not published as medians or attribution.
- **cleanup:** a separate follow-up script and raw native stop receipts cover exactly 32 namespaces. Status 0 for 27, status 1 for five unsupported rsm06 reference namespaces. Stores were not deleted. These are native stop outcomes, not universal process-exit attestation. The coordinator subsequently restored the three rsm07 source files from durable backups, not Git. Later successful source admission is separate evidence, not a retroactive finally record.

## Portable replay and privacy

Portable replay passed, followed by 29 relocation/re-signed semantic controls in 18.679s. Exact actual output is `controls.log` (SHA256 `b2be79977a2dda978966a8d3678fa8b6d2de50999058b0903f82db1d676c4499`). These are offline evidence tests, not additional native executions. Reproduce after the original evidence has been sealed:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-demand-std-v33-20261031/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-demand-std-v33-20261031/test_validate.py -v
```

No Git, native engines, original private paths, or external services are needed for replay. `collect.py` and `collect_followups.py` document the original explicit read-only export and are **not** part of validation. JSON environment maps use the existing receipt allowlist plus only the two explicit experimental controls. Credential/unrecognized keys are excluded. Raw console/profile content and private paths are retained, so this is not general anonymization. Binaries, target trees, caches, ambient environment dumps, and private source workspaces are not published.

`SHA256SUMS`, original private commitments, derivative hashes, parser replay and re-signed semantic mutation controls have different roles. Hashes prove internal consistency, not third-party authenticity. The strict validator checks runtime nonces, exact tests/status/counts, original exclusions, source mutation recipes, restoration, flags, prototype failures, source patch replay and full359 journals independently of a newly coherent checksum seal. Relocation and no-subprocess tests guard portability.

Runtime mutation markers/instrumentation change timings. Background daemon CPU, memory pressure and OS/dependency cache noise are not controlled by child CPU measurements. No claim of clean-machine isolation, causal demand-only attribution, statistical significance, comprehensive coverage, default readiness, or generally beating Blaze follows from this bundle.
