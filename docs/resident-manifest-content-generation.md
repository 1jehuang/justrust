# Resident manifest content generation, v74

## Status and scope

**Accepted only as a default-OFF, scoped same-byte manifest/lock retouch option,
2026-10-31. No general speed, default promotion or causal performance claim.**
The source-authoring worker performed no native builds or measurements. Root
subsequently executed the native correctness gates and pinned measurement below.
The original source-only freeze remains preserved separately from this final
acceptance document. This does not enable the option by default or install it
over the shipping justrust client.

This is the **licensed Blaze-derived experimental resident**, not the clean-room
compiler or an independently implemented compiler. Its Blaze origin is
`5d4f78f7577cf7170993b74f735099dd03f5c6ca` (`v0.20261027.0`). The retained
Apache-2.0/MIT licenses, attribution and import provenance remain applicable,
as documented in `toolchain/resident/JUSTRUST-SOURCE.md`.

The hypothesis is narrow: `whole::checkout` originally takes the maximum
`written_at` timestamp of workspace member manifests plus root `Cargo.lock`.
An identical-byte rewrite can change that discriminator and cause a cold
checkout unnecessarily. This experiment replaces only that discriminator when
`RA_JIT_MANIFEST_CONTENT_GENERATION=1` exactly. The decision is cached at the
first call to checkout generation in each client process. Other values, absence,
and non-Unicode values are OFF. Runtime environment changes after first use do
not change this decision.

OFF executes the original manifest iterator chained with the lock, followed by
`filter_map(written_at)`, `max()`, and `unwrap_or(0)`, without reading file bytes.
The call site no longer carries an intermediate `Option`, but its resulting
protocol `u128` is unchanged. Worker configuration, worker key, ABI/native
configuration guards, build scripts, `kept: false`, source collection, program
reload and fallback guards are not edited. Rejected v64 retention is not revived.

## Identity and protocol domains

The existing sha2 dependency hashes deterministic length-framed fields. Every
length is a little-endian u128. Domain/version, host OS/architecture, canonical
checkout root identity, file count, original native path label, file role,
presence state, canonical target path and full file bytes are framed separately.
There is no lossy path conversion. Native `OsStr::as_encoded_bytes` is scoped to
this host/toolchain, not a portable cross-platform cache format.

The file set is exactly the original workspace member manifests and root lock,
not dependencies or unrelated manifests. Entries are sorted by native path bytes
and role, with identical labels/roles deduplicated. Sorting deliberately does
not use `Path::Ord`, whose normalized components can alias distinct raw labels.
Canonical target paths do not replace original watched path labels: symlink or
`./` aliases remain distinct labels. A checkout-root change changes identity.

A known absent root lock is framed as `missing-lock`, distinct from a present
empty lock. Missing manifests, dangling symlinks, non-regular files,
canonicalization/read failures and observed read races return errors. The root
must canonicalize successfully. Conservative fallback is also used if the
missing lock's spelling is not the canonical root lock spelling.

Each file is read afresh twice with a canonical-path recheck, rather than trusting
mtime, size or a metadata memo. Same-path, same-size, same-mtime sequential
rewrites therefore change identity when bytes change. This is not an atomic
multi-file snapshot, and does not claim to solve concurrent edit races that the
original metadata/load path also lacks a transaction for.

The first 128 SHA-256 bits become a u128 with bit 127 forced to one, leaving a
127-bit probabilistic content discriminator. All original timestamps are
`Duration::as_nanos()`, bounded by u64 seconds plus subsecond nanoseconds and
therefore strictly below 2^94. Their bit 127 is always zero. Thus an original
fallback timestamp cannot equal any known content tag, including unusual future
filesystem timestamps. This is a structural domain separation, not an assumption
about today's epoch value. SHA truncation still has the usual finite-hash
collision risk and is not mathematical proof of byte equality.

On I/O uncertainty the helper returns to the original maximum timestamp
computation over the original file set. Unknown is never assigned a known
content tag or encoded as empty bytes/missing manifest. Transition from a known
content generation to fallback necessarily changes domains. Repeated unknown
states retain the original timestamp behavior and its limitations: an unchanged
maximum timestamp can compare equal to a previous unknown's fallback timestamp.
There is no deterministic fresh-on-every-error guarantee in this patch. Root
explicitly accepted preservation of this original repeated-unknown limitation,
not a stronger freshness promise. Such a stronger guarantee would require a
protocol cold/reject signal or an explicitly approved unique-generation mechanism.
Do not replace unknown with a constant content digest or silently claim guaranteed
cold loads for repeated unknown states.

## Marker semantics

At actual opt-in helper entry, once per process, an ignored-error stderr write
attempts exactly:

```text
ra-jit: manifest-content-generation-v1 entered
```

