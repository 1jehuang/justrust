# Resident engine source-built runtime comparison

`bench/resident_engine_compare.py` is a focused, private, actual filtered-libtest
comparison. It imports the existing `blaze_runtime_compare` edit, ordering,
libtest parsing, CPU observation, digest and bounded process helpers without
modifying them. It does not use the justrust recorder.

## Status and interpretation

The native engine is **derived from Blaze**, not an independently implemented
compiler. Passing this parity gate establishes only matching observable behavior
for these tests and edits. Parity alone is not an independent optimization, nor
proof of a speedup. Medians are descriptive, not comprehensive workload results.
Source-built reference and candidate must have matched build quality. Installed
Blaze is excluded by default because installed PGO/build-quality differences
would confound the initial comparison. Optional `--blaze` adds a separately
labeled descriptive installed reference, never substitutes for the source-built
reference and never changes `source_built_parity` acceptance.

Only mocked Python tests have been run for this harness. No native engine build,
version invocation, preparation or workload is part of that validation. The
source-built baseline is being built separately and the candidate is not ready.
There are no native runtime results or speed claims in this document.

## Inputs and exact CLI

Supply absolute real paths, not symlinks. Primary engines must be executable
native ELF files, not shell/Python frontends. `--reference-engine` is the unmodified
source-built `ra-jit` executable from origin
`5d4f78f7577cf7170993b74f735099dd03f5c6ca`. Its native `--version` is intentionally
still `blaze 0.20261027.0 (5d4f78f7577cf7170993b74f735099dd03f5c6ca)`.
The candidate must identify itself as `jr-engine` via its native `--version`.
Version strings do not prove build provenance: retain the separate source/build
receipts and complete binary hashes alongside these runtime reports.

The candidate's **native source entry** must implement `JR_ENGINE_HOME` and
direct `jr-engine test`, `build`, `check` and `run`. A wrapper that merely
translates argv or renames an installed Blaze binary does not satisfy this
contract. The harness exercises direct `test` only, while mocked command tests
cover construction for the other direct operations. Their real native behavior
requires separate acceptance, not inferred from the mocked tests.

The following is the exact invocation shape, with explicit placeholder absolute
artifact paths to replace once both builds exist. Do not run it while the
baseline build is competing for machine resources:

```bash
python3 bench/resident_engine_compare.py --reference-engine /absolute/source-built/ra-jit --engine /absolute/derived/jr-engine --source /absolute/private/f97fcc2 --output /home/jeremy/re01 --samples 5 --timeout 360
```

