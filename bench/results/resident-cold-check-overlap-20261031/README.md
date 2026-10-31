# Rejected native cold-check overlap evidence, 2026-10-31

**REJECTED for shipping. Production main.rs restored. No speed, zero-overhead or default-promotion claim.** This is licensed Blaze-derived resident-engine history, not clean-room work or an installed feature.

## Candidate and decision

v54 overlapped the complete mandatory whole-source `broken_in` diagnostics with the first immutable cold `build_unheld`. v54b corrected the exact-1 flag to process startup: a `LazyLock<bool>` is forced first in `main`, never re-read per request. Scoped task joins complete and all cloned databases drop before `include_sources` or any later mutable database work. Any include load discards speculative diagnostics and the original full check follows recompile. The final exact crate/file/path plan must match the speculative plan or the complete original check runs. Original unset/proc-macro/error/refusal ordering remains in the candidate. Full original/candidate files and patches, not only excerpts, are retained.

The coordinator's actual native check/build and release suites passed: **53 codegen + 360 resident**, zero failed/ignored/filtered. Whole3757 source and production root-lock checks passed unchanged. All30 actual original/OFF/ON native source observations passed the exact10-case diagnostic contract: valid, unused and reached type errors, denied lint, borrow error, valid/error body include, included module error, macro error and syntax error. Full diagnostic blocks, including source locations and unused errors, match without message/location normalization. Each fixture ON worker has one asynchronous startup marker; OFF and original none. All observations include post-observation stop status and raw streams.

**Not executed:** injected compile/diagnostic panic, unset or proc-macro failure tests, alternate lint-cap/borrow-refusal controls and long-lived include-recovery controls. Passing source fixtures plus lifetime inspection do not supply these missing native tests. Proposed controls in frozen documentation are not passed-gate evidence.

All28 strict actual runtime calls passed, same v54b binary, OFF/ON/ON-repeat/OFF-repeat private cases with seven phases each. Performance did not meet a useful repeatable major-state improvement. Percent change is `100 * (ON / OFF - 1)`, positive means slower:

| State | OFF then ON | Reverse ON then OFF |
| --- | ---: | ---: |
| cold-prepare | -11.4660% | +38.1424% |
| repeated-noop-1 | -1.2650% | +2.2637% |
| manifest-feature-add | -12.1500% | -2.4224% |
| type-error | -12.6505% | +4.5522% |
| type-recovery | +41.1482% | +4.4552% |
| assert-failure | +245.8553% | +9.9943% |
| assert-recovery | +178.6375% | +1.3354% |

Noise is **unknown**, including daemon/background CPU and OS cache state. The cold gain failed reverse replication, manifest's repeat improvement was small, and both recovery states regressed in both orders. These are descriptive counterordered scoped runs, not a comprehensive session-weighted synthetic gate. Root rejected performance and restored the production source. The approximately1.1-second serial diagnostic phase is not additive savings evidence.

Both ON worker snapshot sequences are `[1,1,1,1,0,0,0]`, both OFF all zero. Snapshots are cumulative worker logs, not fresh per-phase work counters. Never sum or causally attribute phase timing to their counts. A positive marker means a real asynchronous diagnostic task started, not that speculation was reused.

## Retained evidence

