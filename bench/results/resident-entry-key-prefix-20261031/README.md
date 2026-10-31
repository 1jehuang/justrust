# Rejected native resident entry-key-prefix experiment, 2026-10-31

**Decision: REJECTED. Source restored. No speed claim or attribution.**
This is experimental licensed Blaze-derived resident work, not clean-room work.
The bundle contains actual v44/v45 journals, not a synthetic runtime substitute.

## Result and admission boundaries

Both candidates passed the actual 57-test native codegen suite and 360-test native
engine suite, plus 28 strict seven-state rows each. All 56 rows, complete raw
stdout/stderr and cumulative worker logs are preserved without truncation.
`rows56.json` is an exact index of those rows, not new measurements.

v44 **failed positive-activation admission**. Its frozen constructor had no
successful-preparation marker, and ON raw worker logs had none. Its timing rows
are retained but unadmitted as runtime-prefix or speed proof. The actual captured
output of coordinator task `013790r9hw` is retained in provenance. It contains an
AssertionError and subsequent unrelated evidence-audit successes in the same
shell. The overall exit 0 masked the first failure, so it is not proof that the
activation gate passed. No separate original failed-admission JSON exists.

v45 added exactly six constructor lines to emit the fixed marker on successful
context preparation, ignoring write errors. There were no key/query/Store/writer
or oracle changes. ON has marker count 1 in all seven cumulative snapshots, OFF
has 0. Those seven snapshots are not seven independent context activations.
Positive/negative activation controls passed, but repeatable usefulness did not.
The actual retained posthoc root validator and its output are
`provenance/validate-prefix-v45.py` and `provenance/prefix-v45-validation.json`.
The portable replay recomputes their activation counts and all 14 percentages.

| v45 phase | First `(ON/OFF-1)*100` | Repeat percent |
| --- | ---: | ---: |
| cold-prepare | +36.944 | -2.252 |
| repeated-noop-1 | -13.835 | +36.545 |
| manifest-feature-add | +6.030 | -5.411 |
| type-error | +1.465 | +0.055 |
| type-recovery | +5.679 | +1.426 |
| assert-failure | +5.663 | +7.828 |
| assert-recovery | +2.552 | -0.928 |

Noise is **unknown**, including daemon/background CPU and OS caches. Waited child
CPU is not daemon CPU. Two counterordered scoped pairs are not the comprehensive
synthetic gate, establish no significance, and justify no default promotion or
causal regression attribution.

## Layout and identities

- `frozen/v44`: exact accepted original cached.rs, full candidate cached.rs/new
  documentation, patch, SHA256SUMS, BASE_HEAD and original readiness receipt.
- `frozen/v45`: exact full v44 originals, complete v45 candidates, marker-only
  repair patch, SHA256SUMS and readiness. Also exact rejected source preserved
  immediately before restoration.
- `native/v{44,45}`: original build/test launcher, exit statuses, source/lock
  manifests and postchecks, engine digest/version, complete justrust display
  journals, and original native run meta/output/summary/unit records. No binaries
  or toolchain artifacts are embedded. All native jobs were run by the root,
  not by this evidence lane. There were no reruns to make this bundle.
- `raw/v{44,45}/{off,on,on-repeat,off-repeat}`: original state.json, full raw
  streams and worker log snapshots, and exact three original input backups.
  Complete source and toolchain manifests before/after are in state.json. The
  exact seven source edit recipes are replayed from backups with the original
  invocation nonce, not just trusted from accepted flags.
- `oracle`: unchanged copies of the five actual Python harness/oracle modules.
  Replay imports those copies, not current repository modules. These preserve
  exact test names/counts/filtered counts, failure statuses, fresh runtime/type/
  assertion markers, and input recipes. This is an offline replay of historical
  correctness evidence, not native execution or an independent language proof.
- `restoration.json`: actually observed source-backup restoration at
  2026-10-31T03:59:19Z, cmp and HEAD diff checks, and ownership release.
- `provenance`: original probe, actual root v45 validator/output, actual tool
  task outputs, and separate cleanup sources/receipts/raw streams.
- `SHA256SUMS.bundle`: transport/inventory checksums for every retained file.
  These are not signatures or proof of provenance against an adversarial editor.

Accepted base source: `cb9d8a01000e2a1efb44e87e74803b4317cdf997`.
Restored cached.rs SHA256:
`11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f`.
Native build fingerprints and engine hashes remain in their original journals:

- v44 build `resident-v1-32d34f4d0e0b97042dfa059101700b73a8289a72b06c59c656c3f495bc787d4b`,
  engine SHA `4aa2e207ba0e0244b7bc4a1e1fb738b750973b8507cdbc96cd1fda6d134e6053`.
- v45 build `resident-v1-afed4be77b377982b9085237403052c1b90566744cb13538493417df6a92fbcd`,
  engine SHA `741c0c371c15f289891e75963de5a864aed8ed68a8c99cf1391f09124e33d35f`.

Toolchain is `nightly-2026-10-27-x86_64-unknown-linux-gnu`; its complete file
identity is preserved per state, including channel-manifest SHA
`34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.
The copied actual launcher explicitly uses local native builds with that pinned
experimental toolchain. Whole-source build fingerprinting applies.

## Launcher and cleanup provenance limitations

There is no retained original shell script for the paired workload launch. The
original per-state probe source, exact per-row commands/startup controls, pair
exit receipts, and actual v45 task `9336608n9r` output are retained. The coordinator
reported launch order `off/on/on-repeat/off-repeat`, private namespaces n1-n4 for
v44 and o1-o4 for v45. That ordering is coordinator-reported, not inferred from a
nonexistent original launch script. The original probe imports historical
absolute paths and is preserved as provenance, not the standalone replay entry.

`completed-v43-v44-stop` retains actual post-validation cleanup of six completed
v43/v44 namespaces with stores preserved. `completed-v45-stop` retains separate
postmeasurement guarded stop receipts for o1-o4, all exit 0, at approximately
2026-10-31T04:00:56Z, with stores preserved. These are operational cleanup only.
They do not prove historical quietness or improve admission of earlier timing.
No cleanup was run by this evidence lane, and no store was deleted.

## Standalone offline replay and falsification

From any directory, with only Python's standard library and this whole bundle:

```sh
python3 -B /path/to/resident-entry-key-prefix-20261031/replay.py
python3 -B /path/to/resident-entry-key-prefix-20261031/test_replay.py
```

The test driver relocates the bundle, uses isolated Python with empty HOME/PATH,
forbids original checkout/scratch reads and native/network execution through an
audit hook, and replays all 56 rows. It also checks coherently re-signed mutations
of runtime/type/assertion nonce challenges, test identity, success/failure exit,
input recipe, unknown-noise classification, ON/OFF activation controls, all
matching toolchain before/after receipts, actual native fixture identity, and
phase order. Raw-mutating cases update inner raw digest/length and parsed data,
row changes update rows56, and all cases regenerate outer checksums first.
Rejection therefore demonstrates semantic checks rather than checksum mismatch.
The v44 failed gate and v45 rejection remain explicit, even when replay passes.

`archive.py` only copies existing actual evidence and requires the original
scratch tree. It is not needed for replay. `record_checks.py` runs only offline
validation/tests and writes the check reports before re-sealing. Neither script
runs native jobs or alters production source. Historical absolute paths in raw
journals remain unchanged for provenance but are never dereferenced by replay.
