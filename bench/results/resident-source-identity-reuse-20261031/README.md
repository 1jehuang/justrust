# Source identity reuse v86: rejected promotion

2026-10-31. Licensed Blaze-derived experiment, shipping source restored. This is portable historical evidence, not a deployed optimization or a speedup claim.

| Gate | Result |
| --- | --- |
| Actual native registry / manifest / metadata / codegen / resident | 36 / 22 / 4 / 53 / 373 passed |
| Actual activation profile | 8 rows |
| Strict OFF / ON / ON / OFF | 28 transitions passed |
| First completed pinned run si86b | 54 calls, 20 samples, 24 nonces, 5 stops |
| si86b OFF / ON median seconds | 0.060376224500942044 / 0.053265487993485294 |
| si86b descriptive difference | -11.777378539702832% |
| Independent fresh repeat si86c | 54 calls, 20 samples, 24 nonces, 5 stops |
| si86c OFF / ON median seconds | 0.054563309997320175 / 0.053647840992198326 |
| si86c descriptive difference | -1.6778106115021507% |
| Initial si86a | INCOMPLETE, 8 calls, factor-OFF prepare buildscript failure, 2 stops |
| Restored full Python harness | 761 passed |
| Decision | REJECT promotion, useful first difference not repeatable, noise unknown |

All first touches remain separate in both completed runs. No pooling hides the first result or fresh repeat. Same candidate ELF in both arms, same live worker PID/start ticks/ELF/socket within each segment. Whole-generation and watcher fixed ON, combined same-byte manifest+lock retouch factor only. This is not separate manifest/lock attribution, statistical significance, general speedup or a causal regression claim. Five stops per completed run, two initial-attempt stops and four post-measurement strict stops are retained. Stores were not deleted.

## Failures and exact provenance

The initial optional-toolchain-proc-server si86a run failed at call007, the first factor-OFF candidate prepare, through num-traits/autocfg with `could not execute rustc`. Its complete eight-row prefix, exact factor controls, optional server identity, selected failure lines and original journal SHA/size are retained. Cause is not proven. Both completed runs explicitly used server=0. The initial root preflight127214ykpx had the wrong archive candidate path and failed before native jobs. Its selected exact diagnostic and original SHA/size are retained. Neither failure was hidden or classified as certainly environmental or a candidate bug.

Initial18 mocked adapters used an earlier driver. At 19:55:03Z root changed exactly two descriptive replacement values, not assertions, input recipes, sampling or activation policy. Both full drivers, initial/final18-mock receipts and description-only.patch are archived. Replay verifies their exact transformed SHA identities and identical pure oracle ASTs. Current timing transform is cf539b9986f5b252de3c155c308af053cf770d0d76459cdcf7ffbade1493a11b. The initial transform is c67c9cdd107db4f45929a669424ee091e3f79a3473be393257df7f0503e1248b.

Full source candidates/originals, rollback live copies and source patch are in frozen/. Original native metadata source is SHA6c225dd077fb24b8fd83afffa739044d1752bb4581fb41390ac12cfd3fb0b59b, candidate SHA b761a33a6c7be01917fbacefd825aba0cb77a8d538371f11e7a54566203525bd. Historical source-only RELEASE remains a source-only timestamp, not a claim subsequent root native execution did not occur. Source was restored only after preserving exact live candidate copies. Full source63, toolchain4020 and native-source3757 inventories are included. Original tuple semantic oracles are hash-bound AST-extracted from full immutable sources, not weakened JSON observations.

## Replay

Use only Python standard library, from this directory:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

No native jobs, network, cargo, cache locks, binaries or installations. Offline tests include coherently resealed raw/parsed/timing/worker mutations, both runs' first-touch retention, marker activation, source and native-outcome corruption, and hiding the initial failure. Relocated empty HOME/PATH replay installs an audit hook denying external reads, all writes, subprocesses, network and dynamic loads, with explicit denial controls. Keep the reviewed validator: its fixed historical observation SHA binds records even after a coherent local reseal. SHA256.json alone is an inventory, not external authentication.

Initial adaptation caught two exporter/validator integration issues before sealing: a hash mismatch exposed the historical description-only driver revision, and a private-state loop shadowed the new dual-run selector. The exact driver revision is now preserved and the selector was renamed. Neither correction changed original semantic oracles or historical observations. checks/ records final offline replay and negative test results.

## Privacy and licensing

export_private.py is a private-input exporter, not required for portable replay. It uses fixed field and raw-line allowlists and records original private input SHA/byte size. It never exports ambient environment maps, unrestricted private logs, private source trees, binary bytes or cache/store contents. Required selected worker identities/path roles are normalized; derivative frozen docs and drivers replace private roots with role placeholders. Full raw private journals remain private. Already-public full pinned git blobs retain historical literal home paths in pinned-source/src/split_index.rs, pinned-source/FINDINGS.md and pinned-source/bench/bench_edit.sh. These are explicit public-source exceptions, not exported private workload paths.

The imported indexable-inc/blaze lineage retains MIT/Apache-2.0 licenses and attribution in lineage/. Public pinned source and original semantic-oracle attribution are retained, not relicensed. The prior v85 evidence bundle was read as methodology only and was not modified.
