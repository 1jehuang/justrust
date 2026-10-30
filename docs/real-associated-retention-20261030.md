# Real associated-item retention evidence, 2026-10-30

This is a count, correctness and storage diagnostic, not a performance result.
The coordinator ran an independent scalar experiment concurrently. Wall-time
comparisons, time-saved estimates, default promotion and global reuse maxima are
out of scope. No compiler rebuild, hosted execution, worktree or existing source
change was used. The associated-item experiment remains default-off.

## Frozen experiment

Results are in `bench/results/real-associated-retention-20261030/`.
The baseline is the retained `bench/results/provider-cost-20261030/` capture,
redecoded read-only into this result directory. Every comparison uses source
`f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d`, exported with `git archive` into a
private source directory. The exact identities match the baseline:

| Component | SHA-256 |
| --- | --- |
| Frozen rustc executable | `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144` |
| Frozen rustc driver | `8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569` |
| Pinned justrust client | `1dd6ee7da925924eaa0c25f9cbefafaabe1441c7b8f95a87b6cddfbaf20510a9` |
| Pinned Cargo | `994419eb0d12f1a4152a23d9030dc4d05c9bfd7a5c7af923f24bb3738afad95b` |

`identity.json` records the source tree, frozen sysroot library hashes and
analyzer hashes. `source-manifest.json` records every archived tracked file's
Git object and SHA-256. After restoration all archived tracked files match
`f97fcc2`, checked with `git hash-object` rather than inferred from a single file.
Per-step receipts retain raw profile paths and hashes, binary identities, flags,
source hashes, compiler status, analyzer statuses and query-cache file hashes.

Private scratch is `/home/jeremy/.jcode/scratch/real-associated-retention-20261030/`.
The runner uses the baseline's rustc/client/Cargo paths, an isolated `home`,
`target` and `profiles` directory, and `justrust-client check --lib` with:

```text
-Zjr-empty-lint-level-fast-path=no
-Zjr-cache-small-query-values=no
-Zjr-cache-associated-items=yes
-Zself-profile=<private>/profiles
-Zself-profile-events=default,function-args
-Zincremental-verify-ich
```

Remote, localcache, depcache, prebuilt and buildd were disabled with the same
local-only environment as the retained baseline. The only experimental flag
change is associated-item retention. The profile destination is necessarily
private to this capture. Native acceptance and long real runs were launched
under `setsid` with durable logs and completion files in private scratch.

## Correctness and actual edit

The existing `bench/reuse-associated-accept.py --help` interface was inspected,
then its ordinary correctness matrix was run against the frozen rustc, without
rebuilding it. `native-acceptance.json` preserves all commands, full diagnostics,
program outputs and artifact hashes. All **34/34 cases passed**, including the
harness's expected compile-error checks. This is correctness coverage, not a
performance benchmark.

Real `check --lib` exits were **0** for prepare, forced unchanged, actual body
edit and source restoration. The forced unchanged step touched `src/lib.rs`
without modifying bytes. The edit inserted exactly:

```rust
std::hint::black_box(123u64);
```

at the start of `src/runs.rs::ago`, matching the baseline edit. Both insertion
and restoration used the edit tool on the private archived file.
`src/runs.rs` hashes match the baseline:

- Original/restored: `8272de3756d2323b05777c851b19c2cc6aec6bdf073af79056da29370daf6bde`.
- Edited: `380d2c460c21db44000355fda5628eec49a9411fb59b1a0af79de662950d6786`.

Four isolated justrust runs were exported to
`data/runs/xps14-real-associated-20261030/2026-10.jsonl`. Each export succeeded.
The export correctly redacts private scratch paths; full unabridged compiler
and reporting output remains in the step `.stdout` and `.stderr` files.

## Selected associated-item observations

`count-comparison.json` counts resolved `associated_items` labels and explicitly
named native marker arguments separately within each profile. Numeric query IDs
are never joined across invocations. The table contains observed records only.
A dash means no matching record was observed, not proof that no work happened.

