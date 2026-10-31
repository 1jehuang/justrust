# Native readiness notification v65: rejected experiment

**Evidence only. Candidate rejected, daemon restored, no promotion or speed claim.**
Date: October 31, 2026. This is the licensed Blaze-derived resident engine, not the clean-room compiler.

## Decision and exact source restoration

The candidate implemented default-OFF exact `RA_JIT_READY_NOTIFY=1`, process-latched at daemon startup. Each worker owned a Condvar using the existing checkout mutex and exact key/generation Loading predicate. Publication released the mutex before notify-all. Waits retained the existing 20 ms bound so worker replacement, failure and normal sync/load semantics were rechecked. Spurious wakes used the standard library predicate loop, not busy polling. No client/worker request protocol changed.

The production source was restored from the full immutable original after preservation of the exact live daemon/doc. Both worker and coordinator independently verified scoped HEAD diff and the worker verified byte comparison. Restored daemon SHA-256: `d488bffe1498847e8e234e6513474c26b38b19939b94915feacaa60decbe0b5c`. Candidate: `cb243a2d02abb2509db6203800efa537d048649ba784911f59c5679485026df7`. The native source is no longer owned by this evidence worker.

`frozen/` retains full original/candidate daemon bytes, candidate historical doc, addition/deletion patch, original doc absence marker and historical FROZEN release. The portable validator replays the exact patch against the original in memory, including the new document addition. No native tool is required. Frozen docs/marker describe the pre-gate source release and are historical, not current acceptance. Human-local scratch paths in those exact historical documents are provenance only, not private ambient environments. The neighboring current explanation is `docs/resident-ready-notify.md`.

## Actual gates and strict runtime observations

Root-owned check/build passed. Native gates passed **53 codegen tests and 368 resident tests**, including these exact actual test names:

- `cargo::daemon::tests::ready_notify_ready_before_wait_and_terminal_errors`
- `cargo::daemon::tests::ready_notify_wrong_key_generation_and_loading_timeout`
- `cargo::daemon::tests::ready_notify_during_wait_uses_the_predicate_mutex`
- `cargo::daemon::tests::ready_notify_generation_transition_during_wait`

The original four fixture result lines are selected from the real justrust output journal, not inferred from test source. Gate source verification covered **3,757 files** and root lock identity. Root's post-run benchmark regression suite passed **733 tests**, which is correctness coverage, not a performance benchmark.

Four sequential same-binary cases ran in order **OFF, ON, ON-repeat, OFF-repeat**. Each has exactly seven accepted cold/noop/manifest-feature/type-error/recovery/assertion-failure/recovery states, 28 total. Binary SHA-256 is `5a0c64792bf11ef68e4ed38e218f2522f6527bd069f70480d543073a11106b27`. A distinct accepted v58 baseline has seven passing rows and is kept separately, not conflated with those pairs.

Every successful native invocation must report these exact four tests, with fresh invocation nonce, status 0 and counters `4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out`. Assertion failure must be the second named test alone, status 101, counters 3/1/0/0/144 and fresh assertion literal. Type error must report status 101, fresh E0308 literal and no completed test summary or runtime nonce. The validator checks raw-selected output as well as structured parsed values, never only `accepted=true`.

Both ON runs observed cumulative marker counts `[1,1,1,1,1,1,1]`, both OFF runs `[0,0,0,0,0,0,0]`. These are snapshots of one once-only marker per daemon process, not seven separate transition events. The marker is `ra-jit ready-notify: observed loading transition`. It was produced only for a matching Loading-to-terminal non-timeout wait, not flag construction or ready-before-wait. Stderr recording was ignored-error `writeln!` on locked stderr, so diagnostic failure could not fail a build. This is positive exercised-path evidence, not proof that a specific notify caused a wake or a useful speedup.

## Rejection and descriptive numbers

Percentage change is `(ON / OFF - 1) * 100`, positive means slower in that observation:

| Phase | First OFF then ON | Reverse-order pair, ON-repeat versus OFF-repeat |
|---|---:|---:|
| cold prepare | +18.924% | +19.901% |
| manifest feature add | +25.320% | -9.543% |
| type recovery | +15.642% | -14.916% |

All seven phase seconds and arithmetic are retained in `observations.json`. Noise classification remained **unknown**. The runs were sequential, not randomized or controlled causal estimates. There was no useful replicated win. The candidate was rejected rather than promoted. These observations do **not** establish a causal regression either. No comprehensive session-weighted performance gate, generic speedup, production acceptance or general performance-goal completion is claimed.

## Failed analyses remain distinct

`analyses/` retains separate v65 and v65b scripts and their actual empty output JSONs. They failed in Python analysis due to JSON list versus parser tuple representation. v65c first verified normalized equality against freshly parsed raw output, then supplied fresh parser tuples to the unchanged strict oracle. Its corrected successful JSON output is separate. No native output, oracle or test count was changed. Historical failure stderr was not retained and is not fabricated or reconstructed by rerunning the failures. Archived analysis scripts have only fixed local-root substitutions and are `.py.txt`, not portable commands to execute.

## Privacy and provenance

`export_private.py` reads only the enumerated private files and publishes fixed selected fields. Environment allowlist is exact and value-constrained: `BLAZE_BACKEND`, `BLAZE_OPT`, and the DEBUG/INCREMENTAL/OPT_LEVEL keys for CARGO_PROFILE_DEV and CARGO_PROFILE_TEST. There are eight allowed environment entries and twelve omitted entries per runtime row. The five startup controls are separately explicit: RA_JIT_DEMAND_STD, RA_JIT_CACHE_MISS_STATS, RA_JIT_STATS, RA_JIT_WHOLE_RELOAD and RA_JIT_READY_NOTIFY. No permissive prefix allowlist is used.

Raw state environments, ambient variables, private home/cache locations, toolchain path lists, credentials, telemetry, worker logs, binaries and targets are excluded. Test stdout is retained, stderr retains only exact full-line E0308/type/assertion patterns, and daemon logs retain only the fixed marker line. Raw-original SHA-256 and byte counts are retained for custodian comparison. Full private originals remain untouched, with read-before/read-after hash checks during export. Selected observations cannot reconstruct the excluded private originals.

Cleanup records concern only **post-validation operational stopping** of the exact completed baseline and four candidate namespaces, all successful with `stores_deleted=false`. They do not attest original historical exit timing or machine quietness. No cleanup is executed by this package.

## Portable offline validation

Python standard library only, from any working directory:

```sh
python3 -I -B /path/to/package/validate.py --manifest
python3 -B /path/to/package/test_offline.py
```

The checks never invoke Cargo, rustc, justrust, the resident engine, native tests or network calls. Tests use only Python subprocesses for relocation, with empty PATH and an unrelated HOME. Set `JCODE_SCRATCH_DIR` to an existing writable scratch directory for test copies. The controls cover 37 semantic mutations, source-context mutation, privacy, and a relocated package whose historical observation is changed and outer manifest recomputed.

`SHA256.json` covers every package file other than itself. A fixed observations commitment and fixed source/patch digests in the validator bind historical content even if that mutable inventory is rehashed. These are repository integrity commitments, not third-party cryptographic attestation. Exact historical sources and patch are retained, while selected observations are intentionally reduced. The current neighboring doc is outside the relocation unit. Offline evidence validation is not a new native correctness or performance result.
