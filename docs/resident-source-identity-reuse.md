# Resident source identity reuse v86: rejected promotion

2026-10-31. This was a licensed Blaze-derived source experiment, not a clean-room compiler implementation. **The candidate was rejected for promotion and the shipping source is restored.** Complete portable evidence lives in `bench/results/resident-source-identity-reuse-20261031/`.

## Results and limits

Root native journals record 36 registry, 22 manifest, 4 metadata, 53 codegen and 373 resident tests passing. The four new metadata tests include real Config and filesystem fixtures. Root also recorded eight actual activation-profile rows, all 28 strict OFF/ON/ON/OFF transitions, and a restored full Python harness regression of 761 tests. Mocked adapter tests are separate from native acceptance.

| Independent pinned run | OFF median | ON median | Descriptive difference |
| --- | ---: | ---: | ---: |
| First completed run, si86b | 0.060376224500942044 s | 0.053265487993485294 s | -11.777378539702832% |
| Fresh repeat, si86c | 0.054563309997320175 s | 0.053647840992198326 s | -1.6778106115021507% |

Each completed run retains its own 54 calls, 20 timed samples, 24 runtime nonces, all first touches and five stops. Results are not pooled. The fresh repeat did not meet the requested 10% floor, so the apparently larger first difference was not a repeatable useful win. Noise remains unknown. These are descriptive uncontrolled observations, not a significance, general speedup or causal regression claim. Whole-generation and watcher stayed ON in candidate arms, so this factor concerns combined same-byte manifest+lock retouch, not separate attribution. No default was promoted.

The earlier si86a attempt is retained as **INCOMPLETE**, with eight calls and two stops. Its first candidate segment's prepare call failed with the factor OFF: the num-traits build script panicked through autocfg with `could not execute rustc`. That attempt specified the optional toolchain proc-macro server, whereas both completed runs used server=0. The cause is not proven and is not reclassified as either an environment fault or candidate bug. A separate initial root preflight used the wrong candidate archive path and failed before native jobs began. Both failures remain in the evidence.

## Frozen candidate contract

Only exact first-use `RA_JIT_SOURCE_ID_REUSE=1` enabled the candidate. Missing or other values stayed OFF. OFF dispatched to the full original fetch body, bytewise preserved alongside the accepted original `manifest_source`. ON used a new map inside each fetch, keyed by the exact original formatted source string, caching successful resolutions only. Formatting, replacement, checksum lookup, fetch batching, root order, manifest reads and error exits stayed in their original positions. No persistent, cross-call, failure, manifest or package cache was added.

The audit found crates.io resolution is configuration-sensitive: `SourceId::crates_io` calls `Config::check_registry_index_not_set`, which calls `get_string("registry.index")`. A cached crates.io hit therefore repeated that exact validation at the original occurrence, preserving config reads, warnings and errors before cloning the full SourceId. Other registry/sparse/git constructors were pure. These assumptions would need reauditing if constructors changed.

Only an actual successful hit after validation emitted the once-per-process fixed stderr line `ra-jit: source-identity-reuse-v86 batch`, using ignored-error `write_all`. Flag entry, misses and failed validation did not emit it. Native marker observations establish activation, not completion or speed.

The frozen four fixtures cover repeated/mixed registry/sparse/git IDs and reference precedence, full SourceId equality, invalid sources, fresh Config instances after edits, populated-cache error validation, a nonempty warning on a hit, and complete exported vendored package/path/order/source/error/warning observations. The original and candidate sources, independent full source oracle and patch are archived. Native acceptance and timing were root-owned, not executed by the source lane.

## Restoration, provenance and replay

Before restoration, exact live source and doc copies were archived and compared to the frozen candidates. Only `toolchain/resident/crates/blaze-metadata/src/lib.rs` was restored via `tools.write` from the frozen original. Bytewise comparison and exact-file HEAD diff passed, and source ownership was immediately released. Original SHA-256 is `6c225dd077fb24b8fd83afffa739044d1752bb4581fb41390ac12cfd3fb0b59b`. Candidate SHA-256 is `b761a33a6c7be01917fbacefd825aba0cb77a8d538371f11e7a54566203525bd`.

The initial 18-mock driver and later timing driver differ only in two descriptive replacement values. Both complete drivers, their incremental patch, exact transformed identities, identical pure-oracle AST proof and separate initial/final 18-mock receipts are retained. No semantic oracle was weakened. Historical source-only RELEASE records remain historical records, not claims that subsequent coordinator native execution did not occur.

The bundle provides stdlib-only independent replay and coherently resealed negative tests, including relocated empty HOME/PATH execution with an audit hook denying external reads, writes, subprocesses, network and dynamic loads. Run `python3 -B validate.py --manifest` and `python3 -B test_offline.py` from the bundle. Explicit fixed-field/raw-line allowlists retain original private input SHA-256/byte sizes without exporting ambient environments, unrestricted private logs, binaries or stores. Full already-public pinned source blobs retain their historical literal paths, disclosed in the bundle README.

The imported `indexable-inc/blaze` lineage retains MIT and Apache-2.0 license files and attribution. This evidence-only archive does not deploy the experiment or claim independent authorship of Blaze.
