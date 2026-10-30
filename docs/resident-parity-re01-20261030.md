# re01: retained Blaze-derived native parity receipts

This is an offline audit bundle of the completed October 30, 2026 `re01` run,
not a new benchmark or a compiler distribution. `jr-engine` is **Blaze-derived**,
origin `5d4f78f7577cf7170993b74f735099dd03f5c6ca`, not clean-room work or an
independently demonstrated optimization. The reference is source-built pristine
Blaze, not the separately installed Blaze binary.

| Five accepted edit samples per stack | Median seconds |
| --- | ---: |
| Source-built Blaze reference | 0.108226630 |
| Blaze-derived native jr-engine | 0.121279609 |

The derived stack was slower. These are descriptive numbers only. No independent
optimization proof, performance promotion, broad workload claim, or quiet-machine
claim follows. The original noise receipts explicitly classify contamination as
unknown. Proc macros used the matching rustc compatibility control, with both
server variables explicitly `0`. Unseeded private stores were prepared by actual
filtered tests. No automatic seeding was performed by the retained harness.

## Portable offline replay

Copy the entire `bench/results/resident-parity-re01-20261030/` directory anywhere.
With Python 3.9 or newer, run from any working directory:

```sh
python3 /path/to/resident-parity-re01-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 /path/to/resident-parity-re01-20261030/test_validate.py -v
```

Neither command needs this checkout, Git, a compiler, network, the original
scratch directories, caches, or engine executables. Tests copy the bundle into a
temporary directory and mutate only that copy. `JCODE_SCRATCH_DIR`, when set, is
used for that temporary copy. Only lightweight Python offline audits were run
while retaining this bundle. Do not execute `collect.py`, the original shell
scripts, or the retained workload harness to replay the audit. They are exact
provenance/reproduction inputs and are intentionally not rewritten as portable
workload runners.

The validator verifies all 63 source files against before/after SHA256 manifests
and retained Git blob IDs at `f97fcc2`, including pristine `src/summary.rs` and
`source.original`. It reconstructs the exact type, assertion, and five runtime
edits with the retained helper's two pure functions. It checks all 22 runtime
rows, including ten accepted actual-output markers, their exact test names,
libtest counts, statuses, source hashes, and both failure/recovery gates per
stack. It recomputes medians, validates tool/toolchain before/after identity,
explicit no-seed policy and command coverage, build exits and source receipts,
and eight native entry/namespace cases against their raw stdout/stderr.
`SHA256.json` seals the allowlisted file set and byte contents. It is an integrity
index, not a signature against an adversary able to replace the whole bundle.

The native entry receipts preserve the operational pass for missing, relative,
shared, linked and hostile-alias home/namespace controls, version without a home,
and rejected management commands. Offline replay checks the retained receipts,
not present-day filesystem permissions or a newly executed native engine.

## Contents and explicit retention allowlist

- `runtime/`: complete original top-level `re01` stdout/stderr for both stacks,
  records, metadata, summary, and `source.original`. No state directories copied.
- `source/`: all **63** pristine workload source files from the pinned snapshot,
  including its original MIT license, notices, licensing/provenance documents,
  and historical documentation. These are audit inputs, not active agent policy.
- `entry/`: original root `re-entry-20261030` acceptance JSON and the sixteen
  stdout/stderr files for its exact eight cases. No private state or symlinks.
- `build/`: exact `build-baseline.sh`, `build-candidate-v2.sh`,
  `run-runtime-v2.sh`, runtime-v2 log/exit, and explicitly named receipt files
  from `baseline-build` and `candidate-build-v2`. Logs are retained byte-for-byte,
  including original local paths and existing justrust output suppression.
- `harness/`: exact runtime runner, native-entry runner, and their two transitive
  Python helpers. The offline validator extracts only `edit_bytes` and
  `test_results`, without importing or executing harness workload entrypoints.
- `provenance/`: Blaze Apache-2.0/MIT licenses, full baseline/candidate compiler
  source hash manifests, exact candidate build input names/lengths/hashes,
  baseline origin/clean-status receipts, workload Git blob manifest, pinned Rust
  channel manifest, and the enclosing repository toolchain import inventory.
  That inventory concerns rustc/Cargo imports, not the Blaze origin identity.
- `collect.py`: explicit one-time allowlist and read-only provenance collection.
  `validate.py` and `test_validate.py`: the portable offline audit implementation.

No binaries, cache stores, target trees, native compiler source trees, or new
native workload execution are included. The compiler source manifests were
captured read-only during retention. Candidate hashes come from committed
`20f9102027d553bc72c003927964b4ca72bc6574`, not later concurrent working-tree
edits, with that commit recorded in `provenance/candidate-commit.txt`. Four
candidate source files have the
original pre-build SHA256 and successful post-build check receipts. Other source
manifest entries are retention-time identity receipts, not a newly invented
historical pre-build attestation. The embedded build ID is preserved by native
version output and tied to the recorded binary SHA256. Hashes alone cannot
reconstruct or independently recompile that compiler, and replay does not claim
to recompute its complete build ID without compiler source bytes.

Original private paths are retained for fidelity. The collector copies only the
explicit files listed above, never inherited environment dumps, account stores,
credentials, caches, symlink targets, or entire scratch trees. A credential-pattern
scan of runtime, entry, build and provenance receipts found no credential matches.
The original scripts explicitly name only build controls and tool paths.

## Licensing and scope

Blaze origin is recorded in the source-built native version receipts and the
baseline Git origin receipt. The derived engine remains experimental Blaze-based
work, with upstream Apache-2.0/MIT license texts retained in `provenance/`.
The complete workload snapshot carries its own original licenses and notices
unchanged in `source/`. Retaining a historical source file or notice does not
change its license. This bundle is parity/provenance evidence only, not an
independent compiler implementation or a release/promotion gate.
