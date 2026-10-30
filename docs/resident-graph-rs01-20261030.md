# Prospective resident graph rs01: offline evidence

This retains the completed October 30, 2026 prospective native suites `rsb01`
(before, accepted v2 engine) and `rsa01` (after, accepted v4 engine). This is an
**evidence-only scoped diagnostic**, not a new timing run or a promotion gate.
Both engines are **Blaze-derived**, origin
`5d4f78f7577cf7170993b74f735099dd03f5c6ca`, not clean-room compilers.

| Stack, five accepted runtime edits | Before median seconds | After median seconds |
| --- | ---: | ---: |
| Derived native engine | 0.119944310 | 0.109617051 |
| Source-built pristine Blaze reference | 0.124489961 | 0.110923157 |

Both stacks improved in these observations. The engine's raw median change is
-8.61%, while its engine/reference ratio changes from 0.963486 to 0.988225, a
relative **+2.57%**. Noise is **unknown**. Neither the raw change nor either
single comparison establishes an attributable graph-reuse speedup, statistical
significance, robust faster-than-Blaze performance, or independent optimization.
The candidate combines graph reuse and telemetry changes, further limiting
attribution. No default promotion or general workload claim is warranted.

## Portable offline replay

Copy all of `bench/results/resident-graph-rs01-20261030/` to another location.
With Python 3.9 or newer on Linux or macOS, from any working directory:

```sh
python3 /path/to/resident-graph-rs01-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 /path/to/resident-graph-rs01-20261030/test_validate.py -v
```

No checkout, Git, original home/scratch directory, compiler, native engine,
network, caches, or toolchain installation is required. The acceptance test
executes the **copied** validator with a detached working directory, a temporary
HOME, and PATH pointing to a nonexistent tools directory. Only Python offline
checks were executed by the retention lane, never native builds or workloads.

The replay uses exact retained `resident_suite.py` **replay and compare** function
bodies, plus its small original helpers. Only the native validator's original
absolute artifact-root/Git lookup is replaced by a resolver of these bundled
receipt roots and the bundled pinned source. The original suite JSON files are
not rewritten. The audit extracts explicitly named pure/read-only functions
without importing workload harness entrypoints, resource measurements, host
inspection, or subprocess runners. Its emitted `native_comparison` is the
original native comparison at a 10% descriptive floor, including its explicit
no-significance/no-speedup disclaimer.

Validation covers all 44 runtime records, 20 accepted actual edit markers,
exact test names and libtest counts, real type/assertion failures and both
recoveries in **both stacks in both sessions**, reconstructed edit hashes,
source restoration, private state roots, no-seed policy/commands, source/tool/
toolchain identities, all native suite artifact and measurement receipts,
paired host/mode/input comparability, compiler build receipts, and actual unit
names/counts read from original full justrust output, not wrapper pass labels.
Mutation tests refresh affected native artifact hashes before checking semantic
rejection, so their failures do not merely demonstrate checksum mismatch.

## Immutable identities

| Role | Native binary SHA256 | Embedded build ID |
| --- | --- | --- |
| Reference, both sessions | `fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263` | Blaze origin above |
| Before v2 | `726edc785271a1907e5d40619fde5239da031f535fff035b62c6650a8b1aafef` | `resident-v1-bc9689b69a6f40e47b93a11f679daa197eb5c1fbc10b9898a310e9cac3efdfc4` |
| After v4 | `deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8` | `resident-v1-c44b9049b4557e5ff69a31664fe9a01e9ed0eb45643aa2fecc81ecb4df2fda18` |

The workload is the complete **63-file** pristine `f97fcc2` snapshot. Every file
is retained, checked against before/after SHA256 maps and original Git blob IDs,
and its edited source backup is retained separately for each session.
Toolchain metadata includes before/after hashes of the matching
`nightly-2026-10-27-x86_64-unknown-linux-gnu` installation. Only the small pinned
Rust channel manifest, file-hash inventories in metadata, and compiler version/
hash receipts are retained, **not** the toolchain tree.

