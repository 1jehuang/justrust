# Native home query v66/v67: rejected experiment

## Decision

**Rejected and restored on October 31, 2026.** V67 passed root's native correctness
and scoped runtime gates, but two same-binary OFF/ON pairs did not show a
repeatable useful latency improvement. Noise contamination remains **unknown**.
There is no speedup claim, causal regression claim, workload cache-hit claim,
general performance claim, or default promotion.

The evidence-only bundle is `bench/results/resident-home-query-20261031/`.
Its standalone Python replay verifies selected historical artifacts without
building Rust, invoking a compiler, running native fixtures, or contacting a
service. The replay is not another native acceptance run.

## Candidate history

V66 added an experimental, default-off `RA_JIT_HOME_MEMO=1` wrapper around the
original `Mentions::of` ownership visitor. It attempted a native Salsa query
keyed by the complete cloned `Instance`, preserving `Option` opaque fallback
and the original `FxHashSet` iteration order as a retained vector. `Homes::home`
borrowed that ordered vector. Empty stored arguments retained the direct path.
The new flag was latched at process first use, not forcibly at worker startup.
Existing dynamic `RA_JIT_NO_HOME` and caller-bounds behavior stayed unchanged.

Root's actual v66 gate `810298m6y1` failed with exit 101. This vendored Salsa
macro treats a single non-database argument as an existing Salsa struct, but
plain `Instance` does not implement `SalsaStructInDb`, `AsId`, or `FromIdWithDb`.
The imported `SliceLike::iter` also selected a copied slice iterator where the
wrapper required borrowed items. Earlier formatting and source checks did not
establish native correctness. The failed source, compiler-error derivatives,
original-log hashes, and gate inventory remain in the bundle.

V67 was a distinct frozen repair, not a relabeling of the failed build. It used
`native_home_crates(db, instance.clone(), ())` to select Salsa's supported
native multiargument interner. `(Instance, ())` preserves the full instance's
equality/hash domain without unsafe/manual identity traits or a mutable cache.
Explicit `<[Crate]>::iter` resolved both memo-slice branches. The actual visitor
and strict fixture sources remained bytewise unchanged. The wrapper never
cached hidden/revealed body/layout results or weakened unknown-owner behavior.

Original implementation baseline:
`76994fa48fedc9057de6e8086c18e6c2ef1d8522`.
V67 pre-edit HEAD:
`a7f5848e389f1224dfd54d5297d5e1074ebd3ddb`.
Actual v67 executable SHA-256:
`c3fa3dad9de29af5884061770dd612b706023509e3733267fcdd6df53b932b70`.
Full baseline, failed-v66, repaired-v67, and live-before-restoration source
snapshots and exact-context repair patches are retained as evidence only.

## Actual correctness evidence

Root's v67 native check and build passed, followed by **58 codegen tests** and
**364 resident tests**. The codegen run includes these five actual named tests:

- `home::tests::native_home_exact_opt_in`
- `home::tests::native_home_real_db_cold_repeat_full_key_and_empty_bypass`
- `home::tests::native_home_real_db_opaque_fallback`
- `home::tests::native_home_real_db_signature_and_dependency_revisions`
- `home::tests::native_home_real_db_closure_owner_revision`

The real `RootDatabase` fixtures discovered actual function IDs and used
resolved signature/inferred closure types. Their independent oracle invoked
the original visitor directly. Test-only TLS counters asserted actual native
provider cold-one/repeated-zero behavior, distinct full-instance keys, two
tracked callers, dependency re-export and signature changes, a closure-owner
move with equal ownership output, unchanged-body reuse, and opaque `None`.
These passing fixture assertions demonstrate native memoization capability,
not that the measured workload avoided providers or benefited from the cache.

