# Broader pinned resident workload comparison

`bench/resident_workload_compare.py` is a separate, configurable diagnostic for
source-built pinned ra-jit versus native jr-engine. It does not change the fixed
`resident_engine_compare.py` fixture or any resident engine source. It is not the
comprehensive session-weighted benchmark, a default-promotion gate, or evidence
for a general speed claim. No native measurements are included in this change.
The root/coordinator owns native execution after the source-only tests pass.

## Fixture selection

The following anchors were inspected read-only with
`git show f97fcc2:src/summary.rs`, `git show f97fcc2:src/shim.rs`, and
`git show f97fcc2:src/record.rs`.

| Workload | File | Exact function opening anchor | Filter | Expected edited execution |
| --- | --- | --- | --- | --- |
| `summary-test` | `src/summary.rs` | `fn shares_overlapping_time() {` | `summary::tests::` | `summary::tests::shares_overlapping_time` |
| `shim-test` | `src/shim.rs` | `fn describes_cargo_rustc_args() {` | `shim::tests::` | `shim::tests::describes_cargo_rustc_args` |
| `record-test` | `src/record.rs` | `fn finds_subcommand_after_global_flags() {` | `record::tests::` | `record::tests::finds_subcommand_after_global_flags` |
| `shim-function` | `src/shim.rs` | `fn parse_pass(json: &[u8]) -> Option<Pass> {` | `shim::tests::` | `shim::tests::parses_time_passes_json` |

The fourth workload edits an ordinary, non-`#[test]` parser function. The pinned
`parses_time_passes_json` test calls it once with a literal JSON input and checks
its parsed fields. This is a deterministic execution path, not a synthetic new
Rust test. The other workloads insert body edits into existing real tests.
Every anchor must occur exactly once. None of the edits changes test names.

## Preconditions and contract

- Use a dedicated **pristine `source63`** copy of the complete `f97fcc2` tree.
  The existing source identity helper verifies all pinned tracked blobs and
  rejects extra input files and symlinks. Never use or edit
  `/home/jeremy/.jcode/scratch/ba30/source` for this harness.
- Supply native executable ELF reference/candidate binaries and the existing
  pinned `nightly-2026-10-27-x86_64-unknown-linux-gnu` toolchain. No toolchain is
  downloaded. The reference must report the exact version required by
  `resident_engine_compare.REFERENCE_VERSION`. Binary identity and reported
  version are receipts, not independent attestation of build provenance. The
  operator must provide the source-built reference, not an installed wrapper.
- Both stacks receive the same pinned toolchain, profile, optional proc-macro
  server, and private, initially unseeded cache policy through the existing
  environment helper. Each stack has its own cache roots, reused across selected
  workloads in CLI order. Selection and order therefore belong in comparisons.
  This is not a cold-cache comparison for every workload.
- Each workload's unmodified **native reference execution** establishes its exact
  sorted test names and passed/failed/ignored/filtered counts. It must include the
  edited function's exercising test and only tests under the requested module.
  Every candidate success and recovery invocation must match that contract.
  An absent/failed reference contract cannot qualify a candidate.
- For each stack and workload, run prepare, deliberate `E0308` type error,
  unmodified recovery, deliberate assertion failure in the edited function, and
  unmodified recovery. The assertion receipt must identify exactly the expected
  failing test and match all other names/counts. A compile-error invocation must
  not produce a parsed successful test receipt. Failing gates exclude that
  stack/workload from timed samples and medians, without seeding or fallback.
- Timed edits include a changing `black_box` assertion and exactly one executed
  `JR_WORKLOAD_EXECUTED_<workload_with_underscores>_<sample>` marker. A stale,
  absent, duplicated, or wrong marker rejects that invocation. Both stacks get
  identical edited bytes for a sample. Stack order rotates with sample number.
- Samples default to five and may be any positive integer. The positive finite
  timeout bounds each engine process, using the shared process-group timeout
  runner. Raw stdout/stderr and process receipts are retained for every call.
- Before writing source, fsynced exclusive backup files are saved. `finally`
  restores their bytes on normal completion, rejected gates, and exceptions.
  The final full source, binary, and toolchain identities must equal the initial
  identities, and toolchain links must remain correct. `source_restored` explicitly
  records whether the full final source manifest matches the initial manifest,
  including on rejected gates and exceptions. Identity failure clears
  all medians. As with any Python `finally`, SIGKILL or power loss requires
  operator restoration from the saved backups before restarting.

## CLI (root-owned native execution)

Use a fresh output directory whose absolute pathname is at most 70 bytes. Set
`REFERENCE_ENGINE`, `NATIVE_ENGINE`, and `PINNED_TOOLCHAIN` to the immutable
absolute executable/toolchain paths already prepared by the root session:

```sh
python3 bench/resident_workload_compare.py \
  --reference-engine "$REFERENCE_ENGINE" \
  --engine "$NATIVE_ENGINE" \
  --toolchain "$PINNED_TOOLCHAIN" \
  --source /home/jeremy/.jcode/scratch/ba30/source63 \
  --output /home/jeremy/.jcode/scratch/ba30/wc63 \
  --samples 5 --timeout 360
```

Default selection runs all four workloads. Add repeated `--workload` arguments
to choose an explicit ordered subset, for example:
`--workload shim-function --workload record-test`. An optional
`--proc-macro-server /absolute/native/server` freezes the same ELF server for
both stacks. Without it both server environment variables are explicitly `0`,
preserving the existing rustc-built proc-macro control.

Output consists of `metadata.json`, `records.json`, `summary.json`, original
source backups, and raw `*.stdout` / `*.stderr` files. These are private receipts
containing absolute paths and raw engine diagnostics, not public benchmark
exports. Environment maps in both metadata and process rows use the explicit
`ENV_RECEIPT_ALLOWLIST`: private cache/source-independent state paths, pinned
profile controls, backend/remote controls, and proc-macro server settings only.
Ambient variables, including `API_TOKEN` and unrecognized future variables, are
omitted. This filters logging only, not the actual child environment. Raw engine
stdout/stderr is not scrubbed and must remain private.

The initial rwc01/rwc02 runs predate allowlist hardening and retain the original
private full-environment metadata. Never publish or rewrite those originals.
Any published evidence must be a separately generated sanitized derivative with
explicitly documented omissions, not a claim that the original raw receipts
were redacted. Even new allowlisted receipts retain private absolute paths.
`summary.json` reports medians separately for each workload/stack and explicitly
lists excluded stacks. Exit status is zero only when every selected workload
has accepted samples from both stacks and unchanged final identities. Never
report a failed stack as fast based on its partial receipts.

## Retained root-owned rwc01/rwc02 observations

`bench/results/resident-broader-rwc01-20261030/` retains a separately sanitized
portable derivative of two native runs by the root session on October 30, 2026.
Each completed 82 invocation rows and 40 accepted executed-marker edits with no
excluded stack/workload. The second run used fresh private stores and reversed
workload order. Full source/tool/toolchain before/after identities agreed.

| Workload | First reference / native median ms | Repeat reference / native median ms |
| --- | --- | --- |
| `summary-test` | 109.276 / 82.207 | 112.474 / 81.560 |
| `shim-test` | 84.929 / 80.938 | 82.718 / 76.565 |
| `record-test` | 85.590 / 81.982 | 90.204 / 81.322 |
| `shim-function` | 63.599 / 62.812 | 78.593 / 75.478 |

These are descriptive observations of four pinned workloads only, not the
comprehensive session-weighted suite, a default-promotion gate, or evidence of
statistical significance. The root's native runs used historical harness
`0dba7c33e`, before receipt-only privacy/restoration hardening `fb9e3f14c`. The
evidence bundle keeps exact historical harness/helper bytes, all 164 rows,
328 raw streams, all input-source/backup hashes and complete pinned source
bytes, full recorded toolchain identities, both summaries, and the actual
root-owned 710-test regression log.

Never publish the original full-environment metadata. Each public derivative
records original SHA256 commitments and an explicit allowlist transformation
receipt with omitted-entry counts, not omitted secret keys/values. All process
row environments use the same allowlist. Originals remain unchanged/private.
See the bundle README for transformation limits and historic path identities.

Portable replay, with no native execution or original absolute paths required:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-broader-rwc01-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-broader-rwc01-20261030/test_validate.py -v
```

The semantic replay reconstructs edited input hashes and reparses every raw
libtest result, error/recovery oracle and executed marker, then recomputes
medians. Its tests coherently re-sign altered checksums and still reject
semantic corruption, and test relocation outside the repository. A checksum
match alone is not admission, and offline replay is not a fresh native run.

## Source-only validation

```sh
PYTHONPATH=bench python3 -m unittest bench/test_resident_workload_compare.py -v
```

The tests read fixture bytes only through bounded `git show` calls, create their
own temporary miniature source trees, and mock all engine invocations and
expensive toolchain verification. They cover exact anchors, ordinary-function
reachability in source, exact names/counts, selected workloads, positive sample
counts, matching bytes and rotation, error/recovery gates, timeouts, stale
markers, identity mutation, failure exclusion, raw receipts, and restoration
following interruption, explicit `source_restored` receipts, and API_TOKEN
omission from every metadata/process environment map while the mock child still
receives the unchanged variable. They do not build Rust, run resident workloads, or
modify the reference source directory. These source-only tests do not establish
native correctness or performance. Those require separate root-owned CLI runs
and inspection of their receipts, not extrapolation from the mock suite.