- `frozen/v54`, `frozen/v54b`: complete originals/candidates, candidate docs, full patches, inner SHA256 manifests, source-check logs, historical release receipts and fixture inputs. v54b preserves preliminary v54 and exact live source/doc before restoration. Historical readiness is not shipping approval.
- `native/candidate-build-v54`, `native/candidate-unit-v54`: actual check/build/53-codegen/360-resident output and exits, complete original justrust journals, full3757 source-before manifest/source-after checks, production-lock before/after, version and whole-binary identity. No binaries, private caches or credentials are embedded.
- `fixtures`: all30 original/OFF/ON raw stdout/stderr, every worker log, source inputs/final Cargo.lock, actual state.json and post-observation stop streams/statuses. Complete diagnostic identities and stops are anchored by the offline verifier.
- `raw`: all28 runtime streams and cumulative worker snapshots, every original state.json and the three pinned original input backups per case. `rows28.json` is an exact index.
- `snapshots/source63.tar.gz`: complete pinned63 workload, used to reconstruct each exact source edit and runtime challenge.
- `oracle`: unchanged original five Python strict semantic-oracle modules. They check exact test identities/counts/statuses, invocation-specific runtime nonces and type/assertion freshness, not only self-reported acceptance booleans.
- `provenance`: original native/paired launchers, both probes, original validators and validator result JSON, pair exit, production Cargo.lock and actual vendored `RootDatabase` implementation. Historical paths are never dereferenced by replay.
- `licenses`: resident MIT/Apache and origin attribution plus justrust workload/oracle license.
- `restoration.json`: exact live original/HEAD equality observed at06:56:09Z and source released06:56:18Z on2026-10-31. Only `toolchain/resident/crates/ra-jit/src/main.rs` was restored, via `write` from the exact frozen original.
- `decision.json`, `checks`: explicit rejection and missing-test boundaries plus actual offline replay, resealed semantic controls and isolated relocation results.

Native candidate build identity: `resident-v1-a78f239e8e6118c3b98a8b417182df5a16e532cefeee442decda648dbc0a5b6e`. Whole binary SHA256: `fa7e283ecc47832e9754f50d6329baa5dbd26f9c60427bfe332e4f44b7684a91`. Fixture original binary SHA256: `1197df17843a18b1cf5af78b22dc805d15bfe45056eefe1a04113599e681e3e0`.

Restored source SHA256: `3ef8143a8f6294335168f9a987d3266ce347f06209e1e3c3f209b72d9c6447fd`. Frozen v54b candidate SHA256: `7c43bf86ba2bf3e1f05f2218f29efd20155b258e9a566312d313581b6142709f`. All28 runtime and30 fixture calls join the same complete4020-file nightly toolchain identity, channel manifest SHA256 `34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.

## Standalone offline replay

Python standard library and this directory suffice, from any cwd:

```sh
python3 -B /path/to/resident-cold-check-overlap-20261031/replay.py
python3 -B /path/to/resident-cold-check-overlap-20261031/test_replay.py
```

Replay verifies complete bundle inventory, both frozen lineages, native build/test/source/lock identities, all30 complete original/OFF/ON diagnostic contracts and stops, pinned63 input recipes, whole4020 toolchain, all28 raw receipts with unchanged original semantic oracles, exact original validator analyses, cumulative marker semantics, explicit missing-test limits and restoration. Native libtest parsing normalizes only the literal `- should panic` annotation. Original fixture validation is preserved verbatim and its historical root binding is relocated only in memory. This is historical evidence verification, not native re-execution or independent proof of all Rust semantics.

The driver relocates the payload to a path with spaces, uses isolated Python and empty HOME/PATH, and installs an audit hook forbidding reads outside the bundle/Python standard library, networking and native execution. Thirty-three negative controls re-seal all outer checksums, update raw lengths/digests/parsed fields and exact indexes where applicable, yet must fail semantic replay. They cover actual test/runtime/diagnostic freshness, source recipe, ON/OFF/duplicate/fixed-line markers, coherent toolchain and frozen-source mutations, exact native tests, phase order, complete fixture diagnostic changes including a coherent all-three-mode mutation, stop/status/startup controls, noise, claims, missing-test limits, restoration and original analysis. Temporary copies live under `JCODE_SCRATCH_DIR` or the current user's `~/.jcode/scratch` and are removed.

`record_checks.py` records these actual offline results and re-seals the inventory. `archive.py` copies only already-completed evidence and requires historical machine paths. Neither is needed for relocated replay. `SHA256SUMS.bundle` provides integrity, not cryptographic signing or provenance authentication. Mutations do not model replacing the verifier itself.

Only documentation and evidence are committed. This evidence lane ran no native build, test, benchmark or cleanup job. Original raw whitespace and patch context are deliberately preserved, not reformatted.
