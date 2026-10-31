# Rejected v81 native parallel manifest syntax experiment

Status on October 31, 2026: **REJECTED, not promoted**. The candidate and harness
were restored after their preserved freezes. This document is historical evidence,
not instructions to enable a feature in the accepted client. The separate v82
experiment is outside this archive.

## Hypothesis and implementation boundary

The accepted base was `c8829bae02fd234f46012ce6996baa14c09b3a1c`. The candidate
split only owned file reading, UTF-8 conversion and native `TomlManifest` syntax
parsing from the existing semantic normalization body. It attempted bounded
parallel syntax parsing for fetched regular registry manifests, with at least
eight eligible inputs and two to four scoped threads. The exact first-use flag
was `RA_JIT_PARALLEL_MANIFEST_PARSE=1`, default OFF. The unchanged OFF fetch tail
and unchanged common semantic body are preserved and checked in the archive.

Context, configuration, workspace roots, diagnostics, warnings, semantic
normalization and ordered result/error consumption remained sequential. No
`Context` Send/Sync assertion, unsafe trait assertion, persistent byte cache or
invalidation shortcut was introduced. Fresh per-invocation reads were required.
The candidate included a Linux x86_64 nonblocking/no-follow opening guard and
safe sequential fallback for unsupported or ineligible inputs. That is not a
claim of immunity to filesystem races. The native compiler, rather than a manual
assertion, established that the owned syntax result could cross scoped threads.

The fixed stderr marker was `ra-jit: parallel-manifest-parse-v81 batch`, followed
by a newline, at most once per process after an actual bounded batch was chosen.
It was not a cache-hit or batch-completion proof. Marker write failures were
ignored. Native fixtures used explicit private boolean controls, not mutation of
cached process environment across parallel tests.

## Actual gates and measurements

The root ran the native jobs before restoration. The preserved full recorder
journals contain 28 manifest, 6 metadata, 53 codegen and 373 full-native named tests,
all passing. The native source inventory actually contains **3,759** files, not
an assumed count. The toolchain identity inventory contains 4,020 entries, and
runtime gates used a pinned 63-file source workload. The archive includes build
and check receipts, full journals, identities, eight profile calls, four strict
seven-phase states, and the actual 54-call pinned ABBA run.

The strict states were OFF, ON, ON-repeat and OFF-repeat. All 28 phases retained
the original exact semantic oracle, diagnostics, failing assertion and recovery
checks. The ABBA order was OFF/ON/ON/OFF, with five timed combined manifest/lock
retouches per segment. All first touches were retained: ten samples per arm,
20 total. Four private candidate workers were checked by PID, process start tick,
executable hash, full argument vector and socket identity. Generation and watcher
controls stayed ON for the candidate arms and all three controls were OFF for
the reference gates. Retouches preserved file bytes and size while advancing
mtime. No store deletion was part of cleanup.

| Comparison | Recorded value |
| --- | ---: |
| OFF median | 0.05775033299869392 s |
| ON median | 0.052218245487893 s |
| Observed median reduction | 9.579317076710248% |
| Required promotion floor | strictly greater than 10% |
| Decision | REJECT |
| Noise classification | unknown |

The original comparison formats the difference as **-9.58%**, which does **not**
exceed the requested 10% floor. This is a descriptive uncontrolled comparison,
not significance evidence, a causal attribution, a general speed claim or a
default-promotion result. The combined retouch workload does not separately
attribute manifest and lock effects. The eight instrumented profile calls are
attribution diagnostics, not a substitute promotion gate.

## Preserved evidence and offline replay

See `bench/results/resident-parallel-manifest-syntax-20261031/README.md` for the
portable bundle and commands. It retains both original/candidate freezes and
patches, source and toolchain identities, allowlisted state projections, full
native output journals, runtime raw stdout/stderr/status receipts, selected trace
spans, all ABBA calls and samples, worker controls and stop receipts. Private
absolute prefixes are mapped to public role tokens. Ambient environment objects
and native executable payloads are not exported. Original private inputs were
hash-checked before and after read-only export.

The standalone Python replay executes only explicitly selected pure definitions
from hash-pinned original oracle source, not archived drivers. It joins unified
patches to original/candidate bytes, reconstructs fixture edits, checks original
strict semantics, recomputes medians, and enforces the rejection. Counterexamples
coherently reseal the outer inventory so semantic failures cannot be explained
by stale outer hashes. A relocated replay runs with empty HOME/PATH and audit
denials for external reads/listing, writes, subprocesses, sockets and ctypes.
This replay checks preserved evidence, not a new native build or execution.

The two production source files were restored from exact frozen originals and
the two newly added native fixture modules were removed only after preservation.
Their original/candidate bytes remain in this bundle. Subsequent owners may
change live native files, so this historical restoration is not a claim about
the current checkout's native contents.