All **28 scoped runtime rows** passed strict fresh-invocation, named-result,
status/count, type-error, assertion-failure, and recovery oracles. The bundle
retains selected original stdout, exact diagnostic/challenge lines, parsed
receipts, and original raw-file SHA/size provenance. It also preserves the
63-file workload inventory, 4,020-file toolchain inventory, and 3,757-file
native source inventory, plus source/lock unchanged checks. Root added only
already-resolved `ide-db` and `test-fixture` lock dependency edges for fixtures,
with no new packages. Source workers never edited the lockfile.

## Descriptive scoped timings, not a speed claim

Percent change is `100 * (ON / OFF - 1)`. Positive means ON took longer.
Both comparisons use the same v67 executable with separate OFF/ON processes.

| Phase | First OFF, s | First ON, s | First change | Repeat OFF, s | Repeat ON, s | Repeat change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Cold prepare | 26.579 | 28.335 | +6.61% | 30.054 | 27.513 | -8.46% |
| Repeated noop | 0.0582 | 0.0560 | -3.81% | 0.0569 | 0.0620 | +8.93% |
| Manifest feature add | 13.101 | 13.904 | +6.13% | 13.766 | 13.142 | -4.54% |
| Type error | 13.655 | 14.246 | +4.33% | 14.282 | 14.418 | +0.95% |
| Type recovery | 12.173 | 13.491 | +10.82% | 12.998 | 12.726 | -2.09% |
| Assertion failure | 3.093 | 3.046 | -1.53% | 2.875 | 3.070 | +6.78% |
| Assertion recovery | 0.623 | 0.681 | +9.33% | 0.718 | 0.550 | -23.41% |

The mixed direction is not a repeatable useful scoped win. Unknown noise
prevents causal attribution. These are focused diagnostics, not the
session-weighted synthetic comprehensive gate or stock-versus-justrust evidence.

Every OFF worker-log snapshot had zero activation markers. Every ON snapshot
had one cumulative line:

```text
ra_jit: native-home-query-v67 QUERY ENTERED (not workload cache-hit evidence)
```

The line is written only once per process on an actual opted-in provider entry.
Later snapshots retain the same earlier line. **Seven snapshots containing one
line do not mean seven executions, seven hits, or per-phase provider avoidance.**
No actual workload reuse attribution was established and none is inferred.

## Restoration and evidence ownership

Before restoration, the exact live `home.rs`, `Cargo.toml`, and proposal document
were preserved and compared with frozen v67. At **11:45:56 UTC on October 31,
2026**, only the two source/manifest files were restored via writes from the
original v66 baseline backups. Bytewise original comparisons and scoped
`git diff --exit-code HEAD` both passed, then source ownership was released
immediately. Root restored its lockfile separately. Subsequent source work by
other lanes is unrelated and is not covered by this point-in-time receipt.
No native jobs or source edits were performed by this lane after restoration.
Only this document and the new evidence bundle are committed by this lane.

## Portable replay and privacy

From the repository root:

```sh
python3 -B bench/results/resident-home-query-20261031/validate.py --manifest
python3 -B bench/results/resident-home-query-20261031/test_offline.py
```

The stdlib-only replay reconstructs all 28 strict outcomes and both comparison
arithmetic tables, checks complete actual native named-result sets and five
fixture names, replays the v66-to-v67 repair, and validates restoration hashes.
Seven offline tests include 40 independent semantic mutations, a coherent
raw/parsed wrong-test mutation, rehashed native-result and coherent timing
history negatives, and relocation under unrelated working directory with empty
HOME, empty PATH, and Python isolated mode. These are offline evidence controls,
not native correctness/performance tests.

Receipts use explicit fixed environment/control allowlists and full-line
output selection. Ambient environment, raw private state JSON, private homes,
caches, worker/daemon logs, telemetry, credentials, executables, and targets
are excluded. Full private artifacts remain outside the repository. Source and
script snapshots with private path constants are clearly labeled sanitized
derivatives, bound to exact-original SHA/size provenance. A fixed historical
observation digest supplements the complete mutable file inventory, so merely
rehashing modified artifacts does not rewrite recorded history.
