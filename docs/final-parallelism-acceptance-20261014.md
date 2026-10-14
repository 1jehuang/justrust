# Final parallelism and measurement acceptance, October 14, 2026

This reruns the mapped checks on the whole frozen result. It demonstrates a
reporting correctness improvement, not a compiler speedup. The broader goal of
maximal compiler parallelism is **not achieved** by the scheduler experiment.

## Frozen identities and scope

- Client source: `e629a004068923360bc2cb8880ca160020c758f7`.
- Client SHA-256: `08d0601aa8bfa33decdcdbeb986a2d9558ba0873d5f1b5d3659117c034660abe`.
- All 69 source/test/asset/manifest blobs in the isolated acceptance snapshot
  matched the corresponding Git objects before and after validation.
- Cargo candidate source: `a45c3546671925c24649e451a673aa71b1385d0d`, clean.
- Candidate SHA-256: `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d`.
- Nightly: `nightly-2026-10-08`. Ordinary hosted calls used the accepted
  west 24-core host. Local fixtures are deliberately tiny compiler-boundary
  checks, not a bypass of normal hosted acceptance.
- No resize, scheduler activation, global reinstall, default compiler change,
  permissions relaxation or worker deployment was performed in this rerun.
  The account host was explicitly released at 05:02:02 UTC.

## Observed improvement, not just inspection

The same unchanged test artifact ran two passing tests with a real expected
nested E0308. Both commands exited zero and retained the diagnostic in full.

| Client | Run | Observed public verdict |
|---|---|---|
| Previous immutable 2153 client | `20261013-215604181-1595675` | Incorrectly reported FAILED to compile |
| Corrected immutable 08d client | `20261013-215604352-1595740` | Correctly reported two tests passed |

Neither invocation recompiled a unit. The nested child executable was identical
for both parents. This isolates parent reporting behavior, not performance.
The original misleading 397-test record and compiled CPU red run remain
unchanged in the earlier acceptance evidence.

## Requirement-to-observation matrix

| Requirement / changed public output | Final observed result |
|---|---|
| Survivor CPU when another child exits | Named regression passed in final full-suite run below |
| Missing snapshots must not recount lifetime CPU | Named regression passed in that same final run |
| PID reuse and overlapping roots | Named regression passed in that final run |
| Counter regression must not create a later spike | Named regression passed in that final run |
| Unit-only and output-only nested failures with successful outer command | Summary regression passed; actual old/new CLI comparison above improved the verdict |
| Genuine outer compile failure | `20261013-220037908-1618025`: full E0308, exit 101, compile_failed true |
| Runtime test failure stays a test failure | Named runtime-failure summary regression passed in final full suite |
| Real expected nested compiler failure | `20261013-220037792-1617967`: two passes, exit 0, compile_failed false, E0308 retained |
| Recorded / unrecorded / recorded cache sharing | `20261013-220052327-1619206`, unrecorded check, `20261013-220052488-1619235`: all Fresh, no Dirty, identical recursive rmeta hashes |
| Tracked target sources survive upload without ignored caches | Named real rsync regression passed in final suite; Cargo public testsuite runs without manual fixture copying |
| Queue prerequisites and candidate ranking | `20261013-215921598-1612465`: ten queue tests passed |
| Configuration fallback and cost-file safety | `20261013-215929493-1613079`: five parser/file tests passed |
| Default-off/on/off freshness and binary/rlib/rmeta/proc-macro/generated output transparency | Multi-output public CLI case passed in `20261013-215931783-1613267` |
| Cold candidate -j1/-j2 builds execute correctly | Same multi-output case passed, executable returned 42 |
| Cold order and invalid/missing cost fallback | Named cold-order public CLI case passed in that run |
| Internal two-token jobserver cap | Named TCP-barrier public CLI case passed in that run |
| Inherited GNU make two-token cap | Named TCP-barrier case actually exercised GNU Make 4.3, not skipped |
| Failed branch, keep-going, fix and retry | Named public CLI case passed in that run |
| Benchmark mode, identity, no-gain/noise guards | All 86 offline benchmark/report tests passed after final code changes |

The receipt records all five public Cargo outcomes individually, including the
serial harness's split-line successes, and all eight named CPU/verdict/source-sync
passes. It does not infer individual success merely from an aggregate count.

## Final ordinary hosted acceptance

| Command | Exact run | Result |
|---|---|---|
| check | `20261013-215951672-1614703` | exit 0, fresh root, 75 compiled with 73 depcache hits |
| full test | `20261013-215956564-1615106` | 404 passed, zero failed, three ignored |
| strict all-target lint | `20261013-220016394-1616485` | exit 0, no warnings |
| repeated check | `20261013-220021787-1617016` | exit 0, zero compiled, 62 fresh |

The frozen runner checked executable digests before and after every command and
verified the actual own-session hosted route and compiler metadata. It did not
use a run-ID override. Tests compiled this isolated source root, not just a stale
test binary. The approximately ten-second public-bundle fixture is a test cost,
not evidence of compiler scaling.

## Final real Desktop workflow

All four invocations used the immutable corrected client and passed three
fps_counter tests. Other peer source changes were preserved.

| Step | Exact run | Compiled / fresh |
|---|---|---|
| warm | `20261013-220127489-1621943` | preparation, not a latency sample |
| harmless one-line body edit | `20261013-220132579-1625514` | 1 / 745 |
| unchanged repeat | `20261013-220143371-1630401` | 0 / 746 |
| exact restored source | `20261013-220151800-1631350` | 1 / 745 |

The restored source matched its backup and original SHA-256
`24de30f7556c2e126bb52f685377f656c6352dfdf83e1bc9f9d03f037d877e8d`.
It was not restored using Git. The edit compiled one UI unit mostly alone.
That is an observed compiler-internal opportunity, not proof that all its work
can be parallelized or that 96 cores would be cost-effective.

## Evidence audit and remaining limits

`bench/results/final-parallelism-acceptance-20261014.json` contains the final
18 exact observations, raw-output/summary hashes, frozen receipt, and mapped
checks. All 18 new IDs were exported from the repository root and appended to
`data/runs/hosted-west-measurement/2026-10.jsonl`, now 39 exact cited records.
The audit also verified the earlier 21 measurement summary hashes and all 56
scheduler IDs/identity checks plus the four immutable segment hashes. Those
scheduler outcomes and noise flags were not rewritten.

The initial final cache-preparation check
`20261013-220037997-1618068` rebuilt because this fixture had been intentionally
edited earlier. A legacy deps-directory hash collector then failed to find the
new Cargo artifact layout. The corrected recursive collector observed three
source-unchanged Fresh checks and identical artifact bytes. Neither mistake is
misrepresented as a cache gain.

The Cargo fork still has ten previously measured baseline strict-lint errors.
The clean strict-lint result above is for justrust, not for the Cargo fork.
The candidate remains default-off: the matched pinned scheduler suite did not
establish a full-loop speed improvement. Cost-weighted scheduling remains
untested, compiler query DAG span has not been measured, and sampling cannot
recover CPU lost between samples. Repository review gates on compiler semantic
changes remain in force. Maximum parallelism, hardware ceiling, and a general
speedup are unresolved, rather than declared complete by passing correctness
checks.