The marker precedes hashing and may accompany conservative fallback. It proves
entry only, not successful identity computation, retention, cache hits, same PID,
or ABI safety. OFF never enters this marker site.

## Source tests and actual root correctness gates

Five helper test sources operate on real temporary manifests/locks and cover:

- Identical-byte rewrite/touch with deliberately changed timestamp and stable tag.
- Changed bytes then restored bytes for both files, including same-size mutation
  with mtime restored through `FileTimes` and checked explicitly.
- Set reordering and duplicate elimination, added/removed paths, root identity,
  raw `./` path aliases and independently framed delimiter-like content.
- Present/missing/empty lock separation and errors for missing manifest,
  directory-as-manifest and unavailable root.
- Unix non-UTF-8 native paths versus replacement-character names, symlink aliases,
  dangling manifests and dangling locks.

These were source-only at the initial worker freeze. Root subsequently ran the
actual candidate's native codegen and resident unit suites, then exercised the
real whole-checkout API through native commands rather than a synthetic metadata
or CLI fixture. The retained root results are:

| Gate | Actual result | Scope |
| --- | --- | --- |
| Native codegen unit suite | 53 passed | Candidate compiler/codegen regressions |
| Native resident unit suite | 369 passed | Includes the new real-file helper sources |
| Changed-manifest diagnostic/recovery | 9 calls passed | Original control, candidate OFF and ON, each valid/error/recovery |
| Native build-script environment A/B/A | 3 calls passed | Runtime values `11 / 22 / 11` |
| Native C-file content A/B/A | 3 calls passed | Constant environment/link directives, runtime `11 / 33 / 11` |
| Strict seven-state replay | 28 calls passed | OFF/ON/ON/OFF, seven states per arm |
| Targeted same-byte replay | 30 calls passed | Original OFF control plus four six-call candidate cases |

The strict seven states were cold prepare, repeated no-op, manifest feature add,
unused `E0308`, type recovery, real assertion failure and assertion recovery.
The changed-manifest gate required the previously unused error to appear after
the manifest changed and successful runtime recovery after repair. The two native
A/B/A gates separately exercised build-script environment and native file-content
inputs. They are correctness observations, not evidence of generally faster
changed-content builds or same-PID reuse across changed manifests.

The original real-workflow requirements, retained as the contract, were:

1. Warm a program and record worker PID, runtime output and marker count. Touch
   each member manifest and root lock without changing bytes. ON must preserve
   the generation and demonstrate same worker PID plus correct runtime, not
   merely report the entry marker. Compare OFF baseline behavior and timings.
2. Actually change a valid manifest, introduce an unused `E0308` source error,
   and require the diagnostic to appear. Repair it and require successful runtime
   recovery. Do not require or infer same-PID reuse across content changes.
3. Exercise the native build-script environment and file inputs through A/B/A,
   requiring each expected runtime value and existing configuration guard
   behavior. Include changed bytes restored to A, not only byte-preserving touch.
4. Exercise missing/present lock and I/O uncertainty as feasible. Check known
   content and fallback tags never alias. Record the repeated-unknown limitation
   rather than describing timestamp fallback as fresh-on-every-error.
5. Check process-first-use opt-in behavior with separate processes. Ensure OFF
   has zero entry markers and ON has at most one attempted marker per client
   process, including an ON error fallback. No claim about stderr delivery if
   the output stream itself fails.

A content identity alone does not establish full toolchain ABI safety. Existing
configuration/native/script/worker-key/fallback guards remain the contract.

## Pinned combined-retouch measurement

Root completed the actual pinned native suite in task `682911b5mv`, with a total
launcher wall time of 368.74 seconds. Workload pin `f97fcc2` contains 63 original
files. Before the measured calls, the suite checked the unchanged original
reference and candidate semantic gates. It then used one fixed, replay-verified
overlay in the existing `shares_overlapping_time` test that reads a fresh nonce
with `std::env::var` at runtime and prints it from the reachable test body. The
overlay retains the original assertions and is unchanged between timed calls.
It is not a per-sample source edit or a compile-time `env!` substitution.

The exact candidate executable was identical in all four private segments:
SHA256 `9cb7bff510a19eedca12e3c38f7085d5c494aac4e71800ecdb05b02f2e01ca1b`.
The source-built Blaze reference control was
`fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263`.
The fixed existing toolchain was
`nightly-2026-10-27-x86_64-unknown-linux-gnu`, with Cranelift, optimization 0 and
the same recorded dev/test profiles. Both proc-macro server variables were
explicitly `0`, the shared compatibility control, not native proc-macro evidence.
Execution was host-local direct native on `xps14`, Intel Core Ultra X9 388H,
16 reported logical CPUs, x86_64 Linux `7.2.11-arch1-1`.

