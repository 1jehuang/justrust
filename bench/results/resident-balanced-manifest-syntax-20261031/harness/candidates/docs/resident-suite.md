# Native resident suite diagnostic

`bench/suite.py resident` adapts the existing `resident_engine_compare.py` runtime
harness. It is a **scoped, descriptive diagnostic**, not the synthetic session-weighted
workload, a comprehensive performance gate, or evidence for default promotion.
Only the coordinator should execute prospective native measurements. Unit tests
use mock processes and do not invoke a compiler or resident engine.

## Before, after, compare

Use the exact pristine 63-file source tree at `f97fcc2`, the frozen source-built
reference executable, and the complete already-installed pinned toolchain. Paths
must be absolute and without symlinks. Output roots must be fresh, disjoint from
the source, and at most 70 bytes. Keep the reference executable and toolchain
unchanged. Preserve both raw output directories for later comparison.

```sh
python3 bench/suite.py resident \
  --reference-engine /absolute/source-built-reference \
  --engine /absolute/before-jr-engine \
  --toolchain /absolute/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --source /absolute/source63 --output /short/resident-before

# Build the optimized candidate separately. Do not change the source63 workload.
python3 bench/suite.py resident \
  --reference-engine /absolute/source-built-reference \
  --engine /absolute/after-jr-engine \
  --toolchain /absolute/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --source /absolute/source63 --output /short/resident-after

python3 bench/suite.py compare \
  /short/resident-before/suite.json /short/resident-after/suite.json --floor 10
```

If needed, supply the **same** `--proc-macro-server /absolute/native-server` to
both invocations. Without it, the harness explicitly disables both resident
proc-macro-server variables and uses its rustc-built compatibility control.
The required sample count is exactly five, `--samples 5` is optional, and
`--timeout 360` sets the positive per-process timeout in seconds. There is no
seed option and no cache seeding. Cold failures remain failures, not a reason
to import warm state. Failed runs retain raw evidence but do not emit a valid
suite result.

## Evidence and fail-closed comparison

The `native_resident/v1` result stores both stacks' five samples and medians,
full source/toolchain/tool identities, host/CPU/execution identity, and SHA256
receipts for metadata, records, original source backup, summary, and every
stdout/stderr file. These private artifacts contain paths and should not be
published without reviewing them. Receipts detect changes, not malicious
fabrication or authenticity. Keep original artifacts at their recorded paths.

Comparison verifies the receipts and **replays raw logs**. Cached `accepted`,
`parsed`, `gate_passed`, and summary medians are not acceptance authorities.
Each stack must have its version query, preparation, real E0308 type failure,
successful type recovery, actual assertion failure, successful assertion
recovery, and exactly five real edits. Successful tests must report the exact
four names and counts, matching filtered counts. Each edit must print its
unique `JR_RUNTIME_EXECUTED_N` marker exactly once. Command and edited-source
hashes must match the recorded phase. Missing logs, process errors, timeouts,
nonpositive/nonfinite timings, stale commands/edits, and incomplete rows fail.

All 63 source hashes are recomputed from the repository's pinned Git blobs.
Source restoration, unchanged engine/reference/server identities within each
run, and unchanged complete toolchain file manifests are required. Before and
after must use the same full toolchain, source, profile, Cranelift backend,
proc-macro policy/server, source-built reference executable, and host/CPU/local
execution mode. Candidate executable hashes and candidate source identities
may differ, since those are the optimization under test. This harness records
executable hashes, not a cryptographic executable-to-source build attestation.
Mixed native and synthetic/named results are rejected before the legacy
runtime-context path. Existing named/synthetic behavior is unchanged.

## Interpretation

Output reports the engine median delta in seconds and percent, plus the
engine/reference median ratio for each run and its relative delta. The
`--floor` threshold is descriptive only. Noise remains **unknown** because
resident daemon CPU is not waited child CPU. Even a delta above the floor is
uncontrolled, with no significance, faster verdict, or speedup claim. Use the
comprehensive synthetic gate and other relevant correctness checks separately
before considering a default promotion.

## Combined same-byte manifest and lock retouch

The explicit `--mode manifest-retouch` mode measures a different workload from
the default unique-edit mode above. It runs the **same candidate executable** in
OFF/ON/ON/OFF order, with five measured calls per segment and a fresh private
state for each segment. It injects the exact
`RA_JIT_MANIFEST_CONTENT_GENERATION=0` or `=1` setting itself. Exporting that
variable around the default mode is not an equivalent experiment: the inherited
native environment is deliberately filtered.

```sh
python3 bench/suite.py resident --mode manifest-retouch \
  --reference-engine /absolute/source-built-reference \
  --engine /absolute/candidate-jr-engine \
  --toolchain /absolute/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --source /absolute/source63 --output /short/mr74

python3 bench/suite.py compare /short/mr74/off.json /short/mr74/on.json
```

This mode requires a fresh absolute output path of **at most 30 bytes**, rather
than the legacy mode's 70-byte limit, because its isolated segment homes add
native socket path components. Source and output must be disjoint. Keep the
complete output private until path and environment fields have been reviewed.

Before measurements, the reference and every candidate segment must pass the
original version, preparation, type-error/recovery and assertion-error/recovery
gates. A fixed, recorded overlay adds a runtime environment read and a nonce
print to the existing reachable `shares_overlapping_time` test. The Rust bytes
then remain unchanged throughout that segment's warmup and measured calls.
Each call receives a fresh nonce and must execute the exact original four-test
contract plus that nonce. The pristine 63-file pin is required before and after
the complete experiment, and the overlay has its own exact hash contract.

