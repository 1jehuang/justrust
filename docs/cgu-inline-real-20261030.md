# Real CGU inline traversal mechanism, 2026-10-30

## Outcome and scope

This is an offline retention of the coordinator's completed `dp47` diagnostic,
not another native build or workload run. The real justrust filtered-test workload
exercised the default-off shared-inline traversal on each enabled prepare, edit,
and restore compile. Each enabled main-crate profile contains one bounded
cross-root revisit marker. All six runs passed the same four tests, both edit
runs printed the exact fresh marker `JR_RUNTIME_EXECUTED_47`, and the original
63-file source snapshot and executable identities were restored/unchanged.

**Mechanism exercised does not mean faster.** The off edit's
`cgu_partitioning_place_items` interval was **34.061566 ms elapsed**, versus
**51.518598 ms elapsed** on. These are nested profiler intervals, not CPU time,
exclusive work, or time saved. This diagnostic is one fixed off-then-on sequence,
includes self-profile overhead and an unknown-contamination machine, and is not
an aggregate benchmark or a default-promotion gate. The option remains off.

A separate protected `br46` repeated runtime comparison retained by another
worker measured median edit subprocess wall time of **1.137700765 s off** versus
**1.233051763 s candidate**. That is a separate evidence set, not a denominator
for these profiler intervals. See `docs/cgu-traversal-br46-20261030.md` and
`bench/results/cgu-traversal-br46-20261030/` where retained. No speedup or
10x frontend claim follows. Larger resident frontend coordination remains needed.

## Exact physical experiment

The original evidence root was `/home/jeremy/.jcode/scratch/dp47`. The coordinator
ran the captured `runner/profile-cgu47.py` through `runner/launch-profile47.sh`,
with exit status 0. These historical scripts are retained verbatim, **not** run
by the archival collector or validator.

The command was the frozen client's
`test --lib -- summary::tests:: --test-threads=1 --nocapture` against the private
source snapshot `/home/jeremy/.jcode/scratch/ba30/source`. Two stacks used separate
homes, targets, recording stores, and profile directories, with the sequence:

1. `owned-off`: prepare original, edit test body, restore original.
2. `owned-cgu-traversal`: prepare original, the identical edit, restore original.

The edit inserts a `black_box(47u64)` assertion and the runtime marker into
`shares_overlapping_time`. Exact original and edited source bytes are retained.
The tests are `parses_libtest_results`, `shares_overlapping_time`,
`splits_passes_without_double_counting`, and `test_failures_are_not_compile_errors`,
all under `summary::tests`. Each run reports 4 passed, 0 failed, 0 ignored,
and 144 filtered out. Marker counts in unabridged journal output are one per
edit and zero for prepare/restore. The console output alone is not the oracle.

Both profiles use optimization 0, debug info 0, incremental true, with self-profile
events `generic-activity,function-args`. Three earlier experimental flags are
explicitly `no`. The off stack omits the new traversal flag and relies on its
tracked default false. The enabled stack adds
`-Zjr-cgu-shared-inline-traversal=yes`. Remote execution, dependency/artifact
caches, prebuilt fetches, proc-macro cache, build daemon, Cargo fork, and
hotpatching are disabled. The exact environment and compiler command metadata
are retained and checked, not reconstructed from the stack name alone.

Frozen executable SHA-256 identities before and after:

* rustc executable:
  `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`
* rustc driver:
  `29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`
* justrust client:
  `1dd6ee7da925924eaa0c25f9cbefafaabe1441c7b8f95a87b6cddfbaf20510a9`
* Cargo:
  `994419eb0d12f1a4152a23d9030dc4d05c9bfd7a5c7af923f24bb3738afad95b`

This is the earlier frozen driver, not the coordinator's later separately built
optimized-release compiler artifact. Do not mix their identities or results.

## Main-crate profile observations

All listed intervals are on recording-local thread 3. Counts are exact native
GenericActivity intervals. Each enabled revisit marker is emitted at most once
per `place_mono_items` invocation and proves that an earlier root in the same
initial CGU had already reached a LocalCopy item, suppressing duplicate DFS.
It is not a counter of all duplicates removed. It does not identify the item,
CGU, total removed edges, or counterfactual runtime.

