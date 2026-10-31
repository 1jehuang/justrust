# Rejected native builder-context-pool evidence, 2026-10-31

**REJECTED for shipping. Production source restored. No speed, zero-overhead or default-promotion claim.** Licensed Blaze-derived resident engine work, not clean-room work. This directory is a portable historical evidence bundle, not an installed implementation.

## Experiment and decision

v52 proposed a one-idle-context-per-thread pool for real native `FunctionBuilderContext`, not compiled `Context`. Contexts were returned only after successful consuming finalize and the builder scope ended. Fill errors/panics/incomplete builders discarded their contexts. The exact opt-in was `RA_JIT_BUILDER_POOL=1`, latched at first native build. v53 added only a bounded fixed successful cached-take marker using a thread-local announced flag plus process `Once`, with ignored I/O results. Original semantic tests, shim behavior, gate and discard path were preserved. Complete sources and proposal docs are retained under `frozen/`.

Actual original-workspace check/build and full release native suites passed for both versions: **57 codegen + 360 resident**, zero failed/ignored/filtered. The four added release-visible fixtures checked native IR/signatures/verifier equivalence, positive reuse counters, exact errors, fill-panic/incomplete discard, and nested usage. **The debug-only finalize-panic fixture was not executed by the release gates.** Offline replay does not supply that missing execution.

All28 strict actual runtime calls passed with the same v53 binary, four private cases in OFF/ON/ON-repeat/OFF-repeat order and seven phases each. Each ON worker-log snapshot contains exactly one successful-reuse marker, each OFF snapshot zero. Worker snapshots are cumulative: they prove actual reuse somewhere in the worker, not fresh per-phase reuse. Never sum them or attribute phase latency to new reuse counts.

Percent change is `100 * (ON / OFF - 1)`, positive means slower:

| State | OFF then ON | Reverse ON then OFF |
| --- | ---: | ---: |
| cold-prepare | +37.2901% | -1.4956% |
| repeated-noop-1 | -18.2826% | +4.8085% |
| manifest-feature-add | -0.6164% | +9.6261% |
| type-error | +0.2089% | +3.2514% |
| type-recovery | +0.3153% | -2.9040% |
| assert-failure | -5.9322% | -9.1732% |
| assert-recovery | +8.4700% | -16.3483% |

There was no repeatable useful broad major-state gain. Noise is **unknown**, including daemon/background CPU and OS cache state. The assertion-failure pair alone is insufficient. These descriptive counterordered cases are not a comprehensive session-weighted synthetic acceptance gate. Root rejected both candidates and restored the production source. No allocation-intuition performance inference is made.

## Retained provenance

- `frozen/v52`, `frozen/v53`: full original/candidate native sources, proposal docs, full/incremental patches, source-check notes, SHA manifests and readiness markers. v53 also retains exact live source/doc before restoration. Historical readiness is not current shipping approval.
- `native/candidate-{build,unit}-v{52,53}`: actual check/build/codegen/resident logs and exits, full original run journals including raw output, metadata, summaries, units, process/sample telemetry, full3757 source before manifest and after checks, unchanged production-lock checks, native engine version/whole-binary SHA receipts. Binaries are not embedded.
- `raw/`: every original state.json, stdout/stderr, cumulative worker-log snapshot and three original input backups for all four cases. `rows28.json` is an exact index, not new data.
- `snapshots/source63.tar.gz`: complete pinned63 workload inputs used to reconstruct every edit recipe.
- `oracle/`: unchanged original five Python semantic-oracle modules. They parse actual raw tests and require exact names/status/counts/filtered counts, invocation-specific runtime challenges, fresh type/assertion diagnostics and correct process statuses.
- `provenance/`: actual native and paired launch scripts, original probe, original runtime analysis and result, original pair exit, production Cargo.lock and inspected Cranelift frontend primary source.
- `licenses/`: resident MIT/Apache attribution, justrust workload/oracle license, Cranelift frontend license. Resident origin is Blaze-derived `5d4f78f7577cf7170993b74f735099dd03f5c6ca`.
- `restoration.json`: byte-identical original/HEAD verification at `2026-10-31T06:11:49Z`, source released at `06:12:01Z`. Only `toolchain/resident/crates/hir-ty-codegen/mod.rs` was restored. The candidate is absent from shipping source.
- `decision.json`, `checks/`: explicit rejection/claim boundaries, actual offline replay and relocation/mutation results.

Native build identities:

- v52: `resident-v1-1d28c3c4652de4a1dace6e79c8508f8383a5826a60b2cdf3b89fd7e44d232aba`, binary SHA `6f7d5ab1526668de39f2c2102738bc492866fa5b642b185b049ea8af389f9631`.
- v53: `resident-v1-b5f8afa936d719f3cbe377273fe573eca6ef75ae0509f71d57302d4a198201c7`, binary SHA `76e527c076e368fd6f2adfa0777c86502fcbee4d3603b12f342afbf1412c2b17`.

The whole3757 native-source delta between v52 and v53 is only `mod.rs`. All28 calls join v53's actual binary hash before/after and the same complete4020-file nightly toolchain identity. Its channel-manifest SHA is `34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`. No private cache, credential, engine binary or toolchain distribution is embedded. Original paths inside receipts remain historical metadata and are never dereferenced by portable replay.

## Standalone offline checks

Python standard library and this directory suffice, from any cwd:

```sh
python3 -B /path/to/resident-builder-context-pool-20261031/replay.py
python3 -B /path/to/resident-builder-context-pool-20261031/test_replay.py
```

Replay verifies complete inventory, frozen/native/runtime joins, pinned63 and4020 identities, exact input recipes, every raw receipt, original strict semantic oracles and original analysis results. Native libtest parsing normalizes only the literal `- should panic` annotation. It reports **historical evidence**, not native re-execution or an independent Rust-semantics proof.

The test driver relocates the bundle to an unrelated path with spaces and executes isolated Python with empty HOME/PATH and an audit hook forbidding external application reads and native/network calls. Twenty-four coherent negative controls re-seal all outer checksums and update raw lengths/hashes, parsed results and row indexes where applicable, yet must fail semantic validation. They cover runtime/test/diagnostic freshness, exits, source recipe, ON/OFF/fixed/duplicate marker semantics, coherent toolchain/source mutations, exact native fixtures, phase order, noise certainty, promotion/speed/zero-overhead/debug claims, restoration and original analysis. Scratch defaults to the current user's `~/.jcode/scratch`, or `JCODE_SCRATCH_DIR`, and temporary copies are removed.

`record_checks.py` captures these actual offline results then re-seals the inventory. `archive.py` only copies already-existing evidence and needs the historical machine paths. Neither is required to replay the relocated bundle. `SHA256SUMS.bundle` is integrity, not cryptographic signing or provenance authentication. Mutation checks do not model rewriting the validator itself.

Only evidence and documentation are committed. The evidence lane ran no native build, test, benchmark or cleanup job. Authored scripts/docs pass scoped whitespace checks. `checks/retained-original-whitespace.log` retains the broader archived-data diff-check warnings from original raw stdout and patch context. Raw historical whitespace is preserved, not reformatted.