The mode was `bench/suite.py resident --mode manifest-retouch`, fixed
OFF/ON/ON/OFF order with five measured calls per segment, ten per arm. Each
segment had its own unseeded private native state and one overlay warmup excluded
from the measured samples.
Before every measured command, both original root `Cargo.toml` and `Cargo.lock`
were rewritten with identical bytes and verified increased mtimes. Only the
direct filtered-libtest command was timed. Identity hashing, rewrites, status
requests and `/proc` inspection were outside that timed interval. They can warm
OS caches, so these are not cold-page-cache measurements.

All 24 warmup/measured runtime nonces passed. Native status, `/proc` start ticks,
executable SHA and private worker socket argv corroborated ON same-worker
retention and OFF worker replacement for each retouch. Entry marker counts were
checked separately, never used as PID proof. All 54 primary calls, raw stdout and
stderr, file hash/size/mtime receipts and per-call source manifests are retained.
All five private-state stops exited 0 and the pristine source63 tree was restored.

Root ran:

```sh
python3 bench/suite.py compare /home/jeremy/mr74a/off.json /home/jeremy/mr74a/on.json
```

Exact compare output:

```text
Native resident combined same-byte manifest+lock retouch scoped diagnostic.
Same candidate ELF, OFF/ON/ON/OFF; 5 samples per segment, 10 per arm.
OFF median: 9.459865s -> ON median: 2.475840s, delta -6.984025s (-73.83%).
Absolute median delta exceeds requested 10% floor.
Descriptive/uncontrolled, noise unknown. No significance or general speedup verdict.
Combined retouch only, not separate manifest/lock attribution or default promotion.
```

Unrounded medians are OFF `9.45986455900129 s` and ON
`2.4758396840043133 s`. **The median does not remove the first-touch regression:**
the first measured ON retouch in each segment took `11.96186254301574 s` and
`12.040021672000876 s`, each slower than every measured OFF sample
(`9.233493571984582` to `10.609115069994004 s`). These two ON samples remain in
all ten-sample aggregates and raw evidence. The remaining eight ON calls ranged
from `2.3167891800112557` to `2.656919036002364 s`. The cause of the first-touch
cost has not been established. It must not be discarded or called warmup merely
because it is slower.

All measured samples, seconds rounded to six decimals:

| Retouch index | OFF segment 0 | ON segment 1 | ON segment 2 | OFF segment 3 |
| --- | ---: | ---: | ---: | ---: |
| 0, first measured touch | 10.082553 | **11.961863** | **12.040022** | 10.609115 |
| 1 | 9.408266 | 2.439534 | 2.499808 | 9.233494 |
| 2 | 9.294915 | 2.483280 | 2.386263 | 9.350303 |
| 3 | 9.686964 | 2.656919 | 2.316789 | 9.541468 |
| 4 | 9.511463 | 2.468400 | 2.434710 | 9.370000 |

This is a descriptive combined same-byte-retouch observation with unknown noise,
not a significance result, proof of causality or independent attribution to
manifest versus lock rewrites. The comprehensive session-weighted synthetic
workload was not the acceptance gate here. No general faster-than-Blaze claim,
shipping-client improvement, broad/default promotion or full-toolchain ABI safety
claim follows. The accepted state is solely the exact default-OFF option above,
with the first-touch cost and existing correctness boundaries retained.

## Frozen handoff

Private root:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/manifest-content-generation-v74`.
The original handoff contains complete original/candidate source, the original
source-only document, unified patch, SHA256SUMS, static source-check evidence,
exact-file rustfmt parse/check output, diff whitespace check and RELEASE. Its
document SHA256 remains
`2e90297acbb2a4d494f932d829468c01f8bfbcee6f073fb01c9977402d4435a2`.
Native source and Python harness sources stayed frozen during root measurement.

The Python harness freeze is separately retained under
`manifest-retouch-harness-v74/` in the same private parent. Its 52 Python-only
mock tests passed, including 13 new retouch tests, and root independently reran
those 13 tests. Synthetic Python test output is not native acceptance evidence.
Actual measurement artifacts are `/home/jeremy/mr74a/` (`off.json`, `on.json`,
`metadata.json`, `records.json` and raw logs), with the exact comparison log at
the private parent's `pinned-manifest-retouch-v74.log`. Correctness receipts are
`manifest-content-v74-targeted-validation.json`,
`manifest-content-v74-seven-validation.json`, `manifest-content-v74-cfg-nine/`,
the two `manifest-content-v74-native-*-aba/` directories and `candidate-unit-v74/`.
The pre-suite seven-state receipt's older "measurement pending" wording is a
historical status, superseded only by the separately retained completed suite.

This final acceptance document has a distinct source snapshot, SHA inventory and
RELEASE under `manifest-content-generation-v74-final-doc/`. It does not replace
or rewrite the prior source or harness freeze. Evidence packaging and the root's
full Python regression/atomic source-harness-doc commit are separate subsequent
steps, not claimed complete here. No commit was performed by this worker.