Each measured call rewrites both original root `Cargo.toml` and `Cargo.lock`
bytes before timing. Replay checks unchanged SHA-256 and size, increased mtime,
raw runtime output, selected executable identities and private state isolation.
Native status output is joined to kernel PID start ticks, executable hash and
worker socket arguments. ON must retain that worker identity through retouches,
while OFF must replace it. The opt-in entry marker alone proves neither reuse
nor successful content hashing. All five private states are stopped without
deleting their stores, including on failed measurements where possible.

Results use the distinct `native_resident_manifest_retouch/v1` schema.
`off.json` and `on.json` contain ten measurements per arm from the same complete
ABBA evidence. Comparison rejects mixed schemas, different runs and reversed
arms. It replays raw observations rather than trusting cached verdicts or
medians. Output is descriptive with **unknown noise**. Preserve and inspect all
samples, including first-retouch costs, rather than reporting only the median.
This combined workload does not independently attribute manifest versus lock
behavior, establish general agent latency, or authorize default promotion.

The new mode's Python-only controls are in
`bench/test_resident_manifest_retouch.py`. Mock tests do not substitute for the
actual native workflow above.

## Watched-manifest byte factor

`--mode watched-manifest-retouch` reuses the same pinned source, fixed reachable
runtime nonce, semantic gates, private states, byte/mtime checks and PID proofs.
It isolates `RA_JIT_WATCHED_MANIFEST_BYTES`: whole-generation content identity is
fixed ON in both candidate arms, while watcher bytes are OFF/ON/ON/OFF. The
reference has both flags OFF. Both candidate arms must retain the same worker
PID, start ticks, executable and socket across their measured retouches.

```sh
python3 bench/suite.py resident --mode watched-manifest-retouch \
  --reference-engine /absolute/reference/ra-jit \
  --engine /absolute/candidate/jr-engine --source /absolute/pristine/source63 \
  --toolchain /absolute/pinned/toolchain --output <home>/wm76b \
  --samples 5 --timeout 360
python3 bench/suite.py compare <home>/wm76b/off.json <home>/wm76b/on.json
```

Use a fresh short output path. The distinct
`native_resident_watched_manifest_retouch/v1` schema records both flags per state
and call and rejects cross-mode comparisons. The original manifest-retouch
schema still requires worker replacement in its OFF arm. A client entry marker
is not proof that the worker skipped a reload. Real native fixtures and changed
manifest, build-script environment and native-file recovery controls remain
separate acceptance requirements.

Preserve every first-touch sample and the original aborted runs. A completed
comparison is descriptive combined manifest+lock evidence with unknown noise,
not independent file attribution, a general agent speedup or default promotion.

## Parallel-manifest parser factor

`--mode parallel-manifest-retouch` uses the same runner and safeguards with
schema `native_resident_parallel_manifest_retouch/v1`. It is a
**noop/clientplanning factor** diagnostic, not separate manifest/lock attribution,
default promotion, or session-weighted performance evidence. The same candidate
ELF runs OFF/ON/ON/OFF, five measurements plus one warmup per segment. All twenty
measurements, including every first retouch, are retained. Whole-generation
`RA_JIT_MANIFEST_CONTENT_GENERATION=1` and watcher
`RA_JIT_WATCHED_MANIFEST_BYTES=1` are fixed in both candidate arms. Only
`RA_JIT_PARALLEL_MANIFEST_PARSE=0/1` changes. The reference has all three flags
explicitly zero. Flags are recorded per state and per call, with exact schema,
run and rule receipts. Each candidate segment must keep its worker PID, kernel
start ticks, executable and socket through both arms' retouches.

Every ON timed fresh-client call must prove an actual chosen parallel batch in
its raw stderr, not merely flag eligibility or a cached summary boolean. OFF
and reference/version calls must have no batch marker. The exact fixed line is
`ra-jit: parallel-manifest-parse-v81 batch` followed by a newline, once per
ON timed client. Other ON non-version calls may have zero or one marker.
Dynamic package/worker fields are not part of this contract.
The fixed reachable nonce overlay,
original source63 and TC4020 identity, original semantic failures/recoveries,
same-byte manifest+lock mtime checks, restoration and private stops still apply.
Replay rejects mixed modes, runs, rules, flags and identities. Native correctness
and source freeze must precede coordinator execution. No native runtime or
performance conclusion follows from the Python-only mock tests.

```sh
python3 bench/suite.py resident --mode parallel-manifest-retouch \
  --reference-engine /absolute/reference/ra-jit \
  --engine /absolute/candidate/jr-engine --source /absolute/pristine/source63 \
  --toolchain /absolute/pinned/toolchain --output <home>/pm81a \
  --samples 5 --timeout 360
python3 bench/suite.py compare <home>/pm81a/off.json <home>/pm81a/on.json
```

Output must be a fresh absolute path of at most 30 bytes. Comparisons remain
descriptive with unknown noise and no significance or general speedup verdict.
Mock controls are in `bench/test_resident_parallel_manifest.py`.

## Existing mode regression tests

```sh
PYTHONPATH=bench python3 -m unittest \
  bench/test_resident_suite.py bench/test_resident_engine_compare.py \
  bench/test_suite.py bench/test_suite_failures.py
```
