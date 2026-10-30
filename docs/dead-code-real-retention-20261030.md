# Real dead-code item retention evidence, October 30, 2026

This is a native mechanism, count, correctness and storage diagnostic, **not a
performance comparison**. Other agents were active during these captures.
No elapsed-time delta, saved-time estimate, default promotion or global reuse
maximum is inferred. No compiler bootstrap or production source change was
performed in this lane. `-Zjr-cache-dead-code-items` remains default-off.

## Frozen same-compiler comparison

All evidence is under `bench/results/dead-code-real-retention-20261030/`.
Unlike a comparison against the older provider profile, both off and on use
exactly the newly frozen compiler and driver. Each mode has a separate
incremental target, justrust home and profile directory. Both modes compile
one shared private `git archive f97fcc2` source directory. There are four
captures per mode: prepare, forced unchanged, actual body edit, and restoration.

| Component | Identity (SHA-256 except source commit) |
| --- | --- |
| Source commit | `f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d` |
| rustc executable | `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144` |
| rustc driver | `3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e` |
| Pinned justrust client | `1dd6ee7da925924eaa0c25f9cbefafaabe1441c7b8f95a87b6cddfbaf20510a9` |
| Pinned Cargo | `994419eb0d12f1a4152a23d9030dc4d05c9bfd7a5c7af923f24bb3738afad95b` |

The source commit is a Git commit ID, not a SHA-256 digest. `identity.json`
records the source tree, frozen sysroot library hashes and analyzer/runner
hashes. Every per-capture receipt records the same binary identities, actual
source hash, selected local environment, profile path/hash, check/export exit
status and immediate query-cache storage snapshot. Decoder metadata also
retains the actual rustc invocation and verifies the experimental flag and
incremental stable-hash verification were in effect.

The private source and runners are in
`/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030/`.
The controlled local-only environment follows
`docs/real-associated-retention-20261030.md`: remote, localcache, depcache,
prebuilt and buildd are disabled, with the same pinned client and Cargo.
The command is `justrust-client check --lib`, with:

```text
-Zjr-empty-lint-level-fast-path=no
-Zjr-cache-small-query-values=no
-Zjr-cache-associated-items=no
-Zjr-cache-dead-code-items=no|yes
-Zself-profile=<mode-private>/profiles
-Zself-profile-events=default,function-args
-Zincremental-verify-ich
```

Profile, home and target paths necessarily differ by mode. The only enabled
compiler experiment is per-item dead-code retention in the on mode. The
older associated-item capture is protocol guidance, not the off baseline.

## Correctness, edit and restoration

All **eight real library checks and eight exports exited 0**. The full
unmodified recorded Cargo/compiler output, raw run metadata and summaries are
preserved as `*-raw-output.jsonl`, `*-raw-meta.json` and `*-raw-summary.json`.
`raw-diagnostics-index.json` records their hashes and run IDs. All eight runs
have zero recorded compiler warnings and errors. Full justrust stdout/stderr
are also retained. The reports and verbose commands differ by paths and
telemetry and are not represented as byte-identical diagnostic transcripts.
The separate 224-compilation native fixture differential gate remains the
coordinator's evidence, not a newly rerun gate here.

The forced-unchanged step touched `src/lib.rs` without changing its bytes.
The actual edit inserted the following at the start of `src/runs.rs::ago`:

```rust
std::hint::black_box(123u64);
```

Insertion and restoration used the edit tool on the private archived file.
The original and restored `src/runs.rs` SHA-256 is
`8272de3756d2323b05777c851b19c2cc6aec6bdf073af79056da29370daf6bde`.
The edited SHA-256 is
`380d2c460c21db44000355fda5628eec49a9411fb59b1a0af79de662950d6786`.
Both modes have the matching source hash at every step. After restoration,
all 63 archived tracked files match the Git archive by Git object identity
and SHA-256, recorded in `source-manifest.json`.

Four runs for each mode were exported to the isolated result locations:

```text
exported-runs/xps14-dead-code-off-20261030/2026-10.jsonl
exported-runs/xps14-dead-code-on-20261030/2026-10.jsonl
```

## Observed native item-query route

`count-comparison.json` resolves query labels and native marker argument names
within each invocation. Numeric event/query IDs are never joined across
invocations. Ordinary means `jr_dead_code_item_usage`. Allow/expect means
`jr_dead_code_item_usage_allow_expect`. A dash denotes **no matching named
record observed**, not proof of no work.