Baseline-v2 compiler source provenance is separate from candidate-v4 provenance.
`provenance/baseline-v2-candidate-source-manifest.json` and its commit receipt
identify original `20f9102027d553bc72c003927964b4ca72bc6574`, not later graph code.
The v4 hash-only manifest was captured at 15:08:49 UTC while all **seven** original
v4 pre-build source hashes still matched. Those hashes also have original
post-build and post-unit `OK` receipts. The graph source was subsequently
committed as `c4a20a5f6`, but that graph commit alone does **not** identify the
complete v4 tree, which also contained telemetry changes. Retention HEAD and
working-diff **statistics**, not compiler source, are included to make that
boundary explicit. Later experimental source edits are not recaptured.

Other full compiler-manifest entries are retention-time hash receipts, not
invented historical pre-build attestations. Embedded build IDs are tied to
original native versions and recorded binary hashes. Without compiler bytes,
offline replay cannot independently recompile a native binary or reconstruct its
complete source-key hash. This bundle does not claim otherwise.

## Rejected v3 unit compilation, corrected v4 fixtures

The v3 production build succeeded, but the v3 **unit compilation failed** with
exit **101**. Its full original logs and rejection exits are retained as
rejection evidence, never treated as accepted tests:

- `E0603`: a telemetry test accessed private `cli::worker_named` from its parent
  module. The worker/build-identity assertions were moved into `cli`'s own test
  module, retaining their negative expectations rather than exposing or removing
  the checks.
- `E0308`: the graph fixture passed an owned `String` to a setter requiring
  `&str`. The corrected fixture borrows `&source`.

The corrections are fixture compilation fixes, not permission to weaken the
correctness oracle. The graph fixture still compares tracked declarations
against the uncached full traversal over eight revisions, including body-only,
nested-item, macro-definition, provided-method, static-item, removal, and
restoration changes. The full graph equality and provided-item expectations
remain. The retained v4 hashes identify the corrected implementation inspected
during retention. The offline audit verifies original successful test execution,
not a new execution of this compiler's correctness oracle.

Actual v4 unit pass receipts cover:

- `tests::resident_graph_reuse_tracks_nested_items_and_macro_edits`: 1 passed,
  345 filtered out.
- The existing complete `the_roots_of_a_listed_crate...` test: 1 passed,
  345 filtered out.
- Exact telemetry tests `both_telemetry_spellings_are_client_only`,
  `disabled_phase_needs_no_clock_or_span`, and
  `telemetry_does_not_select_a_build_or_worker`: 3 passed, 343 filtered out.

## Allowlist, licensing, and limitations

- `rsb01/`, `rsa01/`: complete original top-level raw stdout/stderr, metadata,
  records, summary, suite, and source backup. No state directories copied.
- `source/`: all 63 pristine workload source files, including its original
  licenses/notices. Historical AGENTS files are audit inputs, not active policy.
- `build/`: named baseline/v2 and v3/v4 build receipts, v3 rejection receipts,
  v4 unit logs/exits, exact `build-test-candidate-v4.sh`, original v3 scripts,
  and baseline suite runner/log/exit. Full private justrust runs are restricted
  to the **exact run IDs** identified by those logs and only
  `meta.json`, `output.jsonl`, and `summary.json`. This retains actual stdout/
  stderr, commands and verdicts without account files, telemetry delivery,
  full process-environment dumps, artifacts, target output, or caches. Original
  run metadata retains its small existing allowlisted session/target controls.
- `harness/`: exact native suite implementation and runtime harness/helpers,
  plus the original suite CLI source as provenance. These are not executed as
  workload runners during audit. `collect.py` is a one-time allowlist collector,
  not a replay prerequisite.
- `provenance/`: compiler **hash manifests only**, original source pin receipts,
  toolchain manifest, and upstream Apache-2.0/MIT license texts. No compiler
  source copies or binaries are included.
- `SHA256.json` is the complete byte-integrity allowlist. It is not a signature
  protecting against replacement of the entire bundle.

Original private paths are retained where present, with no credential-bearing
files selected. The raw/metadata/script/provenance receipt allowlist was scanned
for credential patterns. Only existing local build controls and session IDs
were observed, not authentication secrets. No later trace attempt or speculative
prebuild is attributed to these prospective suite records. Blaze derivation is
explicit throughout. Retained license texts and the workload's original notices
remain authoritative for their respective materials.
