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
  identities, and toolchain links must remain correct. Identity failure clears
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
containing absolute paths and full environments, not public benchmark exports.
`summary.json` reports medians separately for each workload/stack and explicitly
lists excluded stacks. Exit status is zero only when every selected workload
has accepted samples from both stacks and unchanged final identities. Never
report a failed stack as fast based on its partial receipts.

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
following interruption. They do not build Rust, run resident workloads, or
modify the reference source directory. Native correctness/performance remains
unmeasured until the root executes the CLI and inspects its receipts.