| Mode and capture | Item query | `Query` | `IncrementalLoadResult` | `jr_incremental_green_query` | `jr_incremental_provider_reexecuted_same_value` |
| --- | --- | ---: | ---: | ---: | ---: |
| off, all four captures | both | - | - | - | - |
| on, prepare | ordinary | 3,216 | - | - | - |
| on, prepare | allow/expect | 180 | - | - | - |
| on, forced unchanged | both | - | - | - | - |
| on, body edit | ordinary | 5 | 3,211 | 3,211 | 5 |
| on, body edit | allow/expect | - | 180 | 180 | - |
| on, restored | ordinary | 5 | 3,211 | 3,211 | 5 |
| on, restored | allow/expect | - | 180 | 180 | - |

The on prepare capture also has 3,216 ordinary and 180 allow/expect named
`IncrementalResultHashing` records. Body edit and restoration each have five
ordinary named hashing records. No selected-query
`jr_incremental_green_recompute` records, or other selected provider marker
kinds, were observed in any capture. The per-query marker dictionaries retain
the exact observed names and counts rather than filling absent families with
inferred work estimates. No marker argument decode failures were observed.

The actual body-edit and restoration profiles thus demonstrate the intended
native route: **3,391 named item-result disk-load records** per capture, with
five ordinary item providers reexecuted and producing the same stable result.
That is evidence that the per-item result cache is used on a real library
check after an actual body edit. It is not a statement that 3,391 scans were
counterfactually eliminated from flag-off, a CPU-work total, or a speedup.
The off path deliberately does not create item-query records and retains the
original direct visitor route. Its lack of these records cannot be treated
as zero original scanning work.

## Strict decoding failure and unknown labels

Every strict CLI analysis (`query_profile`, `reuse_ceiling`, `provider_cost`)
exited **2**: 24 expected failures across eight profiles. Independent strict
record replay in each profile raised
`ValueError: unmapped virtual string ID`. Original CLI stdout/stderr and full
strict replay tracebacks are preserved. All 24 explicit
`--allow-unmapped-event-labels --large-capture` analyses exited **0**.

| Capture | Mode | Unresolved records | Unresolved load labels | Unresolved Query labels |
| --- | --- | ---: | ---: | ---: |
| prepare | off | 1 | 0 | 0 |
| prepare | on | 1 | 0 | 0 |
| forced unchanged | off | 73,091 | 60,754 | 0 |
| forced unchanged | on | 76,487 | 64,150 | 0 |
| body edit | off | 11,633 | 3,454 | 0 |
| body edit | on | 14,679 | 4,981 | 0 |
| restored | off | 11,633 | 3,454 | 0 |
| restored | on | 14,679 | 4,981 | 0 |

In particular, forced unchanged has no named selected item-query/load/marker
records. Its unresolved load labels are **not assigned to item queries**, even
though the on/off unknown totals differ. This experiment does not establish
a selected-query route for that capture. Body edit and restoration have the
explicit selected names shown above, but unknown records elsewhere still
prevent complete whole-profile per-kind attribution. In-memory hit counters
are never added to loads or native marker counts.

## Immediate query-cache snapshots

The runner recorded the one `query-cache.bin` file immediately after each
check, including logical size, `st_blocks * 512` allocated size and SHA-256.
These are whole-cache snapshots, not exact payload sizes for the new queries,
RSS or total target size.

| Capture | Off logical bytes | On logical bytes | Off allocated bytes | On allocated bytes |
| --- | ---: | ---: | ---: | ---: |
| prepare | 13,523,426 | 13,880,783 | 13,524,992 | 13,881,344 |
| forced unchanged | 13,497,084 | 13,854,433 | 13,500,416 | 13,856,768 |
| body edit | 13,539,731 | 13,905,127 | 13,541,376 | 13,905,920 |
| restored | 13,539,289 | 13,904,755 | 13,541,376 | 13,905,920 |

## Validation and limits

`validation.json` verifies all eight checks/exports, binary identity equality,
actual compiler flags, source hashes/restoration, profile and raw-output
hashes, strict failure causes, all explicit-unknown analyzer exits, and the
exact selected observations above. Private runner snapshots are retained
under `runners/` for provenance. No compiler source, client source or default
was changed. No hosted build, comprehensive performance gate, or wall-time
causal claim is part of this mechanism proof.
