# Scalar retention flag-transition acceptance

`bench/reuse-query-transition.py` closes a boundary not covered by the earlier
separate off/on cache acceptance: changing the scalar retention flag in one
incremental cache. This is correctness acceptance, not a speedup claim, a
promotion gate for other retention flags, or proof of actual query reuse.

## Contract

For every source/dependency edit in `reuse-query-accept.py`, run explicit
`-Zjr-cache-small-query-values=no`, `yes`, `no`, in that order against the
**same** pair of dependency/main incremental directories. Keep those directories
across all edits, ordinary errors and recoveries. For each source/flag pair,
compile a fresh reference with its own never-used incremental directories.
Both reference and transition builds use `-Zincremental-verify-ich`.

Both paths use identical source, output, crate-name and extern paths. Compare
exit status and stdout/stderr **byte for byte**, with no diagnostic normalization
or filtering. Successful fixtures must additionally return zero and produce the
known expected stdout with empty stderr. Expected ordinary compiler errors must
return exactly 1, never an ICE, signal, timeout or launch failure. Each successful
output is removed before compiling so a stale binary cannot satisfy runtime
acceptance. Binary digests are evidence, not equality requirements.

The inherited matrix includes unchanged builds, body edits/reverts, constness
changes, async/ordinary changes and async closures, nested owners and owner
addition/removal/reordering, macro changes, lint/type errors and recovery, and
dependency signature changes/errors/recovery. Each case performs a complete
no/yes/no sequence, rather than advancing the flag only when the source changes.

## Invocation

Only run after any active measurement hold is released by the coordinator:

```sh
python3 -m unittest discover -s bench -p test_reuse_query_transition.py -v
python3 bench/reuse-query-transition.py \
  --rustc /home/jeremy/.jcode/scratch/query-retention-bench-20261030/home/toolchains/staged/jr-50f30aea8dbb/bin/rustc \
  --output /home/jeremy/.jcode/scratch/query-retention-transition-NEW
```

The output must not exist and its parent must exist. It is created mode 0700.
The CLI uses umask 077. The script does not modify compiler sources, source
snapshots, the installed client, hosted state or an existing output directory.

## Evidence and fail-closed behavior

`acceptance.json` records the exact resolved compiler path, SHA-256, `-vV`,
sysroot query, identified rustc_driver paths and hashes, harness hashes, selected
relevant environment variables, complete command vectors, exact byte streams as
hex, expected results and completed transition count. Compiler and driver hashes
are rechecked after the matrix to reject mutation during acceptance.

Every subprocess immediately writes its command/status JSON and raw `.stdout`
and `.stderr` files, including partial non-UTF-8 output on timeout. Per-case
source copies, cache directories and current artifacts remain private in the
output directory. On any exception, the report says `failed`, includes the
failure reason, preserves all evidence captured so far and exits nonzero.
Existing output directories are rejected without touching their contents.
Missing compiler/driver identity, an empty matrix, unexpected compiler exits,
diagnostic mismatches, wrong runtime and compiler/driver mutation fail closed.
Abrupt external termination can leave only the already-written per-process
records, never a valid passed report.

Mocked unit tests exercise orchestration and failure handling without launching
a compiler. They do not replace the native acceptance run. A pass proves the
observed diagnostics/runtime contract for this compiler and fixture matrix only.
It does not prove cache reuse counts, performance, all Rust programs, or other
retention modes.

## Validation status

Implementation prepared during the measurement hold on 2026-10-30. The
coordinator subsequently released focused Python tests and original frozen scalar
native correctness acceptance only.

Validation on 2026-10-30:

- All 11 focused Python regression tests passed.
- Original frozen scalar native acceptance passed all **66/66 transitions**
  across 22 source/dependency cases. Every transition matched its own fresh
  reference with exact diagnostics and expected runtime, including errors and
  recovery. This covers 264 compiler fixture invocations, plus runtime and
  identity checks.
- The exact report is committed at
  `bench/results/query-value-retention-20261030/native-mode-transitions/acceptance.json`.
  Full private report and raw per-process evidence are retained at
  `/home/jeremy/.jcode/scratch/query-retention-transition-20261030-native-01/`.
- `acceptance.json` SHA-256:
  `7ed0356418715174f55cdc0574225acecaa40c63ac74cb00cf396dea918e0aa8`.
- Compiler SHA-256:
  `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
- `librustc_driver-e2bd696299265a4a.so` SHA-256:
  `502401b14a878674fddf862e4fe7c7dca0948da8a7b8e8aefd3543ffd57a0eb7`.
- Compiler identifies as `rustc 1.101.0-nightly (50f30aea8 2026-10-14)`,
  commit `50f30aea8dbb58ff7df66b62be955199be935be9`,
  `jr-query-admission-binding: reserved-fd-v2,query-schema=4`,
  host `x86_64-unknown-linux-gnu`, LLVM `23.1.3`.

No broad benchmark, hosted workload, compiler/source-snapshot change, client
change, or performance claim was made by this lane.

### Current real-capture compiler revalidation

The same matrix was rerun on 2026-10-30 against
`/home/jeremy/.jcode/scratch/provider-profile-20261030/sysroot/bin/rustc`,
whose driver SHA-256 is
`8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`.
All **66/66 transitions** passed, covering the same 22 cases and 264 compiler
fixture invocations. This is the frozen compiler used for the real scalar and
associated-item captures, not a newly rebuilt compiler. The complete receipt is
`bench/results/real-scalar-retention-20261030/current-mode-transitions.json`.
Private raw evidence remains in
`/home/jeremy/.jcode/scratch/current-retention-transition-20261030/`.
This closes the scalar shared-cache transition gate for that compiler only.
It does not extend the scalar transition matrix to associated-item retention.