The currently planned baseline artifact is
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/baseline-build/target/release/ra-jit`.
The source argument must be a pristine private archive/copy of justrust `f97fcc2`,
not the engine source checkout and not the active justrust working directory.
Every tracked pinned blob is verified with Git blob hashing before any edit.
Extra files, links and special files are refused, except Git administration.
A fresh archive copy is necessary for each experiment because engine-generated
source-tree files would invalidate the next pin check.

Output must be new, disjoint from source and at most 70 encoded bytes long to
leave room for resident socket paths. The harness creates it mode 0700. Logs and
manifests include private absolute paths and must not be published unredacted.
Optional installed reference: append `--blaze /absolute/installed/blaze`.
Exactly five samples are required, not a configurable shortened success gate.

## Isolation and stores

Each stack has private `BLAZE_HOME`, `BLAZE_CACHE`, `RA_JIT_HOME`, `RA_JIT_CACHE`,
Cargo home, target and XDG directories beneath output. The derived stack also
gets private `JR_ENGINE_HOME`. Inherited engine, justrust, Cargo and Rust override
variables are removed. **HOME retains the caller's semantics** rather than
redirecting relative home lookups to a synthetic home directory. Native engine
code must honor its explicit state roots. No production Blaze toolchain/cache
symlinks are installed and no production store is copied or written by this
harness. Cargo registry state is private too. No compiler installation or
prepopulation is silently supplied to a fresh engine home.

The native entry currently derives effective Blaze roots under the existing,
mode-0700 `JR_ENGINE_HOME/resident-v1` namespace, overriding the launcher-supplied
`BLAZE_HOME`/`BLAZE_CACHE`. This is still private per-stack state. Environment
receipts describe launcher inputs, not introspection of native child overrides.
The candidate build must supply its validated `resident-v1-<64 lowercase hex>`
build identity. Native build-script propagation of private state roots is a
separate native correctness requirement, not proved by this mocked harness.

`BLAZE_BACKEND=cranelift`, `BLAZE_OPT=0` and both Cargo DEV/TEST profiles are
explicit: opt level 0, debug 0, incremental true. These settings match between
stacks. Cold means fresh private application stores, not cold OS page caches.
Untimed preparation warms successful stacks before the timed edit loop.

A fresh-store unsupported-static failure remains failed/incomplete with its raw
logs and private store retained. There is deliberately no `--seed-cache` option
in this first harness. It never blindly copies the old installed Blaze seed:
source-built origin/cache identity is different. A future explicit seeded mode
must use the existing safe `seed_cache`/`seed_manifest` helpers, validate a
manifest tied to the exact origin/tool identity, and label the entire experiment
**seeded warm**. It cannot convert a failed cold experiment into a cold success.

## Acceptance and timing

Reference command:

```text
/absolute/source-built/ra-jit cargo test --lib -- summary::tests:: --test-threads=1 --nocapture
```

Candidate command:

```text
/absolute/derived/jr-engine test --lib -- summary::tests:: --test-threads=1 --nocapture
```

The four required full names are:

- `summary::tests::parses_libtest_results`
- `summary::tests::shares_overlapping_time`
- `summary::tests::splits_passes_without_double_counting`
- `summary::tests::test_failures_are_not_compile_errors`

Before timing each stack must prepare and execute all four, reject an inserted
`JR_RUNTIME_TYPE_ERROR` sentinel with E0308 and exit 101, recover, execute an
inserted `JR_RUNTIME_FAILURE` assertion with exactly three passes and the one
selected failure and exit 101, then recover again. The source-built reference
establishes the filtered-out count. Every successful stack must match its entire
full test-name/state set and counts, not merely a summary or nonempty stdout.
A zero-test run can never pass.

Five identical source edits per stack rotate stack order. Each executes a changed
`black_box` value assertion and prints exactly one fresh
`JR_RUNTIME_EXECUTED_1` through `JR_RUNTIME_EXECUTED_5` marker. Stale, missing,
duplicated or malformed markers exclude that stack. Raw stdout/stderr files are
captured directly and parsed after timing. Successful libtest output is never
reconstructed from a recorder or a truncated UI rendering.

Timing reuses the blocking `wait()` plus timer-thread process-group SIGKILL
watchdog helper. Exit failure, signal, timeout, gate failure, missing sample,
source mutation or binary hash mutation cannot yield an accepted median. CPU
noise observations explicitly do not establish a quiet machine. There is no
headline ratio, and partial successful-stack medians are not complete parity.

The original edited source file is fsynced to `source.original` before mutations
and restored **from that backup in finally**, including exceptions. Metadata
contains full before/after SHA256 manifests of all input files except `.git`,
full executable hashes, native versions, environments and restoration status.
`records.json`, per-invocation `.stdout`/`.stderr`, `metadata.json` and
`summary.json` retain failures rather than deleting or reseeding them. A hard
process kill or power loss cannot run finally, so retain the backup for manual
recovery in that case.

## Mocked validation only

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s bench -p test_resident_engine_compare.py -v
```

The tests mock every native invocation, verify direct argv, environment
isolation/HOME semantics, pin checking, four-test gates, expected-failure gates,
rotation, fresh markers, cold failure retention, identity mutation and source
restoration on exceptions. They do not establish native correctness, actual
Cranelift compatibility, residency or performance.