| Stack | Phase | Run ID | Place elapsed ms | Traversal events | Traversal union elapsed ms | Revisit events |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Off | Prepare | `20261030-062948148-2994336` | 46.173164 | 0 | 0 | 0 |
| Off | Edit | `20261030-063011141-2997139` | 34.061566 | 0 | 0 | 0 |
| Off | Restore | `20261030-063013920-2997519` | 36.571440 | 0 | 0 | 0 |
| On | Prepare | `20261030-063016856-2998118` | 52.499267 | 14040 | 10.194104 | 1 |
| On | Edit | `20261030-063042298-3001059` | 51.518598 | 14041 | 10.822125 | 1 |
| On | Restore | `20261030-063045700-3001740` | 65.932588 | 14040 | 13.894190 | 1 |

Traversal markers scope enabled graph work only, excluding subsequent inline
insertion/size queries. Their elapsed union is nested inside place-item elapsed.
Do not sum the two. Raw revisit intervals were 26 ns, 26 ns, and 27 ns, merely
the fixed-marker scope overhead, not an estimate of the work skipped.

The seven retained label groups include placement, overall partitioning,
partition/assert-distinct-symbols, traversal, revisit, macro expansion, and
codegen. `observations.json` retains every selected interval's exact start/end,
thread, count, and union, not only this rounded presentation.

## Retained bundle and replay

The assigned new evidence directory is
`bench/results/cgu-inline-real-20261030/`. It contains:

* Six **lossless** gzip main-crate `.mm_profdata` profiles. Raw total:
  226,325,520 bytes. Gzip total: 45,151,376 bytes. `profiles.json` pins both
  compressed and uncompressed sizes and SHA-256 hashes. Dependency profiles
  from prepare builds are not part of this selected six-main-profile bundle.
* The original runner state, exact runner/launcher/stdout/stderr/exit receipts,
  all 63 original source files, and the exact edited source bytes.
* Six full run journals, including output, metadata, summaries, unit records,
  process/sample records, and CPU records, plus the separate console logs and
  raw-output receipts. Historical absolute paths remain in raw evidence.
* Six dedicated privacy-redacted `justrust export` entries under bundle-local
  `exports/`, exported from the two isolated recording homes. No global last-run
  lookup and no modification of the shared `data/runs/` directory was needed.
* A pinned copy of the standalone measureme decoder, exact derived intervals,
  an offline validator, mutation tests, and a complete SHA-256 inventory.

Replay from the repository root with Python's standard library only:

```sh
python3 -B bench/results/cgu-inline-real-20261030/validate.py
python3 -B bench/results/cgu-inline-real-20261030/test_validate.py
```

The validator checks the exact file inventory, gzip and raw-profile hashes,
strictly resolves all profile labels, reparses the raw records, compares every
selected per-thread start/end interval against the retained derivation, and
checks counts/unions against the original runner state. Marker intervals must
be contained in the same thread's placement interval, and the revisit marker
must follow a traversal. It verifies four-test runtime output, exact fresh-marker
counts, recorded run IDs, source bytes/hashes/restoration, fixed compiler identity,
flag/environment isolation, and all six dedicated exports.

Mutation tests reject source/identity drift, missing/reordered runs, wrong flags
or enabled caches, runtime corruption and forged marker receipts, missing or
duplicate revisit records, wrong threads, shifted intervals with unchanged
elapsed/counts, unresolved labels, incorrect profile commands, missing/duplicate
exports, failed runner receipts, malformed profiles/gzip, and inventory changes.
No compiler, Cargo, tests of Rust workloads, or captured runner is invoked by
these offline checks.

After the coordinator released the protected timing window, strict raw-profile
replay passed for all six captures with **zero unresolved records**, and all
**28 offline mutation/unit tests passed**. `validation.json` and
`mutation-tests.log` retain these results. Lossless archival copying and metadata
exports completed before that audit, without running native workloads or editing
native sources. The final inventory is frozen by `finalize_inventory.py` only
after receipt generation. Run replay without Python `-O`, since checks use
assertions.

Scoped diff checks on the new replay scripts and documentation are clean. A full
bundle diff check reports four historical whitespace findings: blank lines at
the ends of the two edit console stdout files and the copied
`source/docs/AGENTS-snippet.md`, plus a trailing space in the copied
`source/bench/feature-switch.sh`. These original bytes are deliberately preserved
to keep the lossless console and 63-file source hashes valid, not reformatted.

The finite exact corpus does not establish all Rust-program correctness or
cache semantics. Native semantic acceptance is recorded separately in
`docs/native-cgu-inline-traversal.md` and the coordinator's acceptance bundle.
This real-workload bundle adds exercised mechanism proof, not performance proof.