| Capture | Retention | `Query` | `IncrementalLoadResult` | Native green-query | Native green-recompute |
| --- | --- | ---: | ---: | ---: | ---: |
| Prepare | off | 1,314 | - | - | - |
| Prepare | on | 1,314 | - | - | - |
| Unchanged | off | 641 | - | 641 | 641 |
| Unchanged | on | - | 641 | 641 | - |
| Body edit | off | 757 | - | 757 | 757 |
| Body edit | on | - | 757 | 757 | - |
| Restored | on | - | 757 | 757 | - |

The warm associated-enabled captures therefore exhibit named disk-load records
where the retained off captures exhibit named green-recompute/provider records.
This validates the selected native route on this real library check; it does
not establish a performance benefit or semantic avoidability for arbitrary work.
All Query records have resolved labels in these seven captures. The complete
whole-profile counts are retained, but their differences are not mechanically
attributed to the selected query or to individual keys. In-memory hit counter
records are not added to disk-load records or green-route markers.

## Strict failure and explicit unknown coverage

For all seven profiles, strict `query_profile`, `reuse_ceiling` and
`provider_cost` analyses exit **2**, not success. Direct strict decoder replay
raises `ValueError: unmapped virtual string ID`; full tracebacks and the original
CLI stderr are preserved. Each supported explicit
`--allow-unmapped-event-labels --large-capture` analysis succeeds and keeps
unresolved labels unknown rather than suppressing or inventing events.

| Capture | Retention | Unresolved records | Unresolved load labels | Unresolved Query labels |
| --- | --- | ---: | ---: | ---: |
| Prepare | off | 1 | 0 | 0 |
| Prepare | on | 1 | 0 | 0 |
| Unchanged | off | 73,091 | 60,754 | 0 |
| Unchanged | on | 74,574 | 62,237 | 0 |
| Body edit | off | 11,633 | 3,454 | 0 |
| Body edit | on | 12,190 | 4,011 | 0 |
| Restored | on | 12,190 | 4,011 | 0 |

Unknown load labels prevent a complete per-kind attribution of all disk loads.
The selected named records above are observations, not a claim of complete
cross-session reuse coverage. Provider-cost output retains elapsed scopes as
raw analysis evidence only. This report does not compare those elapsed scopes,
sum nested spans, estimate saved time or assert a global maximum.

## Query-cache storage

Immediately after each real check, before the next edit/check, the runner
recorded each `query-cache.bin` file's `st_size`, `st_blocks * 512` and SHA-256.
There was one such file per step in this isolated target. These are query-cache
bytes, not total target size, RSS or a serialized size attributed to one query.

| Associated-enabled step | Logical bytes | Allocated bytes |
| --- | ---: | ---: |
| Prepare | 13,754,779 | 13,758,464 |
| Unchanged | 13,728,430 | 13,729,792 |
| Body edit | 13,770,346 | 13,770,752 |
| Restored | 13,769,974 | 13,770,752 |

The retained off target, read-only after its body-edit run, contains
**13,539,731 logical / 13,541,376 allocated bytes**. Against that post-edit
snapshot, this associated-enabled body-edit cache is **230,615 logical /
229,376 allocated bytes larger**. This is a descriptive snapshot delta, not
an exact associated-item payload size or a matched storage-cost model.
Earlier baseline steps did not retain cache-size snapshots, so prepare and
unchanged off/on storage deltas are unavailable and are not reconstructed.
`baseline-post-body-edit-storage.json` preserves the exact retained file hash.

## Validation and limits

`validation.json` asserts all real checks/exports, 34 native cases, baseline
binary/source identity matches, exact selected observations, strict failure
causes and explicit-unknown analyzer success. Focused Python suites pass:
query-profile 29 tests, reuse-ceiling 27, provider-cost 8, associated-harness 3.
Their complete logs are retained. No production source, compiler, flag default
or existing baseline file was changed. The comprehensive performance gate was
not run, and no default promotion follows from these count/storage observations.
