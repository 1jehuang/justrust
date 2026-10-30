# Findings: where agent build time goes

## 2026-10-30: smaller account host is live, not a measured speedup

Resized the owner's existing AWS host from 96 to 32 physical cores and 384
to 128 GiB provisioned RAM, preserving its instance, root volume and caches.
Live guest/API checks confirm the target. Hosted check
`20261029-213409377-3008036` kept 62 units fresh but spent 50.167s syncing
on the first connection after reboot, versus 1.30s host work. Subsequent
full tests `20261029-213502310-3012481` passed 837 with 10 ignored and
strict all-target Clippy `20261029-213518592-3013767` passed. Desktop smoke
`20261029-213523554-3008035` failed on four missing `effective_budget`
fields in current project test fixtures, not a host admission failure.
The comprehensive hosted baseline was blocked by unrelated benchmark snapshot
locks. No cross-size speedup, equal-performance claim or global fleet promotion.
See `docs/hosted-sizing-20261030.md` and its selected run exports.

## 2026-10-30: deeper cache reuse is real, but remote no-op latency remains

Inventory: `docs/caching-levels.md`, with 18 reuse levels and their input
contracts. Three native experiments are committed and default-off: associated
item query retention (`84c0f3e16`), overlapping Cargo subtree validation
(`b0878ebcc`), and two audited module-local Clippy passes (`a29b1e69a`).
The query fixture eliminated 10 proven-green recomputations at +1,522 cache
bytes. Same-binary Cargo measurements improved overlapping roots 44.99 to
31.11 ms but regressed a single root 35.37 to 39.16 ms, so no default promotion.
Clippy passed 48 real-driver invocations and the official UI suite; profile
showed lint_mod executions 3 cold, 0 unchanged, 2 after a one-module edit.
Cfg switching required a private versioned incremental namespace. Two initial
passes were rejected for diagnostic-ordering differences without blessing UI
snapshots. Details and raw focused evidence live in the linked experiment docs.

The installed shipping client passed 831 tests (`20261029-205934738-2526594`)
and all-target lint (`20261029-205609282-2475935`). That cached lint still cost
2.58 s client-visible versus 0.05 s Cargo, 62 fresh units, and 1.94 s sync.
This is not a matched performance comparison. The comprehensive baseline was
blocked by an existing benchmark snapshot lock. No general speed claim or
hosted native activation. Safe local whole-command result replay, authoritative
input receipts, and a resident expansion/semantic graph remain unfinished.

Follow-through shipped and installed `f3a7dc752` (derived watch-directory set
reuse) and `1c3656078` (linear sorted-manifest diff), with 834 tests passing.
Both retain fresh source scanning, registration retries and the remote barrier.
Real installed warm Clippy observations: 1.4 s (`20261029-211321346-2704621`)
and 1.2 s (`20261029-211322726-2703167`), 62 fresh units and 0.03 s Cargo each.
These are not matched before/after speedup evidence. `edce4013a` subsequently
bulk-builds fresh observed manifests, preserving last-observation duplicate
semantics against 1,555 sequences, with fresh filesystem mutation regressions.
Native input-prerequisite producer `c6004a097` passes four real Cargo CLI tests
(`20261029-211619753-2745232`) but always denies trust/command replay. It does
not close planning, config, negative-input, toolchain or remote-mapping gaps.
See `docs/caching-levels.md` and the linked component evidence. After explicit
maintenance release, installed unchanged strict Clippy took 1.1/1.7/1.1 s with
62 fresh units and 304–357 ms sync. Real source edit and backup restore each
passed 837 tests, followed by strict Clippy; one parallel cleanup flake passed
isolated retry. Exact run records are committed in `260ac5a0d`. Further tracing
(`20261029-214313393-3063349`) observed 869 ms end to end, of which 470 ms
preceded completed admission and 314 ms was sync. This is not a matched speedup
or evidence of consistently subsecond operation. Client HTTP phase attribution
is the next diagnostic, not permission to cache billing/authorization outcomes.

## 2026-10-29: Clippy's cached path still paid for ignored build-tree scans

Fixed ignored-lockfile discovery in `29c6e250b`: collapse ignored directories
before walking them, pruning build output instead of filtering its lockfiles
after Git traverses all artifacts. The local Jcode scan diagnostic retained
2655 files and seven oversized files. Its five scans went from
1.890-2.239 s to 0.035-0.038 s (`20261029-201053352-1916653`, after).
This is a focused source-scan observation, not a compiler or broad speed claim.
Pinned hosted no-op suite comparison: 0.92 -> 0.83 s, **within noise**.
Installed cached Clippy: 1.57 s client-visible, 0.04 s Cargo, all 62 units fresh
(`20261029-201238735-1925361`). Initial daemon mirror initialization still
cost 14.65 s on this 68k-file tree. Hosted check, all-target Clippy and all
831 tests passed. Actual-edit Clippy profile: 2.25 s compiler, including
0.366 s crate lint traversal (`20261029-200741491-1880584`). No unsafe lint
skipping or speculative compiler optimization. Full evidence, cold-mirror
limitation and blocked comprehensive gate: `docs/clippy-performance-20261029.md`.

## 2026-10-30: measuring incremental reuse opportunity, not a claimed ceiling

Native self-profile markers now distinguish proven-green query value acquisition
from providers rerun because no disk value was available. This is a concrete
resident-value-cache opportunity, not all avoidable frontend work. The compiler
and matching std built successfully. Five real compiler cases (cold, unchanged,
body edit, revert, type error) preserve exact diagnostics/status, successful
binary bytes and runtime output with profiling on/off and verify-ich enabled.
Evidence: `bench/results/reuse-native-acceptance-20261030.json`; methodology and
remaining ownership/invalidation work: `docs/incremental-reuse-roadmap.md`.

Real justrust checks with the local-only newly built compiler, self-profiling,
verify-ich, artifact caches off and explicit proc-macro caching enabled passed:
`20261029-192127135-1482189` (35.9s initial compilation) and
`20261029-192203045-1482188` (3.2s forced unchanged-source warm compilation).
These describe one diagnostic's states, not a before/after speedup, production
latency, or the general reuse ceiling. Profiles are retained in private scratch.

The macro-cache-enabled warm library profile observes 119,322 proven-green
value-acquisition scopes and 70,506 provider-recomputation scopes. On profiler
thread 3 their separate interval unions are 248.143496ms and 110.519125ms.
These nested scope sets must not be added. The latter is a measured candidate
for retained-value investigation, not 110ms of guaranteed removable build time
or the complete maximum reuse gap. Expansion, HIR lowering, resolution and
other non-green/nonquery work remain separate opportunities.

The comprehensive hosted baseline first failed source-activity protection for
benchmark roots (`bench/results/versus-synth1-20261029-191138-4bfa081ce4fc-dirty.json`).
Commit `33900e534` fixes receipt enrollment without making private/cache roots
eligible for GC; full hosted tests passed 830 tests (6 ignored), clippy passed,
and the exact failed justrust prebuild and filtered test passed
(`20261029-191801203-1462638`, `20261029-191850292-1467868`). The broad retry
still failed the pinned Desktop integration test because
`CARGO_BIN_EXE_jcode-desktop` was unavailable at compile time
(`20261029-192227259-1470257381`). It was stopped, and the benchmark backup
journal recovered under its lock. Comprehensive baseline remains blocked;
no aggregate performance improvement or default promotion is claimed.

## 2026-10-30: parallelism configuration audit

Hosted crate scheduling and test parallelism are already active. The missing
layer is the clean-room compiler's frontend: recent hosted check
`20261029-175257566-943851` averaged one of 96 CPUs, whereas the full library
test run `20261029-175106375-937110` averaged 29.2 and peaked at 73.1 CPUs.

Deployed native jr-cargo workspace-only threading locally and on hosted.
Quiet pinned hosted full rechecks measured 13.43s at one thread versus 6.04s
at eight: `bench/suite.py compare` reports -55%, faster. All six recordings
verified the actual requested compiler count. Warm incremental test edits
remained within noise (7.67s vs 7.79s). Clean-room `main` commit `a0cb0e435`
enables eight threads through Cargo's normal `[env]` config. The comprehensive
40-point gate was interrupted twice by server reload, so no global default
promotion or general speedup is claimed. Exact files and acceptance run IDs:
`docs/parallelism-audit-20261030.md`.

Fixed hosted transport dropping explicit Cargo/frontend/libtest/pool limits and
opt-outs. Thread telemetry now recognizes native and split flag spellings.
Hosted validation: check `20261029-181020029-1085452`, four shim tests
`20261029-181117478-1095482`, hosted-control regression
`20261029-181138802-1100882`, full test `20261029-181156594-1106065`
(829 passed). No blanket compiler default promotion follows from this audit.

## 2026-10-26: Next optimization targets from recent real work

Read-only triage of the 90-minute window ending 2026-10-27 00:58:01 UTC
(2026-10-26 17:58:01 PDT) found 267 completed runs from the exact primary
checkout paths, excluding benchmark/scratch copies. The fixed snapshot is
`bench/results/20261026-recent-optimization-targets.json`. Aggregate recorded
wall is 6,887.5s: compilation 2,610.9s, build-lock waits 2,304.1s, measured
test execution 1,047.2s, tail 876.6s, and build gaps 24.9s. These are sums of
overlapping run waits, not elapsed session time or guaranteed savings. No
new remote builds were launched for this investigation.

1. **Shared-build contention is the first target.** 121 runs waited more
   than 0.1s for a build-directory lock. Graph-integrity commands alone
   accumulated 291.2s of lock waits across 26 runs. Run
   `20261026-172317752-1456504` spent 61.06s of 67.67s waiting, versus 1.22s
   compiling and 5.36s in tests. Even the small Jcode policy/protocol test
   run `20261026-175603663-1641969` spent 5.74s of 8.10s waiting, versus
   0.09s executing 205 tests. Investigate native jr-cargo coordination that
   shares identical compilation work, publishes ready artifacts to separate
   test consumers, and keeps lock ownership as narrow as correctness allows.
   Do not allocate per-agent target trees or worktrees. Gate on concurrent
   identical and mixed-edit workloads, cancellation/failure correctness,
   compiled-unit counts, and p50/p95 agent wait. The logs establish contention,
   not which critical section can safely be shortened.
2. **The earlier test-profile change does not speed up local `run` builds.**
   Six `run --bin jr-rust` invocations accumulated 329.1s of compilation out
   of 368.6s wall. The compiler crate alone took 78.8s in
   `20261026-171806491-1425256` and 76.1s in
   `20261026-175541017-1639652`; both executed locally and rebuilt the large
   library. The dev profile still has opt-level=1 and default local ThinLTO,
   whereas only the test profile has LTO off. Next diagnostic: a pinned local
   source-edit/run scenario comparing dev LTO/codegen choices, including the
   resulting compiler's execution time. Do not promote the test-only result
   to dev defaults without measuring this separate tradeoff. A longer-term
   per-module compilation boundary may be needed for the 312k-line library.
3. **Large sequential oracle batches have real execution cost.** Run
   `20261026-173651506-1526940` compiled nothing and spent 65.54s in the
   closures suite; one mutable-capture test accounted for 65.52s. Its 15
   cases call `tests/support/oracle.rs` sequentially, then independently run
   both worker counts and CHECK/BUILD modes in `capture_cases_workers`.
   The CLI rejection batch in `tests/compiler.rs` likewise loops over 28
   cases in one test and took 20.00s in `20261026-175456204-1635122`.
   Instrument per-case stock/native/check/build time, then evaluate bounded
   case parallelism and precisely keyed local stock-oracle reuse. Retain
   every case, rejection assertion, and worker/mode check; do not optimize
   by weakening acceptance coverage or oversubscribing nested workers.
4. **Fix attribution before treating long tails as infrastructure cost.**
   `20261026-175400262-1629957` labels 49.12s as after-build time despite
   libtest reporting that same 49.12s for a failing `full_programs_ratchet`.
   Other failing suites have similar missing test spans. Improve native
   test execution telemetry and the stock compatibility fallback, with
   regressions for failed, filtered, and multi-binary suites. This is a
   reporting/correctness target, not a measured speedup or evidence that
   876.6s of aggregate tail is network overhead.

## 2026-10-26: Local compiler test edits without test-profile ThinLTO

The recent clean-room run `20261026-153635937-779805` spent 30.66s of
31.69s rebuilding the library test binary, versus 0.99s running its selected
test. This was compiler rebuild latency, not a need to test hosted-service
behavior. The user requested local execution for ordinary tests because of
security and control concerns; the following comparison is entirely local.

`bench/cleanroom-test-profile.py` drives the existing `bench/suite.py` harness
on pinned clean-room revision `7bc3f6e5f`, changing a test body before each
filtered semantic test. On this XPS14, nightly-2026-10-08, three measured
samples per side after warmup and with `--quiet-wait 10`, the comparison is:

```
python3 bench/suite.py compare \
  bench/results/20261026-rewrite-local-baseline-confirm.json \
  bench/results/20261026-rewrite-local-lto-confirm.json
rewrite-lib-test-edit          11.30    8.42   -2.88   -26%  faster
```

All six confirmation samples passed the harness's quiet-sample check. The
only candidate change is `[profile.test] lto = "off"` in the clean-room
manifest. Optimization level 1, full debug information, debug assertions,
overflow checks, development builds, and release builds remain unchanged.
The measured benefit is specific to local test-body edits, not a promise
that every compiler source edit or the full suite becomes 26% faster. The
session-weighted synthetic gate was not run and no global client default is
being promoted. Switching profiles incurs a one-time artifact rebuild.

Correctness checks on the pinned snapshot with the actual changed manifest
passed: check, five physical-carrier tests, 17 graph-integrity tests (one
ignored), and 23 canonical-source-replay tests. The active checkout check
hit in-progress frontend E0425/E0308 errors and was not repaired by this
manifest-only change. A longer-suite runtime screen observed 49.07s with
LTO off versus 43.75s with default local ThinLTO, but both runs had heavy
external CPU contention (8.5-10 other cores, about 10s CPU pressure), so this
is not a controlled regression estimate. Compile latency and execution
speed remain a tradeoff, not a universal speedup. For compute-heavy full
suites, `--config profile.test.lto=false` restores the previous setting.

An earlier debug=1 experiment regressed from 11.43s to 16.79s median and was
rejected. Those first comparisons had noisy samples and are retained as
exploratory evidence, not controlled speedup claims. Hosted pinned-suite
attempts were stopped after the user clarified local execution; an attempt
also hit `cannot protect source activity: No such file or directory`.

## 2026-10-26: Slow-run inventory and development DEFLATE tail

`bench/SLOW-RUNS.md` inventories all recorded runs and separates remote client
wait from build-side wall time. The pinned `justrust-public-cache-cap` diagnostic
keeps the 96 MiB incompressible artifact, public publish/fetch/content checks,
and private-cap rejection. Only miniz_oxide's development optimization changes.
`bench/suite.py compare` reports 16.00s -> 5.80s median (three warm samples,
zero compiled units), **uncontrolled (noisy or unknown samples)**. This is a
scoped observation, not a controlled or general compiler speedup claim.
The matching results are `bench/results/public-cache-cap-miniz{0,3}-local-20261026.json`.
Hosted pinned-root measurements were blocked twice by `cannot protect source
activity: No such file or directory`; both sides therefore used local execution.
Real hosted test timing was 11.33s (`20261026-142030694-281363`) before and
3.79s (`20261026-143305145-386513`) after. The serial library suite passed;
default-parallel validation exposed existing remote_evict lock-test races.
Only the development compression dependency is optimized, not the changing
client crate or release profile. No comprehensive workload speedup is claimed.
Full hosted `justrust test -- --test-threads=1` passed 784 tests
(`20261026-143543882-404516`), all-target clippy with `-D warnings` passed,
and all benchmark harness regression tests passed.

Follow-up: detached hotpatch symbol indexing forked without exec and retained
unrelated inherited descriptors, including flock leases and output pipes.
The child now isolates stdio and closes descriptors >=3 before indexing,
or skips optional prebuilding when isolation is unavailable. A subprocess
regression verifies pipe EOF and release of the child's inherited flock while
the child stays alive, without releasing the parent's live lock. An intermediate
hosted default suite exited zero (`20261026-144511276-460041`), but repeated
parallel gates still required serial retries for eviction-lock failures. Focused
hotpatch/eviction tests, full serial tests, and all-target clippy with `-D warnings`
passed. FD cleanup alone did not establish clean default-parallel acceptance,
and is not a separately measured performance claim.

The final test-only follow-up runs exact lock-lifetime assertion bodies in
fresh single-test subprocesses, away from unrelated libtest forks. All existing
assertions remain, child output is captured, and the parent requires exactly
one passed/unignored test so a stale filter cannot silently skip coverage.
No retries, global test serialization, or production lock changes were added.
Two full unsplit parallel hosted suites passed all 785 tests without automatic
failure-confirmation retries: `20261026-145342655-509649` and
`20261026-145352802-510494`. Strict all-target clippy also passed
(`20261026-145358472-509275`).

## 2026-10-25: Jcode preview-updater hosted edit loop

Hosted check `20261025-231934593-200649` passed in 12.30s rust-side
(14.6s including sync), with 11.89s compiling and 26 rebuilt workspace
units. The report attributes about 8.4s to downstream rebuilds, notably
the broad `jcode-config-types` root and `docs` being a build-script input
of `jcode-app-core`. The CLI parser test run
`20261025-232218743-220112` passed in 20.7s rust-side (23.2s hosted),
including 20.30s compiling: `jcode_tui` 5.93s, `jcode_app_core` 5.08s,
and the root test binary 5.05s. The app-core updater run
`20261025-232302158-226381` spent 15.39s compiling its 1,731-test binary
to run 31 tests (30 passed, one expected prerelease-comparison failure
pending a concurrent fix). These are project crate/input fanout costs,
not evidence of a justrust cache defect. Potential follow-up: isolate
embedded docs in a leaf crate and split frequently changed config types.
No unrelated project restructuring or measured speedup claim.

## 2026-10-23: Duplicate-final-response TUI regression loop

Run `20261023-213953174-3845270` (`justrust check -p jcode-tui --tests`)
passed in 72.70s locally after the hosted outage fallback. `justrust remote
status` explicitly reported that hosted builds were temporarily unavailable.
The report measured 71.79s compiling, 65 rebuilt units, and 555 fresh units.
The largest local units were `jcode_tui(test)` (30.8s wall, 21.3s attributed),
`jcode_app_core` (13.0s wall), and `jcode_base` (8.1s wall). Other processes
used 12.2 cores on average, with up to 10 concurrent rustc processes and
30.51s machine-wide CPU stalls. A subsequent targeted test run
`20261023-214112947-3854331` spent 50.6s before failing on an unrelated
in-flight `agentgrep::control` import. The actionable costs are oversized
project crates and cross-agent source/dependency coordination, not a measured
justrust cache regression. No project crate restructuring or speedup claim.

## 2026-10-23: Todo completion renderer test iteration

Hosted run `20261023-184855172-2967684` spent 8.35s compiling the
247k-line `jcode_tui` test crate to run seven completion-report tests
(0.15s test execution, 8.67s measured wall). `justrust show` confirms
remote on hosted, 614 fresh units and one rebuilt unit. This is project
crate granularity, not a client failure. A leaf renderer test crate could
reduce this cost, but restructuring is outside the requested UI change.
The final no-edit run `20261023-185129829-2973952` passed all seven tests
with 615 fresh units, 0s compile and 0.3s measured wall. No speedup claim:
these compare an edited compile against a no-edit cache hit.

## 2026-10-19: Correct the durable-GC rollout's legacy lock-mode regression

The initial durable registry change (`68b0dddec`) rejected group-writable lock
inodes without migrating locks created by older shipping agents. Those agents
used `OpenOptions::create` without an explicit mode, so a normal umask of 002
left 0664 files. Existing fresh-lock fixtures missed this upgrade boundary.
At 17:53:01 UTC read-only inspection confirmed the hosted lock directory was
canonical, owned by UID 1000, and 0755, but most existing lock files were 0664.
The exact tigress source root `/home/jeremy/.jcode/scratch/tigress-algorithms-final`
uses `8a8d5dd8f03721c3.lock`, inode 1864977, UID 1000, single link, mode 0664.
The new guard failed before flock; this was not ENOSPC, an active-holder refusal,
or evidence that leases should be bypassed.

Repair the upgrade in place: validate the canonical owned non-group/world-writable
lock directory, open with no-follow/close-on-exec/nonblocking flags, require an
owned regular single-link inode, then fchmod that same open descriptor to 0600
and fsync it. Never unlink, recreate, truncate, or forcibly unlock the file.
Existing shared holders therefore continue excluding exclusive GC on the exact
same inode. Foreign ownership, symlinks, hardlinks and unsafe parents still fail
closed. Lock acquisition now preserves path, error kind and OS error text instead
of collapsing all failures to a generic `None`; actual contention remains
`WouldBlock`, separate from unsafe paths or I/O failures.

Regression fixtures reproduce an old 0664 shared holder and eight simultaneous
migrators, verify unchanged device/inode/content, exclusive GC exclusion until
that holder exits, unsafe-parent/symlink/hardlink rejection without chmod, and a
foreign-principal metadata check. The actual native agent integration now starts
with a legacy 0664 inode, performs authenticated Synced and an enrolled Run, and
checks that the same inode became 0600 before process-restart dry GC. There was
no host blanket chmod, lock deletion, forced unlock, or tenant guard relaxation.
Local outage-bootstrap validation reused the existing stable target: `justrust
check -j1`, all 743 tests passed (0 failed, 3 ignored), `justrust clippy -j1
--all-targets -- -D warnings`, and `cargo fmt -- --check` all passed before
installing the repaired stable client.

## 2026-10-19: Durable authenticated target-GC authority across agent upgrades

The mirrored-target collector previously kept its only registry in an agent's
`Arc::default()`. Upgrading that agent forgot all roots and activity, so even
an explicit later collection could not account for old targets. Automatic GC
also intentionally defaults to dry mode, so replacing the agent did not fix
repeated hosted ENOSPC incidents.

Added versioned private per-UID/canonical-agent-prefix receipts binding exact
canonical root path, device and inode. Only authenticated `Synced` enrolls a
root. Build activity is fsynced in receipt metadata, including a durable unclean
marker. Shared per-root flock covers the whole run; only exclusive idle
completion clears uncertainty. A crashed or failed end update remains excluded
across restarts until a subsequent successful build completes cleanly. Fresh
receipt/grace/process checks run under the exclusive eviction lock, not just
against the agent's initial plan. Source, private caches, external target dirs,
foreign principals, aliases, and unknown historical mirrors are not authority.

`justrust remote gc` is an explicit native command on the machine where invoked,
with dry-plan default even if automatic `JUSTRUST_REMOTE_EVICT=on` is set.
`--apply` authorizes one watermark-bounded pass, never a filesystem source scan
or a reduction of the two-hour grace. Diagnostics separate exact allocated tree
bytes from measured free-space before/after and observed net delta, since
hardlinks and concurrent writes prevent claiming tree size as physical reclaim.
Deletion failures propagate to its exit status. See `docs/remote-target-gc.md`.

Regression coverage includes real native agent `Synced`, an enrolled framed run,
agent exit, and a separate native GC process reading the durable registry and
preserving recent targets. Hermetic tests cover restart/unclean leases,
concurrent agents, late activity between planning and locking, existing flock
holders, recent activity from another agent, foreign scope/permissions,
symlink receipts, replaced root inodes, manifestless monorepos, unknown roots,
and an actual live process cwd. No hosted source export or production target
collection was performed for this repair. Bootstrap checks/tests/lint reuse the
existing stable target locally under the verified hosted ENOSPC outage.
Automatic deletion is not promoted. Pre-registry historical mirrors still need
a normal authenticated sync and full grace before they become eligible.

## 2026-10-19: Hosted disk recovery using native dependency-artifact LRU

At 16:42:41 UTC the hosted root filesystem had 94,478,336 bytes available
(518,822,424,576 used). Verified executable and effective cache scope first:
`/home/ubuntu/.justrust/cache/depcache`, 27.21 GB, 13,042 entries. Ran only:

```sh
env JUSTRUST_DEPCACHE_MAX=16G /home/ubuntu/.justrust/bin/agent/7d88a669bafab7bda02c6f001602ddee/justrust cache --prune
```

The native command exited 0 at 16:42:49 UTC, evicting 6,518 entries
(reported 12.83 GB), leaving 14.34 GB and 6,524 entries. Default 600-second
grace was unchanged, with the native collector's lock, atomic trash rename,
and concurrent-hit recheck. No build was launched, and no private sources,
private caches, active targets, runs, or toolchains were selected for cleanup.
At 16:43:16 UTC `df -B1 /` reported 13,941,305,344 bytes available and
504,975,597,568 used (98%). Net available-space increase was 13,846,827,008
bytes, which includes concurrent host activity rather than being an exact
native eviction byte count. The 16G cap was command-local, not persisted.

## 2026-10-19: Source-sync errors must not terminate shared hosted jobs

At 09:14:32 PDT (16:14:32 UTC), daemon.log recorded rsync exit 24
("file has vanished") for tigress-algorithms-final, immediately followed by
"agent connection closed". Reconnection occurred at 09:14:37. The watcher
unconditionally called drop_link on any push error, killing the shared SSH
agent and unrelated in-flight builds. Source errors now invalidate only the
affected root, requiring a successful full mirror before its next build.
Only a dead transport is discarded; protocol write failures mark it dead.
Rsync exit 24 receives at most two fresh traversal retries with 25 ms pauses,
and a nonzero final result remains an error. Other rsync errors do not retry.

Bootstrap validation used local justrust because exercising the installed
unsafe hosted watcher could kill unrelated jobs: check passed, all 727 tests
passed (3 ignored), including a real watcher/rsync failure with an unrelated
run completing over the preserved framed transport. Retry tests cover
24-to-success, persistent 24, exit 23, and 24 followed by exit 12. Strict
clippy and cargo fmt --check passed, then cargo install --path . --locked
installed the stable client. No build speedup is claimed.

Live acceptance on the normal billed hosted path also passed. A 45-second
unrelated test completed after a second root's 350 tracked files became
unreadable locally: at 09:22:31 daemon PID 2375875 logged rsync exit 23,
without closing or reconnecting its agent. Restoring permissions and changing
the second root's source caused a full mirror at 09:23:13, followed by the
new identifiable latest_source_version_2 test passing. Hosted receipts
`20261019-092216815-2383903` (survivor) and
`20261019-092311807-2388837` (fresh source) both report exit 0.
Upgrade was non-disruptive: at 09:21:01 the old daemon PID
3666693 logged "shutdown requested, finishing 4 run(s) in flight", while
PID 2375875 started the new daemon. No process was force-killed. Run export
was deferred at the coordinator's request while other jobs are active.

## 2026-10-17: Jcode client usage audit focused validation

Hosted OpenRouter check `20261017-031745643-3580854` spent 6.59 s waiting
for the shared build-directory lock out of 6.8 s build wall time. Its full
43-test package run `20261017-031755286-3580853` then took 0.55 s with no
detected waste. Sidecar run `20261017-031822442-3591182` passed 19 tests in
15.64 s build wall time, compiling the base test crate for 7.78 s and the
ordinary base library for 5.62 s after upstream provider/usage changes.
The report estimates 6.1 s avoidable other-target work by using `--lib` for
this filter. Future focused sidecar validation should use that flag and
coordinate hosted jobs. No restructuring or measured speedup is claimed.

## 2026-10-17: Jcode cache-affinity trait change fans out into large test crates

Hosted check `20261017-031242547-3533067` passed in 11.8 s total, including
9.31 s compiling 11 units after the provider-core trait changed. Focused TUI
run `20261017-031306424-3536032` passed 21 tests but compiled the 233k-line TUI
test crate for 10.71 s and 167k-line app-core for 6.09 s. Router regression run
`20261017-031558018-3562866` passed 3 tests, with 9.26 s compiling the 143k-line
base test crate and 3.53 s of build-directory lock wait. Smaller boundary tests
or coordinated validation could reduce this cost. No project restructuring or
speedup is claimed during the cache correctness fix.

## 2026-10-16: Jcode hibernation configuration fan-out and parallel test lock

Hosted run `20261016-184204525-2857383` passed its targeted config test in
22.1 s build wall time, with 19.1 s compiling the oversized base crate after a
config-types change. Justrust reported 20.8 s of overlapping upstream rebuild
cost. Using `--lib` avoids the reported 7.3 s of unnecessary other-target work.
Later `20261016-184315762-2869994` passed two tests with zero compilation but
waited 4.27 s for another worker's build-directory lock. Coordinate validation
commands when practical. These are measurements, not a speedup claim or a reason
to restructure Jcode during the hibernation feature.

## 2026-10-16: hosted selfdev artifact retrieval stalls after successful compilation

Jcode acceptance required a runnable binary. Explicit hosted build
`20261016-142514110-131517` (`justrust build --remote --profile selfdev --bin
jcode`) compiled successfully in 21.8 s on hosted, but never completed artifact
retrieval. The caller remained in an SSH child without a zstd process and the
local executable retained its old version. After 166 s total, the agent cancelled
only its own task and used a documented local selfdev build for isolated-daemon
acceptance. Follow up on artifact transfer timeouts and progress reporting. This
is an observed blocker, not a diagnosed root cause or a claimed fix.

## 2026-10-16: Jcode Sol Ultrafast shared-core edit rebuilds oversized base

Hosted runtime regression run `20261016-142011228-63792` passed both targeted
Ultrafast tests. It spent 9.94 s compiling, including 6.10 s rebuilding the
142k-line `jcode_base` after upstream provider-core/config changes. Justrust
estimated 5.2 s of oversized-crate waste and 1.8 s of low-parallelism cost.
This is a project crate-boundary opportunity, not a justrust correctness bug.
Avoid restructuring Jcode during the provider feature change. A subsequent
fresh-cache full runtime run `20261016-142046358-70207` compiled zero units and
passed 170 tests with 3 ignored in 0.73 s test time, but hosted synchronization
still took 9.224 s of its 10.3 s wall time. These are observations, not a claimed
optimization or measured speedup.

## 2026-10-15: Desktop diagram verification still pays shared-crate rebuild costs

Desktop diagram and macOS input fixes used hosted workspace test run
`20261015-115639936-2736661`: 1,854 tests passed, with 1.53 s of build-directory
lock wait, 2.93 s compilation and 3.22 s tests. The report attributed 2.8 s
to downstream rebuilding after shared harness, UI and SDK changes. Concurrent
focused and workspace verification serialized on the same hosted target.

The native screenshot build `20261015-115650783-2737617` needed a runnable local
artifact and took 9.2 s, including 6.7 s recompiling the 125k-line UI crate and
1.6 s relinking Desktop. Its source-directory build-script input invalidates
the entire UI after small edits. Keep targeted checks serialized, reuse current
binaries with screenshot `--no-build`, and consider a small leaf crate for that
build-script input rather than restructuring Desktop during a bug fix. These
are observed costs and opportunities, not a claimed speedup.

## 2026-10-15: final mapping feedback closes on real outputs and same-input coverage

The final whole-result rerun passed the complete 14-test native/safety harness,
139 offline checks and hosted check/test/clippy. Test run
`20261015-074812985-1877054` passed 709 tests with 3 ignored. Its slowest test
took 9.98 s, while the preceding hosted check spent 103.718 s synchronizing.
Fresh public replay passed 34 readiness cases, 25 flow-v1 cases, 16 retained
native failure cases and 3 combined coverage cases, plus 6 corrupted-output
rejections. Exact requirements, outputs and receipts are linked from
`docs/mapping-feedback-20261015.md`.

On the same original cold measurement bytes, the task DAG attaches 4,352 CPU
segments / 53,521,730 ns, while invocation flow assigns all 36,736 segments /
513,291,338 ns. The added 32,384 task-null segments conserve exactly
459,769,608 ns and do not acquire invented pool ancestry. A fresh capture
independently repeats this diagnostic coverage gain. This is better mapping,
not evidence of faster compilation, full causal span or a thread ceiling.

## 2026-10-15: actual latch handoffs qualify waits without pretending unknown gaps are zero

The immutable-compiler harness passed all 14 tests in 475.022 s: seven actual-source
tests and seven safety tests.
Cold retained 74,024 flow nodes, 73,538 native edges and 18,199 invocation scopes,
assigning all 36,736 measured CPU segments. Actual latch provenance qualified
138 of 139 encounters, including all 134 registered waits. The remaining
already-ready encounter honestly remained unbound because diagnostic publication
finalization can race its consumer. No records were dropped, yet aggregate and
wait completeness correctly stayed false. Warm observed no encounters; root6
mixed qualified 180, including 170 registered and 10 already-ready encounters.

The cold known CPU chain cost 20.787135 ms, including 17.556408 ms `typeck_root`.
Warm's 16.342947 ms chain includes 5.991975 ms `resolver_for_lowering_raw` and
2.470884 ms `lower_to_hir`. These locate concrete query-execution costs, not
immutable serial work or a useful-thread ceiling. Cold still has 511 unknown
cuts. Visibility, wake-to-consume work, cache publication and uninstrumented
scheduler boundaries are not fabricated zero-weight prerequisites. No speedup
or full work/span/parallelism is claimed.

Native source-stable tests passed 258 with 28 ignored plus 28 doctests. Hosted
client run `20261015-070226556-1549304` passed 709 with 3 ignored, and final check
and strict all-target clippy passed. Three cited runs are exported under
`data/runs/latch-readiness-20261015/`. The combined offline mapping/oracle/control
suite passed 139 tests. Sixteen separate public guard-packet cases cover honest
partial failure/race handling without fabricating missing query authority.
See `docs/latch-readiness-20261015.md` and exact compact analyzer reports in
`bench/results/latch-readiness-20261015/`. Expanded independent public replay
passed all 34 cases in 184.34 s, preserving all 130 original files and the frozen
client. Its authority-withdrawal controls use the actual cold 139-encounter
inventory, not the zero-wait warm capture. Exact full analyzer stdout is checked
against independently byte-preserved neutral snapshots. The receipt is
`bench/results/20261015-latch-readiness-public-acceptance.json`. The invalid/noisy
A2 baseline still blocks performance promotion, not this correctness result.

## 2026-10-15: actual invocation flow connects named query CPU without inventing span

The new native pool5/flow1 diagnostic passed all 12 real-source compile/link/
execute/analyzer tests against an immutable coherent compiler and installed
client. Cold retained 73,038 nodes, 72,550 explicit edges and 18,199 invocation
scopes, assigning all 36,443 measured CPU segments exactly once. Its known
CPU-chain witness cost 44.280185 ms, including 37.521239 ms of `typeck_root`.
Warm retained 1,838 invocation scopes for 1,833 admitted jobs, demonstrating
actual reentry rather than whole-job contraction. Its 15.772376 ms known chain
includes resolver 5.900361 ms and HIR lowering 2.510948 ms.

These are observed-chain attribution facts, not compile-time speed comparisons
or a full critical path. Cold has 175 explicit unknown cuts. Missing latch
readiness, visibility/wake work and scheduler boundaries are not zeroed or
inferred from job completion. Full work/span/parallelism stay unavailable.
Actual bounded-capacity and allocator-exhaustion packets passed partial/no-
numeric replay; unsupported nonzero clock declarations remained hard refusals.
Native tests passed 247 with 28 ignored plus 28 doctests. Hosted client run
`20261015-060855169-1146356` passed 688 with 3 ignored, plus check/strict clippy.
See `docs/execution-flow-20261015.md`, compact reports in
`bench/results/execution-flow-20261015/`, and dedicated run exports in
`data/runs/execution-flow-20261015/`. Paired performance promotion remains blocked.

## 2026-10-15: retained native task DAG explains only a minority of cold CPU

The installed explicit task-DAG analysis independently replayed the original
accepted itoa cohort. Cold had 11 genuine tasks and 12 native fork/join edges,
54.787070 ms attached CPU, and a 45.866307 ms longest recorded task CPU chain.
It excluded 496.824661 ms task-null named query CPU plus 4.015578 ms opaque CPU.
Warm attached 33.195573 ms with a 31.137137 ms recorded chain. These are partial
native-edge diagnostics, not full span, a speedup, or a useful-thread ceiling.
The cold coverage gap is why invocation-level native query flow must follow,
rather than stopping at the task topology or summing query categories.

Actual public CLI/analyzer, exact private artifact, original-byte oracle,
missing-query partial authority and collision controls passed with six original
files unchanged. Hosted run `20261015-055126359-1004947` passed 664 tests with
3 ignored, with check and strict clippy passing. Dedicated exports are in
`data/runs/task-dag-20261015/`; accepted compact evidence and details are in
`bench/results/task-dag-20261015/` and `docs/observed-task-dag-20261015.md`.

## 2026-10-15: named query execution CPU now has native authority

Explicit admitted query execution ownership is independent of pool-task identity.
Real cold/warm itoa captures reconciled 532.930074/19.811304 ms of named query
CPU against original admissions, with complete retained timing and reciprocal
clocks. Cold `typeck_root` accounted for 206.496525 ms, warm
`resolver_for_lowering_raw` for 6.106205 ms. These diagnostic costs are not a
critical path or a matched performance comparison. Full work/span remain unknown.

Native tests passed 235 with 28 ignored. All eight actual-source compile/link/
execute/analyzer tests passed. Hosted client run `20261015-052358578-808947`
passed 655 tests with 3 ignored, and check/strict clippy passed. Exported runs
are in `data/runs/query-cpu-20261015/2026-10.jsonl`. See
`docs/query-cpu-ownership-20261015.md` and its committed compact reports.
The comprehensive performance gate remains blocked by the ineligible A2 series,
not silently replaced by these successful correctness diagnostics.

## 2026-10-15: real frontend ownership coverage, not a threading ceiling

On unchanged itoa 1.0.18 incremental compilation, actual task ownership increased
from 3,028 to 17,024 of 19,596 captured points. Read-derived edges remain 15.
An explicit million-event cold capture retained 911,210 read-slice events and
485,256 binding points with zero loss, but direct reads were all dep-graph
`disabled`, so loss-free capture is not full semantic coverage. CPU work and
weighted span remain unknown. No compile speedup or 96-core ceiling is claimed.
See `docs/frontend-mapping-results-20261015.md` and its committed reports.
Hosted validation: `20261014-180011428-4084443` reported 602 passed after an
unrelated parallel ETXTBSY test passed on isolated retry. Native gates: 205
passed, 28 ignored, plus real cold/incremental artifact and dispatch witnesses.

## 2026-10-14: capped semantic-read recording no longer waits for its event mutex

The real large native fixture attempts over 30 million events after retaining
its bounded 100,000-event prefix. Previously every rejected event still took
the event mutex. A monotonic cap flag now skips that lock without skipping
independent ID allocation or loss accounting. Accepted append ordering and
reservation retry behavior remain under the original mutex. A separate loss
wrap latch prevents a wrapped u64 loss count from restoring completeness.
Fifteen real-module tests pass, including a lock-held bypass witness, racing
cap transitions, interruption, poison accounting and forced loss/ID wrap.

The initial separate sweep showed a one-thread mixed regression
0.699351s -> 0.925979s, which was retained. A 48-sample explicit ABBA follow-up
with fixed CPU affinity observed mixed one-thread medians 0.687678s/0.389319s
and eight-thread medians 5.117650s/2.100425s for baseline/candidate. Accepted
100k-event controls overlapped (0.004890s/0.004872s). No quiet-system guarantee
or comprehensive suite comparison exists, so these are focused diagnostics,
not a general/native speedup or default-promotion claim. Source/binary/build
provenance, all raw samples, regressions and reproduction are retained in
`bench/query-read-cap`.

Actual frozen compiler/std/proc_macro build and all seven native gates passed
in task `773938x4fq`. Source-matched baseline/candidate observations are in
`data/runs/query-read-cap-20261014/native-{baseline,candidate}.json`. The native
candidate still reports large read loss and incomplete read mapping; its small
incremental fixture has zero loss, 5870 resolved job edges and executed output
42. Native single-sample timings include both dispatch and read-recorder
changes and do not establish attribution. CPU/span and full-pipeline claims
remain disabled. Future explicit query-to-task point binding is design only
in `docs/native-query-task-binding-plan.md`.

## 2026-10-14: helper dispatch must not inherit logical task ownership

Real one-thread pool controls exposed existing false lineage: unrelated opaque
scope work inherited a helping root/child, and its nested join forked and
mutated the helping root. Baseline `20261014-114100301-42923` failed all three
controls. Capture-enabled worker dispatch now temporarily hides/restores the
exact task slot with a non-Send RAII guard, without inventing dispatch edges.
Ten new regressions cover scope/nested/panic/FIFO/CountLatch/broadcast/stolen/
inline/cross-pool paths. Final pool run `20261014-114616930-102856` passed 95
with 26 ignored; strict lint `20261014-114616920-102851` passed. All cited slim
summaries, including the failure, are in `data/runs/query-read-cap-20261014`.
The actual rebuilt compiler plus std/proc_macro and seven native admission,
wait/read/pool, binding and macro neutrality tests passed in task `773938x4fq`.
This native gate used the combined dispatch and capped-read candidate, not an
isolated performance experiment. Manual raw contexts on unobserved registries
and recorder-failure inline ownership remain explicit limits. Full mapping,
CPU/span and performance promotion remain unproven. See
`docs/native-pool-dispatch-isolation-20261014.md`.

## 2026-10-14: native causal mapping passes scoped acceptance, full graph still partial

Actual rebuilt stage1 wait/read/pool acceptance passed: serial zero waits,
331 waits at 4 threads and 1253 at 8 threads, plus real cycle and poison
notifications. The small incremental read fixture resolved 5870 job edges
with zero loss. Large contention semantic-read capture overflowed its bounded
inventory and remains explicitly partial. Executable off/on/off identity and
error diagnostics were preserved. No CPU/span or full-pipeline claim is made.

Client full hosted test run `20261014-104130902-3437189` passed 540 tests,
with check `20261014-104125366-3435193` and strict all-target lint
`20261014-104152011-3434666`. Exported summaries are committed under
`data/runs/causal-map-20261014`. Details and bootstrap evidence are in
`docs/native-causal-map-results-20261014.md`.

Scoped sequential overhead medians for the same frozen fixture were
0.1617s off / 0.2862s on at one frontend thread and 0.1233s off / 0.3007s
on at four threads. Quiet provenance is unknown and the baseline showed
an on/off inversion, so these are recorder-cost diagnostics, not speedup
or comprehensive promotion evidence. Capture overhead needs optimization.
The real proc-macro probe caught a private-argv recording-neutrality bug.
Descriptor-only transport and an explicit unsafe native process-entry ownership
boundary fixed it. After a coherent frozen rebuild, exact macro-visible argv,
program output and executable SHA256 off/on matched, and genuine binder plus
admission gates passed. The accepted source cohort is pushed as `d3a5319b9`.
Full pipeline coverage and a theoretical parallelism ceiling remain unproven.

## 2026-10-14: toolchain cleanup completed, default promotion blocked

Owned release compiler/Cargo `jr-910e39a6c3ef` was built and staged without
changing the existing default. Root check, 457 tests (3 ignored), strict
Clippy, LLVM smoke and the real Desktop fps_counter edit passed. Legacy
checkouts were retired reversibly, with all Git/build data preserved.

The comprehensive seed-1, 40-point promotion baseline completed under a
frozen client, but all 40 observations were noisy. The reporter also flags
outcome-provenance gaps and a suspect rerun cache artifact. Baseline run
`20261014-093757263-2242292135` detected foreign compiler activity; candidate
run `20261014-101956044-2917310751` did too. Candidate was stopped after 15
noisy points because the baseline cannot support a valid comparison.
`bench/suite.py compare` blocks the pair. No speed or default-promotion claim
is made. Full baseline, explicit partial candidate and all 55 observed run
receipts are committed. See `docs/toolchain-cleanup-promotion-20261014.md`.

## 2026-10-14: native admitted-query lifecycle acceptance

The owned stage1 compiler, std and libtest were rebuilt, then the opt-in
`-Zjr-query-graph` capture passed native serial/parallel, failure and unwritable
sink acceptance. Compiling the production observer module recorded 47,517
admitted jobs and 45,130 descriptive parent edges with no assessment issues;
all 16 observer tests passed. This proves the declared admitted-job lifecycle
slice, not cache/read/wait dependencies, Cargo binding or whole-pipeline coverage.
The final client suite passed 476 tests (3 ignored), run
`20261014-093727237-2236830`; consumer and strict lint records are exported under
`data/runs/query-admission-20261014/`. Native bootstrap/direct rustc commands
have no invented justrust run IDs. Details and assessment are in
`docs/execution-graph-query-admission.md` and
`bench/results/query-admission-20261014/native-source-assessment.json`.

Warm executable bytes were identical with capture off/off/on/off. Incremental
rlib container bytes differed even off/off because per-invocation temporary CGU
names are serialized in private `lib.rmeta-link`; metadata and object payloads
were stable. No universal rlib byte-neutrality or performance claim is made.

## 2026-10-14: hosted target preflight for direct WASM checks

Direct default `justrust check --target wasm32-unknown-unknown` also reached
hosted nightly without libcore, failing E0463 in 8.23s in run
`20261014-094525777-2376582`. Hosted routing now preflights explicit CLI or
environment targets: unsupported non-native targets automatically stay local
with recording and a clear explanation, before submitting a tenant build.
Forced remote requests fail clearly instead of silently changing location.
Native/no-target requests retain hosted routing. No billing or isolation
rules are changed, and no blanket local-mode setting is required.

Follow-up check `20261014-094741293-2416052` passed, library tests
`20261014-094754162-2420895` passed 443 with 3 ignored, and all-target clippy
with warnings denied `20261014-094851654-2433211` passed. Hosted connection
timed out during this validation, causing the existing automatic local
fallback. The check took 12.1s, tests 56.9s including 29.43s compilation,
and clippy 29.1s under heavy CPU contention. Reinstalled successfully with
`cargo install --path . --locked --offline`. These are correctness checks,
not performance comparisons.

The coordinating agent verified installed default direct WASM check
`20261014-095101889-2462467`, exit 0 in 0.6s, with the unsupported-hosted
local recorded route explicitly recorded. Forced `--remote` WASM check
exited 1 with the clear preflight diagnostic before submission. Direct
release WASM build `20261014-094610003-2391409` also passed (4.5s client
wall). Follow-up evidence is in
`data/runs/wasm-target-compatibility-20261014/hosted-preflight.jsonl`.

## 2026-10-14: target-compatible recorded WebAssembly builds

The cargo proxy selected nightly-2026-10-08 without wasm32-unknown-unknown
libcore, producing E0463 even though system rustc 1.98.1 had the target
installed. Explicit `--target` (including repeated/equal forms) and
`CARGO_BUILD_TARGET` now probe the selected compiler's own target libraries.
When a compatible system compiler exists, justrust selects the system
Cargo/compiler path together and keeps recording. Required toolchain pins
instead fail with an actionable diagnostic. Custom JSON targets and explicit
build-std retain their original compiler. Config-file-only build.target
selection remains a follow-up, not covered by this scoped fix.

Hosted check `20261014-094008294-2277685` passed, recorder regression run
`20261014-094131170-2303748` passed 16 tests, all-target clippy with warnings
denied `20261014-094136594-2304919` passed, and library test run
`20261014-094256324-2322841` passed 442 tests with 3 ignored. Reinstalled with
`cargo install --path . --locked`. The coordinating website agent verified
normal `trunk build --release` through the cargo proxy, exit 0 in 38.62s.
Recorded WASM build runs `20261014-094322090-2328173` and
`20261014-094322090-2328174` both exited 0, retaining recording and selecting
the target-compatible system toolchain. The second concurrent build waited
35.633s on Cargo locks. No website files were changed by this fix. This is a
correctness recovery, not a benchmark speedup claim. Redacted summaries are
in `data/runs/wasm-target-compatibility-20261014/2026-10.jsonl`.

## 2026-10-14: native expected-owner coverage gate

Run `20261014-083519591-872352` validated one independently declared native
BuildRunner scheduler capture, containing 86 nodes and 126 edges, with no
coverage issues. The library build took 33.741s under contention, with 11
dependency-cache hits and 62 misses after changed configuration. Unrelated
processes averaged about nine cores of use. This is correctness/coverage
validation, not a performance baseline or speedup claim. Capture off/on/off
fixture runs `20261014-083632675-893606`, `20261014-083633187-893812` and
`20261014-083633383-893845` preserved binary hashes and reused all artifacts
on both switches. Missing child receipts, missing declaration journals and
contradictory execute lifecycles fail coverage validation. Other authorities
are explicitly excluded, not silently covered. Full pipeline and child-result
agreement remain unverified. Evidence: `docs/execution-graph-owner-acceptance-20261014.md`.

## 2026-10-14: end-to-end profiling diagnostic, rejected baseline

All 40 synth seed-1 points were attempted. The final gate rejected mixed
client binaries after a concurrent reinstall. Across 32 noisy hosted samples,
recorded wait was 51.93% outside host execution, 29.01% compilation and
16.95% tests. Sync was a nested 314.335s of 688.580s total. Run
`20261014-074749669-140934645` spent 120.048s syncing versus 11.123s host
work, matching daemon logs dominated by sync lock wait behind a large
unrelated source mirror. This is diagnostic evidence, not a speed claim or
accepted session score. See `docs/e2e-profile-20261014.md` and its rejected
checkpoint and isolated exported phase receipts. Compiler pass attribution
was unavailable. Preserve the identity rejection and investigate sync
head-of-line blocking before using this run as an optimization baseline.

## 2026-10-14: final owned-source archive acceptance

Fresh verification of frozen migration commit `6d898a9ea` is mapped in
`docs/toolchain-vendoring-verification.md`, with separate fidelity, native,
root and Cargo evidence under `data/toolchain-vendoring/`. Root archive
check `20261014-074729328-178399` passed with 4.1s recorded work but
137.4s initial hosted sync. Tests `20261014-074954275-215740` passed all
439 tests (3 ignored), and strict clippy `20261014-075034482-177397` passed.
The native compiler/std build, 289 bootstrap tests, 23 Python tests, and
six backend runtime tests passed. A clean HEAD archive reproduced the
built compiler identity across all 5,591 selected files/symlinks and
1,075 directories. Eighty license/attribution files and all 19 original
CRLF/mixed entries retained their exact original blobs and modes.

After the complete matrix existed in `f01b651db`, the final indexed snapshots
were rerun through the entire mapped loop. Root 439 tests, source tests 10,
hosted Cargo identity 4 and version 31, native bootstrap 289, Python 23,
actual compiler/std/Cargo builds and six runtime tests all passed. The final
indexed-project hosted version run `20261014-081644459-561932` preserved
fixture discovery, unlike the separately documented no-Git mirror failure.
Native job `971799pstx` took 1570.93s with explicit package-cache and another
process's build-directory lock waits; this is a contention observation, not
an implementation speed measurement. Exact returned outputs and a fresh
whole-tree/identity audit are in `data/toolchain-vendoring/2026-10-14-postmap.txt`.

Frozen Cargo binary build `20261014-074317981-128533` and hosted identity
acceptance `20261014-074524962-151355` passed. Hosted archive version run
`20261014-074614928-128514` failed before tests: no-Git source mirroring
excluded the tracked test fixture directories `cargo_add/target` and
`cargo_remove/target`. Those source blobs are intact in the actual archive.
The final version acceptance uses a documented local exception for that
unsupported archive mirror, with the failed hosted receipt retained.
Upload batching and archive fixture discovery are concrete integration
opportunities, not speedups demonstrated by this source migration.
Concurrent execution-graph edits were isolated from the frozen acceptance
and were neither reverted nor included in this evidence-only change.

## 2026-10-14: compiler and Cargo become owned source

Both forks are ordinary monorepo source, with shared bootstrap Cargo and
vendored backtrace. Native source-content identities distinguish dirty edits
and archive builds without using the outer Git commit as upstream identity.
The frozen CI LLVM artifact commit is independent. OwnedSource is not an
upstream release tarball: treating it as one accidentally selected the dist
profile and changed bootstrap stage defaults. Missing optional-source paths
are registered from the explicit source manifest rather than hidden by dummy
checkout directories. After these fixes, all 289 native bootstrap tests passed
without changing any expected snapshots, including nine source-identity tests
and six CI/local compiler-rt source-selection tests. The real dev compiler,
Cranelift and optimized std build then passed without an LLVM source checkout.
Metadata checking and all three arithmetic/std/thread smoke tests passed on
both LLVM and Cranelift. Durable redacted native stdout/stderr is retained in
`data/toolchain-vendoring/2026-10-14-native.txt`.

A post-migration source audit found our failed inline snapshot run's generated
`.pending-snap` journal was still an identity input. Exclude that generated
suffix alongside `.snap.new` and test both, without changing actual compiler
flags or expected snapshots. The clean rerun passed nine identity tests,
all 289 bootstrap tests, the real dev compiler/optimized std build, and the
three smoke tests on each backend. Final source identity is
`910e39a6c3ef8b6fe54663a4f513213687b9bf4b`. A staged-tree Git archive matched
all 5,591 selected files/symlinks and 1,075 directory entries and produced the
same identity. There were no included untracked files or empty directories.

Hosted Cargo identity acceptance `20261014-065210842-3760408` passed all four
tests, including credential helper and Windows manifest changes. The actual
Cargo binary build `20261014-063157746-3596184` passed locally in 95.73s using
justrust's documented debug-build local route. Hosted version acceptance
`20261014-064302039-3684155` passed 31 tests. A broader accidental version
substring filter (`20261014-064035177-3661945`) passed 167 tests but failed one
cross-target test because i686 std was unavailable, not a complete Cargo
suite pass. Final Cargo binary build `20261014-070837591-3919225` and final
31-test hosted version acceptance `20261014-070929167-3919209` passed against
the complete source-fingerprint inputs. Receipts are isolated under
data/runs/xps14-toolchain-vendoring.

These are migration correctness observations, not a measured compile-time
improvement or an experimental toolchain default promotion. Full migration
provenance, build commands and compiler acceptance are in
`docs/toolchain-vendoring.md`.

## 2026-10-14: incremental local output survives interrupted builds

The local recorder now streams existing OutputLine JSONL into a 64 KiB
buffered journal, flushing on nominal one-second sampler ticks and at
completion. It adds no network/raw-log telemetry path and does not change
cargo's environment, fingerprint, or exit status. Successful completion no
longer truncates and rewrites output.jsonl. A failed journal is repaired from
the existing summary capture through staging and atomic rename. Failed repair
preserves the persisted prefix and reports no complete logs. The pre-existing
summary Vec remains, but the journal adds only a bounded buffer rather than a
second unbounded output copy.

Installed binary acceptance on isolated scratch Rust fixtures:

- SIGKILL run `20261014-044013076-2711739`: the actual test printed its marker,
  the sampler persisted it, then the process group was killed. Exact-run-id
  `justrust log` recovered the marker without a summary or index entry.
- Successful output-heavy run `20261014-044014241-2711809` saved exactly 3,000
  unique diagnostic lines and one final marker, without duplication.
- A 64 KiB RLIMIT_FSIZE with SIGXFSZ ignored and the resident daemon disabled
  caused real journal and staging writes to fail. The test still returned
  cargo exit 0 and printed its final marker. Recording failure was visible.
  Log and summary persistence failures also emit one bounded diagnostic per
  failed category through the lifecycle owner's API: fixed recording_persist
  stage, log/summary detail, actual exit, and validated run ID only. No error
  chain or path is accepted by that API.
- Five regression tests cover exact JSONL, batching/bounded buffer with large
  lines, disabled-on-failure/no retry, unavailable paths, and atomic repair
  preserving an existing prefix. Hosted check/full test/clippy passed on the
  integrated snapshot, coordinated with the lifecycle owner.

Focused overhead only, not a compile speedup claim or a comprehensive gate:
10 warm local synthetic_pass commands, one discarded warmup, telemetry and
resident daemon disabled identically. Before median **82.62 ms**, mean 85.54
ms; after median **90.99 ms**, mean 91.92 ms. Observed median increase **8.37
ms** in this short sequential noisy sample. Samples and run IDs are in
`bench/results/journal-overhead-{before,after}-20261014.json`.
The broad session-weighted suite was not run for this visibility-only change.

Limitations: hard kill may lose buffered output since the last tick, with
scheduling/sampler and I/O delays extending the nominal one-second window.
An unterminated line held by the tee, or output held by a test-pool block,
may not have reached the journal. Flush means readable local bytes, not fsync
or power-loss durability. Running heartbeat availability means a persisted
local prefix; terminal availability requires complete successful persistence.
No public alpha release is authorized by these checks alone.

## 2026-10-14: actual scheduled sharing fleet runtime accepted

The naturally scheduled 09:00 UTC producer selected the accepted native-safe
0a19 client and corrected full JR/nightly compiler pins. Actual log sections
exercised all four default/debug0 seed cells and both public-workspace compiler
passes. Its terminal receipt reports complete/succeeded at 10:14:49 UTC with
zero optional failures. Publication reported 81 new bundle files, zero public
build-script runs and a final index of 19,994 sorted unique artifact keys.
The instance powered off and was observed terminated before its three-hour
deadline. This closes the runtime observation gate, not just a config check.

Private cloud ca2e957 records the version-pinned status/log/index, scheduler
join and 12-file archive manifest. Independent review and final lossless hash,
matrix, terminal-pin and credential-exclusion checks passed. Raw CloudTrail
session credentials were not archived. Public acceptance and limitations:
`docs/sharing-fleet-runtime-20261014.md`. No normal CLI run ID is invented for
this external provider observation, and no performance ratio is claimed.

Public native script sharing and genuine first-new-tenant bootstrap remain
separate unpassed gates. Registry eager seeding stays off. This does not claim
all 81 bundles individually restored or a complete parallelism speedup.

## 2026-10-14: comprehensive README comparison completed, speed gate blocked

The frozen-client seed-1 workload observed all 40 points across stock local,
justrust local, and the remote stack. Receipt
`bench/results/versus-synth1-20261014-040027-8e60305db025-dirty.json`
retains every sample, including errors. All 40 local pairs were measured, but
25 points contain hosted errors, only seven remote samples confirm hosted
execution, and background load makes the samples noisy. Forced-local fallbacks
are not hosted evidence. The gate also flags outcome provenance for point 37.
No comprehensive speedup or default promotion is claimed. An earlier run was
rejected because the installed client changed, so the completed attempt used
a frozen executable. Detailed limitations and reproduction are in
`docs/readme-synth-benchmark-20261014.md`, with 53 available recordings exported
to `data/runs/xps14/readme-synth1-20261014.jsonl`. README factors instead use
the separate successful three-round pinned uv comparison, not this blocked run.

## 2026-10-14: diagnostic telemetry covers pre-result failures

Local recorder setup failures, required-toolchain refusals, and cargo exec
failures now emit bounded `build_failure` events without raw error chains.
Remote setup/admission/transport/artifact/summary failures also retain their
stage, elapsed time and any known exit code without changing the original
Result, rerun, billing, or build-exit semantics.

Actual installed-binary acceptance on an isolated local Rust workspace passed:
`20261014-040142704-2318604` compiled the fixture, an unrecorded check reported
`Fresh`, and `20261014-040143226-2318732` compiled zero units. A required system
toolchain refusal preserved exit 1 and queued exactly one `toolchain_required`
event. `telemetry off` removed unsent fixture events, and the subsequent check
`20261014-040143596-2318864` queued no telemetry. The three summaries are exported
under `data/runs/xps14-telemetry-acceptance/`. Local routing was intentional for
this artifact-sharing and opt-out acceptance, not a hosted-build substitute.

The focused recorder privacy test and ten remote tests passed, as did check and
warning-free all-target clippy. Plain hosted validation initially encountered a live backend 404 for
`POST /internal/builds/charge` and correctly fell back locally. After restoring
the prior backend route implementation, charged hosted check
`20261014-040706043-2400213` passed. Actual installed client run
`20261014-040753300-2405164` was verified in production D1 with a trusted account
link, stable event ID and sampling metrics. Final full tests passed 427 tests
(3 ignored), with warning-free clippy. `docs/telemetry-acceptance-20261014.md`
records the complete evidence and remaining limits. This is correctness/coverage
work, not a compile-speedup claim.


## 2026-10-14: Jcode custom search configuration rebuild cascade

Hosted run `20261013-235133714-2340696` spent 24.8s in compilation/testing
(30.7s client wall) running two default-config unit tests after changing
`jcode-config-types`. `jcode_base` took 9.25s and its test unit 12.78s.
The report attributed approximately 22.3s to downstream rebuilds and 9.8s
to building unnecessary non-lib targets. Subsequent focused `--lib` run
`20261013-235346425-2355583` ran one custom-search TOML test in 7.0s build
wall (13.8s client), with the base test unit taking 6.53s. These are different
edits/cache states, not a controlled speedup comparison. Use `--lib` for these
unit filters. Splitting frequently edited config types/base tests is a project
architecture opportunity, not part of the scoped search-provider change.

## 2026-10-14: actual compiler invocation IDs join query providers and waits

The new offline graph/profile/mapping tools, committed as `4b7e8bd`, pass all
164 benchmark-package tests and eight final actual public CLI checks. The
same-session Desktop capture `20261013-224631463-1915834` has 1,052,237 saved
nodes, 6,496,600 dependency edges, and 973,105 observed query-provider records
that all join their numeric saved IDs. 79,132 nodes remain untimed. Its 151
QueryBlocked records total 129.890026 ms of thread waits, and 55 observed
caller-to-producer pairs lack a saved read edge. CPU work and execution span
remain null, not inferred from occupancy, read-graph depth or frontiers.

This was a cold-target, warm-shared-cache hosted **check**, not a build or test:
19.987 seconds recorded / 20.632 seconds client, UI unit 3.607524395 seconds
and 10.048405 reported CPU seconds. The resolver-for-lowering provider took
396.254078 ms of exclusive elapsed time, making it an investigation target,
not an established serial fraction or safe parallelization. The earlier
all-event check `20261013-224426718-1907396` had different cache misses and a
1.12 GB profile beyond our decoder cap, so their ratio is not a speedup.
Tiny one/four-thread captures `20261013-223840760-1863795` and
`20261013-223842465-1863794` preserve sparse IDs and explicitly unresolved
virtual labels. The failed first local debug-build harness invocation
`20261013-223745790-1859236` is retained separately, not a hosted failure.

See `docs/compiler-query-graph-20261014.md` and its compact result/acceptance
files for exact pins, format boundaries, independent actual-pair acceptance,
per-requirement observations, and missing causal/CPU instrumentation. No
compiler/default/permissions/tenant changes or performance promotion were made.

## 2026-10-14: native rollout uses a frozen installation source

The native-platform client was validated from the exact committed main snapshot
`cbeea7d74f122d31d8d40854d9c744bcde6edfc1`, rather than installing unrelated
in-progress remote sync edits. Local run `20261013-211836511-1314515` passed
395 tests (zero failed, three ignored, ten binaries) in 37.6 seconds. Its
slowest test, `depcache_prebuilt::tests::public_publish_fetch_allows_bundle_above_worker_cap`,
accounted for 19.50 seconds. Warning-free all-target clippy passed in
`20261013-211914198-1323534` in 19.3 seconds. These are validation durations,
not a before/after speedup claim. The separate target directory intentionally
isolates installation evidence from mutable peer work, not a routing default.

The global CLI installation completed at October 14, 2026, 04:21:34 UTC,
with binary SHA256 `57d39f0bcc6bdac593942cbf835d03f3152e6a43d3dfad74f92010e8a0e8fcee`.
Native Windows/macOS acceptance is still separate from this Linux/client
validation. Cloud negotiation is deployed, but native images and real target
build/test/artifact round trips must pass before advertising native support.

Data: `~/.jcode/sessions` on one developer machine (16-thread Core Ultra X9,
62 GiB). Covers Jcode and Jcode Desktop work from late August to early October
2026. Collected with `justrust history`. "Foreground" means the agent was
blocked waiting on the tool call. Calls over 30 minutes (hung processes) are
excluded.

## 2026-10-13: verified shared toolchain prefixes, inactive JR downloads

Maintainer-published HTTPS prefixes now support explicit JR installation/pins
and official nightly prefixes with a compatible shared static jr-cargo. A
bounded immutable-ID manifest anchors the archive SHA-256 and exact rustc/cargo
identities. Extraction rejects links, extensions, traversal, duplicate paths,
and publisher-supplied markers. Downloads are serialized and atomically renamed,
and JR always remains staged with `installed_at: 0`, never a new default.

Real fresh scratch-home downloads verified JR `jr-b7907f1fe8f7` (457865116
compressed bytes, 41.3s observed install) and stock `nightly-2026-10-08`
(618894666 bytes, 58.6s). Both shipped static jr-cargo `125bdc8c1` on upstream
`f3865b2a4`. Both passed a real check, three tests, clippy, and an expected compile
failure, with unchanged marker inode on repeat install and no unpinned default
promotion. Test runs `20261013-194432826-732012` and
`20261013-194913404-764117` plus six other acceptance summaries are committed in
`data/runs/xps14-toolchain-download/`. Full local correctness validation
`20261013-194746505-749789` passed 380 tests, including cross-process installation
locking. Details, all IDs, protocol, and limitations are in
`docs/toolchain-distribution.md`.

The real nightly archive caught a missing `libexec` root for rust-analyzer's
proc-macro server. Verified official fallback worked, and the legitimate root
was added with a regression fixture. These are functionality/security findings,
not a measured build-speed gain. No current host or default compiler was changed.

## 2026-10-13: Claude stale-provider-failure regression validation

Hosted check `20261013-073407319-605237` ran remotely and took 10.33s,
with 9.95s compiling the 165k-line `jcode_app_core` test unit. Test run
`20261013-073441261-610471` encountered a verified hosted connection reset
(`curl: (56) Recv failure: Connection reset by peer`) and fell back locally.
It took 71.1s, including 50.47s for the 1570-test app-core binary and 16.54s
for `jcode_base`, to run 21 provider-control tests. The warm rerun
`20261013-073631256-628246` took 3.2s with 466 fresh units. Large test units
and concurrent upstream invalidation dominate the cold edit loop. No crate
restructuring or routing change was made, and these different conditions are
not a measured implementation speedup.

## 2026-10-13: Jcode OAuth retry regression edit loop

Whole-result verification follow-up: run `20261013-074413945-725744` (`justrust check` across five changed Jcode packages with `--tests`) passed in 134.51s, but automatically selected local because of recent hosted failure. `justrust show` reports 132.46s compiling, 90.99s machine-wide CPU stalls, other processes averaging 14.1 of 16 cores, up to 19 competing rustc processes, and `docs` triggering app-core rebuilds. No project restructuring was performed. Hosted provider-core run `20261013-074628834-748806` took 15.9s with 13.948s sync. Hosted TUI rejection run `20261013-074658699-755366` took 47.6s with 25.547s sync. These are observations, not a speedup claim. The corrected `credential_failure_breaker` filter encountered two hosted mid-run `No such file or directory (os error 2)` failures at 14:48:58 and 14:49:09 UTC. The hosted owner was notified. The verified-outage local rerun `20261013-074936502-779903` passed all three tests in 18.4s. Full requirement/run ledger: `/home/jeremy/.jcode/scratch/oauth-acceptance.DlQN9r/whole-result.md`.


Hosted run `20261013-071601854-429318` spent 10.26s of 13.2s waiting for
another Cargo build-directory lock before a 2.51s provider-core test compile.
Coordinating one owner for sequential checks/tests avoided that lock wait in
subsequent runs. Run `20261013-072151410-478023` then spent 16.37s compiling
(`jcode_app_core` 6.98s, `jcode_tui` test 9.39s) to execute three tests in
0.26s after an unrelated concurrent app-core edit. This is measured upstream
invalidation and a large TUI test unit, not a reason to change routing or claim
a speedup. Consider future leaf-crate extraction only with project approval.

Also, TUI tests included from files such as `remote_events_reload_04.rs` are
flattened into `tui::app::tests`. Filtering on the included filename selected
zero tests in runs `20261013-072033291-465912` and
`20261013-072058102-470040`. Use actual test names and verify nonzero counts.

## 2026-10-13: Native MCP verification build costs

Jcode MCP validation showed existing contention and large-unit costs, not a
reason to restructure the user's project during the feature implementation.
Hosted check `20261013-071447869-422258` took 88.4s overall, including 66.5s
source sync and 18.6s measured Cargo/check time. Hosted base MCP tests
`20261013-071706772-437129` took 40.0s overall, with 13.38s of build-directory
lock wait, 19.34s compilation, and 112 passing tests in 1.10s. The later
113-test run `20261013-072154290-478171` still waited 13.51s for the lock even
though a local-cache hit avoided roughly 11.4s of rustc.

A forced hosted CLI test at 14:38:51 UTC failed after 5.4s with
`remote build failed mid-run: No such file or directory (os error 2)`.
This verified hosted-path error justified the subsequent local checks, rather
than disabling hosted builds routinely or bypassing tenant/billing controls.
Local app-core run `20261013-074441115-730779` spent 64.10s compiling its
165k-line test unit and finished 54 MCP tests in a 72.2s run under heavy CPU
contention. No controlled implementation speedup is claimed across these
machines, toolchains, or load conditions.

The root Jcode CLI lives in the library target. Run
`justrust test -p jcode --lib mcp`, not `--bin jcode mcp` (the latter matched
zero of eight binary glue tests in `20261013-073508257-614585`). The corrected
local library run `20261013-073940009-664079` executed 11 tests successfully
in 14.7s. Always verify a nonzero test count before treating a filter as proof.

## Headline numbers

| Repo                     | Foreground cargo calls | Agent-blocked time | p50   | p90    |
|--------------------------|-----------------------:|-------------------:|------:|-------:|
| Jcode + Jcode Desktop    | 3,470                  | 52.3 h             | 16 s  | 134 s  |
| Jcode Desktop only       | 2,557                  | 41.1 h             | 18 s  | 148 s  |

By kind (Desktop): test 27.7 h, build 8.7 h, check 4.7 h, fmt 0.1 h.

## 1. Test calls are 90% compile, 10% tests

For test calls whose output included libtest results:

- Wall time: 21.0 h
- Actually executing tests: 2.1 h (10%)
- Compiling, linking, and overhead: 18.9 h (90%)
- Median tests run per call: about 20 (p90 about 1,400)

Agents typically ask for about 20 tests and pay for rebuilding and relinking
the whole test binary to get them.

## 2. One crate dominates

`cargo test -p jcode-desktop-ui [--lib]` alone accounts for 18 h, almost half
of all Desktop wait time. `jcode-desktop-ui` has 143k lines and 941 `#[test]`s
in one crate. Its debug test binary is about 670 MB.

Measured a controlled single-line edit in `fps_counter.rs`, using
`cargo test -p jcode-desktop-ui --lib --no-run` with mold and the parallel
frontend already enabled:

| Scenario                           | Wall   |
|------------------------------------|-------:|
| No-op (nothing changed)            | 0.5 s  |
| Body edit (first, upstream dirty)  | 32.3 s (jcode-base 10.1 s + desktop-ui 21.7 s) |
| Edit adding an item                | 9.9 s  |
| Revert edit                        | 9.5 s  |

So a best-case one-line change in this crate costs about 9 to 10 s. The
historical p50 is 24 s and p90 is 163 s, meaning most real calls pay well
beyond that best case: upstream crates rebuilding, cold incremental caches,
flag or target-dir changes, and lock waits.

## 3. A fifth of the time ends in a compile error

581 foreground calls (11.7 h, 22%) ended with `error:` output. Those calls
waited on full test or build pipelines just to learn about a type error that
`cargo check` (or a warm rust-analyzer) could have reported sooner.

## 4. Smaller effects

- Cargo lock contention: 107 calls, 0.8 h. Parallel agents in the same
  checkout serialize on `target/`.
- 1,300 background cargo calls are not counted above. They still use CPU and
  the cargo lock, which slows down foreground calls.
- `target/` in Jcode Desktop is 466 GB.
- Test execution is rarely the bottleneck: p50 0.5 s, p90 about 20 s per call.

## 5. Observed: a dependency edit forces a full rebuild of the downstream crate

Run `20261007-214608994-2456373`. A one-line test-body edit in
`jcode-desktop-ui` took **46.3s**, against 9.5s for the same edit earlier.
`jcode_base` (a path dependency in `~/jcode`, edited by another agent at the
same time) had changed and rebuilt in 10.5s. Because its metadata changed,
`jcode_desktop_ui (test)` reused no codegen units: `codegen_crate` took 23.6s,
monomorphization 9.4s, and LLVM 11.9s, using 118s of CPU.

Implication: parallel agents editing shared upstream crates turn cheap
incremental builds into near-full rebuilds for everyone downstream. Candidates
are isolated per-agent build dirs pinned to a stable upstream snapshot, and
sharing or avoiding codegen when only an upstream body (not its API) changed.

## Implications for justrust

1. Optimize compile and link for test binaries, not test execution.
2. Answer with check-speed diagnostics first. Only build test binaries once
   the code type-checks.
3. Huge single-crate test binaries are the main cost center. Build and link
   only what the requested tests need, or keep the test binary hot-patchable.
4. Keep incremental state warm and isolated per agent so calls hit the 10 s
   best case, or better, instead of the 24 s median.

## 6. Jcode Desktop compiles gpui twice: build vs test (gpui `test-support`)

Measured 2026-10-07 on a scratch copy of `~/jcode-desktop` (HEAD 9a92572,
separate `CARGO_TARGET_DIR`s under `~/.jcode/scratch/gpuifeat`). Benchmark:
delete the `gpui-*` fingerprint dirs (forces gpui and its dependents to
rebuild), then `justrust build -p jcode-desktop -p jcode-desktop-ui --lib`
followed by `justrust test -p jcode-desktop-ui --lib -- fps_counter`.

Cause: both Desktop manifests enable gpui's `test-support` only under
`[dev-dependencies]`. With resolver 2 the test unit graph turns on
`test-support` (= `leak-detection` + `backtrace` + `proptest` +
`collections/test-support` + `http_client/test-support`). Unit graph diff
(`cargo build/test --unit-graph -Z unstable-options`, comparing units by
features, profile and transitive deps): build 950 units, test 959, 927 shared,
32 test-only. The expensive test-only ones are gpui, gpui_linux, gpui_wgpu,
gpui_platform, gpui_thinking_orbs, sum_tree, zlog, ztracing, plus
jcode_sdk/jcode_desktop_api/jcode_desktop_ui and proptest's deps. A second
split comes from the `hdrhistogram = "7.6"` dev-dep (default features
`serialization`, `sync`) versus gpui's `default-features = false` one.

| run | build | test | gpui-family units (build+test) |
|---|---|---|---|
| before1 | 53.3s | 61.5s | 3+3 |
| before2 | 73.3s | 103.9s | 3+3 |
| before3 | 93.0s | 97.5s | 3+3 |
| after1 | 72.8s | 10.9s | 3+0 |
| after2 | 85.3s | 25.0s | 3+0 |
| after3 | 89.3s | 13.7s | 3+0 |

"after" = gpui `test-support` in normal `[dependencies]` of both crates and
`hdrhistogram` dev-dep with `default-features = false`. The unit graph then
differs only in the jcode_desktop_ui test unit itself. Medians: before
73.3s + 97.5s = 170.8s, after 85.3s + 13.7s = 99.0s, about 72s (42%) saved
per gpui-invalidating build+test cycle. The test step drops from about 98s to
about 14s. The machine was shared (other agents compiling), so absolute
times are noisy (build 53-93s for the same work). The test-step drop is far
larger than the noise. Run ids: before1 20261007-223529986-2792438 /
20261007-223623262-2799898, before2 20261007-224549993-2900285 /
20261007-224703347-2909978, before3 20261007-224847324-2927272 /
20261007-225020339-2939751, after1 20261007-224420268-2886179 /
20261007-224533055-2898124, after2 20261007-225158015-2964597 /
20261007-225323336-2976054, after3 20261007-225348582-2979157 /
20261007-225517874-2988351.

Not applied, because no option was safe:

- Enabling `test-support` in `[dependencies]` also enables it for release and
  shipped builds. `[target.'cfg(debug_assertions)'.dependencies]` does not help.
  Cargo evaluates target cfgs against the target platform, not the profile, so
  the unit graph of `cargo build --release` still had `test-support` (checked).
- It changes runtime behavior of the dev app (Ctrl+R hot reload and the
  running Desktop use dev builds): `leak-detection` takes the
  `EntityRefCounts` write lock and inserts into a HashMap on every
  `Entity`/`AnyEntity` creation, clone and drop, and `LeakDetector::drop`
  panics with "Exited with leaked handles" if any entity handle is alive when
  the App is torn down. It also adds `debug_bounds` and `debug_selector`
  bookkeeping each frame (Desktop uses `debug_selector` in about ten places),
  the test scheduler branch in `ForegroundExecutor::new`, and turns off the
  profiler's foreground runnable counter only for test dispatchers.
- Making tests drop `test-support` is not possible. 134 Desktop test files use
  `#[gpui::test]`, `TestAppContext`, `run_until_parked` and `add_window_view`,
  which only exist with that feature.
- A Desktop cargo feature (say `dev-tools = ["gpui/test-support"]`) that Ctrl+R,
  the launcher and agents always pass would unify the graphs, but every build
  path (rust-analyzer, `desktop_selfdev`, scripts, CI) must agree or it
  reintroduces the split and an ABI split between host and plugin. Ctrl+R
  builds `-p jcode-desktop -p jcode-desktop-ui` so the host and plugin share
  one GPUI feature set. It also carries the same behavior change above.

The safe part on its own: the `hdrhistogram` dev-dep with
`default-features = false` only removes the hdrhistogram/crossbeam_channel
test-only units (small, under 1s). It is not worth a separate change because
the gpui split dominates.

Possible real fixes, all upstream in the gpui fork (github.com/1jehuang/gpui):
split `test-support` so the test APIs (`TestAppContext`, `gpui::test`,
`TestPlatform`) compile always (or behind a no-runtime-effect feature),
while `leak-detection` stays opt-in. Or make leak detection a runtime toggle.
Then Desktop could enable the API-only feature in `[dependencies]` with no
runtime change and the 32-unit split disappears. For justrust: a finding
that detects "the same package compiled with different feature sets in check,
build and test" (dev-dependency feature unification) and names the
dev-dependency causing it would have found this immediately.

## Cranelift backend for Desktop debug/test builds (2026-10-07)

Availability: the system toolchain (Arch rust 1.98.1, no rustup) ships no
`codegen-backends/` dir and no `rustc-dev`, so a cranelift backend cannot be
built or loaded for it. The only practical route is a rustup nightly with
`rustc-codegen-cranelift-preview`, installed user-local under
`~/.jcode/scratch/cranelift/{rustup,cargo}` (rustc 1.101.0-nightly
1d81eb4ad 2026-10-07). Implications: a different rustc means separate
fingerprints and a separate cold dependency build (~3 min, ~4 GB per target
dir), nightly drift, and `-Zcodegen-backend` needs nightly anyway.

Setup: rsync copies of ~/jcode and ~/jcode-desktop in scratch, one
CARGO_TARGET_DIR per variant, all on the same nightly so only the backend
differs. Variants: `llvm`; `clif` (RUSTFLAGS=-Zcodegen-backend=cranelift for
every crate, so gpui and the other opt-level 2 crates lose their
optimization); `cliflocal` (wrapper adds the flag only for non-CARGO_HOME
crates at opt-level 0, chained in front of Desktop's parallel-frontend
wrapper). Scripts: `bench/cranelift/`. Command: `cargo test -p
jcode-desktop-ui --lib`.

Wall seconds (medians; machine shared, rustc counts from other agents 0-34):

| step                                  | llvm | clif (all) | cliflocal |
|---------------------------------------|------|------------|-----------|
| cold `--no-run` (740 units)           | 182  | 203        | 195       |
| one-line edit + fps_counter tests     | 7.0  | 7.5        | 7.3       |
| one-line edit, recorded by justrust   | 8.8  | -          | 8.3       |
| full desktop-ui codegen, quiet (x3)   | 20.0 | 16.8       | 16.8      |
| full codegen, recorded, quiet         | 26.6-32.2 | -     | 23.9-24.7 |
| full lib test suite run (1655 tests)  | 16-28 | 45-80     | 13-16     |

Recorded run ids (full codegen = delete desktop-ui incremental dir in the
scratch target and touch lib.rs): llvm 20261007-225541367-2992294 (32.2s,
codegen 17.2s, cpu 113s) and 20261007-225637578-2998150 (26.6s, codegen
14.9s, cpu 103s); cliflocal 20261007-225613624-2995425 (23.9s, codegen
11.8s, cpu 81s) and 20261007-225704287-3001214 (24.7s, codegen 11.7s, cpu
84s). Edit loop: llvm 20261007-225745049-3005459 / -225801786- /
-225819136- (codegen 1.08-1.44s), cliflocal 20261007-225753010-3006812 /
-225810648- / -225827972- (codegen 1.16-1.43s).

Findings:
- Codegen of jcode_desktop_ui drops about 25% (15-17s to 11.8s) and CPU
  about 20% with Cranelift, peak RSS ~400 MB lower. That only matters when
  the whole crate regenerates (upstream jcode crate changes, flag changes).
- The small-edit loop is unchanged (codegen ~1.2s either way, inside noise):
  the loop is dominated by frontend and macro expansion, not codegen.
- An upstream body edit in jcode-sdk did not force full codegen in any
  variant (8-17s, noisy), incremental reuse already covers it.
- Cranelift for everything is a loss: gpui and friends lose opt-level 2, so
  the test suite runs 2-4x slower (45-80s vs 16-28s) and cold builds are not
  faster. Keep deps on LLVM.
- Correctness: all variants pass the 1655 tests in most runs. The same
  handful of timing/layout tests (bundled_github_applet_renders_valid_documents,
  live_tabs_switch_rows_and_stay_detached_from_panels,
  the_hint_chip_uses_only_spare_tab_space,
  precise_horizontal_scroll_moves_rendered_panels_smoothly) fail
  intermittently under load in all three variants including llvm, and pass
  in isolation. Flaky, not miscompiles. Failures cluster under clif(all)
  because its slow gpui makes timing tests slower.

Recommendation: not worth it now. The win (about 4-8s on a full
desktop-ui regeneration, nothing on the common edit loop) requires
switching agents to a nightly toolchain, which costs a ~3 min cold rebuild,
another ~4 GB target per variant, and nightly churn, and it conflicts with
the "recording must never change build results or caches" rule if justrust
toggled it. No justrust hook added. Revisit if Arch or stable ships the
cranelift component, or if full-crate regenerations become frequent.

## 7. Per-agent build slots (2026-10-07)

Problem: cargo holds an exclusive flock on `target/<profile>/` for the whole
build (`.cargo-lock`, `.cargo-build-lock`, `.cargo-artifact-lock`; all three
block). Two agents in one checkout fully serialize. Cargo 1.98's
`build.build-dir` does not help: with a separate build-dir per run but a
shared target-dir, the second build still blocked on "artifact directory"
(22.4s then 44.3s for two justrust builds).

Design (src/slots.rs): agent-mode `check`/`test`/`clippy` get
`--target-dir <target>/justrust-slots/<n>`. `build`/`run` and the plain
`cargo` proxy keep the shared dir (scripts expect binaries there,
`JUSTRUST_SLOTS_ALL=1` opts in). Skipped when CARGO_TARGET_DIR,
CARGO_BUILD_TARGET_DIR, or a target-dir argument is set. `JUSTRUST_SLOTS=N`
caps the pool (default 4), `JUSTRUST_SLOTS=0` disables. A session
(JCODE_SESSION_ID, else Unix sid) sticks to its slot. A busy slot is skipped
via a non-blocking flock. Pool full means the shared dir, never waiting.
`justrust slots [--clean]` lists or removes free slots.

Seeding: parallel reflink copy (6 `cp --reflink=always` workers) of
`.fingerprint` first, then `build`, `deps`, `examples`, and `incremental`
dirs modified in the last 3 days (129 of 1970 dirs, 11 of 181 GB on the
Desktop). mtimes are preserved, so a seeded slot is fully fresh: 740 fresh
units, 0 compiled, 1.6s (run 20261007-222839390-2662980). Disabled when the
filesystem cannot reflink.

First-use seed cost on the Desktop (load avg 20-39): 13.8s, 25.8s, 37.1s
sequential cp (runs -224514294-2896271, -224733055-2917773); 6.4s with the
parallel seeder (run 20261007-225143060-2962538). Raw copy of the
deps/.fingerprint/build trees: 8.8s with one cp, 5.0s with 6. Seeding is paid
once per slot, then sticky reruns are free (0.8s no-op check).

Disk: `btrfs filesystem du` per slot after some use: 0.26, 1.6, 1.7, 8.3 GB
exclusive. About 125 GB is shared extents with target/debug, so 4 slots cost
about 12 GB, not 4x150 GB. Exclusive use grows as the slot rebuilds crates
the shared dir has not.

Concurrency, two sessions, one-line edit in fps_counter.rs, warm slots
(bench script ~/.jcode/scratch/slotbench/conc.sh):

| pair | shared dir (JUSTRUST_SLOTS=0) | slots |
|---|---|---|
| A test + B test (same command) | pair wall 11.5 / 9.9 / 11.6s; B lock wait 11.0 / 9.1 / 11.2s | pair wall 15.4 / 14.7 / 15.0s; lock wait 0.3-0.9s (package cache) |
| A test + B check -p desktop-ui --lib | B wall 17.7 / 13.6 / 12.4s (lock wait 8.7 / 10.8 / 9.0s), pair 18.0 / 13.9 / 12.7s | B wall 3.4 / 3.2 / 3.5s (no lock wait), pair 10.8 / 10.1 / 11.6s |

Run ids: shared mixed -225747943 / -225748251, -225816867 / -225817168,
-225840955 / -225841256; slots mixed -225805987 / -225806288,
-225830853 / -225831154, -225853719 / -225854020; identical pairs
-225606396 .. -225710013.

Reading:
- Different commands (the real multi-agent case): the second agent goes from
  about 14s median to 3.4s, and the pair finishes in 10.8s instead of 13.9s
  (median). Lock wait goes from about 9-11s to 0.
- Identical commands at the same moment: the shared dir wins (10-12s vs
  15s), because B waits and then finds A's work done, while with slots both
  compile the same crate in parallel on a busy 16-core machine. That
  case only arises when two agents run the exact same build at once. Each
  agent still sees its own state, not one that another agent's edit just
  churned.
- The remaining 0.2-0.9s waits are cargo's CARGO_HOME "package cache" lock,
  which slots cannot remove. The lock_wait finding now names the lock and
  only suggests JUSTRUST_SLOTS when it was the build directory.
- Live use: within minutes another agent (hibiscus, ui crate split) was
  given slot 1 automatically. A plain `cargo check` through the proxy at the
  same time still waited 11.4s on the shared dir (run -225913558-3020615),
  which is exactly the cost slots remove for agent runs.

Risks / follow-ups:
- Stale slots: a slot seeded long ago does not get dependency rebuilds that
  later happen in the shared dir (Cargo.lock bump, gpui rev). It rebuilds
  them itself once. Re-seeding a slot when the shared dir has many newer
  fingerprints would be cheap with reflinks and is not implemented yet.
- Disk: exclusive bytes per slot grow with divergent rebuilds. No automatic
  GC. `justrust slots --clean` removes free slots.
- `target/debug/<bin>` from agent test/check runs no longer appear in the
  shared dir. Test binaries live under the slot. Builds and runs are not
  slotted, so the Desktop launcher and Ctrl+R are unaffected.

## Splitting jcode-desktop-ui: harness crate pilot (2026-10-07, hibiscus)

Desktop commit 751f1cd moves the GPUI-free harness cluster (harness*.rs,
remote, managed_cloud, managed_cloud_parity, remote_targets, platform: 7.3k
lines, 94 tests) from jcode-desktop-ui (142.5k lines, 1668 tests) into
crates/jcode-desktop-harness. The UI re-exports the modules at their old
paths. Hot reload: the new crate is an rlib linked into the UI cdylib, so the
host/UI ABI is unchanged. Ctrl+R on the running host built and activated UI
generations 3 and 4 (main) and 3 (single-panel).

Module map (crate:: edges between the 70 top-level modules): workspace (48.6k
lines with its #[path] children) and panel (42.8k) are a mutual hub that
depends on almost everything. input (6.8k) has back-edges to panel/workspace.
The clean leaves without gpui are harness+transports (5.5k plus 1.5k, used by
12 modules, only out-edge was build_info::VERSION), diff_model (2.5k, 62 tests,
no deps), learning (1.6k, 34 tests, no deps), diff (1.1k, 32 tests, no deps),
updates (1.8k, deps harness+build_info), global_voice_input (1.2k),
accounts (1.2k, deps harness+platform), remote_targets, pdf_render, todoist,
preview_control, render_stats. Churn over two weeks (593 file-touches in
ui/src): workspace 44, panel 40, input 19, harness cluster 44, voice files 50,
login/account files 59.

One-line edit benchmark (insert `let _probe = N;` in a test fn body, `justrust
test`, restore from backup; median of 3; "others" = cores used by other
processes during the run). The before runs used the shared target dir, the
after runs justrust slot 1 (both warm incremental).

| edit in | command | before compile / wall | after compile / wall |
|---|---|---|---|
| harness.rs | before `-p jcode-desktop-ui --lib -- harness::`, after `-p jcode-desktop-harness --lib -- harness::` | 9.99s / 13.5s (runs 19.7, 10.0, 8.9; others 14, 6.5, 3.2) | 1.25s / 4.2s (runs 1.69, 1.17, 1.25; others 5-7). 2.56s of the wall is the tests themselves |
| fps_counter.rs (UI) | `-p jcode-desktop-ui --lib -- fps_counter` | 14.75s (runs 21.6, 14.8, 9.5; others 14, 11, 5) | 8.02s (runs 9.3, 8.0, 7.7; others 5, 2, 3) |
| harness.rs, then UI test | `-p jcode-desktop-ui --lib -- fps_counter` | same as row 1 | 10.2s compile / 10.9s wall (harness 0.5-1.1s + UI 8.7-9.5s) |

Run ids: before 20261007-223543659, -223606014, -223621987 (ui), -223648196,
-223711646, -223725262 (harness); after -225722087, -225732166, -225740763
(ui), -225751905, -225756538, -225800732 (harness crate), -225815555,
-225826549, -225837526 (harness edit, UI test).

Reading:
- The win is for edits inside the extracted crate tested in that crate:
  about 8x less compile (8.9-10s to 1.25s) and a 1.2 GB smaller rustc. The
  harness test binary is small, so the frontend (0.3s) no longer pays the UI's
  2.4s macro_expand and 1s incremental cache save.
- Edits in the UI are not measurably faster. The UI lost 5% of its lines, so
  the expected gain is ~0.3-0.4s, below the contention noise here (quiet runs
  7.7-9.5s both before and after).
- A harness edit followed by a UI test is not faster, and cannot be: the
  harness rmeta changes, so the UI recompiles (the harness crate adds 0.5-1s).
  Agents get the speedup only when they run the harness crate's own tests,
  which hold all 94 harness tests.
- Expected value: the harness cluster was 44 of 593 two-week file touches
  (7%). Saving ~8s each is small in aggregate. The pattern is what matters.

Recommended next splits (same recipe: move, re-export, test-support feature
for cross-crate test helpers, inject UI-only constants through a setter):
1. Pure-data leaves with no crate:: deps: diff_model + diff (3.6k lines, 94
   tests), learning (1.6k, 34 tests), todoist, preview_control, pdf_render.
   Trivial, but low churn, so mostly a UI frontend shrink.
2. accounts + updates (3k lines, 57 tests) onto the harness crate once
   build_info's VERSION is reachable (already via client_version()).
3. The big lever is panel/workspace (91k lines, 1000+ tests, 84% of churn).
   They depend on each other, so a split needs a trait or a shared state
   crate first. Splitting gpui view code by feature (voice: ~6k lines across
   panel_voice*/workspace_voice/global_voice*; login/accounts UI ~8k) into
   crates that depend on a small `jcode-desktop-ui-core` (theme, render_stats,
   text_selection, scrollbar, image_cache, config) would let the most-edited
   features compile alone. Precondition: break the panel <-> workspace cycle
   (workspace::* used from panel, panel::Panel from workspace).
4. Cheapest per-edit win without any split: run the narrow test crate. A
   `justrust test -p jcode-desktop-ui -- fps_counter` still builds a 1574-test
   binary. justrust's waste line already says so.

## 8. Shared dependency cache (depcache, 2026-10-07, mushroom)

Goal: fresh target dirs, scratch checkouts, and build slots stop recompiling
the same registry/git dependencies. Code: `src/depcache.rs`, hooked into the
shim (`src/shim.rs`). On by default for non-local units, `JUSTRUST_DEPCACHE=0`
disables it. Store: `~/.justrust/cache/depcache/` (override with
`JUSTRUST_DEPCACHE_DIR`).

### sccache does not work across target dirs

sccache 0.18 with `RUSTC_WRAPPER=sccache`, `cargo check` of justrust into two
fresh target dirs: **0 hits / 86 misses** (also 0 with `SCCACHE_BASEDIRS`
listing both target dirs). Its hash includes the absolute `-L dependency=`,
`OUT_DIR`, and `CARGO_TARGET_DIR`, which differ per target dir. Rejected.

### Design

- Key, two levels: base key = rustc `-vV` + args with `<target>/<profile>`
  replaced by a placeholder + content hash of every `--extern` file (memoized
  in an xattr by inode/size/mtime) + linker content + native lib dirs inside the
  target + `CARGO_*`/`RUSTC_*`/`OUT_DIR` env. Result key = base + content of
  every dep-info source (including `OUT_DIR` generated files) + every
  `env-dep` value.
- Hit: reflink the outputs into `--out-dir`, rewrite `.d` paths, replay rustc's
  stderr (artifact notifications keep pipelining working), exit 0. Miss: compile
  normally, then store only if rustc exited 0.
- Units with `-C incremental` (local crates), `--emit` paths, `-o`, or
  `--print` are never cached. Any cache error compiles normally.

### Measurements (jcode-desktop, `justrust check -p jcode-desktop-ui`, fresh CARGO_TARGET_DIR)

| case | run id | wall | depcache |
|---|---|---|---|
| cache off | 20261007-231843110-3108683 | 181.7s | off |
| cold, empty cache | 20261007-232144859-3139425 | 128.3s | 0 hit / 833 miss |
| second fresh target dir | 20261007-232353322-3168951 | **35.8s** | 833 hit / 0 miss |
| scratch checkout (pincheck), cache on | 20261007-232541361-3194212 | **29.2s** | 795 hit / 30 miss |
| same checkout, cache off | 20261007-232610551-3208355 | 118.1s | off |

- The machine was busy throughout (load 7 to 28, up to 38 other rustc during
  the cold runs), so the off/cold numbers are noisy; the off vs cold1 gap is
  contention, not the cache. The hit runs are dominated by the 49-51 local
  crates (jcode_base 9.0s, desktop-ui 3.9s, jcode_protocol 4.3s): 833 hits took
  2.4s of rustc wall in total, max 0.16s (aws_sdk_bedrock), versus ~1129s of
  original rustc time.
- justrust itself: deps-only cold check 5.1s -> 1.4s (52/52 hits); `build`
  10.5s -> 4.3s, and the resulting binary and `justrust test` work.
- Store and miss overhead: hashing and storing added nothing visible to the
  cold run (it was faster than the cache-off run under lower contention).
- Disk: 1.4 GB after the Desktop dependency set (check mode), 2.9 GB after the
  pincheck checkout too (different features for 30 units plus its own path
  crates are not cached; the growth is mostly rmeta for differently-featured
  units). Reflinks on btrfs share extents with the target dirs. No eviction
  yet.
- Correctness: after a hit build, `JUSTRUST_DEPCACHE=0 cargo check` on the same
  target dir is fresh (740 fresh, only the desktop-ui unit rebuilt for an
  unrelated `.git/index` change), so restored artifacts do not dirty cargo.
  `.d` files are byte-identical after path substitution. Restored rmeta are
  identical to a fresh compile except for embedded absolute target paths
  (240/762 rmeta mention `OUT_DIR` or target paths; the same 240 differ between
  any two uncached target dirs, so this is not cache-induced).

### Risks / follow-ups

- Restored binaries embed the target-dir path of the build that produced them
  (debuginfo, `include!(concat!(env!("OUT_DIR"), ...))` file names). Harmless
  for check/test; panics and backtraces in deps may name another target dir.
- No size cap or eviction yet (`rm -rf ~/.justrust/cache/depcache` is safe).
- Build-script *execution* is not cached, only its compilation; cargo still
  runs every build script in a fresh target dir (the 8.9s of gaps in the hit
  run). Addressed in 8c.

### 8a. Size cap and LRU eviction (2026-10-07, dep cache worker)

Code: `src/depcache_gc.rs`, `justrust cache [--prune|--clear]`.

- LRU clock: a hit or a repeated store touches `o/<key>/meta.json`. After each
  recorded run, at most once per 10 minutes (one `stat` of `gc.stamp`
  otherwise), a detached, `nice 19` `justrust cache --auto` evicts the least
  recently used entries until the cache is under 90% of the cap. Default cap
  30 GB, `JUSTRUST_DEPCACHE_MAX=50G|800M|0` (0 = no cap). Evictions are logged
  to `gc.jsonl`.
- Concurrency: one collector at a time (`gc.lock` flock). Entries are renamed
  into `trash/` before deletion, entries used in the last 10 minutes are never
  evicted, and an entry touched between scan and rename is put back. Manifest
  lines for evicted outputs are dropped. A restore that loses a race fails open
  (compiles normally, overwriting partial outputs).
- Stress test, worst case: a scratch copy of the cache with a loop moving 3
  random entries to trash and deleting them every 0.1s (900 entries evicted)
  during a fresh-dir Desktop check (run 20261007-234551593-3434267): exit 0,
  774 hit / 51 miss, and a following `JUSTRUST_DEPCACHE=0 cargo check -v` on
  that target dir reported 732 Fresh, only the desktop-ui unit Dirty (same as
  without eviction). Same with `justrust cache --prune` at a 1.2 GB cap in a
  0.4s loop (1569 evictions, grace 0): all units compiled, cargo fresh after.
- Cost: `justrust cache` on the real 4.1 GB / 1643-entry cache: 0.2s.
  `--prune` of a reflinked copy down to 2 GB evicted 1319 entries in 1.0s.
- Hit runs (`bench/depcache-fresh.sh 3`, fresh `CARGO_TARGET_DIR`, warm cache,
  825/825 hits every run):

| | runs | walls | median | load | hit restore sum |
|---|---|---|---|---|---|
| before | 20261007-233243709-3309755, -233310202-3317914, -233338842-3327229 | 26.1, 28.3, 23.9s | 26.1s | 12.5-14.8 | 1.4-2.6s |
| after | 20261007-234647588-3448800, -234726717-3459509, -234820383-3471186 | 38.8, 52.8, 31.7s | 38.8s | 21-30 | 2.1-10.0s |

  The "after" runs ran at twice the machine load (4-6 other rustc), so the wall
  difference is contention, not the change: the only per-hit addition is one
  `utimensat`, and the median per-hit restore stayed at 1.1-1.7 ms. No speedup
  is claimed for this step; it bounds disk use.

### 8c. Cached build-script runs (2026-10-08, pig + depcache follow-up)

Code: `src/buildscript.rs`. Follows c3c9903 (never store outputs that embed a
foreign target path). This step caches build-script *runs* for registry/git
packages. When the shim compiles or restores a build script, it moves the
binary to `.jr-real-<name>` and puts a `/bin/sh` wrapper in its place. The
wrapper runs `justrust __build-script`, or the real script directly when that
justrust binary is gone. Cached runs replay stdout/stderr and restore
`OUT_DIR`. They are only stored for scripts with `rerun-if-*` directives, exit
0, inputs outside the target dir, and no binary `OUT_DIR` file naming the
target dir. Off with `JUSTRUST_BUILD_SCRIPT_CACHE=0`.

Review fixes before shipping: (1) the base key now includes a system stamp
(mtime of `/etc/ld.so.cache` and the package database: pacman, dpkg, rpm,
nix). A system upgrade misses, so pkg-config/cc probing scripts that only
declare env vars re-probe in a fresh dir. (2) `rerun-if-changed` symlinks
are signed by target content, not the link text. (3) A script killed by a
signal now kills the wrapper with the same signal (it used to exit 128+n).
The tests cover fail-open (wrapper with justrust missing or with a disabled
justrust: same stdout, stderr, args, cwd, exit code), non-cacheable runs (missing
input, input inside target, non-UTF-8 stdout, symlink in OUT_DIR), directory
inputs, and the cross-target-dir round trip.

Correctness on Desktop (`justrust check -p jcode-desktop-ui`, fresh dirs):
- cold run 20261008-205843070-3935519: 74 scripts run, gaps 6.58s. Second
  dir 20261008-205907582-3955347: 45 hit / 29 miss (the misses have no
  `rerun-if` or are local), gaps 0.86s.
- Fingerprints: on the cached dir, a second `justrust check` was 742 fresh /
  0 compiled, and plain `/usr/bin/cargo check -v` (no shim, no depcache) was
  742 Fresh, 0 Dirty. Swapping the binary does not dirty cargo, because cargo
  fingerprints the build-script *source* and its `output`, not the binary.
- OUT_DIR diff against an uncached dir (`JUSTRUST_BUILD_SCRIPT_CACHE=0`), with
  target paths normalized: 76 out dirs, identical file lists. The one content
  diff is local `jcode-desktop-ui/out/changelog.md` (embeds a build
  timestamp, never cached). The `output` files of aws-lc-rs, aws-lc-sys, and
  onig_sys differ only in line order (parallel cc output), and libm
  in `CFG_CARGO_FEATURES` order (`["arch","default"]` vs reversed: HashMap
  iteration order in the script's own run, not the replay: the cached
  copy is a verbatim earlier run of the same script).
- `justrust test -p jcode-desktop-model` in the cached dir: 140 passed.
- `cargo clean -p aws-lc-sys` removes the whole `build/<pkg>-<hash>` dir
  (wrapper and `.jr-real-`), and the next check rebuilds and re-wraps it.
  Slots copy whole `build/<pkg>-<hash>` dirs (`cp -a`, hidden files
  included), so wrapper and real binary always travel together. A wrapper
  copied into another dir finds its real binary through `dirname "$0"`.
  Eviction of a script entry only causes a miss (the real script runs).

Hit runs, `bench/depcache-fresh.sh 1` alternated on/off, 833/833 rustc
depcache hits each:

| | runs | walls | median | gaps |
|---|---|---|---|---|
| script cache on | 20261008-210127927-4050724, -210205780-4063343, -210246936-4077015 | 14.8, 16.6, 16.2s | **16.2s** | 0.75-0.83s |
| `JUSTRUST_BUILD_SCRIPT_CACHE=0` | 20261008-210143096-4056390, -210222703-4068925, -210303453-4082648 | 22.4, 23.9, 27.4s | 23.9s | 5.78-6.92s |

Load rose from 5.7 to 10.2 during the series (0 to 2 other rustc), so each
"off" run had equal or higher load than the "on" run before it. The gap
seconds are the robust number: 5-6s of build-script time removed from every
fresh-dir build (aws-lc-sys alone 10.8s CPU, onig_sys 2.6s). The wall
median went from 23.9s to 16.2s (-32%).

Remaining risk, accepted: a registry script that declares `rerun-if` inputs
but also reads undeclared state (a system file not covered by the package
stamp) replays stale output in a fresh dir. Cargo makes the same assumption
in an existing target dir.

## 9. gpui `test-api`: one GPUI build for Desktop build and test (2026-10-07, rose)

Follow-up to section 6. Fixed at the source in the gpui fork, commit
`1cfbc5d` on github.com/1jehuang/gpui main: a new API-only feature `test-api`
(TestAppContext, `#[gpui::test]`, TestPlatform, run_until_parked,
debug_bounds, layout stats, FakeHttpClient, proptest re-export).
`test-support` is now `test-api` + `leak-detection` + wayland/x11, so
existing users are unchanged. gpui_apple, gpui_linux, gpui_macos,
gpui_platform and gpui_windows forward `test-api`.

cfg audit (131 `feature = "test-support"` sites in gpui, all moved to
`test-api`): every one is either an API item that a real app never calls or a
branch only reachable with a test dispatcher (`dispatcher.as_test()` returns
None for real dispatchers, `GpuiMode::Test` is only set by test contexts).
Executor branches: with a real dispatcher, `BackgroundExecutor::new` and
`ForegroundExecutor::new` take the same PlatformScheduler path as before,
including the profiler's foreground runnable counter. The one real per-frame
cost, `debug_selector` recording into `debug_bounds`, now only records once a
test app exists in the process (`fast::test_api`, an AtomicBool set by
`GpuiMode::test()`). With `test-support` it always records, as before. Leak
detection stays only in `leak-detection`/`test-support`. Platform crates'
`test-support` sites (macOS `set_framebuffer_only(false)`, render_to_image)
stay on `test-support` and are not reached by `test-api`. gpui's own 388 lib
tests pass with `test-support`; `cargo check -p gpui` passes with no
features, `test-api`, `test-support`, `bench-support`, `test-api,profiler`.
`script/check-upstream`: only the 2 violations that predate this change.

Desktop change (gpui rev 1cfbc5d with `features = ["profiler", "test-api"]`
in `[workspace.dependencies]`, the `gpui` dev-deps drop `test-support`,
`hdrhistogram` dev-dep `default-features = false`, `GPUI_REVISION` bumped).
Unit graph (`__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS=nightly cargo
build|test --unit-graph -Z unstable-options`, compared by package, features,
profile minus name, and transitive deps): build 960 units, test 960, 958
shared. gpui, gpui_linux, gpui_platform, gpui_wgpu, gpui_macros,
gpui_thinking_orbs all identical. The only differences are jcode_desktop_ui
(lib vs test) and jcode_desktop_harness (its own `test-support`, a small local
crate, separate issue). Release unit graph: gpui has `test-api` and no
`leak-detection`, i.e. no runtime change per the audit above. All Desktop
workspace tests passed (1731/1732 on the first run; the one failure,
`the_hint_chip_uses_only_spare_tab_space`, passed alone and in two full
reruns of the 1404 UI tests, so it is flaky under load, not caused by this).

Benchmark (scratch copies under ~/.jcode/scratch/gpuitestapi, separate target
dirs, `JUSTRUST_DEPCACHE=0 JUSTRUST_SLOTS=0`: depcache otherwise restores gpui
and hides the cost; delete `gpui-*` fingerprints, then `justrust build -p
jcode-desktop -p jcode-desktop-ui --lib` and `justrust test -p jcode-desktop-ui
--lib -- fps_counter`):

| run | before build | before test | before total | after build | after test | after total |
|---|---|---|---|---|---|---|
| 1 | 45.3 | 50.4 | 95.7 | 58.1 | 9.8 | 67.9 |
| 2 | 82.1 | 99.4 | 181.5 | 59.2 | 8.9 | 68.1 |
| 3 | 59.3 | 85.3 | 144.5 | 68.8 | 10.2 | 79.0 |

Medians: before 144.5s (test 85.3s), after 68.1s (test 9.8s), about 76s
(53%) saved per gpui-invalidating build+test cycle. The test step drops from
~85s to ~10s because it compiles 1 unit instead of 8. Other rustc on the
machine at start: 0-2, but the build column varies 45-82s for identical work,
so the totals are noisy; the test-step drop is far outside the noise.

Status: landed in Desktop as d206de0 (2026-10-08). The running hosts were
restarted onto gpui 1cfbc5d with Desktop's built-in `--restart-all-worker`
(8 windows restored), and a Ctrl+R build-reload on the new host activated a
fresh UI generation. Build and test now share every gpui unit (unit graph:
961/961, 958 shared, the 3 differences are only the local crates' own test
vs lib units). Workspace tests: 1731 of 1732 pass;
`bundled_github_applet_renders_valid_documents` failed once under load and
passes alone (also flaky under LLVM per the Cranelift section).

Caveat: `test-api` pulls the optional `proptest` dependency into normal and
release builds. It is compiled but not used at runtime.

## 10. Build slots follow-ups: refresh, GC, joining identical builds (2026-10-07, cow)

### Refresh stale slots (commit 6725be0)

Before a slot run, units the shared `target/<profile>` built since the
slot's last refresh are reflink-copied in. A unit is the granule: its
`.fingerprint/<name>-<hash>` dir, its `build/<name>-<hash>` dir, and every
`deps/` file carrying the hash (checked on the Desktop: all 28k deps files
and all build dirs map to a fingerprint hash). Artifacts are copied first,
fingerprints last, with mtimes preserved, so cargo's fingerprints decide
freshness exactly as for a seeded slot. Copied: units missing in the slot,
and units whose newest fingerprint file is newer in shared AND whose hash
files differ (identical state is skipped, copying it would only bump
mtimes). A unit the slot built more recently is never touched. Runs only
while a non-blocking shared flock on the shared dir's `.cargo-build-lock`
succeeds (cargo holds it exclusively while building; verified that a
running cargo blocks it and a reader holding it shared blocks cargo), and
only when the shared dir mtimes moved since the last refresh. Scan cost on
the Desktop: 0.15s for 9.4k fingerprint dirs (36k files).

| case (Desktop, `justrust check -p jcode-desktop-ui --lib`) | time |
|---|---|
| slot missing all 210 gpui fingerprints, no refresh, depcache off (run 20261007-233515707-3342162) | 23.2s, gpui rebuilt 12.8s |
| same, no refresh, depcache on (run -233601632-3346804) | 23.3s, depcache 0 hit / 6 miss |
| same, with refresh (run -233544850-3345335) | 1.0s: 236 new + 1 rebuilt units (540 files) copied in 0.8s, 0 compiled |
| real stale slot 2 (34 missing, 2 rebuilt units, run -233326676-3325406) | refresh 0.2s, then only the 4 local crates compiled |

Scratch project (serde/regex, then rayon+syn added to Cargo.toml, a
Cargo.lock-bump analog): stale slot 6.6s cold / 0.4s with depcache hits
vs 0.0s after a 26-unit refresh in 0.1s. depcache and refresh are
complementary: depcache needs the exact rustc invocation to have been
cached; refresh also skips cargo and build-script work.

### Automatic GC (commit 6bb60d4)

After a slotted run, a detached `justrust slots --gc-root` runs at most
hourly per workspace. Removes free (not flocked) slots idle over 7 days,
then least recently used free slots until btrfs exclusive bytes fit 40 GB
(`du -sb` off btrfs). Slots used in the last hour are never reaped for
budget. `justrust slots` shows exclusive/total size and age.

`btrfs filesystem du` costs 3.8s per 170 GB Desktop slot and does not
parallelize (13.4s for 4 at load 33), so the listing uses sizes cached by
the last GC (34 ms vs 13.4s); `--du` re-measures. Current Desktop: 4 slots,
0.0 / 1.8 / 7.3 / 11.9 GB exclusive (21-31 GB total), well under budget.
The hook adds nothing measurable to a run (93 ms no-op check with GC spawned).

### Joining identical concurrent builds (commit 84dda77)

Each slot records its current request key (cwd, cargo args before `--`,
compile-relevant env) in `<n>.req`. A new run whose key matches a slot that
is flocked right now waits for that flock (max 300s) and then runs cargo in
that slot, so cargo still checks freshness and the joiner sees its own
test output. The wait is shown as a `slot_join` finding.

Desktop identical pair, one-line edit, `justrust test -p jcode-desktop-ui
--lib -- fps_counter`, B starts 0.3s after A (bench
~/.jcode/scratch/slotbench/join.sh), quiet machine (0 other rustc):

| | pair wall | A+B CPU |
|---|---|---|
| parallel slots (JUSTRUST_JOIN=0) | 9.9 / 9.4s | 20.4 / 19.3s |
| joined | 9.3 / 9.0 / 9.0s | 9.8 / 9.7 / 9.7s |

Runs: parallel -235008527/-235008828, -235027474/-235027776; joined
-234959162/-234959463, -235018471/-235018775, -235036897/-235037198.
B finishes about 0.4s after A instead of compiling in parallel. On a quiet
16-core machine the wall gain is small (0.4s) because one crate's codegen
does not saturate the cores; the CPU halves, which matters on this shared,
usually loaded machine (section 7 measured 15s vs 10-12s at load 20+).
Conservative: only a held slot with the exact key, and the joiner still runs
cargo, so a file edited in between is still rebuilt correctly.

### 8b. Embedded target paths in restored artifacts (2026-10-08, dep cache worker)

Question: can a restored artifact behave differently from a fresh compile
because it embeds the first producer's target dir?

Evidence, scanning every cached output (2546 files, 1643 entries):
- 41 of 2546 (15 rlib, 26 rmeta, 0 `.so`, 0 build-script executables) contain
  a target-dir path at all. All 82 occurrences are the full path of an
  `OUT_DIR` source file that the unit `include!`s (serde/serde_core
  `private.rs`, thiserror, ref-cast, pulp `x86_64_asm.rs`, crunchy `lib.rs`,
  mime_guess, aws-types `build_env.rs`, unicode-general-category, libsqlite3-sys
  `bindgen.rs`). 0 occurrences are anything else (checked against each entry's
  dep-info inputs).
- Those file paths come from `SourceFile` names in rmeta and from panic
  locations / line tables (`file!()`, `Location::caller`). Their content is in
  the result key (dep-info inputs are content-hashed), so the restored code is
  the same code. Only the *name* differs: a panic message or backtrace frame
  inside, say, serde's generated `private.rs` names the other target dir. It
  never reads that file at runtime. Desktop uses `debug = 0` for non-local
  packages, but a test with `-C debuginfo=0/1/2` shows the same set (full file
  paths only, never a bare dir).
- What *would* change behavior: a dependency keeping `env!("OUT_DIR")` (or
  another target path) as a runtime value, e.g. opening a generated file at
  runtime. Then a restored artifact would read another build's directory,
  which may have been deleted. None of the cached Desktop or justrust deps do
  this, but nothing prevented it.
- Tracked env with target paths: `OUT_DIR` (30 entries) and
  `MIME_TYPES_GENERATED_PATH` (3). Both are normalized in the key, and the
  above scan covers what they are compiled into.
- `--remap-path-prefix` was considered and rejected: it changes the bytes of
  every output and would have to be applied identically to uncached builds
  (or every unit differs between cached and uncached builds), and it does not
  remap `env!` values (verified: `env!("OUT_DIR")` stays absolute while
  `file!()` is remapped), so it would not fix the dangerous case anyway.

Fix (`target_refs_are_inputs` in `src/depcache.rs`): at store time, every
occurrence of the target dir in a non-dep-info output must be immediately one
of the unit's own input file paths (exact path, followed by a non-path byte).
Otherwise the unit is not stored and always compiles normally.

- Cold store into an empty cache with the guard (run
  20261008-000652409-3638276, 159.9s at load ~30): 833 misses, 833 stored, 0
  rejected, so the hit rate is unchanged on Desktop.
- Guard cost: scanning all 927 MB of Desktop outputs takes 0.52s CPU in total
  (release microbench), spread over the cold misses. Hits do no extra work.
- Hit runs with the guarded cache (`JUSTRUST_DEPCACHE_DIR=<fresh cache>
  bench/depcache-fresh.sh 3`): 44.3, 41.8, 30.8s (median 41.8s), 833/833
  hits each, hit restore sum 1.9-2.6s, median 1.1-1.4 ms per hit (same as
  8a). Runs 20261008-000941671-3672951, -001026422-3695394,
  -001108803-3705552. Load average was 27-31 (vs 12-15 for the 26.1s
  baseline), so wall times are contention-bound; the guard cannot affect hit
  runs.

## 11. Splitting jcode-desktop-ui, part 2: leaf crates and the view cycle (2026-10-08, cricket)

Follows the harness pilot above. Desktop commits, each verified separately
(`justrust test --workspace` 1746 passed / 13 ignored every time, `justrust
check --workspace --all-targets`, offline screenshot, Ctrl+R on the running
host activated a new UI generation, release-desktop.py MANIFESTS updated):

- 51adb8d: new `crates/jcode-desktop-model` (diff_model + its tool-result
  parser, diff, learning, todoist, pdf_render: 6.1k lines, 160 tests, no
  GPUI). `accounts` (1.2k) joins jcode-desktop-harness. `updates` stays in
  the UI: its Linux path uses jcode-desktop-api's LaunchMode, and the api
  crate depends on gpui, so moving it would put GPUI in the harness crate.
- 12f4118: breaks the panel <-> workspace cycle (below). No crate change.
- c3fd05c: AGENTS.md tells agents to test the smallest crate containing the
  change and records the "panel/input must not use workspace" rule.
- 3d6e312: new `crates/jcode-desktop-ui-core` (theme + palettes, config,
  render_stats, text_selection, prompt_background, scrollbar, image_cache,
  animation_clock, transition, pulse_text: 6.1k lines, 70 tests). Test-mode
  `cfg(test)` behavior (config persistence stubs, user-config bypass, fixed
  animation epoch, theme lock, render counters) moved behind a
  `test-support` feature the UI enables only from dev-dependencies, the same
  pattern as the harness crate. GPUI via `gpui.workspace = true`, so it
  shares the test-api GPUI from section 9.

jcode-desktop-ui is now 121.9k lines (from 142.5k before the pilot).

### One-line edit benchmark (median of 3, `let _probe = N;` in a test fn)

| edit in | before: `-p jcode-desktop-ui --lib -- <mod>` | after: `-p <new crate> --lib -- <mod>` |
|---|---|---|
| diff_model.rs | 8.60s compile / 9.2s wall (8.11, 8.60, 8.70; others ~2 cores) | 0.65s / 1.1s (0.57, 0.65, 0.74; others 5-10 cores) |
| theme.rs | 8.01s / 8.5s (9.49, 8.01, 7.83; others ~2 cores) | 1.49s / 2.1s (1.10, 1.52, 1.49; others 6-12 cores, 2-15 other rustc) |
| accounts.rs | (UI, same ~8s class) | 1.02s / 1.4s (1.02, 1.00, 1.03; quiet, others ~2 cores) |
| fps_counter.rs (stays in UI) | 8.0s (hibiscus, quiet) | 8.36s / 8.9s (8.25, 8.61, 8.36; others 2-5 cores) |

Run ids: diff before 20261007-230932200, -230940912, -230950204; diff after
-232606302, -232607336, -232608528; theme before -235629590, -235639641,
-235648217; theme after 20261008-001122633, -001124328, -001126488; accounts
-235232963, -235234407, -235235782; fps_counter -235206178, -235214927,
-235223983. A first run under heavy load (others 11-15 cores, 20-28 other
rustc, load average ~25) measured 5.4-6.8s for accounts and 13-54s for
fps_counter; those runs are discarded as contention.

Reading: edits in an extracted crate, tested in that crate, are 6-13x
faster to compile (0.6-1.5s vs ~8s). UI edits are unchanged within noise
(8.0 -> 8.4s) even though the UI lost another 14% of its lines. The UI
crate's per-edit cost is dominated by frontend + codegen of the crate as a
whole, not by the extracted leaves. ui-core costs more than model (1.5 vs
0.65s) because it is GPUI code, with gpui generics to monomorphize.

### The panel <-> workspace cycle, concretely

Module-graph SCC over the 70 top-level UI modules (each module with its
`#[path]` children; edges are `crate::<module>` references outside
`#[cfg(test)]` items). Before 12f4118, one SCC held 86.1k lines: workspace
(40.6k), panel (36.5k), input (6.4k), applet_runtime, applet_view,
applet_surface. Inside it the forward edges are large (workspace uses
panel 34x and input 10x; panel uses input and the applet modules). The
back-edges were few and small (~20 call sites):

| back-edge | items | fix |
|---|---|---|
| panel -> workspace | actions OpenResume, OpenAppletShowcase, ToggleOnboardingSimulator, OpenChangelog, ClosePanel, PublishDesktop, OpenAccounts, change_review::{Open,Close}ChangeReview | moved to new leaf `ui_actions.rs`, names unchanged (`actions!(workspace, ...)` keeps the namespace), re-exported from workspace |
| panel, input -> workspace | `resume::is_resume_command` | moved to `commands` |
| panel -> workspace | `restart::spawn_worker` | new leaf `restart_spawn.rs` (spawner, detach, flags) |
| panel, input -> workspace (test) | `panel_cache_tests::record_render` | `render_stats::test_renders` |
| input -> panel | `shortcuts::background_tool_bindings` + its 3 actions | `ui_actions` |
| input -> panel | `pretty_provider_name` (pure string fn) | `accounts` (harness crate) |
| applet_runtime -> workspace | `applets::SHOWCASE_ID` | const moved into applet_runtime |

After: no SCC contains workspace, panel or input. The only remaining
cycles are theme <-> config (now inside ui-core) and markdown <-> diff_view
<-> html_preview <-> diff_block (5.6k, rendering, one call site each way).
Test code still uses `Workspace::for_test` from panel tests (69 sites);
that is fine within one crate but must become a test-support helper when
panel moves out.

### Plan for the feature-crate split (not done: needs its own session)

The big lever is still moving the most-edited view code out of the UI
crate. Churn over two weeks: voice files 50 and login/account/usage files 67
of 631 .rs touches in ui/src. Sizes (non-test): voice 7.8k lines in 11
files, login/accounts/usage 9.1k in 11 files. The blocker is no longer the
module cycle but Rust's crate rules:

1. Almost all of that code is `impl Panel { ... }` or `impl Workspace
   { ... }` blocks in `#[path]` child files (panel_voice*.rs, panel_login*.rs,
   panel_usage.rs; workspace_voice.rs, workspace_global_voice*.rs,
   workspace_account_sign_in.rs, sidebar_account*.rs). Inherent impls cannot
   live in another crate, and they read private fields freely (panel_voice.rs
   touches 41 distinct `self.` fields, workspace_account_sign_in.rs 47).
2. So a feature crate must own its state and views: e.g. `VoiceState` (the
   Panel's voice fields) with methods taking `&mut VoiceState` plus a small
   trait the host implements for what it needs from the panel (bridge send,
   session id, notify). Panel keeps a `voice: VoiceState` field and forwards.
   Same for `AccountSignIn` on the workspace side.
3. Order that keeps every step shippable:
   a. Leaves first, no Panel/Workspace involvement: global_voice_input,
      global_voice_overlay, global_voice_session, panel_voice_tag
      (1.9k) and login_input, panel_login_catalog, panel_login_status (1.1k)
      move into `jcode-desktop-voice` / `jcode-desktop-accounts-ui` crates
      depending on ui-core and harness. Mechanical, like ui-core.
   b. Extract the voice field group from Panel into `VoiceState` inside the
      UI crate (pure refactor, tests unchanged), then move it and
      panel_voice*.rs into the voice crate behind a `VoiceHost` trait.
   c. Repeat for login (panel_login*.rs) and workspace_account_sign_in.
   d. Before moving panel itself, turn `Workspace::for_test` uses in panel
      tests into a test-support constructor or move those tests to the UI
      crate's integration layer.
4. Expected payoff: per-edit compile for voice/login work drops from ~8s to
   the ~1.5s class (the ui-core number for GPUI code of that size), for
   ~19% of UI churn. UI-only edits stay ~8s until panel/workspace
   themselves shrink.
5. Hot reload: every new crate is an rlib linked into the UI cdylib, so the
   host/UI ABI is unaffected. Globals keyed by TypeId (Sounds, Runtime,
   StatusAccounts, ...) stay in one crate instance per generation, so moving
   them is safe as long as the host never holds one.

Process notes: the shared tree had another agent's uncommitted gpui rev bump
twice during this work. Ctrl+R then fails with "plugin GPUI revision differs
from host" until the host restarts. Commits were staged by writing HEAD
plus only my hunks into the index (`git hash-object` + `update-index`) so
the other agent's in-flight Cargo.toml/Cargo.lock edits stayed out. The
UI test suite has two pre-existing flaky tests
(`workspace::tests::the_hint_chip_uses_only_spare_tab_space`,
`applet_runtime::tests::bundled_github_applet_renders_valid_documents`),
about 1 failure in 5 full UI runs, reproduced on a clean d206de0 worktree.
The offline screenshot sometimes captures the 3s startup beta toast
(same on d206de0 and after, 1 of 2 runs each); that is screenshot timing,
not a layout change.

## 12. Splitting jcode-desktop-ui, part 3: voice and login leaves (2026-10-08, cricket)

Step 3a of the plan in section 11, plus the first slice of 3b. Desktop
commits, each verified with `justrust test --workspace` (1746, then 1747
passed / 13 ignored), `justrust check --workspace --all-targets`, an offline
screenshot, and a Ctrl+R reload that activated a new UI generation
(release-desktop.py MANIFESTS and AGENTS.md's narrow-crate list updated):

- 1430ed1: new `crates/jcode-desktop-voice` (global_voice_input,
  global_voice_overlay, global_voice_session, panel_voice_tag -> `tag`:
  1.9k lines, 27 tests) and `crates/jcode-desktop-accounts-ui`
  (login_input, panel_login_catalog -> `catalog`, panel_login_status ->
  `connection`: 1.1k lines, 10 tests). All seven were real leaves: their
  only crate deps were `theme`/`render_stats` (ui-core), `harness`/`platform`
  (harness crate) and catalog -> connection (moved together). The UI
  re-exports them at their old paths (`crate::global_voice_*`,
  `crate::login_input`, `panel::voice::tag`, `panel::login::{catalog,
  connection}`), so no caller changed. Neither crate needs a
  `test-support` feature: the moved code has no `cfg(test)` behavior beyond
  its own tests. gpui via `gpui.workspace = true`, libc/zbus/async-io only on
  Linux, with the UI's libc features.
- 5bf7aff (3b, first slice): Panel's voice fields were already grouped
  in `voice: VoiceState` (panel_voice.rs), so the "extract VoiceState"
  half of 3b had already been done. What remains is moving the logic behind a
  `VoiceHost` trait. This commit moves the parts with no panel state into
  `jcode-desktop-voice::chrome`: shortcut keycap and tooltip, pill status
  labels (`short_voice_status`, `global_pill_*`), and the idle
  microphone/shortcut crossfade view `VoiceSwap` with its 2 tests.
  panel_voice.rs: 2085 -> 1840 lines.

### One-line edit benchmark (median of 3 after one warmup, `let _probe = N;` in a test fn)

| edit in | compile (median) | wall | runs (compile s) |
|---|---|---|---|
| UI `panel_voice.rs`, `-p jcode-desktop-ui --lib -- panel::voice` (old home of tag.rs) | 9.41s | 10.2s | 8.66, 21.08 (8 other rustc), 9.41 |
| voice `tag.rs`, `-p jcode-desktop-voice --lib -- tag` | 1.39s | 2.0s | 1.63, 1.39, 1.19 |
| accounts-ui `catalog.rs`, `-p jcode-desktop-accounts-ui --lib -- catalog` | 0.97s | 1.5s | 0.97, 1.02, 0.84 |
| voice `chrome.rs` (GPUI views), `-p jcode-desktop-voice --lib -- chrome` | 3.9s, contended | 5.2-6.8s | 4.89, 3.19, 4.02 (others 15.3 of 16 cores, 56-64 other rustc; own CPU 2.3-2.4s) |

Run ids (all 20261008-): UI 225901370-402380, 225910853-403300,
225932795-406079; tag 225654565-372432, 225656959-372833,
225659035-373219; catalog 225804731-391461, 225806260-393194,
225807855-394959; chrome 231001940-579130, 231009186-581497,
231014935-583831 (earlier contended set 230724158, 230731419, 230737731:
3.91, 3.24, 3.20).

Reading: edits in the new crates are 7-10x faster to compile than the same
code in the UI (about 1.0-1.4s vs 9.4s), the same class as ui-core and
model in section 11. The chrome number is inflated by contention (CPU use
of the build itself was about 2.4s, versus 11-12s of CPU for a quiet UI
edit). justrust startup was also 1.8-2.6s under that load, versus 0.45s
quiet. jcode-desktop-ui is now about 119k lines.

### Left for step 3b and later

- The rest of panel_voice.rs (1.8k), panel_voice_overlay.rs (0.9k) and
  panel_voice_pills.rs (0.7k) are `impl Panel` blocks. They read
  `self.voice.*` plus a few panel members (`input`, `session_id`,
  `status_line`, `submit_or_queue`, `supports_voice`, `terminal`,
  `todoist`, `orchestration`, `preview_state`, `is_side_document`,
  `is_pending_session`). That is a small, explicit surface for a
  `VoiceHost` trait. `Phase`, `VoiceTrace` and `VoiceState` can move as
  soon as their private-field access from Panel tests (`panel.voice.error
  = ...` in panel_voice_overlay.rs tests) goes through methods.
- Login (3c) and `Workspace::for_test` in panel tests (3d) are unchanged.

Process notes: during this work another agent released Desktop 0.5.1
(version bump in every manifest, including the two new ones, which it
picked up on its own) and the main Desktop instance exited. The 5bf7aff
reload was verified on the single-panel host (generation 3), and 1430ed1
on both hosts (main generation 4, single-panel generation 2).

## 13. Machine-wide build priority (src/sched.rs, 2026-10-08, jaguar)

Problem (index.jsonl, 477 runs over 48h): runs over 60s were 12% of calls
but 59% of wait time. The 30-200s tail of one-unit `test -p
jcode-desktop-ui` runs was mostly contention: other agents' builds used
9-15 of 16 cores, up to 58 foreign rustc, and the edit build got 0.5-1 core.

Mechanism: every recorded run moves itself (over the user bus, `busctl
StartTransientUnit`, ~40ms) into `app-justrust.slice/justrust-<id>.scope`
before starting cargo, so everything cargo starts is inside it.

1. v1, cgroup `cpu.weight` = 1000 * 60 / (60 + CPU-s used) (least attained
   service). **No effect.** This machine runs sched_ext `scx_lavd`, which
   ignores cgroup weights, nice and `cpu.max`. Synthetic check: a 4-thread
   probe next to a 48-thread hog got 4.4s of CPU at weight 10 and 4.9s at
   weight 1000; a 200% quota let a 16-thread hog use 67 CPU-s in 6s. Desktop
   probe runs 20261008-231039903 .. -231731304 vs off runs: the same.
   The weights stay in (they work under the normal fair scheduler).
2. v2 (c068015), affinity. lavd honors `sched_setaffinity`: the same probe
   got 13.3s of CPU when the hog was pinned to 8 CPUs, versus 3.6s. So while
   another justrust build is young (< 200 CPU-s) and actively running, a
   build that has used more than 200 CPU-s pins all its threads to the
   slower half of the CPUs (by `cpuinfo_max_freq`: here E and LP-E cores
   8-15, keeping P cores 0-3 and E cores 4-7 free). It gets every CPU back
   within 0.5s once no young build runs. Idle young scopes (a `justrust run`
   server) do not count.

Benchmark `PROBE=desktop bench/sched-contention.sh 4 2 "off on off on off on"`:
two cold `justrust check -p jcode-desktop-ui` loops (fresh target dirs,
depcache and build-script cache off) as other "agents", and as the probe a
one-line test-fn edit in fps_counter.rs + `justrust test -p jcode-desktop-ui
--lib -- fps_counter` in a scratch copy. Same load in each mode, alternated.

| probe wall, 12 runs each | median | mean | max |
|---|---|---|---|
| `JUSTRUST_SCHED=0` | 24.5s | 25.4s | 53.2s |
| sched on (affinity) | **12.9s** | **12.9s** | **16.0s** |

Off: 9.3, 9.6, 11.2, 12.9, 14.1, 22.5, 26.4, 32.8, 33.9, 37.0, 41.5, 53.2.
On: 10.3, 10.8, 11.3, 11.5, 11.9, 12.9, 12.9, 13.3, 13.7, 14.3, 15.4, 16.0.
Run ids 20261008-234357414 .. -235809675. The quiet-machine number is
about 9s, so contention cost drops from about +15s median / +44s worst to
about +4s / +7s.

Cost to the long builds: the loads yielded 47-55s each while probes ran.
No clean wall comparison (the bench kills loads at mode switches), but
loads in "on" rounds reached 867-879 of 887 units in 115-122s, while "off"
loads finished all 887 in 121-147s: no visible slowdown, because the
probe only needs 1-4 cores and the loads keep 8.

Open: the threshold (200 CPU-s) and the half split are guesses that fit
this machine. A per-build jobserver (fewer rustc threads for old builds)
would also cut the 50+ runnable threads that make even the reserved cores
contended under the fair scheduler.

## 14. Jcode TUI repo: parallel front-end was bypassed, giant crates (2026-10-09)

The Jcode TUI repo (`~/jcode`) had `scripts/rustc-parallel-frontend`, but
only `scripts/dev_cargo.sh` exported it as `RUSTC_WRAPPER`. Agents call
`justrust` (and plain `cargo`), which skip dev_cargo.sh, so jcode-base
(139k lines), jcode-app-core (160k) and jcode-tui (221k) compiled with a
single-threaded front-end. Fixed in jcode 04c7d2b04 by setting
`build.rustc-wrapper` in `.cargo/config.toml` (CI and Windows set
`CARGO_BUILD_RUSTC_WRAPPER=""`).

Measured on the XPS (16 threads, others using 2-4 cores):

| Scenario (`check -p jcode-tui`)                  | 1 thread | -Zthreads=8 |
|--------------------------------------------------|---------:|------------:|
| Non-incremental after an upstream edit (x2)      | 44.0 / 45.3s | 24.0 / 23.3s |
| Incremental, new pub fn in jcode-base (x3)       | 9.2-10.1s | 10.5-11.0s |
| `test -p jcode-base --lib --no-run`, same edit (x2) | 13.9 / 15.5s | 13.3 / 13.5s |

So the threaded front-end halves full rebuilds of a big crate (app-core
18 -> 10s, base 11 -> 6s, tui 10 -> 5s) but does nothing for warm
incremental edits, which stay at ~3s per giant crate in the chain. That
floor is crate size, not flags. New finding `giant_crate` (local crate
>= 40k lines in `src/` costing >= 5s of the run) tells the agent to split,
with the crate's line count and seconds; it also flags single-threaded
front-ends. Cost estimate assumes split-out crates of ~20k lines.
First real report: run 20261009-005101282-1501305, jcode_base 138k lines,
41.0s, ~35s attributable.

## 15. `justrust split`: which code to move out, from recorded edits (2026-10-09)

`giant_crate` and `upstream_cascade` said "split", not what. `justrust split`
answers that from this workspace's recorded runs plus a regex module graph:

1. Each run's `Dirty <pkg>: the file X has changed` lines locate the edit
   (module file, crate root, or build-script input) and the run's cost is the
   edited crates plus their dependents' unit time (dev-dependency edges are
   ignored: they only affect the crate's own tests and otherwise create false
   cycles such as jcode-base -> openrouter-runtime -> jcode-base).
2. Every crate's module tree is parsed from `mod` items (with `#[path]`), and
   `crate::`/`super::`/`self::`/child paths resolve to the deepest module they
   name. `pub use` re-exports are not edges. Moving module M forces out every
   module that references it, transitively; above 35% of the crate it is
   reported as entangled with the references to cut.
3. Inherent `impl T { .. }` blocks for a type defined in a module that stays
   are a hard blocker (Rust only allows them in the defining crate), so those
   candidates are marked "needs prep".
4. The saving replays the runs whose edits in that crate all fall inside the
   moved code: the crate costs only its moved share of lines, dependents that
   do not use the moved code (word search, including `crate::` through glob
   re-exports like `pub use jcode_base::*`) drop out.

First run on ~/jcode (96 runs with edits, 2105s in edited crates and
dependents, last 30 days), 0.5s wall. Correction: 19 of those runs (and
more in other sections of the report) were jcode-desktop runs, because the
run filter matched `/home/jeremy/jcode` as a string prefix of
`/home/jeremy/jcode-desktop`. Fixed to a path-component match; with it the
report has 77 runs and 1918s. Numbers below are from the first run unless
marked.

- Biggest costs are not modules at all: jcode-config-types' single 1.7k-line
  lib.rs (39 runs, 566s cascade into 17 dependents), jcode-base's root file
  (127s), jcode-build-support (180s), and `docs/` as a build-script input of
  jcode-app-core (169s). Git hunk headers name the churned items in a root
  file (AgentsConfig 10 edits, WebSearchConfig 7, DiffDisplayMode 6) and which
  dependents use them; for config-types nearly every dependent uses them, so
  moving items does not help, the fix there is fewer dependents per item.
- `jcode-base::external_auth` (1030 lines, nothing else in jcode-base
  references it, checked by hand) was ranked first at ~24s over 2 runs,
  sparing 10 provider/runtime crates. Both of those runs were jcode-desktop
  runs (`check -p jcode-desktop-ui` at 31.8s and 17.7s, Desktop building
  jcode-base as a path dependency), so the ranking came from the prefix bug.
  After the fix no jcode run edited it and it is no longer listed.
- Needs prep: `config::config_file` (~31s), `provider::startup` (~16s), and
  three `tui::app::*` files (13-19s each) are all `impl Config`/`impl
  MultiProvider`/`impl App` blocks split across files, so moving them first
  needs free functions or an extension trait.
- Entangled: `session::persistence`, `provider::openrouter`, `auth::cursor`
  drag along 83% of jcode-base; the report lists the referencing modules.

Takeaway: in this repo the cascade cost is dominated by small, widely used
root files (config-types, build-support) and a build-script input, not by
modules inside the giant crates. Splitting the giant crates still matters for
their own incremental floor (section 14).

Caveat: the savings are a model, not a measurement. The skipped dependents
follow from cargo's rules, but the new crate's cost is assumed to equal its
share of the old crate's lines, and users are found by word search.

Validated on `jcode-base::external_auth` in a scratch worktree at jcode
04c7d2b04 (moved to `crates/jcode-external-auth`, `crate::` -> `jcode_base::`,
`pub(crate)` -> `pub`, app-core re-exports it under the old path, workspace
checks, the 10 moved tests pass). Same body-only edit, `check --workspace`,
three runs each:

| | wall | units rebuilt | jcode_base | provider/runtime crates |
|---|---|---|---|---|
| before (runs ...011923788, ...011935867, ...011948463) | 11.6 / 12.5 / 11.5s | 17 | 3.0-3.5s | 10 rebuilt |
| after  (runs ...012002880, ...012011969, ...012019539) | 8.8 / 7.1 / 7.4s | 7 | not rebuilt | none rebuilt |

- The predicted set of spared crates was exact. Cargo's rebuild reasons
  before: jcode-base, the 10 provider/runtime crates (anthropic, antigravity,
  copilot, cursor, gemini, grok-build, openai, openrouter, doctor) plus
  tui-permissions, then app-core, tui, jcode. After: only jcode-external-auth,
  app-core, tui, jcode, the crates the report listed as users. The new crate
  costs 0.1s.
- Saving: ~4.1s per edit (35%), measured. The report's ~24s over 2 runs
  (~12s each) is not comparable: those were Desktop runs (see the
  correction above), so the per-edit figure was replayed from a different
  workspace's unit times. The estimate replays old unit times in any case,
  so it overstates savings after the toolchain gets faster; the structure
  (what stops rebuilding) is what to trust.
- One manual step the report does not mention: the moved tests used
  `jcode_base::storage::lock_test_env`, gated on jcode-base's `test-support`
  feature, so the new crate needs it as a dev-dependency feature.

### 15b. Second real split (Desktop) and a calibrated saving model

Split `jcode-desktop-ui-core::scrollbar` (481 lines) into
`jcode-desktop-scrollbar` in a scratch worktree of jcode-desktop 400400d
(sibling `jcode` symlink for the `../../../jcode` path deps). `check
--workspace --all-targets` ok (one pre-existing `pulse_text` warning), the 4
moved gpui tests pass. Same body-only edit, `check -p jcode-desktop-ui`, 3x:

| | wall | rebuilt (cargo) |
|---|---|---|
| before (...144526866, ...144530399, ...144533461) | 3.0 / 2.9 / 3.2s | ui-core, voice, accounts-ui, ui |
| after  (...144548605, ...144551673, ...144554725) | 2.6 / 2.9 / 2.7s | scrollbar, ui |

Spared set predicted exactly again (ui-core, voice, accounts-ui). Measured
saving ~0.3s/edit, i.e. not worth doing; the old report said ~3.5s/edit.

Why the old estimate was 3-10x high: it replayed each historical run's own
unit times. 2 of the 5 scrollbar runs were on a machine with 14 of 16 cores
taken by other builds (60+ foreign rustc), where ui-core took 2.8-3.2s
instead of ~0.5s, and test units (`ui_core (test)`) were counted as if a
split would spare them.

Model now: per package, the median over quiet `check`/`clippy` runs (other
processes under a quarter of the cores; check because it is the edit loop
and test/build units do codegen, 2-3x the cost), counting only lib/bin units
of packages cargo marked dirty. Validation via the hidden
`justrust split --runs <ids>`, which replays given runs against the current
checkout, with typical costs taken from the workspace's own history:

| candidate | old estimate | new estimate | measured |
|---|---:|---:|---:|
| jcode-base::external_auth | ~12s/edit | 7.7s/edit | 4.1s/edit |
| desktop ui-core::scrollbar | ~3.5s/edit | 0.7s/edit | 0.3s/edit |

Correction: a first version of this table showed 3.8s and 0.5s. That was
circular: the typical costs were computed from the same benchmark runs the
estimate was checked against. With costs from history only, the estimate is
still about 2x high on both. Cause, from the per-run data: history mixes
single-threaded and threaded-front-end runs (jcode-base check 2.5-15s across
the switch in section 14), and the cascade crates (10 provider runtimes) have
few quiet check samples, so their medians come from test/build runs.
Restricting to threaded runs gives 6.0s, but then most of the provider crates
have no samples at all. The estimate improves as history accumulates under
the current toolchain; it is a ranking signal, not a forecast.

Also fixed: the root package's sources included every member crate under
it (and `target-release/`), so `jcode-desktop` was listed as a user of
anything any member used. The predicted users now match the crates that
still rebuilt in both splits (jcode: jcode, app-core, tui; Desktop:
jcode-desktop-ui). They are not the set of manifests to edit: in the jcode
split only app-core needed the dependency, and tui and jcode reached the
module through app-core's `pub use jcode_external_auth as external_auth`.
The report now says so.

Remaining limits: two validation points only, both overestimated about 2x
(the Desktop one is also inside the run-to-run spread of 2.6-3.2s walls).
Which crates stop rebuilding was exact both times; the seconds are not. The
saving is a per-package median, so a candidate seen in few quiet runs
inherits whatever runs exist; and the moved share is
by lines, which ignores that a small crate has fixed overhead (~0.1-0.2s
per new crate in both splits).

## 16. Skipping downstream rebuilds after body-only edits: measured, not viable as a wrapper (2026-10-09)

Question: can justrust skip rebuilding dependents when an upstream edit
leaves the public interface unchanged, with a background full build as the
correctness check?

Size of the prize (~/jcode, 77 recorded runs with edits): 930s of 2199s wall
(42%) was local units rebuilt only because a dependency rebuilt. Per crate,
a cascade-only rebuild costs: jcode-base median 6.1s, jcode-app-core 4.1s,
jcode-tui 3.1s, jcode-protocol 1.9s (check runs).

What a wrapper can see. Scratch workspace `up -> down -> top`
(`~/.jcode/scratch/rmeta-exp`), rustc 1.98.1, `cargo check`:

| Edit in `up`                         | `up` rmeta bytes changed |
|--------------------------------------|-------------------------:|
| non-generic fn body                  | 32 (crate hash only)     |
| private fn body                      | 32                       |
| `#[inline]` fn body                  | 32                       |
| generic fn body                      | 1106 (MIR is exported)   |
| comment that shifts lines            | 1129 (spans)             |
| pub field type                       | 34                       |

The 32 bytes are the crate hash (SVH, `rustc -Z ls=root`), which covers
private items too. So:

1. A byte hash of the rmeta changes on every edit, and the SVH is not an
   interface hash. A body-only edit and a pub field type change both differ
   by just the SVH, so the wrapper cannot tell them apart from the outside.
2. Skipping is not even mechanically possible: the downstream rmeta embeds
   the upstream SVH. Keeping `down`'s old rmeta and compiling `top` against
   the new `up` fails with E0460 ("found possibly newer version of crate `up`
   which `down` depends on"). Rewriting SVHs would be required, and that is
   exactly where upstream ran into ICEs and miscompilations: DefIds shift
   when items are added, invalidating artifacts that were not rebuilt.
3. rustc's own incremental cache already absorbs part of it: in the scratch
   workspace a 9k-line dependent rechecks in 0.6s after an upstream body edit
   vs 1.75s cold; in jcode a cascade recheck of jcode-tui is ~3s.

Upstream status: this is the Rust project's "Relink, don't rebuild" (RDR)
work (compiler-team MCP 790, project goal 2025h2, rust-lang/rust#143249 for
comment-only changes). The 2025h2 goal was marked "will not complete"; it is
listed again on the 2026 Fast Builds roadmap with a 5-10x target for
body-only changes.

Verdict: no-go as a justrust wrapper feature. A correct version needs a
rustc change (an interface hash that excludes bodies, private items and
spans, plus SVH-stable DefIds), which fits the vendored/forked toolchain
plan in docs/toolchain.md and should track upstream RDR rather than
duplicate it. Until then the levers that work today are the ones in 14 and
15: fewer dependents per hot file (`justrust split`) and the threaded
front-end for the rebuilds that remain.

Real-project confirmation (jcode 04c7d2b04, scratch worktree, run
20261009-011923788-1604530): a body-only edit (`&& 1 > 0` inside
`can_prompt_for_external_auth`) rebuilt 16 dependent crates, every one with
cargo reason "the dependency ... was rebuilt", 11.6s wall. The E0460 failure
of a forced skip was reproduced only in the scratch workspace; forcing it on
jcode would mean planting stale rmeta files in a target dir, which proves
nothing more than the minimal case.

## 17. `justrust split --apply`: deterministic splits in the user's own tree (2026-10-09)

Why: AI agents split crates by hand, turn by turn. Two recorded Desktop
split sessions: tigress (2 crates, 7 modules, ~2h) made 87 tool calls (71
bash, 9 compiles), 7.9M prompt tokens processed (97% cache reads), 45k
output, peak context 141k; cricket (74 min) made 192 tool calls, 30.5M prompt
tokens, 161k output, peak 271k. Most turns were mechanical (grep users, read,
sed paths and manifests, git). Each turn resends the growing context, so cost
scales with turn count, not with judgment.

What it does (`src/split_apply.rs`), in the working tree the user is in:

- Refuses unless `git status --porcelain` is empty (untracked included), so
  HEAD is the pre-split state. One commit afterwards, with the user's git
  identity, naming the pre-split commit; undo is `git revert` or
  `git reset --hard <pre>`.
- Refuses what it cannot do safely: modules other modules reference (the
  closure from section 15), inherent impls of types that stay, crate-root
  uses, test-only modules, feature-gated code, optional deps, glob
  re-exports of the module, and crate-identity macros (`include_*!`,
  `CARGO_MANIFEST_DIR`, `CARGO_PKG_NAME`, `CARGO_CRATE_NAME`,
  `module_path!`), whose values silently change with the crate. Warns
  when log/tracing macros will change target.
- Rewrites: moved files (`crate::` -> old crate, `super::` chains that leave
  the subtree, tracking inline `mod {}` blocks and skipping strings and
  comments; top-level `pub(super)` -> `pub`); users (`old::m` paths, `use
  old::m;` -> `use new as m;`, multi-line `use old::{..}` groups, names the old
  root re-exported from the module); glob re-exporters (`pub use old::*`) get
  `pub use new as m;` so `crate::m` keeps resolving.
- Manifests with toml_edit: package fields, lints and dependency specs
  copied from the old crate (only crates the moved code names), the old crate
  by the spec its users use, workspace members and
  `[workspace.dependencies]`, user deps in their existing style.
- Verifies: `check -p new --all-targets` with two rule-based fixes (missing
  dependency of the old crate; old crate's `test-support` feature for moved
  tests), formats only touched files that were rustfmt-clean at HEAD (stdin,
  so `mod` declarations are not followed), `check --workspace
  --all-targets`, and `test -p new`, which must run at least as many tests
  as the moved files declared. Any failure restores every touched path from
  HEAD and removes the new crate dir.

Checks:

- `bench/split-apply-e2e.sh` (fixture workspace, real cargo and git, 15
  checks): success path (clean tree, one commit on the pre-split commit,
  tests and app output unchanged, emptied dir removed), dirty-tree refusal
  (tree untouched), root re-exported name rewritten in a dependent, a
  behavior change caught by tests (tree byte-identical by sha256, HEAD
  unchanged), refusals for an entangled module and `CARGO_PKG_NAME`.
- Real jcode (local clone at 04c7d2b04): `--apply
  jcode-base::external_auth` succeeded unattended in 4.5 min (cold clone
  build), 10/10 moved tests, 24 caller tests in `jcode` pass. Same edit,
  `check --workspace`, 3x: 10.9/11.0/10.5s before, 8.6/7.2/7.6s after (the
  hand split in section 15 measured 11.9 -> 7.8s).
- Real Desktop (clone at 400400d with a committed sibling jcode): `--apply
  jcode-desktop-ui-core::scrollbar --name jcode-desktop-scrollbar`
  succeeded unattended in 1.7 min, 4/4 gpui tests, diff of 7 files and
  rustfmt-clean (the 6 files `cargo fmt --check` flags were already
  unformatted before). `check -p jcode-desktop-ui`: 2.8/2.7/2.9s before,
  2.2/2.3/2.7s after.
- Refusal on the user's real ~/jcode-desktop, which has 3 uncommitted files:
  refused with the list, tree untouched.

Bugs the real runs found and fixed before this commit: `super::` inside
inline test modules was rewritten as if it escaped (scope tracking added);
module depth was taken from file paths instead of the module tree; users
reached only through a glob re-exporter got a needless dependency (and
`add_user_dependency` failed on them); `use old::{.., m, ..}` became `use
new;`, losing the name `m`; whole-directory rustfmt reformatted files the
split never touched.

What an agent now does for a ready candidate: `justrust split` shows
`apply: justrust split --apply <crate>::<module> --dry-run`, the agent
commits its work, runs it, and reads one result line. The report's `apply:`
hint appears only for candidates with no blockers.

## 18. One feature set per workspace: no dependency rebuilds when switching packages (2026-10-09)

Code: `unify_features` in `src/record.rs`. Bench: `bench/feature-switch.sh`.

### Why the depcache held so many variants

`justrust cache` showed 21.8 GB in 9,106 entries. Grouping entries by cargo's
own output file name (which already encodes version, features, profile, and
dependencies), 9.7 GB were extra entries for the same output under different
keys, and most of the rest were genuine feature variants (`aws_sdk_bedrock`
11 builds, `gpui` 17, `reqwest` 32, `syn` 29).

The cache key itself is stable. With `JUSTRUST_DEPCACHE_EXPLAIN=<dir>` (dumps
the normalized key inputs of every unit), two fresh target dirs and a
separate checkout of Desktop produced identical keys for all 643 crates and
833/833 hits. The duplicates came from builds that really were different:
cargo resolves features per command, so `check -p jcode-desktop-model` builds
`subtle` with no features and `mio` without `log`, while `-p
jcode-desktop-ui` builds them with `default,i128,std` and `log`. Each
different set means a different `-C metadata`, different `--extern` hashes
for everything downstream, and new entries.

### Fix

Every recorded cargo run sets `CARGO_RESOLVER_FEATURE_UNIFICATION=workspace`
(cargo's `-Zfeature-unification`): features are resolved once for the
whole workspace, so every `-p` selection compiles the same build of each
dependency. The nightly gate is opened with cargo's channel override, not
`RUSTC_BOOTSTRAP`: rustc reads `RUSTC_BOOTSTRAP`, so cargo fingerprints it,
and setting it rebuilt 304 units of desktop-ui once (measured). The override
is read only by cargo. Off with `JUSTRUST_UNIFY_FEATURES=0`, and an explicit
`CARGO_RESOLVER_FEATURE_UNIFICATION` (for example `selected`, cargo's
default) wins.

### Measurements

`bench/feature-switch.sh`: fresh target dir, private depcache, `check -p`
each package twice through the list. Machine busy (other agents building).

Jcode Desktop (`model, harness, ui, desktop`):

| | pass 1 compiled / deps | pass 1 wall | pass 2 |
|---|---|---|---|
| unify on | 166+329+392+8 = **895** / 764 | 91s | 0 compiled |
| unify off | 114+395+642+8 = **1,159** / 971 | 121s | 0 compiled |

Jcode (`jcode-base, jcode-tui, jcode-app-core, jcode-protocol`):

| | pass 1 compiled / deps | pass 1 wall | pass 2 |
|---|---|---|---|
| unify on | 418+230+0+0 = **648** / 525 | 110s | 0 compiled |
| unify off | 412+354+85+85 = **936** / 737 | 179s | 0 compiled |

With unification, once any package is built the others need no further
dependency work (jcode-app-core and jcode-protocol: 0 compiled, 0.3s,
instead of 85 units and 34s/14s). That is 23% and 31% fewer units compiled
for the same four commands, and one depcache entry per dependency instead of
one per feature combination.

On the warm main checkout, switching `model -> harness -> ui -> desktop ->
model -> harness -> ui` compiled 0 units on every step (0.4-0.5s each).
Before, `check -p jcode-desktop-model` after `-p jcode-desktop-harness`
rebuilt 27 units and the reverse 131.

Correctness: `justrust test -p jcode-desktop-model` (140 passed) and
`justrust test -p jcode-protocol` (89 passed) with unification on. The
first unified run in an existing target dir rebuilds dependencies whose
feature set grows (one time, up to ~130 units in Desktop); after that,
plain cargo without unification and justrust with it disagree on features,
so mixing them in one target dir rebuilds the differing dependencies.

Note: in this repo, `justrust build --release` goes through a build slot,
so `target/release/justrust` is not updated. Use `JUSTRUST_SLOTS=0` when the
binary itself is the output.

## 19. Split recommendations at build time, from incrementally stored evidence (2026-10-09)

`giant_crate` and `upstream_cascade` told the agent "split, run `justrust
split`" without saying what. The specific answer needs the workspace's edit
history, a module graph and, for root files, git churn, and `justrust split`
recomputed all of it per call: 1.37s on ~/jcode (re-reading 1,370 run
summaries, re-parsing ~1,100 source files, 4 `git log`s).

Every piece is now stored by whatever already has it (`src/split_index.rs`):

| Piece | Written by | Store |
|---|---|---|
| Edit record: changed files, local unit times, quiet/check | every recorded run, local and remote, when its summary is written | `split/edits.jsonl` (append; old runs back-filled once from `summary.json`) |
| Per-file parse facts: lines, `mod` items, written module paths, types, inherent impls | any module-graph build; only files whose size/mtime changed are re-parsed | `split/facts-<ws>.json` |
| Root-file churn (git hunk headers) | report/refresh, keyed by HEAD and day | `split/churn-<ws>.json` |
| Per-file hint: the concrete recommendation for edits to that file | niced, detached `justrust split --refresh-hints` after builds with edits (at most once a minute per workspace, flock'd), and every `justrust split` | `split/hints-<ws>.json` |

A finishing build reads only the hints file and attaches the hints for the
files it edited (14 us measured), and the costliest of `giant_crate` /
`upstream_cascade` shows it instead of the generic text. Path resolution
was split from extraction (`written_paths` stores paths as written,
resolution runs against the module tree), so facts are cacheable per file.

Measurements on ~/jcode (release build):

| | before | after |
|---|---:|---:|
| `justrust split`, warm | 1368 ms | 196-274 ms |
| `justrust split`, empty `split/` (back-fill + parse) | 1368 ms | 587-967 ms |
| `--refresh-hints` (background, niced) | n/a | 156 ms |
| hint lookup at the end of a build | n/a | 14 us |

Regression: on a frozen copy of the run history (1,402 summaries), the
`justrust split --days 365` report is byte-identical before and after for
both ~/jcode (161 lines) and ~/jcode-desktop (63 lines), cold and warm.

End to end, in ~/jcode: appending a comment to
`crates/jcode-base/src/config/config_file.rs` and running `justrust check -p
jcode-app-core` (6.8s, run 20261009-213927583-3383129) printed:

> jcode-app-core rebuilt only because jcode-base changed (edited
> crates/jcode-base/src/config/config_file.rs) (3.2s). Specifically, edits to
> jcode-base::config::config_file cost ~17.3s each here (2 recorded runs).
> Moving jcode-base::config::config_file (863 of 134594 lines) into its own
> crate that depends on jcode-base would stop them rebuilding jcode-base and
> 13 dependents, saving ~15.8s per edit. First turn its `impl Config` blocks
> (the type stays in config) into free functions or an extension trait ...

The same build appended its edit record and triggered the background
refresh (hints file rewritten 0s after the run).

Hint kinds: a movable module (saving, moved lines, spared dependents,
remaining users, and either the `--apply` command or the blocker to fix
first), an entangled module (share of the crate it would drag along and
the references to cut), a crate root file (most edited items and how many
dependents use each; says when moving them would not help), a build-script
input (embed in a leaf crate or load at runtime).

Limits: hints are as fresh as the last refresh (at most a minute behind the
latest build plus the refresh time); the first build in a workspace with no
`split/` state shows the generic advice and starts the refresh. The savings
model is unchanged from 15b (ranking signal, about 2x high in two
validations).

## Remote sync daemon protocol v2 (ed048bb): sync cost unchanged, remote post-build tail found

Warm `justrust remote check` on c7i.8xlarge us-west-2, same machine, before
(v1 daemon) and after (v2):

| workload | v1 | v2 |
|---|---|---|
| ~/justrust no-op | 0.6s (sync 30-33 ms) | 0.6s (sync 29-31 ms) |
| ~/justrust one-line edit | 0.6-0.8s (sync 32-49 ms) | 1.1s (sync 30-31 ms) |
| jcode-desktop-model no-op | 0.6-0.7s (sync 47-49 ms) | 0.6-0.7s (sync 46-50 ms) |
| jcode-desktop-model one-line edit | 0.6-0.7s (sync 79-81 ms) | 0.7-0.9s (sync 79-86 ms) |

Sync is unchanged. The ~/justrust edit total moved because of the machine,
not the daemon: run on the machine directly, a `justrust check` after
`touch src/remote_watch.rs` takes 1.04s with the v1 binary (213cd03^), with
213cd03 and with v2 alike, while plain `cargo check` takes 0.50-0.55s. An
strace (run 20261010-044440144-6736 on the machine) shows the last rustc
exit at +0.82s and justrust exiting at +1.11s: about 0.29s after the build
plus ~0.06s before cargo starts, in the remote justrust wrapper, not in sync.
Since all three binaries show it today, something on the machine changed
since the v1 measurement (cause not identified yet). Worth chasing next.

Daemon ping during a 2,102-file cold mirror of a jcode clone: 0.2-0.4 ms
(previously it shared the one state lock with rsync and pushes).

## Hosted backend end to end (2026-10-09, local wrangler dev + real AWS dev stack, c7i.4xlarge 16 vCPU us-east-1)

Client `src/remote_hosted.rs` against the build-host API (`/v1/build/host*`).
146 ms round trip from the laptop (us-east-1, vs 29 ms to the us-west-2 aws
backend), so small edits cost more than on aws.

| step | time |
|---|---:|
| first `remote up`, server creating the machine | ~75 s (server: 84-119 s launch to ready) |
| `remote up` with the host running (fresh key + connect + new master) | 4.6-4.7 s |
| `remote down`, then `remote up` (wake, new public IP pinned) | 43-46 s |
| `remote check` ~/justrust, cold machine | 11.9-12.5 s (sync 2.9-5.7 s) |
| `remote check` ~/justrust, warm no-op | 0.7 s (sync 71 ms) |
| `remote test` ~/justrust, 157 tests | 10.3-11.5 s |
| `remote check -p jcode-desktop-model`, cold, two roots | 25.4-25.6 s |

Server bugs found by the run and fixed by the server side: c7i.8xlarge over
the 16 vCPU on-demand quota (503, client guidance correct), a readiness
marker broken by systemd `$(...)` expansion (wake stuck at booting, client
timed out cleanly at 360 s), and systemd-tmpfiles resetting /home's ACL mask
(fixed with jcode-mkdir creating /home/<u>).

## 20. Benchmark suite on pinned snapshots, and 0.5s lost at every build exit (2026-10-10)

`bench/suite.py` replaces ad-hoc single-scenario scripts as the basis for
speed claims: pinned `git archive` snapshots (jcode-desktop 7d38fb7 + jcode
abff94eeb, justrust f97fcc2), private target dirs, a unique edit per
iteration, 1 warmup + 5 measured runs, phases read from each run's
summary.json, JSON results in `bench/results/`. See `bench/README.md`.

Baseline (f97fcc2, `bench/results/20261010-005520-f97fcc2d7490.json`),
median wall as the agent sees it: reference loop `desktop-test-edit` 8.06s,
body-edit test 8.05s, body-edit check 2.54s, first type error 2.04s, no-op
test 0.53s, upstream ui-core body 9.54s, ui-core new pub fn (check) 3.04s,
jcode-core body 15.05s, justrust small-crate test 3.03s.

The first run showed every agent-visible wall time on a 0.5s grid (x.03 /
x.53). Agent-visible wall minus recorded wall was median 0.27s, max 0.52s:
`sched::Scope::finish` joined the cgroup weight thread, which slept
`UPDATE_EVERY` (0.5s) between checks. Fixed in c219b49 (the thread waits on a
channel with a timeout and wakes on drop). After
(`bench/results/20261010-010147-c219b496046c.json`): overhead median 0.034s,
max 0.082s. desktop-body-check 2.54 -> 2.27s, desktop-type-error 2.04 ->
1.78s, no-op 0.53 -> 0.47s (all with non-overlapping ranges); the long
scenarios moved by about -0.4s, within their run-to-run spread.

Where the reference loop goes now (7.59s): startup 0.48, frontend 3.83,
codegen 1.11, incremental persist 0.77, link 0.47. jcode-core body edit:
16 units, frontend 10.6s, the worst warm case.

Cold baseline (`bench/results/20261010-011659-cold.json`, 1 warmup + 3
runs, fresh target dir each, check -p jcode-desktop-ui, 887 units):
depcache on 16.1s (15.8-16.4), every cache off 70.4s (70.2-70.9). The
desktop app itself kept 7-8 other cores busy, so all samples are flagged
noisy. A first attempt measured 0 depcache hits: the suite forced
`JUSTRUST_PASSES=always`, and units compiled with -Ztime-passes are never
stored. The suite no longer forces pass timings for cold scenarios.

Suite noise check (A/A): re-running six warm scenarios on the unchanged
build (`bench/results/20261010-012107-ba5914b-repro.json` vs
`20261010-010147-c219b496046c.json`, about 20 min apart) moved medians by
-9% to +2%, and one scenario (desktop-upstream-sig, -8%) had disjoint
ranges. So `compare` now also needs the median to move more than
`--floor` (default 10%). With it, the A/A pair reads all "within noise"
and the sched fix still reads faster on body-check (-11%), type-error
(-13%) and no-op (-11%). Exit overhead (agent wall minus recorded wall),
the direct measure of that fix: 0.271/0.515s med/max before, 0.034/0.082s
after, 0.036/0.069s on the A/A rerun. The rerun also exposed a flaky
justrust test (ETXTBSY exec race in buildscript tests, fixed ba5914b); the
suite now records a failing test instead of aborting.

## 2026-10-10: slot refresh copied a half-written shared target (jcode)

Symptom: `justrust test -p jcode-tui --lib -- <filter>` in ~/jcode failed with
197 errors starting `E0463 can't find crate for jcode_app_core` /
`jcode_tui_permissions`, three runs in a row, while `cargo test` on the same
tree passed. Each run printed `slot 0: copied 0 new and 8 rebuilt units (21
files) from the shared target dir`.

Context: a second justrust run in a `git worktree` of the same repo was using
`CARGO_TARGET_DIR=~/jcode/target` (the shared target) at the same time, and
another agent session owned slot 1. The slot refresh copied a partial set of
rlibs/rmetas from the shared target mid-rebuild, leaving slot 0 with metadata
for crates whose artifacts were missing.

Recovery: one plain `cargo test` rebuilt the shared target; the next justrust
run copied 47 new + 10 rebuilt units and passed.

Fix idea: refresh only from a shared target that is not locked by a running
cargo (respect `target/debug/.cargo-lock`), or verify copied rmeta/rlib pairs
exist before trusting a refreshed slot and fall back to a rebuild.

### Root cause and fix (2026-10-10)

Not a half-written copy. The lock check was already right: cargo 1.98 holds
`.cargo-build-lock` exclusively for the whole build (seen with `lslocks`),
so the refresh's shared flock cannot be taken during a build. The real
cause is that **unit hashes do not depend on the checkout path**: two
copies of a workspace in different directories produce identical
`.fingerprint/<pkg>-<hash>` names (verified with a 2-crate workspace copied
to two dirs). The worktree at `~/.jcode/scratch/base2-wt` built with
`CARGO_TARGET_DIR=~/jcode/target`, so its `jcode-app-core-463b...` and
`jcode-tui-permissions-d481...` (different sources) landed in the shared
dir under the same names as ~/jcode's. Refresh saw them as newer and copied
them into slot 0 with fresh mtimes, and cargo then trusted them for ~/jcode's
sources. The extern rlibs loaded fine on their own (checked with rustc),
but the set came from another source tree, so `jcode_tui` failed with E0463
at the crates' first use. Six runs in a row failed in slots 0 and 1
(runs 20261010-025433901-1482017 .. -025705623-1502688).

Fix (src/slots.rs):

- Refresh never copies workspace-local units. Locality comes from the
  unit's dep-info: rustc writes workspace sources as relative paths and
  everything else as absolute ones. It is decided per package, so a
  member's run-build-script units are skipped too, and only the first 4 KB
  of one `.d` per package is read. Refresh exists for dependency changes.
  The slot always builds its own members, which cargo's fingerprints
  handle correctly.
- Safety net: after a failed slot run whose output has E0463/E0460 for
  crates the slot has units for, those units' fingerprints are deleted and
  justrust says so, so the next run rebuilds them instead of failing the
  same way again.

Checked on ~/jcode: plain `cargo check -p jcode-tui-permissions` rebuilt 25
shared units (registry and members). The next `justrust check` refreshed
9 units in 0.2s, all registry crates (azure_core, reqwest, ...), and none
of the members (jcode-base, jcode-tui-permissions stayed slot-built). It
then compiled 0 units.

## 2026-10-10: `justrust clippy` reported 0 compiled units

cargo runs workspace members under clippy as `$RUSTC_WORKSPACE_WRAPPER
$RUSTC <args>`, and clippy-driver compiles in-process, so the shim never ran
and the report said "0 compiled, compile 0.00s, startup 1.36s" while cargo
printed `Checking justrust` (run 20261010-061717076-2544870). Pointing the
wrapper at the shim is not an option: cargo hashes the wrapper path into
unit hashes (two wrapper paths gave different `.fingerprint` names in a
scratch workspace), which would stop sharing artifacts with plain `cargo
clippy`. The summary now reconstructs these units from the sampled
clippy-driver processes (200 ms resolution, no pass timings). Same edit
after the fix: "3 compiled (3 local), compile 1.99s" (run
20261010-062530043-2617015).

## Remote vs local on uv, for the /rust landing page (2026-10-10)

uv at 44b2e58 (674 units, 758 lockfile crates). Local: this laptop, 16
threads, load 7 to 13 from other agents. Remote: hosted dev build host
(c7i.4xlarge, 16 vCPU, us-east-1) through local justrust-cloud and backend.

| case | local | remote (client wall) |
|---|---:|---:|
| cold `check --workspace`, no depcache | 70.3 s, peak 3.4 GB RSS, 563 CPU-s, target 1.8 GB | 52.2 s incl. 4.5 s first sync |
| fresh target dir, depcache warm on that machine | 77.7 s (local depcache cold for uv: 5/574 hits) | 22.9 s (574/574 hits, saved ~171 s rustc) |
| `test -p uv-resolver --no-run` | 73.6 s, peak 4.1 GB, target grows to 4.2 GB | 38.5 s |
| one-line edit, `check -p uv-resolver` | | 1.0 s |

Local footprint of remote builds: 0 bytes of target, no local rustc. The
remote numbers are on a 16 vCPU machine; production default is c7i.8xlarge
(32 vCPU) once the quota increase lands.

Over 272 recorded Jcode Desktop builds over 20 s on this laptop: median peak
build RSS 4.3 GB (p90 5.9, max 8.5), median peak 11 cores, median 237 CPU-s.
Local target dirs here: jcode 744 GB, jcode-desktop 1.1 TB.

## 2026-10-10: pinned toolchain (phase 1) vs system rustc

`justrust toolchain` now installs dated toolchains from static.rust-lang.org
into `~/.justrust/toolchains/<id>` (commit 2e5721d). nightly-2026-10-08 with
rustc, cargo, std, rust-src, clippy, rustfmt, cranelift, rustc-dev and
llvm-tools: 23.4 s to download, verify and unpack (9 components in
parallel), 1.66 GB on disk. Cranelift loads (`-Zcodegen-backend=cranelift`
hello world runs).

Reference loop, same pinned snapshots, own target dir per toolchain
(`bench/suite.py run --toolchain`), 5 samples each:

| scenario | system 1.98.1 | nightly-2026-10-08 | compare |
|---|---:|---:|---|
| desktop-test-edit | 8.49 s (8.15..10.73) | 7.44 s (6.76..8.78) | -12%, within noise (ranges overlap) |
| desktop-body-check | 2.58 s (2.43..3.05) | 2.44 s (2.02..2.75) | -6%, within noise |

Results: `bench/results/20261010-190616-5b0be04862d3.json` (system),
`bench/results/20261010-192507-5b0be04862d3-dirty-nightly-2026-10-08.json`
(the dirty flag is an uncommitted README edit only). Frontend 4.27 -> 3.59 s
and incr 0.86 -> 0.71 s moved most, which fits a newer rustc, but the
samples overlap, so no speedup is claimed. The cold dependency build on the
new toolchain took 155 s (run 20261010-190717445-4098100643, 887 units),
and a later warmup rebuilt 662 units in 64 s (run
20261010-191741408-4098100974): switching toolchains costs one cold build
per target dir.

Takeaway: phase 1 does not slow the loop down, and it delivers what phase 2
needs: one exact nightly on every machine, unstable flags without
`RUSTC_BOOTSTRAP` (the shim now gets per-pass timings for every local crate
on a pinned nightly), Cranelift, and rustc-dev. The wins come from the
phase 2 flag experiments on this toolchain.

## 21. Two benchmark systems: rolling agent log and one-factor-at-a-time (2026-10-10)

**System 1, `bench/trend.py`** (committed log in `bench/trend/`): every
foreground check/test/build an agent ran in Jcode, Desktop, and justrust,
from Jcode session history (September onward, so it includes the time
before justrust) plus justrust run records. `justrust history` had two
bugs that made early weeks look like 0.1s medians: queued self-dev builds
and reload acknowledgements counted as blocking calls, and `justrust ...`
commands were dropped (fixed 6987dbd, with a test). Median blocked wall,
baseline (before 2026-10-07) vs 2026-10-04..10: Desktop 26.6s -> 21.8s
(-18%), Jcode 35.3s -> 25.5s (-28%), justrust 8.5s -> 6.3s (-26%); `check`
-47 to -54% in all three. Not uniformly better: same-command
`test -p jcode-desktop-ui --lib` 24.2s -> 31.6s (653 vs 54 calls, not
controlled). Reuse ledger for W41: 86% (Desktop) to 95% (Jcode, justrust)
of units were not recompiled; Desktop depcache hits replaced about 727 min
of rustc. Raw agent data: the work mix changes, so it shows the trend,
not causes.

**System 2, `bench/factors.py`**: each factor switched off alone on the
pinned suite snapshots, on/off interleaved ABBA, private target dirs and
caches, reuse ledger per sample. First run
`bench/results/factors-20261010-194503-f7c1b3e840c7-dirty.json` (the
"dirty" is another agent's uncommitted depcache work in the tree, not the
bench). Machine busy with other agents, so the cold rows are flagged noisy.

| factor | workload | on | off | saved |
|---|---|---:|---:|---:|
| depcache (+script cache) | fresh target dir, `check -p jcode-desktop-ui` | 18.6s (18.4-18.7) | 104.1s (100.2-123.8) | 85.5s (-82%), 833 units restored |
| build-script cache | same, depcache warm in both | 19.6s | 27.7s | 8.1s (-29%), 45 scripts replayed |
| feature unification | fresh dir, `check -p` model, harness, ui | 97.9s, 887 units | 122.8s, 1151 units | 24.9s (-20%), 264 fewer units |
| slots | agent B's reference loop while agent A builds | 10.6s | 13.8s | 3.3s (-24%): 4.2-4.8s lock wait gone |
| parallel front-end | reference loop | 8.67s | 8.71s | none |
| parallel front-end | jcode-core body edit, 16 crates | 16.3s | 16.5s | none |
| mold vs lld | reference loop | 9.26s | 9.37s | none (link pass 0.8 vs 1.4s) |
| incremental (CARGO_INCREMENTAL=0 off) | reference loop | 9.04s | 25.20s | 16.2s (-64%) |
| pinned nightly vs system stable | reference loop | 8.08s | 8.78s | 0.7s (-8%), below the 10% floor |
| cranelift (all crates, nightly both) | reference loop | 8.25s | 8.05s | none |

Notes:
- The first depcache run lost one sample to a mid-run justrust reinstall
  (new depcache key scheme, ab89431/0ab7874: 833 misses). Rerun
  `factors-20261010-204006-c3c63f0156d5-dirty.json` via
  `bench/factors-isolated.sh` (frozen justrust copy, private
  JUSTRUST_HOME) is the row above; the second attempt had aborted on
  another reinstall, which the new guard caught.
- Parallel front-end: switch verified (frontend_threads 8 vs none), and
  it does not help the warm incremental loop: the edited crate's frontend
  is mostly incremental query reuse. Its measured win is on full
  re-checks (FINDINGS 14: 44s -> 24s). So it matters for cold or
  upstream-cascade builds, not the reference loop.
- mold: switch verified (ld.mold vs collect2+ld.lld). The link pass is
  0.65s shorter but within the loop's noise. The system default here is
  already lld, not GNU ld.
- Cranelift: no gain on the warm reference loop (1 unit, mostly frontend
  and incremental); the first cranelift build was 121s cold. Pinned
  nightly ran 8% faster with disjoint ranges, but under the 10% floor.
- Bottom line: the measured wins are reuse (depcache 82%, incremental
  64%, build-script cache 29%, slots 24%, feature unification 20%). The
  CPU-side knobs (parallel front-end, mold, cranelift, nightly) are each
  within noise on the warm loop. The warm loop (~8-9s) is now bounded by
  one crate's incremental frontend + persist, so the next big step needs
  less frontend work per edit (crate splits, resident compiler), not
  faster codegen or linking.

## 22. Test execution: one slow test sets the suite's wall time (2026-10-10)

Across 1193 recorded test runs on this machine, compile is still most of
the wait (jcode-desktop: 94 min compile vs 9 min running tests; jcode: 95
vs 16). But the jcode-desktop-ui lib suite (1290 tests) runs 10-38s, and
its wall time was bounded by single tests: under load,
`startup_each_native_submission_paints_the_newest_row` took 31.6s (9.2s
alone), 35 tests over 2s were 43% of summed test CPU. Running slowest-first
with one process per slow test did not help (19-22s vs 19-21s, load 44):
the suite is CPU-bound, so the fix is cheaper tests, not scheduling.

Causes, from a samply profile and reading the tests:

- `simulate_input` dispatches one keystroke and one full window render per
  character. A test typing 20 prompts of ~210 chars rendered ~4200 times
  over a growing transcript. Fix: `test_input::commit_text`, one IME commit
  through the same input handler (9.15s -> 0.17s).
- Scroll tests travelled 500+ rows in 180px wheel steps, each settled for
  up to 16 frames. Long strokes reach the same end state (7.13s -> 0.35s;
  mutation-checked that a wrong target still fails).
- Real `thread::sleep` for 2x tween durations, and PDF pages rasterized at
  2400px (~450ms of pdftoppm each) in tests that only check layout.

Result, interleaved A/B of the old and new test binaries on the same
machine (load 26-40), whole suite: old 17.3/18.8/16.9/12.2s, new
10.7/11.0/10.2/8.1s, median 16.9s -> 10.5s (-37%). Runs
20261010-203941085-1026350 (baseline, 9.15s slowest) and
20261010-204556033-1118868
(after). jcode-desktop commit a780ae3.

justrust now records per-test durations on every `test` (255d152): a cargo
`runner` (`justrust __test-runner`) adds libtest `--report-time` with a
process-local `RUSTC_BOOTSTRAP`, so artifacts stay Fresh. The report lists
the slowest tests when one is >=1s and >=10% of the test phase, and
`slow_tests` names the slowest one.

Remaining, measured alone after the fixes (load 16): pending_editor 2.6s
(~2.2s of it real `thread::sleep`, because motion tweens read
`Instant::now`; an injectable animation clock in jcode-desktop-motion would
remove it and the other 2x-duration sleeps in workspace.rs), tail_glide
tests 1.9-2.7s (rendering plus ~1s of 8ms frame sleeps the glide relies
on), everything else under 1.7s. tall_preview went 1.16s -> 0.07s.

## 23. Local and remote dependency caches combined (peer cache, 2026-10-10)

Code: `src/peercache.rs` (dfae1ad and follow-ups), depcache schemes 3 to
5, `docs/remote.md` "Peer dependency cache". Bench: `bench/peer-cache.sh`
on a pinned Jcode snapshot (abff94eeb, `check -p jcode-base`, 376
dependency units, nightly-2026-10-08), laptop (16 threads, load 5 to 15
from other agents) and the AWS c7i.8xlarge in us-west-2 (29 ms, ssh
throughput 15 to 20 MB/s down, 6 to 25 MB/s up).

### Why the two caches could not be combined before

Measured by building the same snapshot on both machines with
`JUSTRUST_DEPCACHE_EXPLAIN` and diffing the key inputs, then fixing each
cause and measuring again:

| cause | effect | fix |
|---|---|---|
| gitignored `Cargo.lock` (Jcode ignores it) was not mirrored | the machine resolved newer versions (itoa 1.0.18 vs 1.0.17): **remote builds compiled different code than local ones** | mirror `Cargo.lock` even when ignored (remote_sync) |
| system rustc differs (Arch 1.98.1 vs rustup 1.99.0) | no key can match | pinned toolchain (`justrust.toml`) on both sides |
| `--extern` keyed by whole-file content | rlib object code (sysroot paths in debuginfo) and proc-macro `.so` (gcc 16 vs 13, CRT) differ per host and cascaded up the graph: 24/56 keys equal | key `--extern` by crate metadata only (rmeta, rlib `lib.rmeta` member, `.rustc` section), scheme 3 |
| exact host glibc in proc-macro/bin keys | 2.44 vs 2.39: no proc macro shared | key the libc family, record the needed `GLIBC_x.y` (from `.dynstr`) and refuse to restore where the host is older |
| registry source paths in metadata (`/home/ubuntu/.cargo` vs `/home/jeremy/.cargo`) | rmeta differs, SVH differs | agent builds with `CARGO_HOME` at the client's path (registry/git symlinked to its own) |
| `CARGO_HOME`, `CARGO_TARGET_DIR` keyed | cargo passes them to rustc only when set: one side set them | not keyed (values are placeholders anyway), scheme 4 |
| generated `OUT_DIR` sources `include!`d (serde, serde_json, toml, ring, aws-lc) | the absolute target dir is in the metadata: 58 of 314 crates differed **even between two local target dirs** | compile dependency units with `--remap-path-prefix=<target>=/justrust-target`, scheme 5: 314/314 equal across target dirs |

After the fixes, keys of independently built caches: two local target
dirs 314/314 crates equal. Laptop vs machine 288/314: the 26 left are the
aws-lc/ring/sqlite `-sys` chain. Their build scripts compile C with the
host gcc, and the static archives are keyed by content.

A test binary for `jcode-protocol` linked from 28 rlibs fetched from the
machine (other gcc, other glibc) builds and passes 89/89 tests.

### Results (wall as the agent sees it, bench/peer-cache.sh)

| case | wall | dependency units |
|---|---:|---|
| remote-cold (both caches empty) | 61.5 to 64.5 s | 0 hit / 376 miss |
| local-cold (peer off) | 62 to 72 s | 0 / 376 |
| local-pull (local empty, machine warm, fetch on miss) | 66.3 s | 63 peer / 313 miss |
| remote-pull (machine empty, laptop warm) | 54.2 to 60.0 s | 88 to 101 peer |
| synced (after a remote build, background sync only, peer off at build time) | 41.1 to 57.9 s | 349 hit / 27 miss |
| floor: fresh target dir, local cache fully warm | 14.5 to 18.2 s | 376 / 0 |

Readings:

- **Background sync is what works.** After one remote build, the daemon
  pulled 376 entries (350 MB) in about 90 s, and the next cold local build
  hit 349/376 (-30 to -40% wall here). That is about as good as the
  cross-machine key overlap allows. The 27 misses are the `-sys` chain plus
  `aws_sdk_bedrock` (15 s alone), which keeps the wall well above the 15 s
  floor.
- **Fetch on miss is about break-even on this link** (local-pull 66 s vs
  62 to 72 s cold). The cost model declined 170 to 296 entries: the median
  declined unit is 480 KB and compiles in 0.09 s, which loses to a 5.8 MB/s
  measured single-stream link. The big ones that won were fetched.
  Fetching into a busy laptop also competes with the rustc processes it
  replaces for the link and the disk.
- **remote-pull saves 5 to 10 s** (88 to 101 entries) on a machine with 32
  cores, where compiling is already cheap.
- The remaining gap to the floor is the 26 `-sys`-chain crates. Making
  their C builds hermetic (one vendored C toolchain, toolchain.md step 3)
  would close it.

### Verification: paired A/B, cache filled only by the background sync

Clean protocol (`peer-ab2.sh`): wipe the machine's cache, build there once
through a fresh private home, wait for that home's daemon to sync, then
alternate cold (empty cache) and synced (a copy of the sync-filled cache)
local builds in fresh target dirs, peer fetching off. The sync pulled all
376 entries (350 MB) in about 30 s.

| pair | cold | synced | synced hits | load |
|---|---:|---:|---|---|
| 1 | 78.0 s | 60.9 s | 290/376 | 12 / 15 |
| 2 | 75.4 s | 43.8 s | 349/376 | 12 / 10 |
| 3 | 66.1 s | 45.1 s | 349/376 | 10 / 9 |

Median 75.4 s -> 45.1 s (-40%), every pair faster. Pair 1 got fewer hits
because the private home's daemon was still syncing while it was copied.
The cache directory had grown to 487 keys by then, so the copy caught part
of an in-flight install.

Two earlier runs of this A/B found real bugs, now fixed: a remote build
with `JUSTRUST_PEER_CACHE=0` journaled nothing, so the sync had nothing to
offer (0 entries pulled, fixed in 036f9cb). The sync also dropped journal
entries beyond the first 400 of a window instead of deferring them
(1fd90a2).

Bugs found and fixed on the way: a stale peer key set ruled out every
entry the machine compiled after the daemon connected (0/376 hits). The
daemon inherited `JUSTRUST_PEER_CACHE=0` from the build that started it.

## 24. jr-cargo: justrust's cargo fork, first change (2026-10-10)

Forked cargo at f3865b2a4, the cargo that the pinned nightly-2026-10-08
ships (`~/jr-cargo`, github.com/1jehuang/jr-cargo, branch `justrust`).
`justrust toolchain cargo-fork <binary>` installs a build as the
toolchain's `bin/jr-cargo` (refuses a binary built from another cargo
commit); builds with that toolchain run it, `JUSTRUST_CARGO_FORK=0` uses
stock cargo.

Where a no-op cargo run goes (Desktop, `test -p jcode-desktop-ui --lib
--no-run`, warm, `CARGO_LOG_PROFILE` + samply): 0.39 s wall. Full
workspace resolve against the registry index 110 ms (only to confirm
Cargo.lock), targeted resolve 300 ms, reading 713 dependency manifests
for downloads 136 ms, fingerprints ~60 ms, job queue 25 ms.

Change 1, resolve stamp: after a full resolve, store a hash of everything
it reads (lock file, member, path-dependency and [patch] manifests and
their workspace roots, cargo config files, version, flags, resolver env)
in `CARGO_HOME/.justrust-resolve/`. When it matches, the lock file is the
previous resolve and only the targeted resolve runs. Workspaces with
`[replace]` or unused patches always resolve fully, so their warnings stay
exact (`replace::overriding_nonexistent_no_spurious` caught that). The
first version missed unused-patch manifest edits
(`patch::update_unused_new_version`). Cargo testsuite: 4045 passed, the
same 2 environment failures as stock.

| Desktop, warm, 10 alternating runs | stock | fork, stamp off | fork, stamp on |
|---|---:|---:|---:|
| `cargo check -p jcode-desktop-ui` no-op | 0.435 s | 0.437 s | **0.367 s** |
| `cargo test --lib --no-run` no-op | 0.413 s | 0.482 s | **0.402 s** |
| justrust reference loop (test-fn edit), 10 each | 7.39 s (6.9-9.7) | | 7.55 s (6.7-8.4) |

So the stamp saves ~70 ms per cargo invocation, which is invisible in the
7.5 s reference loop (within noise).

Rerun with the channel-matched build (`build-jr.sh`, the binary actually
installed), raw cargo, 12 alternating runs each, 0 units rebuilt in any
run: `check -p jcode-desktop-ui` stock 0.327 s, fork with the stamp off
0.329 s, fork with the stamp on **0.271 s** (-17%). `test --no-run` was
noisy in this round (busy machine, 0.4-1.5 s spread; medians 1.29 / 1.45 /
1.07 s), so the earlier table is the cleaner number for it.

System 2 (`bench/factors-isolated.sh run cargo-fork cargo-fork-loop`,
`bench/results/factors-20261010-234515-fb4b860a68dd-dirty.json`), through
justrust with the pinned toolchain, on/off interleaved: no-edit `test`
0.49 s vs 0.52 s (10 each, -5%, within noise: justrust's own startup and
the test run dominate), reference loop 7.77 s vs 7.64 s (8 each, within
noise). So at the agent level the first fork change is not yet
measurable; raw cargo `check` is where the 17% shows. The point of this step is ownership:
the fork now builds, installs, and is selected by justrust, so the bigger
cargo changes (in-process fingerprints, a resident cargo, replaying a
recorded unit graph) have somewhere to land.

Pitfall found: a fork built without `CFG_RELEASE_CHANNEL` reports channel
`dev`, and cargo's defaults depend on it (`-Zembed-metadata=no` only when
cargo and rustc are both nightly). That changes every unit's fingerprint
("the config settings changed"), so switching between stock and fork
rebuilt all 887 units each time. `build-jr.sh` builds with the stock
cargo's release and channel; after that, switching rebuilds 0 units.

## 25. Remote builds ran on 4 of 32 cores (2026-10-11)

Of 1225 runs in 24 h, 42 ran remote (952 s of 26,000 s wall), and the
remote shared dependency cache served 0 hits (it is off by default,
`JUSTRUST_REMOTE_CACHE`, while the backend moves to AWS; the cloud worker
has `CACHE_ENABLED=false`). The first routed `test -p jcode-tui --lib` on
the c7i.8xlarge (run 20261011-003340339-57893, 194 s) never had more than 4
rustc units at once and averaged 3.0 of 32 cores: Jcode's
`.cargo/config.toml` sets `build.jobs = 4` for 15 GB laptops, and the remote
agent inherited it. That run was only observable because remote runs now
ship their run directory back (protocol v4 `RunFiles`).

The agent now sets `CARGO_BUILD_JOBS` to the machine's cores, capped at
one job per 2 GiB (30 on the c7i.8xlarge) unless the client passed `-j` or
`CARGO_BUILD_JOBS`. Cold `check -p jcode-tui` on the machine, fresh target
dir, all caches off: `-j4` 85.8 s (20261011-074216045-122932), `-j30`
40.6 s (20261011-074341880-137389), 2.1x.

## 26. System 3: stock cargo+rustc vs justrust, head to head (2026-10-11)

New `bench/versus.py`: the same agent workloads on `stock` (/usr/bin/cargo,
system rustc 1.98.1), `jr-nocache` (justrust, every cache off) and
`justrust` (defaults), round-robin on the pinned snapshots, with
user-space instructions retired by the whole build tree as a low-noise
second metric (spread 0.0-1.8% across runs; wall varies 5-50%).

First full result (`bench/results/versus-20261011-010303-084664294e8c-dirty.json`, n=5):

| workload | stock | jr-nocache | justrust | instr jr/stock |
|---|---:|---:|---:|---:|
| reference-loop | 10.28s | 10.40s | 11.56s | 1.03 |
| body-check | 2.61s | 3.02s (slower) | 3.05s (slower) | 1.05-1.06 |
| first-error | 2.12s | 2.60s | 2.38s | 1.06 |
| deep-edit | 16.50s | 19.26s | 23.99s | 1.04 |
| noop-test | 0.52s | 0.55s | 0.56s | 1.06 |
| small-crate | 2.15s | 2.25s | 2.27s | 1.02 |
| cold-check | 79.05s | 81.71s | **19.17s (4.1x)** | 0.13 |

- **The only win over stock today is caching on cold builds (4.1x).**
  Without caches justrust does 2-6% *more* work than stock on every warm
  workload. Geomean of warm + cold: jr-nocache 0.91x, justrust 1.08x.
- Where the extra work is (follow-ups `versus-20261011-010853-*`,
  `versus-20261011-011620-*`, `versus-20261011-012455-*`):
  - Not the toolchain: `JUSTRUST_TOOLCHAIN=system` has the same 1.05-1.06
    instruction ratio as the pinned nightly.
  - Not feature unification: `JUSTRUST_UNIFY_FEATURES=0` is identical.
  - `-Ztime-passes` on local crates (pinned nightly default) costs about
    3 points: body-check 1.06 -> 1.03 with `JUSTRUST_PASSES=0`.
  - The rest is fixed per-invocation overhead: on a no-op `test`,
    `JUSTRUST_DISABLE=1` is 2.230G instructions (same as stock), a
    recorded run is 2.353G (+0.12G, +5%, +0.05-0.1s wall): the rustc
    shim per unit, the stderr pipe threads, summary writing.
- The incremental loop is a regression we pay on every agent call:
  making the recording path free (and time-passes off by default, or
  sampled) is worth 3-6% on every warm workload before any fork lands.

## 27. jr-rustc build config: release and PGO vs the official nightly (2026-10-11)

Same rustc commit on every side (jr-rustc at the pinned nightly's commit,
CI LLVM, which upstream already ships PGO+BOLT optimized). `bench/versus.py`
with every justrust cache off and `RUSTC` pointed at each compiler; `instr`
is user-space instructions retired by the whole build tree relative to stock
(system rustc 1.98.1), the nearly noise-free column.

| compiler | reference-loop | instr | body-check | instr |
|---|---:|---:|---:|---:|
| official nightly (PGO+BOLT) | 9.13 s | 0.95 | 2.75 s | 0.95 |
| jr-rustc release (thin LTO, cgu 1) | 9.12 s | 1.05 | 2.75 s | 1.12 |
| jr-rustc + PGO (trained on our workloads) | 9.27 s | 1.00 | 2.90 s | 1.06 |

Results: `versus-20261011-021709-bde782c2cedb-dirty.json` (release vs
nightly) and `versus-20261011-024221-bde782c2cedb-dirty.json` (all three).
Wall times overlap within noise. Instructions show the real gap: release
does 10-18% more work than the official build, PGO closes about half of it
(5-11% left). The earlier 31% (finding in `bench/FACTORS.md`, jr-rustc row)
was the dev build (incremental, 256 CGUs).

PGO for a stage 1 compiler needed a bootstrap patch (upstream applies PGO
and BOLT link flags only to a stage 2 dist). Training (`pgo-train.sh` in
jr-rustc): cold and incremental check and test of the Desktop UI crate plus
cold builds of justrust, 407 s, one merged 45 MB profile. The rest of the
gap is BOLT on librustc_driver, which the official build has (its driver
carries `.bolt.org.text`), next: `bolt.sh` in jr-rustc.

Then BOLT (`bolt.sh`, llvm-bolt built from the in-tree LLVM, opt-dist's
flags, instrumentation mode since there is no `perf` here) and frame
pointers off (the `compiler` bootstrap profile turns them on, the official
driver has none), with PGO and BOLT retrained on that build:

| compiler | reference-loop | instr | body-check | instr | first-error | instr |
|---|---:|---:|---:|---:|---:|---:|
| official nightly | 5.33 s | 0.86 | 1.73 s | 0.93 | 1.32 s | 0.93 |
| jr-rustc PGO+BOLT, frame pointers on | 6.97 s* | 0.99 | 2.11 s* | 1.04 | 1.89 s* | 1.05 |
| jr-rustc PGO+BOLT, frame pointers off | 5.56 s | 0.90 | 2.04 s | 1.03 | 1.69 s | 1.03 |

(*) from the earlier run `versus-20261011-040904-d150dca4c0c1.json`, whose
nightly measured 0.94/0.93/0.93: same ratios, different machine state.
Final: `versus-20261011-063822-7a10e0d35fa5-dirty.json`. BOLT relaid out 61%
of profiled functions but moved instructions by under 1 point, and dropping
frame pointers by about 1. The fork still retires 4-10% more instructions
than the official build. Wall ranges overlap on every workload.

Not yet tested: the official compiler is a stage 2 build (the compiler is
compiled by a stage 1 compiler of the same source, LLVM 23.1.3), ours is
compiled by stage 0 (1.100 beta, LLVM 23.1.1). That is the largest
remaining difference in how the two are built.

Pitfalls: a wait loop `while pgrep -f "bench/versus.py"` matches its own
command line and never exits. A concurrent `suite.py` run injected its
type-error edit into a `versus.py` run on the same snapshot: every bench
script now takes an exclusive snapshot lock (`bench/README.md`).

## 28. First type error in 0.09 s: a resident rust-analyzer (2026-10-11)

Goal (`docs/toolchain.md`): the first type error in under 0.2 s. New
`src/early.rs`, opt-in with `JUSTRUST_EARLY=1`. A daemon per workspace
(`justrust __early-daemon`, socket in `~/.justrust/early/<hash>/`) runs
the managed toolchain's rust-analyzer (`rust-analyzer-preview` of the
pinned nightly, added on first use by `toolchain::add_component`, with
its own target dir). When an agent build starts, the recorder asks the
daemon in a side thread. The daemon stats the workspace and path-dependency
sources (780 files, a few ms), pushes changed files to rust-analyzer,
pulls diagnostics for them, and keeps only severity-error diagnostics that
carry a rustc code. They are printed as one block marked early, and cargo
keeps running and printing its own errors unchanged. After the build,
`summary.early` scores them against rustc's errors (matched, early only,
rustc only), and the report adds an `early:` line that flags anything
rustc did not confirm. The cargo command, env and output are untouched
(fail open: no daemon or not ready means no early output). `justrust
early start|stop|status`.

**Latency** (`bench/suite.py`, new `first_error` column: seconds until the
first `error` line on the agent's stderr; new scenario
`desktop-type-error-early`, daemon started untimed). Run
`20261011-045152-5cea93061a6c-dirty.json`, n=6, a busy machine (all
samples noisy):

| scenario | wall median | first error median | range |
|---|---:|---:|---:|
| desktop-type-error (rustc) | 3.08 s | 2.96 s | 1.76-4.04 s |
| desktop-type-error-early | 2.46 s | **0.09 s** | 0.08-0.11 s |

Confirmation on a quiet machine with the final defaults (experimental
diagnostics on, inference-limit filter), `20261011-early-final.json`, n=6,
all samples quiet: first error **1.83 s -> 0.08 s** (0.07-0.09), wall
1.96 s vs 2.02 s (within noise: early adds no wall time).

Of the 0.09 s, the daemon's answer arrives 0.06-0.085 s after the run
starts (`summary.early.secs`). The rest is justrust startup and the
toolchain pin. The spike (`bench/ra_spike.py`) measured a didChange plus
pull at 115-130 ms per edit on the same crate. The first pull after a
load takes 0.5-1 s. Costs: about 50 s to load the Desktop workspace cold
(15-28 s with rust-analyzer's own target dir warm), 4.7 GB RSS resident,
idle exit after 3 h (`JUSTRUST_EARLY_IDLE_SECS`).

**Accuracy** (`bench/early_accuracy.py`, untimed): a clean sweep pulls
every one of the 278 workspace files on pristine sources, which rustc
accepts. A corpus of 19 one-error edits of the kinds agents make, each
compared with a real `justrust check -p jcode-desktop-ui`.

| config | clean-sweep FP | first rustc error found | FP in mutations | result |
|---|---:|---:|---:|---|
| stable diagnostics only | 0 | 7/19 | 0 | `early-accuracy-20261011-051136.json` |
| + experimental, unfiltered | 3 | 13/19 | 57 | `...-051952-experimental.json` |
| + experimental, minus E0282 / `{unknown}` (shipped) | **0** | **13/19** | **0** | `early-accuracy-20261011-052951.json` |

- The only false positives were rust-analyzer's own inference limits:
  three `E0282 type annotations needed` in `panel_gmail_read_card.rs`,
  on `assert_eq!` over a type it could not compute (`{unknown}`).
  Dropping E0282 and any message mentioning `{unknown}` removed all of
  them and cost no true positive.
- Found early: E0308 (5 shapes), E0599, E0425, E0432, E0609 (reported as
  E0559 on the same line), E0061, E0063, E0277, and an error in the
  sibling dependency crate `jcode-core`.
- Missed, by design of rust-analyzer: borrow check (E0382, E0499,
  E0384), match exhaustiveness (E0004), `format!` argument checks, syntax
  errors (rust-analyzer reports them without a rustc code, so they are
  filtered), and errors at *callers* of a changed signature in other
  files (E0061 and E0593: only the changed files are pulled).
- So rust-analyzer finds 68% of first errors at 0.09 s, with no wrong
  ones in this corpus. The rest still come from cargo at 1.7-3 s. Still
  opt-in: 4.7 GB per workspace, and the corpus is small.

Next: pull reverse dependencies of a changed signature (callers), map
rust-analyzer's syntax errors onto a code, and start the daemon
automatically for workspaces with recent agent builds. A resident rustc
(phase 4) would cover borrowck and give rustc's exact diagnostics.

## 29. Early diagnostics: callers, syntax errors, auto-start; recall 13 -> 14 of 19 (2026-10-11)

Follow-ups to finding 28 (`src/early.rs`), measured with
`bench/early_accuracy.py` on a private copy of the pinned Desktop
snapshot (`EARLY_ACCURACY_ROOT`, so the shared snapshot lock stays free;
same commits and fixup as `bench/suite.py`).

- **Callers of a changed signature.** The daemon keeps the text of every
  source file. On each query it diffs the changed files: a changed line
  that declares an item (`fn`, `struct`, `enum`, `trait`, `type`,
  `const`, `static`), or a changed parameter line of a multi-line `fn`
  signature, marks that item's name. Body edits mark nothing, so they cost
  no extra pulls. Files that mention a marked name as a whole word are
  queued, module-qualified mentions first (`effort::label` for a change in
  `effort.rs`), then by path proximity, 24 at most. If the edited files
  themselves are clean, the client asks for them in a second `callers`
  query, 4 files per round, stopping at the first round with errors. The
  agent sees a second early block (`in files using a changed signature`).
  - First version: every file mentioning `label` (81 files), nearest first,
    all at once: 4.3-5.1 s, and the right file was not in the first 24
    (`early-accuracy-20261011-065632.json`). Module-qualified ranking plus
    batches: the caller's E0061 at **0.59 s** (`...-070028.json`).
  - rust-analyzer reports `fn label(&str, u8): FnOnce<(&str,)>` not
    satisfied (E0277) on the line before rustc's E0593. It was the only
    false positive this added and is now filtered.
- **Syntax errors.** rust-analyzer's `syntax-error` diagnostics (no rustc
  code) are now reported as plain `error:` lines, the first per line only.
  When a file does not parse, its type errors are dropped (they are
  artifacts of the parser's recovery).
- **Auto-start** (`JUSTRUST_EARLY=auto`): after an agent build, start the
  workspace's daemon in the background when it had 3 or more agent builds
  in the last hour and at least 8 GB of memory is available. Auto mode only
  queries running daemons and never starts one on the critical path.
  Checked end to end on a scratch crate in a scratch `JUSTRUST_HOME`: no
  daemon after builds 1-2, started after build 3, the next failing build
  printed `early errors ... after 0.01s`, confirmed by rustc. Still opt-in.

| corpus (19 mutations) | first error found | false positives | clean sweep FP (278 files) | result |
|---|---:|---:|---:|---|
| finding 28 (shipped) | 13/19 | 0 | 0 | `early-accuracy-20261011-052951.json` |
| + callers + syntax errors | **14/19** | **0** | **0** | `early-accuracy-20261011-070028.json` |

The remaining five misses are not reported by rust-analyzer at all
(checked with `bench/early_probe.py` and `rust-analyzer diagnostics` on a
3-line crate): use after move (E0382), two `&mut` borrows (E0499),
assigning an immutable binding (E0384), a `match` on `&str` missing its
catch-all (E0004; rust-analyzer only checks exhaustiveness for enums and
bools), and `format!("{}")` with no argument. This rust-analyzer build
runs no MIR borrow check diagnostics, even with experimental diagnostics
on. So 17/19 is out of reach with rust-analyzer alone. Borrowck and these
lints need rustc itself: a resident rustc (phase 4, jr-rustc), or a fast
`rustc -Zno-codegen` of the one edited crate, which is about 1.3 s on the
UI crate today (finding 28's `front` column). Auto stays opt-in.

## 30. Prebuilt dependency store: portable keys and a cold Desktop check from an empty cache (2026-10-11)

Code: depcache schemes 6 and 7 (3d48354, 3c5ac34), `src/depcache_prebuilt.rs`
(`JUSTRUST_PREBUILT`, `justrust cache --publish`), suite scenario
`desktop-cold-check-prebuilt`, S3 store infra and the prebuild script in
justrust-cloud (96e266f). Docs: `docs/c-sysroot.md`.

### Keys were not portable across users

`bench/prebuilt-portability.sh` builds the same snapshot under two cargo
home paths (same registry, private depcaches) and compares per-crate base
keys from `JUSTRUST_DEPCACHE_EXPLAIN`:

| scheme | justrust snapshot f97fcc2, 56 dep units | why |
|---|---:|---|
| 5 | 32/56 equal | registry source paths are written into rmeta, so every crate with a dependency got a different extern hash per cargo home |
| 6: `--remap-path-prefix=<cargo home>=/justrust-cargo-home` | 56/56, outputs byte-identical, no-op rebuild 0 compiled | |
| 6, Desktop, publisher with a symlinked registry | **0/833** | rustc's cwd is the resolved registry path, normalized as a literal path in the key |
| 7: resolved registry/git paths normalized like the cargo home | 808/833 hit (below) | |

### desktop-cold-check from an empty local cache

`bench/suite.py run desktop-cold-check-prebuilt desktop-cold-check
desktop-cold-check-nocache -n 3 --toolchain nightly-2026-10-08`, private
snapshot copy (`JUSTRUST_BENCH_WORK`), under the bench lock, laptop busy
(other processes 6 to 10 cores). Result
`bench/results/20261011-074513-86681282a9a0-dirty-nightly-2026-10-08.json`.
The prebuilt store was published once (865 bundles, 0.30 GB) from a
different cargo home path and target dir, and served over local HTTP. Each
measured run starts with a fresh target dir and an **empty** local depcache.

| scenario | median (min-max) | rust units compiled | dep hits | build scripts replayed |
|---|---:|---:|---:|---:|
| nocache (every justrust cache off) | 65.6 s (64.8-66.8) | 887 | 0 | 0 |
| **prebuilt, empty local cache** | **50.8 s (50.5-60.7)** | 79 | 808 (prebuilt) | 0 |
| warm local depcache (the floor) | 14.6 s (14.3-15.3) | 54 | 833 | 45 |

Run 20261011-073713067-2892610404 (prebuilt): the 808 hits took 57 s of
summed restore wall (max 1.1 s, fetch + verify + unpack), saving 882 s of
rustc. What is left:

- **Build scripts: none replayed.** `publish` skips build-script entries
  because their key includes the host package-db stamp. aws-lc-sys 12.8 s,
  libsqlite3-sys 7.6 s, ring 4.8 s, onig_sys 2.3 s ran in this build and
  sit on the critical path.
- **25 Rust units missed**, all downstream of the C chain (aws-lc-rs,
  ring, rustls, aws-sdk-*, reqwest, azure_*): 30 s of rustc, aws_sdk_bedrock
  alone 11 s. Their keys depend on build-script outputs (aws-lc/ring static
  libs) that differed between the publisher's and the consumer's runs.
- The local crates (jcode_base 10.5 s, jcode_desktop_ui 5.5 s) are in
  both the floor and this run.

Readings: the Rust half of a store works across cargo homes (808/833
restored, -23% wall versus nocache). Closing the remaining 36 s to the 14.6
s floor needs the store to carry build-script runs and those runs to be
host-independent, which is the vendored C toolchain (`docs/c-sysroot.md`,
layers 1 and 2). Not measured yet: a real network store (S3 over the
internet), and other seed sets than the Desktop lockfile.

### Follow-up: why the 25 missed, scheme 8 (2026-10-11)

Probe: two cold Desktop builds without a store, consumer cargo home
(`~/.cargo`) vs publisher home (a dir whose `registry`/`git` symlink to
`~/.cargo`), then compare the C archives and every dependency key.

| change | ring archive | aws-lc archive | equal dep keys |
|---|---|---|---:|
| scheme 7 (finding above) | differs | differs | 808/833 |
| + `-ffile-prefix-map` in CFLAGS for registry build scripts | **identical** | differs | 740/833 |
| + same `--remap-path-prefix` set on every machine (scheme 8) + per-binary rustc shim | identical | differs | **807/833** |

Three causes, all fixed in the working tree:

- **C objects embed the source path.** ring and sqlite compiled by cc put
  the registry path (via the publisher's symlinked home) into DWARF line
  tables, so the static archive bytes, and every dependent key, differed.
  `-ffile-prefix-map` for the cargo home, its resolved registry/git dirs and
  the target dir makes ring and sqlite byte-identical.
- **rustc folds matching remappings into the SVH.** With identical source
  bytes, rustix's rmeta differed only in its 16-byte hash because one side
  passed one matching `--remap-path-prefix` and the other two (replayed by
  hand: home only `369b45b0`, home+registry `98962ae1`). 16 crates were
  roots like this (rustix, alsa, png, onig, rusqlite, ...). Scheme 8 passes
  the same set (cargo home, resolved registry, resolved git, target dir) on
  every machine, whether or not anything is symlinked.
- **The bench was measuring a mix of binaries.** `~/.justrust/bin/shim/rustc`
  is a single symlink every justrust build repoints, so concurrent builds by
  other agents' binaries ran their shims inside my builds (the 740 and an
  intermediate 444 were partly that). The shim is now per binary
  (`bin/shim/<hash>/rustc`); a build by another binary on the same target
  dir stays fresh (0 compiled, 48 fresh, no Dirty).

Remaining 26 keys: aws-lc's archive. All 365 objects have identical
content, but their names differ: cc names objects
`<hash of dirname relative to CARGO_MANIFEST_DIR>-<file>.o` with std's
`DefaultHasher`, and the dirname is relative only when it starts with the
manifest dir spelling, which differs between a symlinked and a plain home
for aws-lc's generated sources. The next step is to key the `--extern`/native
lib inputs by archive member content rather than names, or to run registry
build scripts with `CARGO_MANIFEST_DIR` resolved, then make build-script
runs publishable (see `docs/c-sysroot.md`).

## 31. jr-rustc stage 2 vs stage 1 (2026-10-11)

Same release config (thin LTO, cgu 1, no PGO/BOLT, frame pointers off),
compiler compiled by stage 0 (1.100 beta) vs by our own stage 1 (as the
official dist is). Instructions relative to stock, run
`versus-20261011-074900-608b369adb66-dirty.json` (walls are noisy in this
run: other sessions' benchmarks queued on the lock and load was high, so
only the instruction column is usable):

| compiler | reference-loop | body-check | first-error |
|---|---:|---:|---:|
| official nightly | 0.86 | 0.94 | 0.93 |
| release, stage 1 | 0.97 | 1.11 | 1.10 |
| release, stage 2 | 0.93 | 1.05 | 1.05 |

Stage 2 removes 4-6 points, about as much as PGO did on stage 1. The build
takes 24 min instead of 6.5. Next: PGO+BOLT on stage 2 (`JR_STAGE=2` in
jr-rustc's build-jr.sh, pgo-train.sh, bolt.sh).

## 32. Pinned-nightly flags on the reference loop: none ship (2026-10-11)

Phase 2 of `docs/toolchain.md`: each flag measured alone with
`bench/factors.py` (new slow factors, both sides on the pinned nightly,
the flag injected through cargo `--config` so manifests stay untouched).
Results `bench/results/factors-20261011-045132-nightly-flags-a.json` (a)
and `factors-20261011-051155-nightly-flags-b.json` (b, `--quiet-wait 120`,
0-1 noisy samples per side). n=6 per side, medians with min-max.

| factor | workload | on | off | saved | verdict |
|---|---|---:|---:|---:|---|
| `-Zcache-proc-macros` (UI crate), b | desktop-test-edit | 5.54 s (5.1-5.7) | 5.79 s (5.6-6.7) | +4% | within noise |
| same, a (all samples noisy, 9-15 other cores) | desktop-test-edit | 14.49 s | 12.57 s | -15% | noise |
| same | desktop-body-check | 1.60 s (1.5-1.7) | 1.55 s (1.3-1.9) | -3% | within noise |
| `-Zshare-generics=y` on every dep | desktop-test-edit | 5.39 s (4.6-5.7) | 5.49 s (5.1-6.1) | +2% | within noise |
| `-Zhint-mostly-unused` on the 10 biggest deps | desktop-test-edit | 5.59 s (5.0-6.1) | 5.63 s (5.2-6.0) | +1% | within noise |
| same, cold `test --no-run`, depcache off (n=2) | desktop-cold-test-build | 114.7 s | 114.6 s | 0% | within noise |
| `-Zthreads=16` vs 8 (local crates) | desktop-test-edit | 6.15 s (5.7-6.4) | 5.61 s (5.4-5.9) | -10% | within noise, worse |
| `-Zthreads=4` vs 8 | desktop-test-edit | 5.42 s (5.3-6.2) | 5.38 s (5.1-5.7) | -1% | within noise |

Why, from the per-pass split of the UI crate (medians, run b):

- **cache-proc-macros**: `macro_expand_crate` 1.87 -> 1.59 s (-0.28 s),
  the only pass that moved. The flag caches *derive* macros only; the UI
  crate has 190 derives but 724 `#[gpui::test]` attributes and 1309
  `div()` builder calls, which it does not touch. Most of the 2.7 s
  "macro expansion" in `docs/toolchain.md` is not derive output, so this
  flag cannot reach the goal. Also flagged "potentially unsound" by
  rustc. Not shipped. A cache for attribute proc macros (jr-rustc) is the
  real target.
- **share-generics on deps**: `monomorphization_collector_graph_walk`
  0.37 -> 0.30 s, codegen 0.75 -> 0.71 s, about 0.1 s total. The opt-level 2
  deps (gpui, wgpu, text stack) default to not sharing; turning it on
  needs a full dependency rebuild and would change every depcache key for
  a 2% median move. Not shipped.
- **hint-mostly-unused**: works as designed on the hinted crates (cold:
  gpui 42.8 -> 24.6 s, wgpu-core 22.5 -> 6.9 s) but the codegen moves to
  their users (aws-sdk-bedrock 12.0 -> 16.2 s, jcode-desktop-ui 14.6 ->
  17.8 s), and the cold build is critical-path bound, so wall is
  unchanged at 114.7 s. Warm loop unchanged. Not shipped; with the
  depcache cold builds rarely compile these crates anyway.
- **-Zthreads**: 8 (Desktop's wrapper default) stays. 16 is worse
  (macro_expand 1.80 -> 1.88 s, more sync overhead), 4 is the same.

Bottom line: no pinned-nightly flag moves the warm reference loop beyond
noise; no justrust default changes. The loop's 5.5 s here is frontend
(macro expansion, resolve) plus incremental persist, which only a
resident compiler or attribute-macro caching in jr-rustc will cut. The
factors stay in `bench/factors.py` (slow) to rerun after a toolchain bump.
Pitfall: an unlocked `versus.py` run by another agent edited the shared
snapshot during the first attempt (type error injected into a test-loop
sample); all timed runs must hold `/tmp/justrust-bench.lock`.

## 33. Benchmarking the remote route from the agent's seat (2026-10-11)

New: `bench/versus.py --remote` adds a `remote` stack (`JUSTRUST_REMOTE=force`
from the laptop, wall includes sync and ssh; a local fallback is an error),
and `bench/on-remote.sh` runs any benchmark *on* the build machine
(c7i.8xlarge, Xeon 8488C, 32 threads). Laptop: Core Ultra X9 388H, 16
threads. Same pinned Desktop snapshot, warm target dirs on both sides.

The first run (`versus-20261011-081038-69d8c04310ce-dirty.json`) said remote
was 0.39x local overall. Three bugs, all fixed:

| bug | effect | fix |
|---|---|---|
| a workspace outside git was mirrored crate dir by crate dir, so `jcode-build-meta`'s `rerun-if-changed` root `Cargo.toml` did not exist remotely | its build script reran every call and 8 local crates recompiled: no-op `test` 11.4 s remote vs 0.5 s local | mirror the whole workspace when a path dependency is not in a git repo; take over root-created dirs inside a widened root |
| the cached IP was never refreshed after the machine restarted (new public IP every start) | every remote call timed out, then fell back | `describe()` keeps `~/.justrust/remote/ip` equal to AWS's |
| the machine lacked `libxkbcommon-x11`, `libX11`, `dbus`, `EGL` | every Desktop binary or bin-test build failed to link there | provisioning installs them |

Also: the machine linked with mold where the laptop uses wild (link pass
1.26 s vs 0.25 s on the reference loop); justrust now installs wild there
whenever it links locally. Routing skipped any run with a pinned run id,
so a benchmark could never take the remote path; the remote agent now
marks its builds explicitly (`JUSTRUST_ON_REMOTE_AGENT`).

After the fixes (`versus-20261011-084326-26e91bc15a8f-dirty.json`, n=5,
and `versus-20261011-085118-bd4ede273a6d-dirty.json` with wild on both):

| workload | justrust local | remote | remote vs local |
|---|---:|---:|---:|
| reference loop (test, 1 crate) | 7.07 s | 8.35 s | 0.85x |
| deep edit (16 crates, test) | 12.59 s | 16.44 s | 0.77x |
| body check | 2.06 s | 2.37 s | 0.87x |
| first error | 1.60 s | 2.03 s | 0.88x |
| no-op test | 0.50 s | 0.56 s | 0.82x |

Sync and ssh are now small (40 to 100 ms sync, ~0.1 s fixed). The rest is
the machine: per-phase, the UI crate's frontend is 4.1 to 4.3 s remote vs
2.9 to 3.6 s local, codegen 1.2 to 1.5 s vs 0.8 to 1.4 s. One rustc thread
on the Xeon 8488C (spot, shared host) is slower than a laptop P-core, and
warm incremental loops are single-crate, nearly single-threaded work. The
machine's 32 threads only pay where parallelism exists: cold and
multi-crate builds (finding 25: cold `check -p jcode-tui` 40.6 s remote)
and big test suites. Routing already decides per command from measured
walls; these numbers say warm single-crate loops should stay local on this
instance type. For warm loops a high-clock instance (c7i is 3.2 GHz
all-core; z1d/m7i-flex/c8g are worth measuring with
`bench/on-remote.sh`) is the lever, not more cores.

The synthetic workload (`bench/versus.py --synth 1`) is running on the
machine with `bench/on-remote.sh`. One point there (`test -- reload_config`
in jcode-desktop) fails its pre-build with `CARGO_BIN_EXE_jcode-desktop`
undefined for the `version` integration test; the same command builds
locally, so it is a state left on the machine by the builds that failed to
link before the library fix, not yet explained.

## 34. Test binaries in parallel, likely failures first (2026-10-11)

Mined 1359 recorded `test` runs: cargo runs test binaries one after
another, and small binaries queue behind each other (Desktop
`test --workspace`, 11 binaries: ~5 s per run spent behind the longest
one; `jcode -p jcode-sdk`: 10-12 s). Within one binary, libtest already
uses every core, so the remaining headroom inside a binary was small
(10.5 s over 707 s of binary wall); what agents wait on there is when the
failure they caused shows up. 51 of 196 failing runs failed again on a
test that had failed in the previous run of the same workspace.

What changed (`src/test_pool.rs`, commits 586c035, bf56d97, 29f4c71): in
agent mode the cargo runner (`justrust __test-runner --pool=<sock>`) hands
each libtest binary to the recorder and exits 0, so cargo moves on. The
recorder runs up to ncpu/4 (2-8) binaries at once, failed-last-time and
longest first, and prints each binary's output as one block when it ends.
With history (`~/.justrust/tests/<cwd hash>.json`: per-test times over
0.2 s, tests that failed and have not passed since, last start), one
binary runs as several processes by exact name from its own `--list`
with the user's filters applied: failed-last-time plus tests in modules
whose files changed since the binary last ran, each test that alone is
over 25% of the binary's recorded time (max 4), and the rest. Every
selected test runs exactly once; "filtered out" counts are adjusted.
A failure next to other processes is rerun by exact name with one
thread: reproduced, it is reported at once; otherwise the process is run
alone with nothing else running and that result is reported, with a note
naming the tests that failed only in parallel. Doc tests, benchmarks,
`--nocapture`, `--test-threads 1`, `--format`, unknown libtest flags:
run by cargo as before. `JUSTRUST_TEST_PARALLEL=0` turns it off.

Results, `bench/suite.py`, off (`JUSTRUST_TEST_PARALLEL=0`) vs on, n=3,
round 2 on a quiet machine (other cores 0.6-3.7; round 1 had 6-15 other
cores and is noise, files `testpool-*-1.json` kept):

| scenario | off | on | |
|---|---:|---:|---|
| justrust-body-test (full `test`, 4 binaries) | 2.57 (2.25-2.86) | 1.76 (1.69-1.76) | -31%, faster; test phase 1.07 -> 0.32 s |
| desktop-workspace-test (no edit, 11 binaries) | 13.57 (13.30-17.89) | 8.66 (8.61-15.33) | -36%, ranges touch; test phase 13.0 -> 8.2 s |
| desktop-ui-full-test-fail: first failing test on screen | 14.43 (14.23-14.57) | 6.99 (6.37-8.56) | -52%, disjoint |
| desktop-ui-full-test-fail: wall | 14.60 (14.40-14.73) | 22.04 (13.83-26.46) | slower, see below |
| desktop-test-edit (reference loop, 1 binary, filter) | 6.04 (5.99-7.02) | 7.16 (6.83-7.25) | +1.1 s, test phase 0.04 -> 0.004 s; the difference is compile (5.6 -> 6.6 s), not the pool |

Files `bench/results/testpool-off-2.json`, `testpool-on-2.json`. Run ids
for the failing scenario: 20261011-085222007-4038215624,
20261011-085248489-4038215378, 20261011-085302331-4038215804.

The failing-scenario wall is longer because Desktop has two flaky tests
(`the_hint_chip_uses_only_spare_tab_space`, which also failed 7 times in
plain serial runs since 10-07, and `bundled_github_applet_renders_valid_documents`,
which fails only next to a selection test in the same process). When one
of them fails next to other processes, the pool confirms it alone: a
whole extra run of the 1290-test binary (+8-12 s). The agent already
saw the real failure at 6-8 s. Off, those flaky failures are just
reported as failures; on, the report names them as passing alone.

Remaining: the confirmation run could rerun only the flaky test alone
instead of the whole binary when the rest passed; the reference loop
(`-- fps_counter`, 3 tests) gets nothing from the pool and its +1 s here
sits in compile, rerun with more samples to rule out a pool cost.

Stage 2 with PGO+BOLT (`JR_STAGE=2`, needs `build.profiler = true` so the
stage 1 std has `profiler_builtins`), run
`versus-20261011-092245-17daf818e79f-dirty.json`, instructions relative to
stock:

| compiler | reference-loop | body-check | first-error |
|---|---:|---:|---:|
| official nightly | 0.86 | 0.94 | 0.93 |
| jr-rustc stage 1 PGO+BOLT | 0.90 | 1.03 | 1.03 |
| jr-rustc stage 2 PGO+BOLT | 0.89 | 1.01 | 1.01 |

Stage 2 adds only 1-2 points on top of PGO+BOLT (it overlaps with what PGO
already recovers). The best fork build is now +3.5% instructions on the
reference loop and +7-9% on check and first error vs the official build;
wall ranges overlap everywhere. Every config difference against the
official dist that we know of is now matched except the training set
(rustc-perf vs our own workloads) and the toolchain that builds the C/C++
parts. Returns per step are now 1-2 points for an hour of build each, so
this is where we stop chasing parity: fork changes are measured as
`jr-rustc stage 2 PGO+BOLT` with and without the change, never against the
official nightly. `bench/factors.py` factor `jr-rustc` defaults to
`build/jr/pgo-bolt-stage2`.

## 35. Hot patching test binaries (phase 5 prototype): works on Desktop, saves the link, ~0.35 s (2026-10-11)

Prototype in `bench/hotpatch/` (README there), not wired into justrust,
nothing default-on. Subsecond (Dioxus) patches a long-lived process
through a jump table its code must call through (`subsecond::call`). Test
processes are fresh every run, so jr-hot patches before `main` instead and
needs no source changes:

- jr-hot is the linker. The first link with `JR_HOT=1` is a normal link
  plus `--no-gc-sections` and records each codegen unit's inode. rustc
  hardlinks reused units from the incremental cache, so a new inode *is* the
  set of regenerated units, with no diffing.
- Later links do not relink: `ld.lld -shared` links only the changed units
  (5-7 of 257 for the reference edit) at a fixed address 256 MiB below the
  no-ASLR PIE base. Every other symbol is defined in a generated linker
  script as its address in the base binary (locals matched by their CGU's
  `FILE` symbol group). Writable globals bind to the base's copy.
- jr-hot is also the runner: ASLR off, `LD_PRELOAD` a 140-line C loader
  that maps the image, applies its relocations, writes `jmp rel32` over the
  entry of every replaced function (287-403 per edit), and registers its
  `.eh_frame` so panics unwind through patched code.

Correctness: on a toy crate, function body, string literal, test body,
new private fn, new `#[test]` fn (libtest's test list lives in the
patched unit) and a panic caught through patched code all give the same
results as a full link. A new static falls back. On a copy of the Desktop
snapshot, a test edit that makes `assert_eq!` fail is reported failing,
and fixing it passes again, all patched. What forced fallbacks before each
fix: TLS definitions in a changed unit (harmless, only TLS *accesses* fall
back now), unit-local statics (the patch's fresh copy is the only one
reachable), instances `--gc-sections` had dropped from the base (hence
`--no-gc-sections`), and 1-4 byte functions too small for a jump (skipped
when their bytes are unchanged, which they always were). After that, every
Desktop edit tried (about 25) patched.

Timing, `bench/hotpatch/bench.py` (flock, alternating blocks of 3, 6
samples per mode, machine shared with other agents' benches):

| workload | normal wall | hot wall | link pass normal -> hot | jr-hot patch step |
|---|---:|---:|---:|---:|
| Desktop reference loop (test fn edit) | 8.97 s (5.6-13.1) | 8.02 s (5.2-10.0) | 0.54 -> 0.20 s | 154 ms |
| Desktop non-test body edit | 8.54 s (7.3-22.9) | 7.17 s (6.9-8.7) | 0.60 -> 0.18 s | 133 ms |
| toy crate (3 tests) | 0.103 s | 0.099 s | ~0 | 10 ms |

Results: `bench/results/hotpatch-{desktop-20261011-090939,desktop-body-20261011-091218,toy-20261011-093114}.json`,
run ids in the samples (for example normal 20261011-090740134-36074, hot
20261011-090810250-38553). Wall ranges overlap (noisy shared machine), so
read the link column: justrust's own per-pass split shows the link going
from 0.54-0.60 s to 0.18-0.20 s (that is jr-hot's patch step plus process
overhead), and linker CPU from 1.2-2.9 s to 0. The patch step is 115-195
ms, of which ~120 ms is reading the 640k-symbol table of the 245 MB base
binary; caching the base's symbol index across edits would bring it to
~30 ms (lld on 7 objects is 18-35 ms).

Assessment for the reference loop: hot patching removes the link, about
0.35-0.45 s of ~7.5 s today (5%). The loop is frontend-bound (3.4-3.6 s)
plus codegen and incremental persist (~1.6 s), which patching cannot touch.
So it is not worth shipping ahead of phase 4. Once a resident compiler
brings frontend + persist under a second, the link is the largest fixed
cost left, and patching is then worth 30-50% of the loop. Next steps when
that happens, in order: cache the base symbol index (patch step -> ~30 ms),
wire it into justrust's linker shim and test runner behind
`JUSTRUST_HOTPATCH=1`, rebase automatically after N patches or when the
patch exceeds a size, and verify results with a periodic full link.

## 36. Linking: wild for local crates, dependencies as one dylib (2026-10-11)

The link of the Desktop UI test binary (283 MB, 582 rlibs) runs after
every edit on the reference loop. Two ways to make it cheaper, measured on
the pinned snapshot.

**Link alone, replayed.** The exact link command of one reference-loop
build (args, object files, rustc temp dir) captured and replayed 7 times
per linker on the same inputs (`~/.jcode/scratch/linkcap/replay.py`
method: capture via a `clang` on PATH, relink with `-fuse-ld=mold` or
`--ld-path=wild`):

| layout | mold 2.42 | wild 0.10 | output |
|---|---:|---:|---:|
| stock (all deps static) | 0.92 s (0.60-1.24) | 0.35 s (0.34-0.41) | 283 / 242 MB |
| deps as one dylib | 0.54 s (0.39-0.93) | 0.20 s (0.18-0.23) | 101 / 99 MB |

wild links the same inputs 2.6x faster than mold, and its variance is
far smaller. Both binaries pass the UI suite (1303 tests, 2 runs each,
no failures; one earlier run under load had 1-2 flaky failures with
either linker).

**Shipped: wild for local crates** (commit 7a10e0d, `src/linker.rs`).
`justrust toolchain wild` downloads the pinned release (sha256 checked).
The shim adds `-B ~/.justrust/bin/ld` to local linking units only. gcc
and clang take `ld`/`ld.mold`/`ld.lld` from `-B` before their own,
whatever `-fuse-ld` says, so it covers `cc`, Desktop's
`clang -fuse-ld=mold` wrapper, and rustc's self-contained lld (via
`-Zpre-link-arg` on a pinned nightly). Those names are justrust, which
runs wild and on failure runs the linker that was asked for, with the
same args (fallback tested with a failing stub: lld and mold picked up,
logged to `~/.justrust/linker-fallbacks.jsonl`). Link args are untracked:
toggling `JUSTRUST_LINKER=off` on/off/on over a warm Desktop target dir
compiled 0 units (744 fresh). Dependency units are never touched, so
depcache keys do not move.

On the reference loop (`bench/factors.py run wild mold -n 6`, ABBA,
frozen binary, quiet: 0 noisy samples,
`bench/results/factors-20261011-linker.json`):

| factor | on | off | link pass on/off |
|---|---:|---:|---:|
| wild (vs Desktop's mold) | 7.03 s (6.1-7.8) | 7.03 s (6.3-7.9) | 0.23 / 0.44 s, disjoint |
| mold (vs lld via cc, wild off) | 7.21 s (6.2-7.6) | 8.14 s (6.7-8.4) | 0.46 / 1.26 s |

Suite runs, `JUSTRUST_LINKER=off` then on (sequential, not interleaved,
`bench/results/20261011-linker-{off,wild}.json`): 9.86 s -> 6.96 s
median, `compare` says within noise (off had 4 of 6 noisy samples);
link 0.71 -> 0.22 s.

So wild halves the link pass (-0.2 s, -3% of the loop), which is real
but below what wall time resolves at n=6. It ships because it is free
(no rebuild, fail open) and the link share grows as the rest of the loop
gets faster (hot patching, resident compiler). wild's incremental mode
is not implemented in 0.10 (its README: the end goal, not yet
available), so there is no incremental variant to measure.

**Prototype: deps as one dylib** (not shipped). A generated
`jcode-desktop-deps` crate (`crate-type = ["dylib"]`, `pub extern crate`
of every UI dependency), a dev-dependency of the UI crate, referenced by
`#[cfg(test)] extern crate`. rustc then links the test binary against
`libjcode_desktop_deps.so` (680 MB) instead of 582 rlibs: binary 283 ->
101 MB, link 0.92 -> 0.54 s (mold) and 0.35 -> 0.20 s (wild). On the
loop (ABBA n=5, very noisy machine, 10-15 other cores): wall 14.0 s
stock vs 12.6 s dylib, ranges overlap, link pass 1.11 -> 0.48 s,
codegen 2.40 -> 1.64 s. The saving on top of wild is ~0.15 s per edit,
and it costs a 3.3 s dylib link whenever any dependency changes, a
second copy of every dependency, a manifest change in the project
(or cargo fork work to do it transparently), and `LD_LIBRARY_PATH`
plumbing for running the binary outside cargo. Not worth it at this
size. Worth revisiting only if the link pass becomes a big share again
(for example after hot patching removes codegen), and then in jr-cargo,
not in project manifests.

## 37. Cold Desktop check from an empty cache: 53.3 s -> 14.8 s, build scripts in the prebuilt store (2026-10-11)

Follow-up to 30. Code: d6bc187, 2c0eced (`src/buildscript.rs`
`HERMETIC`, `hermetic_stamp`, `v1/s/` store tier; `src/depcache_prebuilt.rs`
publish). The prebuilt store now carries build-script runs for packages that
compile vendored C and probe nothing else: ring, aws-lc-sys,
libsqlite3-sys (bundled), onig_sys. Their key is the C toolchain (resolved
`cc`/`ar` by content, `cc --version`, which of the package's fallback
pkg-config modules exist) instead of the host package database, and the
package + `build.rs` instead of the script binary (it embeds
`CARGO_MANIFEST_DIR`). Other scripts stay host-keyed and are never
published.

Cross-machine causes fixed on the way (probe: two cold Desktop builds,
plain vs symlinked cargo home, `bench/prebuilt-portability.sh`-style):

| cause | effect | fix |
|---|---|---|
| cc names objects by a hash of the source dir relative to `CARGO_MANIFEST_DIR`; aws-lc builds from its canonicalized dir | aws-lc archive member names (and bytes) differed per cargo home spelling | hermetic scripts run with `CARGO_MANIFEST_DIR` resolved |
| `CARGO_HOME` keyed in script env | set by one side only | not keyed (as for rustc units) |
| registry `rerun-if-changed` files signed by size+mtime under the cargo home path but by content under its resolved path | no hermetic entry matched across homes; only onig_sys (no inputs) hit | resolved registry/git dirs count as cargo home |

Probe result: ring, aws-lc, sqlite archives byte-identical; 4/4 hermetic
script keys and 831/833 rustc keys equal (807 in 30's follow-up; the 2
left are ratex_font_loader and ratex_svg, downstream of
ratex_katex_fonts whose rmeta hash differs: rust-embed folder, not yet
diagnosed).

### Measured

`bench/suite.py run desktop-cold-check-prebuilt desktop-cold-check
[desktop-cold-check-nocache] -n 3 --toolchain nightly-2026-10-08`, private
snapshot copy, under the bench lock. Store republished each time from a
different cargo home (833 bundles 0.31 GB + 4 build-script runs), served
over local HTTP; every prebuilt run starts with a fresh target dir and an
empty local depcache.

| scenario | median (min-max) | rust units compiled | dep hits | scripts replayed | result |
|---|---:|---:|---:|---:|---|
| nocache | 53.3 s (52.6-54.7) | 887 | 0 | 0 | `20261011-093818-3c691739da09-dirty-nightly-2026-10-08.json` |
| prebuilt, finding 30 | 50.8 s (50.5-60.7) | 79 | 808 | 0 | `20261011-074513-86681282a9a0-dirty-nightly-2026-10-08.json` |
| prebuilt, scheme 8 + hermetic keys, before 2c0eced | 20.9 s (20.8-21.1) | 54 | 833 | 1 (onig) | `20261011-093818-...json` |
| **prebuilt, 2c0eced** | **14.8 s (14.7-14.9)** | 54 | 833 | 4 | `20261011-094421-82adf0a496f6-dirty-nightly-2026-10-08.json` |
| warm local depcache (floor) | 12.7 s (12.6-12.7) | 54 | 833 | 45 | same file |

A new machine with only the store is now 2.1 s from the warm floor
(-72% vs nocache). The gap: the other 41 build scripts still run (all
small, the largest unicode-general-category 0.39 s), and aws-lc's 20 MB
OUT_DIR unpacks in 0.54 s. The 54 compiled units are the workspace crates,
same as the floor.

Remaining: a real network store (S3 deploy pending the user), the two
ratex keys, and other projects' seed sets.

## 38. Where a no-op cargo run goes, and jr-cargo with mimalloc (2026-10-11)

Workload: `desktop-noop-test` (Desktop pinned snapshot, warm, 0 units
rebuilt), jr-cargo at c400320c3 (resolve stamp). `CARGO_LOG_PROFILE`
traces of 4 runs through justrust, plus samply at 8 kHz on cargo.

| phase (cargo tracing span) | ms | notes |
|---|---:|---|
| total cargo `main` | 660-790 | busy machine; ~0.3-0.5 s when quiet |
| `resolve_with_previous` (targeted resolve) | 250-300 | ~half is registry index summaries: read `.cache` files (~7% of samples) and parse them |
| `download_accessible` | 180-290 | not downloads: `read_manifest` of 721 unpacked registry crates, one thread (TOML parse 45, deserialize 50, normalize 90, to_real_manifest 55) |
| `compile` (fingerprints, job queue, output replay) | 110-150 | `fingerprint::calculate` ~120 ms incl. mtime stat of dep-info files |
| `load_pkg_lockfile`, workspace, init | ~40 | |
| rustc -vV probes | ~0 | already cached by cargo (`.rustc_info.json`) |

Samples by library: **48% of cargo's samples were in libc**, mostly
glibc malloc/free (manifest, summary and unit-graph allocations) plus
`read`/`statx`. The in-process upper bound for a resident cargo was also
checked: running `cli::main` 8 times in one process (`JR_REPEAT`
experiment, no caches kept between iterations) took the same 0.15-0.55 s
per iteration as a fresh process, so process startup itself is negligible
(`cargo -V` 15 ms). A daemon only wins if it keeps parsed manifests,
index summaries and fingerprints across runs; the state that costs time is
the parsing, not the exec.

Change 1 (jr-cargo 7ad6b3cf8): mimalloc as cargo's global allocator.
Raw cargo `test -p jcode-desktop-ui --lib --no-run`, 10 alternating runs
each under the bench lock (`jr/noop-bench.sh` in jr-cargo):

| binary | median | min-max |
|---|---:|---:|
| stock cargo (nightly-2026-10-08) | 0.366 s | 0.261-0.430 |
| jr-cargo c400320c3 (resolve stamp) | 0.298 s | 0.234-0.591 |
| jr-cargo + mimalloc | **0.257 s** | 0.179-0.343 |

-14% vs the previous fork, -30% vs stock, 0 units rebuilt in any run,
identical output (Fresh lines sorted). Installed as the pinned
toolchain's `jr-cargo`; submodule `toolchain/cargo` bumped to 6f30c42ee.

Next (in progress): parse the 721 registry manifests in parallel while
`Downloads` holds the package cache lock (`download_accessible` 250 ms ->
~70 ms in traces), then cache index summaries / parsed manifests across
runs.

Update: the point-3 failure is explained. Jcode Desktop's rustc wrapper
`scripts/rustc-parallel-frontend` is `#!/bin/sh`. On Debian/Ubuntu `/bin/sh`
is dash, which drops environment variables whose names contain `-` when it
`exec`s rustc, so `CARGO_BIN_EXE_jcode-desktop` (cargo does set it, checked
with `-vv`) never reaches rustc and `tests/version.rs` fails to compile.
Arch's `/bin/sh` is bash, so the laptop never sees it. Same on cargo 1.98.1
and 1.99.0, and with a clean target dir on the machine. Fix belongs in
jcode-desktop (`#!/usr/bin/env bash` for the wrapper, or do not exec
through sh); it affects any Ubuntu machine, including its CI runners if
they build that test with the wrapper.

## 40. Where macro expansion goes on the reference loop (2026-10-11)

jr-rustc stage 2 PGO+BOLT, one-line edit in a `#[test]` of
jcode-desktop-ui, `-Ztime-passes` and `-Zself-profile`: rustc total 5.32 s,
`expand_crate` 1.78 s (the largest pass), codegen 0.91, dep-graph and
result-cache persist 0.63, link 0.45, resolve 0.43. Proc-macro time inside
expansion is 1.01 s over 883 calls, and **0.96 s of it is `#[gpui::test]`
(724 calls)**. Derives are 0.05 s in total (Deserialize 0.026, Action
0.010, Serialize 0.009).

So `-Zcache-proc-macros` (factor `cache-proc-macros`, within noise) could
not help: it caches derive macros only. On every edit rustc re-runs the
`gpui::test` attribute macro on all 724 tests although 723 have identical
input tokens. The fork change: cache attribute proc-macro expansions the
same way upstream caches derives (an incremental query keyed by invocation
and input tokens), worth up to ~0.9 s of the 5.3 s compile.

## 39. jr-cargo: parallel manifest prefetch, cargo-fork factor now helps (2026-10-11)

Follows 38. After resolving, `Downloads` turned 721 `PackageId`s into
`Package`s one at a time, each a full `Cargo.toml` parse (250 ms of a
Desktop no-op). jr-cargo 08fc34c92 parses the already-unpacked registry
manifests on all cores first, while the package cache lock is held, and
`read_package` takes a parked result only for the same path and source.
`.cargo-ok` checks, last-use tracking and checksums stay on the
sequential path. Speculative parses print nothing. `Manifest`/`Package`
moved from `Rc` to `Arc` so they can cross threads. Kill switch
`JUSTRUST_CARGO_PREFETCH=0`. In traces `download_accessible` went from 180-290 ms to ~70 ms.

Raw cargo `test -p jcode-desktop-ui --lib --no-run`, 12 alternating runs,
two rounds under the bench lock (machine at load 12-16, so absolute
numbers are higher than in 38):

| binary | round 1 | round 2 |
|---|---:|---:|
| stock cargo | | 0.746 s (0.73-0.79) |
| jr-cargo mimalloc (6f30c42ee) | 0.558 s (0.51-0.61) | 0.555 s (0.52-0.60) |
| jr-cargo + prefetch (08fc34c92) | **0.471 s (0.44-0.48)** | **0.476 s (0.44-0.63)** |

Output is identical up to the (already nondeterministic) order of Fresh
lines, 0 units rebuilt. Cargo testsuite: 4042 passed. The failures were the 2 known environment ones,
`death::ctrl_c_kills_everyone` (hangs without a tty, killed), and 3
(`lto::cdylib_and_rlib`, `lto::dylib`,
`package_features::other_member_from_current`) that pass on rerun (load).

System 2, agent level (`bench/factors.py run cargo-fork`,
`bench/results/factors-20261011-103622-cargo-fork.json`), through justrust
with the pinned toolchain, 11 on/off interleaved: **0.22 s (0.2-0.3) vs
0.39 s (0.4-0.5) for stock cargo, -43%, verdict helps** (finding 24 had
0.49 vs 0.52 s, within noise).

Remaining in a no-op, from the traces: targeted resolve ~200 ms (index
summaries from `~/.cargo/registry/index/*/.cache`, then the resolver),
fingerprints ~120 ms. Next: a per-workspace cache of the targeted resolve
result keyed like the resolve stamp, which would skip the index entirely
on a no-op.

Fork change (jr-rustc `attr_macro_expansion` query: attribute proc-macro
outputs cached in the incremental session like upstream's derive cache,
under `-Zcache-proc-macros`, `JUSTRUST_RUSTC_ATTR_CACHE=0` to disable).
Correctness on the pinned Desktop snapshot (`bench/attr-cache-check.py`):
an edited `#[gpui::test]` body runs (the test panics as edited), a type
error inside it is reported, restore passes, and a one-line edit expands
`#[gpui::test]` once instead of 724 times.

Effect, same jr-rustc build (stage 2 PGO+BOLT) on every side, caches off,
run `versus-20261011-111710-07262f4deb42-dirty.json`, 8 rounds:

| | reference-loop | instr vs stock | body-check | instr |
|---|---:|---:|---:|---:|
| no proc-macro cache | 5.58 s | 0.93 | 1.93 s | 1.06 |
| upstream derive cache only | 5.74 s | 0.93 | 1.94 s | 1.03 |
| + attribute cache (jr-rustc) | 4.75 s | **0.74** | 1.87 s | 1.03 |

20% fewer instructions on the reference loop (0.93 -> 0.74), wall median
-15% (ranges overlap; the factor run `factors-20261011-105626-07262f4deb42.json`
agrees: 5.42 -> 4.39 s, -19%). Check is unchanged because `check -p`
does not compile the tests. First fork change that moves the loop.
Caveat inherited from the derive cache: a proc macro that reads files or
the environment is not re-run when only those change.

## 41. First synthetic agent workload: justrust 0.81x of stock overall, lost on big test suites (2026-10-11)

`bench/versus.py run --synth 1 --points 40 -n 1` on the build machine
(c7i.8xlarge, run with `bench/on-remote.sh`), result
`bench/results/versus-synth1-20261011-181043-21e77126c6ff-dirty-remote-c7i.8xlarge.json`, report `bench/VERSUS-SYNTH.md`.
40 points drawn from 4,711 real agent build calls (96% map onto the pinned
snapshots); 38 measured on all three stacks. Points 2 and 28 failed on
every stack: the dash/`CARGO_BIN_EXE` wrapper bug of finding 33.

| | points | stock | jr-nocache | justrust | justrust vs stock | instr justrust/stock |
|---|---:|---:|---:|---:|---:|---:|
| **expected wait per call** | 38 | 14.55 s | 18.99 s | 18.00 s | 0.81x | 1.35 |
| check | 9 | 2.19 s | 2.51 s | 2.62 s | 0.83x | 1.05 |
| test-filtered | 10 | 7.03 s | 7.72 s | 8.12 s | 0.87x | 1.20 |
| test (no filter) | 7 | 22.19 s | 34.04 s | 34.08 s | 0.65x | 1.55 |
| test --workspace | 1 | 70.5 s | 140.3 s | 93.8 s | 0.75x | 6.15 |
| build | 6 | 7.26 s | 7.64 s | 7.90 s | 0.92x | 0.92 |
| build --release | 4 | 48.1 s | 49.0 s | 49.5 s | 0.97x | 1.00 |

This is the first benchmark that weights by what agents actually run, and
it disagrees with the hand-picked scenarios: on the warm one-crate loops
justrust is within noise of stock, but most of the expected wait is in
unfiltered test runs of big suites, and there justrust loses badly. Of the
131 s total lost over 38 points, 65 s are three `test -p jcode-tui --lib`
points (2,470 tests): stock 22.6 / 29.8 / 38.3 s, justrust 41.3 / 51.0 /
63.3 s. In justrust's own phase split those runs spend ~17 to 20 s in
`test_run` and another 24 to 26 s in `tail` (after the build, outside test
binaries), with ~1.7x the CPU of stock, while the compile part matches.
`test --workspace` (8,911 tests counted by justrust vs 1,862 by libtest
lines for stock, so the stacks may not run the same set) is the other
large loss. Instructions are 1.35x stock overall, nearly all of it test
execution. The same jcode-tui command was not covered by any named
scenario, which is why this never showed up before.

Next: profile justrust's test runner on `test -p jcode-tui --lib` (what
the 25 s `tail` is, whether the parallel runner or the per-test
confirmation of overlapped failures re-runs tests), make the stacks run the
same test set on `--workspace`, and fix finding 33's Desktop wrapper bug
so points 2 and 28 count. Also: one run per point (`-n 1`) on a spot VM,
so single points are noisy; the per-command means over 7 to 10 points are
the reliable rows.

Acceptance through the real agent path (`justrust test`):

- `bench/attr-cache-accept.py` on the pinned snapshot, jr-rustc stage 2
  PGO+BOLT: the full jcode-desktop-ui lib suite (1313 tests) has the same
  test set with the cache on and off. The only outcome difference in the
  suite run (`changelog_panel::...hot_reload_keeps_position...` in one run,
  `workspace::...the_hint_chip_uses_only_spare_tab_space` in another) also
  fails intermittently with the cache off and passes alone on both sides
  (3 full-suite runs per side: one flaky failure on each side). Edge cases,
  all PASS: a new `#[gpui::test]` appears and passes; a renamed test runs
  under its new name and the old one is gone; edit A, edit B, revert to A
  runs A, B, A; changing only the attribute (`#[gpui::test]` ->
  `#[gpui::test(iterations = 3)]`) re-expands (body runs 1 -> 3 times); an
  invalid attribute argument is reported, and fixing it builds and passes.
- Reference loop on the real `~/jcode-desktop` checkout (AGENTS.md target,
  `justrust test -p jcode-desktop-ui --lib -- fps_counter` after a one-line
  edit in fps_counter.rs, file restored from a backup copy), 8 paired runs,
  alternating order: cache off 5.90 s median (5.08-6.52), on 4.56 s
  (3.60-5.24), **-23%, faster in 8 of 8 pairs**. The UI crate's frontend
  went from 2.69 s to 1.30 s (runs 20261011-120719442-2276316 off,
  20261011-120724754-2276848 on). Every run exited 0.

Pitfall: a shell `exec 9>lock; flock 9` leaks fd 9 into children; an
orphaned `wl-copy` inherited it and held the bench lock for 30 minutes.
`suite.bench_lock` now names the holders when it waits, and jr-rustc's
pgo-train.sh uses `flock -o`.

Update (cause and fix): the 25 s "tail" was the test pool's faithfulness
check. jcode-tui's lib tests have 3 real failures plus a few that fail
only next to other test processes; when the quick by-name rerun did not
reproduce every failure, the pool reran the *whole* 2,500-test binary
alone. Now only the names that passed are rerun alone; those that pass
again are reported as parallel-only (named in a note, not counted), the
rest stay failures, and only a crash with no named failure reruns the
binary. Laptop, warm, `test -p jcode-tui --lib`, 2 runs each under the
bench lock: stock 11.5 / 11.7 s; justrust before 22.7 / 22.6 s; after
11.3 / 11.5 s, same 3 real failures as stock. `JUSTRUST_TEST_PARALLEL=0`
and `JUSTRUST_TEST_TIMES=0` were already at stock speed (11.0 to 11.5 s),
which isolated the pool.

Rerun points (`change: rerun`, justrust 1.22 s vs stock 0.74 s mean): the
gap is mostly noise on the shared spot host, not a fixed cost. On the
machine with nothing else running, no-op `check -p jcode-base` is ~0.29 s
justrust vs ~0.21 s stock (4 of 6 pairs). Laptop, run
`20261011-155942999-1`: 0.355 s wall = 23 ms before cargo starts, 300 ms
cargo (stock alone ~265 ms), 26 ms tail, 7 ms exit, so the fixed overhead
is ~0.1 s. Neither `CARGO_TERM_VERBOSE` nor `RUSTC`=shim moved plain
cargo's no-op CPU time measurably (interleaved, 20 rounds: 354 / 310 /
362 ms medians). The synth run's 0.90 s and 1.28 s rerun walls were all
`cargo startup` with io stalls of 0.5 to 0.8 s in the same runs. A
`--no-run` then filtered test sequence on the laptop matches stock (0.6 to
0.8 s both). Caution: ad hoc timing on the build machine during a
benchmark skews it. Ad hoc runs at 23:00 to 23:05 UTC overlapped synth2
points 8 and 9 (stock 70 s / 30 s, flagged noisy), so rerun those points
before quoting them.

Remote failures of points 2 and 28 fixed in 1c50a2b: justrust makes
`/bin/sh` bash on its own AWS machine (at provisioning and on each
justrust install), checked there: `cargo test --no-run` in jcode-desktop
builds `tests/version.rs`. `bench/versus.py run --synth 1 --only-points
2,28` reruns single points for debugging (nothing saved).

Early synth2 on the machine (justrust 9ab215a, both fixes in, partial at
point 13 of 40, one run each): the former point-2 command (`test --
reload_config`, now listed as point 3) measures on every stack (stock
12.73 s, justrust 12.56 s). `test --workspace` (point 12) is stock 74.5 s,
justrust 59.1 s, jr-nocache 57.7 s, where synth1 had justrust 93.8 s
against stock 70.5 s. Justrust's samples are flagged noisy, so wait for the
full result before quoting it.

Full synth2 (`versus-synth1-20261012-002025-9ab215affbd3-dirty-remote-c7i.8xlarge.json`,
justrust 9ab215a: pool rerun fix + fail-fast + /bin/sh fix): **40/40
points measured** (synth1: 38). The report's headline, justrust 1.14x
stock (16.01 vs 14.10 s), is wrong. Stock's points 7 (`build`, 70.2 s)
and 8 (`check -p jcode-desktop-motion`, 30.0 s) ran while I was timing
by hand on the machine. Without those two: stock 14.22 s,
justrust 14.45 s, **0.98x** (synth1 was 0.81x). Per command against
synth1:

| | synth1 stock / justrust | synth2 stock / justrust |
|---|---:|---:|
| `test -p jcode-tui --lib` (3 points) | 22.6/41.3, 29.8/51.0, 38.3/63.3 | 22.0/22.6, 29.2/30.9, 35.5/37.3 |
| `test --workspace` | 70.5 / 93.8 | 74.5 / 59.1, 11.2 / 7.7 |
| `command: test` (no filter, 7) | 22.19 / 34.08 (0.65x) | 20.62 / 21.80 (0.95x) |
| rerun points (excl. 7, 8; synth2 adds `test --workspace` rerun) | 0.74 / 1.22 | 2.10 / 1.95 |

So the test-suite loss of finding 41 is gone, and what is left is a 2 to
10% overhead on most warm points. Remaining outliers: point 22 (a no-op
filtered test, 0.36 vs 1.43 s) was 1.40 s of `cargo startup` with 1.11 s of
machine-wide io stall in that run. On the idle machine afterwards it is
0.38 s justrust vs 0.35 s stock (3 rounds). I tested one hypothesis, that
the detached `split --refresh-hints` after an edited build slows the next
call: 4 paired rounds with `JUSTRUST_SPLIT_HINTS=0` vs on, refresh stamp
removed, gave 373 to 396 ms either way, so it is not the cause. A clean
rerun of synth with nothing else on the machine is needed before a
headline number is quoted.

Clean synth3 (`versus-synth1-20261012-030041-6e924fb57e90-dirty-remote-c7i.8xlarge.json`,
nothing else on the machine, 40/40 points): **justrust 0.98x of stock**
(13.80 vs 13.55 s per call), jr-nocache 13.51 s. By command: test
--workspace 1.30x, build 0.96x, test 0.95x, build --release 0.95x,
test-filtered 0.90x, check 0.85x. The losses are now a handful of
sub-second calls that took 0.6 to 1 s longer than stock, all spent in
`cargo startup` with 0.5 to 0.8 s of io stall (points 16, 17, 22, 32, 35:
stock 0.44 / 0.97 / 0.36 / 0.41 / 1.54 s, justrust 1.11 / 1.75 / 1.04 /
1.34 / 2.17 s). They did not reproduce by hand, not even with the same
repo switch, output to a file, or an edited-then-restored file with its
old mtime (0.38 to 0.40 s every time), and only sometimes through
`--only-points` (point 22: 1.35, 1.06, then 0.40 s; the jr-nocache stack
too). Their unmeasured pre-builds restore hundreds of depcache artifacts
(~1.2 GB dirty after a jcode-tui pre-build), so the measured call
competes with writeback.

Harness fix: `run.py` now calls `sync()` before the measured call, since
an agent's think time between calls covers the writeback. Every stack
gets the same treatment. With it, the five points with their predecessor
(`--only-points 15,16 16,17 21,22 31,32 34,35`, one run each): justrust
0.48 / 0.92 / 0.40 / 0.46 / 1.57 s vs stock 0.42 / 0.72 / 0.39 / 0.51 /
1.51 s. One jr-nocache sample (point 32, 1.06 s) was still slow, so the
flush removes most of the effect but not all of it. `sync()` changes the
generator-independent harness only, so synth1 to synth3 stay comparable
with each other but not exactly with later runs.

synth4 with the `sync()` (`versus-synth1-20261012-053402-7556b5ad52f7-dirty-remote-c7i.8xlarge.json`,
same AWS c7i, 40/40): **justrust 0.97x of stock** (13.28 vs 12.88 s),
jr-nocache 0.97x. The rerun outliers are gone (rerun points 1.31x
*faster* than stock). Remaining losses, in order of size: release builds
(+4 to 6 s of 63 to 69 s), check 0.83x, type-error points 0.71x. Two
were followed up on the machine:

- Release build (`build --release -p jcode-desktop` after a jcode-base
  edit, 3 rounds): stock 62.4 / 62.1 / 62.5 s, justrust 71.1 / 62.2 /
  61.9 s, `JUSTRUST_SCHED=0` 62.0 / 61.7 / 62.3 s. Same instructions on
  both stacks. No steady gap: one slow justrust round, as in the synth.
  Not pursued further without more rounds.
- Type-error check (`check -p jcode-app-core --tests` with a type error,
  6 rounds): stock 3.67 to 3.75 s, justrust 4.11 to 4.15 s,
  `JUSTRUST_PASSES=0` 3.73 to 3.80 s. **`-Ztime-passes` costs ~0.4 s
  (10%)** on this unit. The shim added it to every local unit compiled
  with `RUSTC_BOOTSTRAP=1`, which Jcode's parallel-frontend wrapper
  sets, and its output only fills the per-phase split in the report.
  Now off by default (`JUSTRUST_PASSES=on` restores it). After: justrust
  3.77 to 3.88 s, opt-in 4.06 to 4.19 s, stock 3.69 to 3.75 s. Flipping
  it compiles 0 units on jcode-desktop-ui (no Dirty).

synth5 with time-passes off (`versus-synth1-20261012-072519-1aabb66382ca-dirty-remote-c7i.8xlarge.json`,
same machine, 40/40): **justrust 1.03x of stock** in expected wait per call
(12.89 vs 13.23 s), jr-nocache 0.98x. The time-passes change shows up where
predicted: type-error points 2.41 -> 1.74 s (stock 1.71), check 2.54 ->
2.19 s (stock 2.12). Read the headline with care: it rests on the two
`test --workspace` points (58.5 vs 73.0 s, 7.0 vs 11.4 s). Dropping the two
best and two worst points gives 0.99x. The per-point geomean is 0.94x, and
justrust is faster on only 9 of 40 points. So on most calls justrust still
costs a few percent: the per-call overhead of finding 26 (~0.05-0.1 s
recording, shim per unit) plus `jcode-tui --lib` test runs ~1.3 s (5%)
slower. Point 7 (no-op `build` on jcode-desktop, 0.38 vs 1.16 s) looked
like the largest relative loss, but it is intermittent. Across synth3 to
synth5 justrust took 0.39, 0.99 and 1.16 s, all in `cargo startup` with io
stall. Rerun 3 times with its predecessor (`--only-points 6,7`) it took
0.39 s (jr-nocache 0.39, stock 0.36), and 3 times by hand 0.38 s. So it
is not a steady cost; the cause of the occasional startup stall is still
open.

The `test -p jcode-tui --lib` gap (machine, warm, 4 rounds each): stock
20.0 to 20.8 s, justrust 20.7 to 22.0 s, `JUSTRUST_TEST_PARALLEL=0` 20.4
to 20.6 s, `JUSTRUST_TEST_TIMES=0` 20.4 to 20.6 s. Timeline of one
justrust run: the "failed last time" group (4 tests) is done at 0.14 s,
the main group (2,482 tests) at 20.07 s, about the same as stock's single
process. Then the pool spends 0.9 s rerunning, alone, the tests that
failed only beside other test processes, before it reports them as
parallel-only. That faithfulness check costs ~1 s only on suites with
order-dependent tests (jcode-tui has several; the failure count varies
from 5 to 14 between runs). It is the price of not reporting a flaky
failure as real, so I left it. Making it cheaper would mean reporting
first and confirming after, which changes what the agent sees.

Before and after, `bench/versus.py compare` synth1 (21e7712) vs synth5
(1aabb66), same seed and machine, the 38 points both measured:

| stack | synth1 | synth5 | wall | instructions |
|---|---:|---:|---:|---:|
| stock | 14.55 s | 13.28 s | -9% | -0.1% |
| jr-nocache | 18.99 s | 13.62 s | -28% | -23.3% |
| justrust | 18.00 s | 13.06 s | **-27%** | **-23.6%** |

Stock's instructions did not move, so its -9% is machine variance. Most
of justrust's gain shows in instructions too (less test work through the
pool, fail-fast, no `-Ztime-passes`), not only in wall time.

## 42. Public prebuilt store on S3: one timeout paused it for the whole build (2026-10-11)

Deployed `infra/prebuilt-store.yaml` (justrust-cloud) as stack
`jcode-justrust-prebuilt-production`, us-east-1. Client URL:
`JUSTRUST_PREBUILT=https://jcode-justrust-prebuilt-production-280196252242.s3.us-east-1.amazonaws.com`.
Checked from outside with curl: anonymous GET under `v1/` 200, outside
`v1/` 403, listing 403, anonymous PUT 403, plain http 403. Filled with
`prebuild/run.sh` (43 seed crates, check/build/test --no-run): 576
bundles, 0.47 GB, 6 build-script runs.

First end-to-end cold `check` of the seed project from another directory,
empty depcache: **18 of 313** dependency units from the store. Every one of
the 313 base keys and manifest entries matched the store, so keys were
fine. The breaker file said `manifest: HTTP 0`: one curl with a 3 s connect
timeout failed while dozens of shims connected at once on a loaded laptop,
and the first failure paused the store for 10 minutes, i.e. the rest of
the build. 200 parallel manifest GETs from the same machine afterwards: all
200, connect up to 2.1 s.

Fix (src/depcache_prebuilt.rs): connect timeout 10 s, one retry for
connection failures and 5xx, and the breaker trips after 5 consecutive
failures (a success resets the count) instead of on the first.

Result, interleaved, fresh target dir and empty depcache each run, machine
busy (load 39-49, other agents' builds):

| run | S3 store | no store |
|---|---:|---:|
| 1 | 43.0 s, 313/313 prebuilt | 70.2 s, 0/313 |
| 2 | 38.6 s, 313/313 prebuilt | 60.1 s, 0/313 |

-38% wall on a cold build of 43 popular crates over the public internet
path to S3, with the CPU mostly taken by other builds (the store run's own
CPU was 32.5 s, it waited 33.9 s for CPU). On a quiet machine the floor is
the restore time; not measured yet.

## 2026-10-11: remote sync dominated a cold `justrust check` (run 20261011-154955613-3707337)

jcode-website fresh worktree, `justrust check` auto-picked remote (cold target, no history).
Remote total 282.1s, of which sync 250.1s; compile itself was 31.1s. Two files over 8 MB were
skipped, so the cost is the long tail of small files (public/benchmarks/data trials, thousands of
JSON transcripts). Local cold check would likely have been ~60-90s. Ideas: send only files the
cargo build can read (crate dirs + build-script inputs), or count the file total before choosing
remote for a cold run.

## 2026-10-11: wild `-B` broke wasm32 links (fixed in 290bded)

`trunk build` of jcode-website failed: `rust-lld: error: unknown argument: -B/home/jeremy/.justrust/bin/ld`.
The shim added the wild `-B` dir to every local linking unit regardless of `--target`.
Now only Linux targets (or no `--target`) get it.

Update (fail-fast, 9ab215a): the pool now honors cargo's fail-fast. Real
Jcode workspace, laptop, warm, under the bench lock: `test --workspace`
stock 76.0 s (1,868 tests, exit 101), justrust 44.5 s (1,792 tests, exit
101, the unstarted binaries named in a note). Before, justrust ran every
binary (8,911 tests). Counts differ slightly because the pool orders
binaries by history (likely failures first), so a different set has
finished when the first failure stops the run; both report the failure.

## 2026-10-11: three-way uv benchmark, remote loses on new checkouts to upload time

uv 44b2e58, a new git worktree per case, mean of 2 interleaved rounds. Laptop
load 12 to 39 from other agents. Remote c7i.8xlarge us-west-2, both caches warm.

| case | stock cargo | justrust --local | justrust --remote |
|---|---:|---:|---:|
| `check --workspace` | 89.7 s, 484 CPU-s | 54.0 s, 111 CPU-s | 83.2 s, 0.6 CPU-s |
| `test -p uv-resolver --no-run` | 121.1 s | 39.3 s | 109.4 s |
| one-line edit, `check -p uv-resolver` | ~1 s | ~1.2 s | ~1.0 s |

Remote compile itself was 17.7 to 23.2 s (574/574 and 475/475 depcache hits).
The rest is the first rsync of a new checkout: 40 to 121 s for ~45 MB of source
(sync 87484/120623/39825/49956 ms). Fix belongs in justrust: content-addressed
upload or seeding the remote tree from the same git commit (fetch on the machine,
then sync only the diff) would put remote at ~20 s here, 2.7x local.
Logs: ~/.jcode/scratch/uvbench/final.txt.

## 30. jr-rustc and the proc-macro cache on by default (2026-10-11)

`justrust toolchain jr-install` packages a jr-rustc build (stage 2
PGO+BOLT driver, rustdoc and clippy built against the same driver, cargo
and rustfmt from nightly-2026-10-08, rust-src) as toolchain `jr-<commit>`.
With no `justrust.toml` pin, recorded local builds now use the newest
installed jr toolchain, and the shim adds `-Zcache-proc-macros` to
incremental units whose real rustc is that toolchain's. Opt out:
`JUSTRUST_CACHE_PROC_MACROS=0` or `JUSTRUST_TOOLCHAIN=system`. Remote builds
are unchanged. Installed here as `jr-ac20de5e7921`.

Checked with the default (no env) through `justrust` on real projects:

- justrust itself: `test` 263 passed, `clippy` clean, doctests pass, run
  meta shows `real_rustc` = the jr toolchain.
- The shim adds the flag to an incremental unit and not with
  `JUSTRUST_CACHE_PROC_MACROS=0` (observed through a wrapper rustc), nor for
  a non-jr rustc or a non-incremental dependency (unit test).
- jcode-desktop-ui full lib suite: 1279 passed, 1 failed:
  `panel::selection_tests::pinned_task_selection_copies_without_collapsing_card`.
  It fails with the system rustc too (`JUSTRUST_TOOLCHAIN=system`, 1 of 1
  failed): the checkout has another session's uncommitted `panel.rs` edit,
  not a toolchain effect. Doctests pass.
- jcode `check -p jcode-core` ok.
- Reference loop on the real `~/jcode-desktop` (one-line edit in
  fps_counter.rs, `justrust test -p jcode-desktop-ui --lib -- fps_counter`,
  file restored from a backup copy), 8 alternating pairs: **default 5.70 s
  median (5.2-8.8) vs system rustc 8.65 s (8.2-10.1), faster in 8 of 8.**
  That combines the newer nightly, PGO+BOLT build and the proc-macro cache
  versus the distro's stable rustc 1.98.

Caveat that ships with it: a proc macro whose output depends on files or
environment variables (not its input tokens) is not re-run when only those
change.

End-to-end over the whole default path, installed binary, no settings
(`bench/e2e-default-toolchain.sh`, fixture `bench/e2e`: a workspace with
its own attribute, derive and env-reading proc macros), 15/15 PASS: default
toolchain is jr; body edit, attribute-arguments-only change, derive-input
change and revert each produce the right values; a type error inside an
attributed fn is reported, and the fix passes; doctests through jr's
rustdoc pass, also after an attribute change; clippy through jr's clippy;
`JUSTRUST_TOOLCHAIN=system` uses the system rustc; a `justrust.toml` pin to
an uninstalled jr toolchain fails open to the system rustc with a message;
the documented caveat is real (an env-reading macro keeps its cached
output after an unrelated edit) and `JUSTRUST_CACHE_PROC_MACROS=0` re-runs
it. Also rerun after the change: justrust's own 263 tests and clippy at the
committed HEAD, `jcode check -p jcode-core`. Toggling the cache flag does
not rebuild an up-to-date crate (cargo does not fingerprint the flag the
shim adds), and the next edit compiles with the current setting.

## 43. Public prebuilt store: fleet on AWS works, but downloading loses to compiling on a quiet laptop (2026-10-11)

Deployed: daily prebuild fleet (justrust-cloud `infra/prebuild-fleet.yaml`,
EventBridge Scheduler -> one egress-only c7i.8xlarge with the prebuild role,
latest justrust release, `prebuild/run.sh`, self-terminates, log to
`fleet/logs/`), and CloudFront in front of the bucket
(`https://d1j555vqgf2xkj.cloudfront.net`). Seeds 43 -> 105 crates. Two
fleet runs triggered through the real scheduler role: 1486 manifests,
1.47 GB in the store, ~2 min per run on 32 cores.

Bug found: the static musl release keyed proc macros by `cfg!(target_env)`
("musl") instead of the host's C library, so release-built keys of every
proc macro and its dependents never matched a glibc build's: 86 of 647
misses on fleet artifacts from the laptop. Fixed (host libc detected at
runtime, glibc version via getconf), released as 0.1.0-alpha.5: 615/647
prebuilt.

Then the speed. 105-crate seed project, cold `check`, empty depcache,
fresh target dir, alpha.5 binary, laptop in San Francisco on ~10 MB/s,
interleaved:

| run | CloudFront | S3 direct | no store (compile) | load |
|---|---:|---:|---:|---:|
| 1 | 80.7 s (615 hits) | 126.5 s | 36.3 s | 8-16 |
| 2 | 112.1 s | 138.2 s | 39.2 s | 4-14 |

Earlier on a busy machine (load 24-48) the store won (finding 42: 39-43 s
vs 60-70 s, 43 crates), and a second busy round here was mixed (73-78 s vs
63-86 s). What it costs: 615 bundles, 343 MB compressed (gzip fast) for a
`check` that compiles in ~280 CPU-seconds; ~34 s of pure transfer at
10 MB/s, plus a curl process, TLS handshake and manifest round trip per
unit (median restore 1.2 s, p90 2.9 s, ~11 in flight). On 16 free cores
compiling those crates is cheaper than moving their outputs. CloudFront
helps (edge hit 0.13 s vs 1.3 s for a 1.6 MB bundle) but not enough.

Decision: the store stays **opt-in** (`JUSTRUST_PREBUILT=default`); the
alpha.4 default is reverted. It pays off where CPU is scarce (busy
machines, small VMs, CI) or the network is fast. What would make it a
default win, in order: (1) fetch only units whose recorded compile cost
beats their size at the measured bandwidth (per-unit cost decision, the
same rule as the remote route), (2) one persistent HTTP/2 connection
instead of a curl process per request, (3) zstd and no rlib for `check`
(rmeta is enough), (4) sizes in the index so the client can decide without
a manifest request.

## 2026-10-11: cargo shim breaks `trunk build` for wasm32 (pinned toolchain has no wasm std)

`~/.local/bin/cargo` -> justrust. `trunk build` in ~/jcode-website runs
`cargo build --target=wasm32-unknown-unknown`, and justrust builds with its
pinned toolchain `jr-ac20de5e7921`, which has no `wasm32-unknown-unknown`
std: `error[E0463]: can't find crate for core`. The system toolchain
(/usr/lib/rustlib) has the wasm targets, and `PATH=/usr/bin:$PATH trunk
build` succeeds. Fix in justrust: when `--target` names a target the pinned
toolchain lacks, either add it to the pin or fall through to the system
rustc, and say so in one line instead of failing the build.

### Rerun after the fixes (2026-10-12)

Same seed, generator and machine,
`versus-synth1-20261012-002025-9ab215affbd3-dirty-remote-c7i.8xlarge.json`,
`bench/VERSUS-SYNTH.md`. Fixes since the first run: the test pool settles
parallel-only failures by rerunning those names, not the whole binary
(7138120); it honors cargo's fail-fast (9ab215a); `/bin/sh` is bash on the
machine (1c50a2b), so points 2 and 28 now build (40/40 points, no errors).

| | stock | justrust | justrust vs stock |
|---|---:|---:|---:|
| before, 36 shared points | 15.34 s | 18.98 s | 0.81x |
| after, same 36 points | 14.34 s | 14.69 s | 0.98x |
| after, all 40 points (raw headline) | 16.01 s | 14.10 s | 1.14x |

The raw 1.14x overstates it: on points 7 (`build`, rerun) and 8
(`check -p jcode-desktop-motion` after `check -p jcode-desktop`) stock's
own warm target dir was invalidated by the unmeasured pre-build (70.2 s and
30.0 s, against 0.37 s and 0.18 s in the first run), a harness artifact,
not something justrust did better. Leaving those out (and 2 and 28, which
had no data before), justrust moved from 19% slower than stock to even.
By command (all 40): check 2.41 s vs 5.41 s (stock includes point 8),
test-filtered 0.95x, test 0.95x, build --release 0.93x, test --workspace
1.28x. justrust is now close to stock on warm loops and still a few
percent behind on test and release builds; the remaining gap is the
remote machine's per-thread speed and justrust's ~80 ms per call
(`bench/noop-overhead.py`), not test-runner bugs.

Next for the harness: detect a stock pre-build that recompiles more than
the previous point left warm (a cache invalidation) and flag the point,
and run -n 2 so single outliers do not move the means.

## 44. Prebuilt store: fetch only what beats compiling, then it wins by default (2026-10-11)

Finding 43's per-unit numbers (same laptop, 105-crate seed project): fetch
median 1.18 s, compile median 0.18 s. Fetching was faster for only 34 of
455 units. So the store now publishes `v1/costs` (`<key> <compile ms>
<bundle bytes>`, written by `justrust cache --publish`, sizes equal to the
real S3 bundles for 615/615 keys) and the client fetches a unit only when
`0.7 s + bytes / rate < compile ms`, with `rate` an EWMA of this
machine's measured bundle downloads (5 MB/s until measured; it settled at
5.7-6.0 MB/s here). Unpriced keys are not fetched from the public store.
`JUSTRUST_PREBUILT_ALWAYS=1` restores fetch-all.

Quiet laptop (load 6-14), cold `check`, empty depcache and target dir,
CloudFront store, interleaved:

| round | cost-aware store | fetch all | compile only |
|---|---:|---:|---:|
| 1 | **22.0 s** (76 fetched) | 32.7 s (615) | 35.1 s |
| 2 | **19.0 s** (71) | 55.4 s (615) | 35.3 s |
| 3 | **25.8 s** (63) | | 34.0 s |
| default build, var unset (load 16-20) | **28.9 s** (77) | | 43.5 s |

-30 to -45% wall against compiling, fetching ~12% of the units (the
long-compiling ones: tokio, axum, syn, zerocopy, h2, ...). A project the
store knows nothing about (nalgebra + bevy_ecs, 102 units, 0 hits): 12.4,
12.8, 16.3 s with the store vs 11.1, 12.3, 12.8 s without, within noise;
the index and costs downloads are 77 KB, ~0.25 s once per 10 minutes.

Shipped on by default again (`JUSTRUST_PREBUILT=off` disables), the fleet
publishes `v1/costs` every run. Remaining: one HTTP/2 connection instead
of a curl process per request (the 0.7 s fixed cost is most of a fetch),
rmeta-only bundles for check, and pricing by this machine's own compile
speed instead of the fleet's.

Verification of the released 0.1.0-alpha.7 (installed with the public
installer into a scratch dir, var unset), interleaved on the seed project:
store 23.0 s / 32.1 s (78 / 65 fetched) vs off 46.8 s / 43.1 s (load
14-20). Fail open: unreachable store, a garbage `JUSTRUST_PREBUILT` value,
and a private store without `v1/costs` all built the uncovered project in
11.7-12.9 s (baseline 11.1-12.8 s); the dead store tripped the breaker
after 6 failures. A fleet run triggered through the real scheduler
installed alpha.7 and rewrote `v1/costs` (1177 keys; sizes equal to the
real bundles for 1145/1145 checked; fleet compile times are 0.86x the
laptop's, so its prices err toward compiling).

## 45. macOS: recording, sampling, and the depcache on real Macs (2026-10-12)

Issue #1. GitHub Actions `macos-latest` (arm64) and `macos-15-intel`, CI
run 38196948040. Cross-checking from Linux compiled clean but missed every
runtime difference below; only runs on real Macs found them.

- `ru_maxrss` is bytes on macOS, KiB on Linux: the first report showed an
  81,552 MB rustc. Fixed, now 67-80 MB for the smoke crate.
- ld64 writes a dylib's output path (inside the target dir) into
  `LC_ID_DYLIB`, so the store refused every proc macro as carrying the
  target dir: a second checkout of a serde-derive crate got 16 hit/1 miss
  (serde_derive). With `-Wl,-install_name,@rpath/...` on proc-macro
  misses: 17 hit/0 miss, ~14.4 s of rustc saved, and the restored proc
  macro loads (the test passes). Linux on the same crate: 17 hit/0 miss.
- A lock probe released by close can stay held by a child another thread
  is forking (it shares the open file until exec). Flaky slot tests on
  both platforms; probes now unlock explicitly.
- Pressure stall info does not exist on macOS: reports say "not measured".

## 45. Remote `build` with the binary brought back: release wins, debug ties (2026-10-11)

Question: `build` and `run` always stay local (`src/route.rs`) because the
binary must end up here. Is a remote build plus artifact download faster?

Setup: the same `git archive` snapshot of justrust on the laptop (12
threads, load 22-32, other agents building, the realistic case) and the
hosted c7i.8xlarge (32 cores, us-east-1, 145 ms round trip). Plain cargo on
both sides, `cargo build --bin justrust`, dependencies fetched first. Edit:
one line appended to `src/summary.rs`, 3x per profile. Scripts were ad hoc
(`bench.sh`, `edit.sh`, not committed); numbers in ms.

| case | laptop | remote compile |
|---|---:|---:|
| cold dev | 14.7 s | 13.3 s |
| edit dev | 1.73-1.87 s | 1.66-1.67 s |
| cold release | 22.3 s | 17.0 s |
| edit release | 21.6-28.2 s | 13.3-13.4 s |

Artifacts: dev binary 101 MB, release 10.1 MB. A real code edit (changed
string plus a new branch in `summary.rs`) shifts addresses, so the delta
is far from zero. Compressing the new binary with `zstd --long=31
--patch-from=<previous binary>` on the machine:

| artifact | full | zstd -3 | delta -3 (compress time) | delta -19 |
|---|---:|---:|---:|---:|
| dev (101 MB) | 101 MB | 21.9 MB | 5.4 MB (~0.3 s) | 2.1 MB (35 s, useless) |
| release (10 MB) | 10.1 MB | 3.3 MB | 2.2 MB (~0.04 s) | 0.9 MB (5.7 s) |

Fetch the delta and apply it on the laptop against the previous binary:
release 0.38-0.43 s, dev 0.64-0.80 s. The result is byte-identical
(sha256 equal). Raw download from the machine varied 5-19 MB/s, so a full
debug binary (100 MB) would cost 5-19 s and only deltas are viable.

Reading:
- **Release edit: 21.6-28.2 s local vs ~13.8 s remote (13.4 compile +
  0.4 fetch), 1.6-2x.** Worth shipping: release builds parallelize
  (codegen units, LLVM) and the 10 MB binary deltas to ~2 MB.
- Dev edit: 1.8 s local vs ~2.4 s remote (1.7 + 0.7 fetch). Remote loses.
  The 100 MB debug binary is the problem: most of it is debug info, and
  `objcopy --strip-debug` leaves 29 MB. Shipping it needs split debuginfo
  that stays remote, or lazy fetching.
- Cold: small wins (1.1x dev, 1.3x release) before transfer. A cold
  build has no base binary to delta against, so a dev binary costs a full
  zstd download (22 MB, 1-4 s). Worth it only for bigger projects, where
  the shared dependency cache does most of the work.
- Not 10x anywhere: this crate's local build is mostly one crate's
  frontend, and the machine is 1.3-2x per core-bound step, not 10x.

Plan: route `build --release` (and `run --release`) remote when history
says the local build takes over ~5 s, ship the binary as a zstd
`--patch-from` delta against the last binary fetched (a full zstd copy
when there is none), and keep dev `build`/`run` local until split
debuginfo is in place.

Later the same night (CI run 38199891022, both Macs green), more runtime
bugs found only by exercising the workflows on real runners:

- clonefile keeps the cache entry's mtime, so restored rlibs looked older
  than the unit's `invoked.timestamp`: a seeded slot rebuilt 8-10
  dependency units ("quote changed"). Restores now get a fresh mtime;
  a new slot after a one-line edit builds 1 unit, 11 fresh.
- Pinned toolchains failed to install on every Mac: 1.98.1 ships no
  cranelift for apple-darwin. Optional components are now skipped;
  `1.98.1` installs and builds on both Macs.
- `peer_cache_serves_and_installs_entries` failed twice in Linux CI: each
  CachePut chunk was appended on its own thread, so a last chunk could
  complete the bundle before an earlier one landed. Chunks now append on
  the reader thread. Not reproduced locally (0/60 before and after).

Requirement to evidence (macos-latest / macos-15-intel, run 38199891022):

| Requirement | Check | Observed |
|---|---|---|
| builds on macOS | clippy -D warnings, cargo test | 254 passed / 254 passed |
| xattr arity | depcache key memo tests | pass |
| reflink | depcache restore + slot seed | 17 hit/0 miss; slot seeded 0.0-0.1 s |
| affinity, cgroups | sched off, builds succeed | ok |
| sampler | `show --json` asserts | build cores 1.56 / 1.50, rustc 111 / 63 MB, avail 3.6 / 10.4 GB |
| no silent zeroes | report text, `psi_unavailable` | "stalls not measured" |
| pid guard | `status --json` after a run | `active: []` |
| proc macros cached | 2nd checkout of serde-derive | 17 hit/0 miss, test passes |
| routing | `show` where line | local, reason given |
| toolchain pin | 1.98.1 install + test | "pinned toolchain 1.98.1", 1 passed |
| release packaging | thin-LTO stripped build runs a test | 1 passed |

## 46. Five days of agent builds: where 13.7 h went and what the ceiling saves (2026-10-12)

`bench/runlog.py` over `~/.justrust/runs` (5785 runs, 2026-10-06 to
2026-10-11). Bench snapshot runs excluded (3270, 15.1 h) and one hung
jr-cargo test (6985 s, run 20261011-083315454-3785807): 2514 agent runs,
13.7 h wall, median 5.0 s.

| kind | runs | wall | median | ceiling assumed | saved |
|---|---:|---:|---:|---|---:|
| dependencies rebuilt | 369 | 6.34 h | 37.6 s | deps from cache, 5 s + local (max 2 s) + tests | 5.57 h (88%) |
| local crates only | 1233 | 5.09 h | 7.1 s | 0.5 s + tests | 4.66 h (91%) |
| compile error | 202 | 1.88 h | 10.3 s | first error in 0.2 s | 1.87 h (99%) |
| no-op | 710 | 0.38 h | 0.5 s | 0.05 s + tests | 0.18 h (48%) |
| **total** | 2514 | 13.7 h | | | **12.3 h (90%)** |

Dependency rebuilds are the largest bucket and almost all cold targets:
173 runs (3.1 h) with zero fresh units and no rebuild reason, 11 more
(0.75 h) cold release builds, despite 34k depcache hits in these runs.
That is the cold-build path (prebuilt store, remote), not the incremental
loop. Test execution is only 0.67 h of the 13.7 h.

LLVM in local-edit runs: 22-32% of rustc time in test/build (jcode test
0.39 of 1.39 h), more than the reference loop's ~2% because jcode's edits
regenerate more codegen units. Check runs have none.

## 47. Shadow split: moving one leaf module out of the giant crate makes its test loop 12x faster (2026-10-12)

Manual prototype (`~/.jcode/scratch/shadow-proto`, `ab.py` alternates
variants, each iteration edits the file then runs `justrust`). `orig/` is the
Desktop snapshot, `man/` moves `fps_counter.rs` from `jcode-desktop-ui`
(the big crate) into a leaf crate `jr-shadow-fps`. Direction: the leaf is a
dependency of the old crate, which re-exports it under the old path
(`pub(crate) use jr_shadow_fps::fps_counter;`), so `crate::fps_counter::*`
in `workspace.rs` keeps working and test names stay
`fps_counter::tests::*`.

Edit `fps_counter.rs`, run its tests (n=6, ab-run2):

| variant | command | median | range |
|---|---|---:|---:|
| orig | `test -p jcode-desktop-ui --lib -- fps_counter` | 6.0 s | 5.23-7.3 |
| man | `test -p jr-shadow-fps --lib -- fps_counter` | 0.49 s | 0.47-0.53 |

Earlier pass (ab-run): 7.97 s vs 0.69 s. About 12x. Run ids: orig
20261011-214107008-2994872 (6.0 s), 20261011-214117810-2996443 (5.23 s);
man 20261011-214117316-2996324 (0.49 s), 20261011-214100356-2993688 (0.47 s).

After a leaf edit, `check -p jcode-desktop-ui` in man costs 2.14 s median
(1.83-2.15 warm, run 20261011-214058213-2993551): the UI crate must be
re-checked because it depends on the leaf.

Edits that stay in the big crate (ab-run3, n=6): the split costs a little.

| command | orig | man |
|---|---:|---:|
| `check -p jcode-desktop-ui` | 2.33 s | 2.56 s |
| `check -p jcode-desktop-ui --lib --profile test` | 3.33 s | 3.88 s |

(orig 20261011-215144495-3054752 / man 20261011-215039775-3045639;
orig 20261011-215156485-3056267 / man 20261011-215149468-3055296.)

So the win needs command routing: when the agent runs
`test -p jcode-desktop-ui -- fps_counter` and only leaf files changed, run
`test -p jr-shadow-fps -- fps_counter` in the shadow; keep the big crate in
the command whenever its own files changed, and always add the leaf
package when the target package had modules split out so moved tests do
not vanish from the filter. The productized opt-in is `JUSTRUST_SHADOW=1`
(below, once measured).

## 48. Hot patching through `justrust test`: fires on every edit, link 0.33 -> 0.12 s, wall within noise; a test-process zygote would save under 0.5% (2026-10-12)

Follow-up to 35 (prototype) and the opt-in port in 8ec1f42
(`JUSTRUST_HOTPATCH=1`, `src/hotpatch.rs`). The first factor run of that
port (`factors-20261011-165959-...`, -2%) never patched: it predates the
response-file fix.

**It fires.** On a private copy of the Desktop snapshot through
`justrust test -p jcode-desktop-ui --lib -- fps_counter`, outcomes in
`<run dir>/hotpatch.jsonl`: the reference test edit and the non-test body
edit both patch 5-6 of 257 units (361-526 functions redirected). One edit
fell back: a regenerated unit held a `thread_local!` access
(`R_X86_64_TPOFF32` to `PREVIEW_CACHE`, `panel_diff.rs`). 12 TPOFF32 and 1
GOTTPOFF sites exist across the UI crate's 256 units.

**Fixed: thread-locals bound to the base** (ea4bae1). In an executable a
thread-local's offset from `%fs` is a link-time constant
(`st_value - round_up(PT_TLS memsz, align)`), so the changed object is
copied, each local-exec site gets the base's offset written in, and
initial-exec `mov x@gottpoff(%rip),%r` is relaxed to `mov $off,%r` the
way ld relaxes IE to LE; the relocations become `R_X86_64_NONE`. Dynamic
TLS models (never emitted for an executable's own code) still fall back.
An edit inside the `PREVIEW_CACHE.with` closure now patches (`1 tls`), and
the `panel_diff` tests that exercise that cache pass on the patched binary.

**Fixed: base symbol index cached** (ea4bae1). Reading the base's 962k
symbols was 130-160 ms of every patch step. Now an on-disk index
(`<exe>.jrhot/index`, 43 MB: name hashes sorted, pointing into the
base's own string table, keyed by inode, size and a hash of the section
headers) is built by a detached niced child right after each base link
(0.3 s, nobody waits) and mapped by every patch. Patch step 150-350 ms ->
36-58 ms quiet (`basesyms 0`, lld on 5-6 objects 23-39 ms is now most of
it). Unit test: the index resolves every symbol of a real binary exactly
like the in-memory table.

**Correctness** (`~/.jcode/scratch/hp/loop.py`, private target dir):
full `jcode-desktop-ui --lib` suite, patched (2 runs, 6/257 units
patched): 1303 passed, 0 failed, 10 ignored; normal relink (2 runs): 1303
passed, 0 failed, 10 ignored (one parallel-only flake in a patched run,
`workspace::tests::the_hint_chip_uses_only_spare_tab_space`, passed alone,
as the pool reports it). Failing edit (expected value of an existing
`assert_eq!` changed), patched: 2 passed, 1 failed with the right
`left: "FPS · idle"` message; the fix, patched: 3 passed; repeated twice
(runs 20261011-214133-hp2998720 fail, -214141 fix, -214148 fail, -214156 fix).
An edit that adds a new `assert_eq!` (a new generic instance) or adds a
line before other code regenerates 131-146 of 257 units (CGU
repartitioning, span shifts), which is above the half-of-units limit and
relinks normally: correct, just not patched.

**Measured**, `bench/factors.py run hotpatch hotpatch-body -n 8 --warmup 2
--quiet-wait 60` (ABBA, frozen binary for body after other agents
reinstalled justrust mid-run twice; every `on` sample patched, per-sample
link pass and outcome now recorded by the bench):

| factor | on | off | link pass on / off (median) | patch step |
|---|---:|---:|---:|---:|
| hotpatch (test edit) | 8.23 s (7.8-17.3, 3 noisy) | 8.39 s (7.0-10.3) | 0.12 / 0.33 s, disjoint | 47-105 ms |
| hotpatch-body | 7.78 s (7.5-8.9) | 7.93 s (7.7-8.5) | 0.10 / 0.32 s, disjoint | 40-52 ms |

Results `bench/results/factors-20261011-214238-891dfea50aae.json`,
`factors-20261011-221526-239bef0338bc-dirty.json` (sample run ids, e.g.
hotpatch on 20261011-214813385-3005003103, off 20261011-214921978-3005003856).
The link pass drops by 0.2 s in every sample, but that is 2.5% of a loop
whose frontend (3.9-4.2 s) and codegen + incremental persist (~2.2 s) are
untouched, so wall is within noise. Cost: each patched process pays ~2 ms
more startup in the loader (map parse, 6k relocations, 361 jumps: 9.2 vs
7.2 ms no-match run). **Stays opt-in**: no measurable wall win today; it
becomes worth turning on when a resident compiler makes the frontend
small (35's argument still holds).

**Zygote (fork per run after the loader patched): not built, measured
first.** A zygote saves at most what a test process spends before `main`:
exec, ld.so relocation (587k relative relocations in the UI binary, 93% of
ld.so's 2.3M cycles), the hot patch loader, and static init. Measured by
timing exec up to an `LD_PRELOAD` constructor that `_exit`s (40 runs,
medians) vs a full no-match run, and counting test-binary processes per
`justrust test` with a preload that logs each pid
(`~/.jcode/scratch/hp/zy/`, runs 20261011-zy-tui-3428028-{1,2,3}):

| binary | exec to preload ctor | full no-match run | fork+exit+wait instead | processes per warm full-suite run | suite test_run | startup share |
|---|---:|---:|---:|---:|---:|---:|
| jcode_tui lib (2,518 tests) | 4.2 ms | 5.1 ms | 0.17 ms | 10 | 13.6-13.8 s | ~50 ms, 0.4% |
| jcode_desktop_ui lib (1,313), patched | 8.5 ms | 9.2 ms | 0.29 ms | 2 | 10.8-11.3 s | ~18 ms, 0.2% |

The pool does not start a process per test: libtest runs a whole group
per process, and the pool splits a binary into at most ~6 groups plus a
`--list` and the by-name confirmation reruns of failures seen next to
other processes. The jcode-tui suite's time is test bodies (27 s CPU,
libtest-reported 16.8 s across processes), and its 2.5 s `tail` is the
confirmation rerun of 2-4 overlapped failures alone (test bodies again),
not startup. So a zygote could save at most ~50 ms of a 16.6 s run, and it
would need a fork-safe point before `main` in arbitrary test binaries
(threads started by static init, open fds, `/proc/self/exe` users).
Not worth building. What would pay on big suites is less test work (run
only affected tests, cheaper confirmation), not cheaper processes.

## 49. Why agents rebuild dependencies: cold targets and depcache misses (2026-10-12)

Follow-up to 46 (dependency rebuilds: 6.3 h of 13.7 h agent wall). Mined
`~/.justrust/runs` (`bench/runlog_misses.py`), agent runs only.

**Cold target dirs** (184 runs, 3.8 h, zero fresh units):

| cause | runs | wall |
|---|---:|---:|
| experiments in throwaway target dirs (`~/.jcode/scratch/...`) | 125 | 2.09 h |
| target dir emptied (cargo clean / deleted) | 32 | 0.63 h |
| first release/profile build | 7 | 0.57 h |
| toolchain switched in the same target dir | 22 | 0.46 h |
| first build of a checkout | 10 | 0.20 h |

**Every dependency miss** (repeat the same rustc + package + version +
opt level + emit, 10.6 h of rustc):

| cause | units | rustc |
|---|---:|---:|
| key changed for the same crate and rustc (upstream metadata cascade: a git dep like gpui at a new rev, feature unification, scheme-era churn) | 16.6k | 5.73 h (54%) |
| first time with this rustc (system -> nightly -> jr toolchains this week) | 6.7k | 3.33 h (31%) |
| evicted by the 30 GB GC cap | 1.8k | 0.89 h (8%) |
| justrust key scheme bumped (8 bumps in 2 days) | 1.6k | 0.65 h (6%) |

Most of the churn was the week's own work (toolchain switches, scheme
bumps, cache experiments). Live check today, two fresh target dirs on
Desktop, `check -p jcode-desktop-ui`, local: first 818 hit / 14 miss (gpui
and its 13 workspace crates: Desktop's gpui git rev moved), second 832/0.
**The cache works now, but a fully hit cold check still takes 29.5 s**
(run 20261011-232856920-140417): dependency restore spans 13.3 s (832
restores, up to 57 at once; the machine had 22 other rustc running, 19 s
CPU stall), then 57 local crates are compiled from scratch (58.7 s rustc,
jcode_base 7.2 s alone). So the next wins for cold targets are:

1. local crates from a cache (`localcache.rs`, opt-in, not yet hitting on
   Desktop: build.rs stamps time via `env!`), or cloning a warm slot's
   target dir instead of starting empty;
2. restoring the whole dependency set as one reflink of a prepared
   target tree, not 832 shim round trips through cargo's scheduler.

Shipped: the GC cap now defaults to a quarter of the free disk, 30 to 150
GB (here 150 GB, cache at 27 GB). At 30 GB it evicted 3-5 GB every 1-3 h.

### 45b. Shipped: optimized `build` routes remote, binary comes back as a delta

`src/remote_artifacts.rs`, routing in `src/route.rs` (`routable_build`).
Same laptop (load 20-22, other agents building), justrust repo, one-line
edit to `src/summary.rs` then `justrust build --release`, interleaved:

| round | remote (client wall, incl. fetch) | local |
|---|---:|---:|
| 1 | 18.0 s | 25.0 s |
| 2 | 17.5 s | 30.7 s |
| 3 | 17.0 s | 37.9 s |

Remote median 17.5 s vs local 30.7 s, **1.75x**, disjoint ranges. Fetch:
full zstd copy 3.5 MB in 0.8-1.6 s after a local build; consecutive
remote builds come back as a 0.1 MB delta in 0.4-1.0 s (the comment-only
edit barely changes the binary; a real code edit measured 2.2 MB in
FINDINGS 45). No-op remote build: 1.7 s, artifact check 0.38 s. Run ids
20261011-235746732-770608 (local 1), 20261011-235817389-780593,
20261011-235834417-782823, remote runs 20261011-235919370-786512 onward.

## 50. Local crate cache: workspace crates restored by content, revert 10.6 -> 0.7 s, fresh target dir 66 -> 6.3 s (2026-10-12)

Code: `src/localcache.rs` (239bef0 and follow-ups to 8f1aff6+), shim wiring
in `src/shim.rs`, `depcache::plan_with`. Factors `localcache-revert`,
`-revert-sde`, `-revert-small`, `-loop`, `-fresh` in `bench/factors.py`
(new samplers `revert` and `fresh`). Measured with a pinned copy of the
justrust binary (`JUSTRUST=<copy>`): other agents reinstalled justrust
mid-run three times.

**What it is.** depcache only took registry and git units. Workspace and
path crates now go through the same two-level key (args with the profile
dir normalized, `--extern`s by crate metadata, then every dep-info source
and tracked env var by content), plus the link args the shim adds (wild).
Differences from depcache: no extra rustc args (local units compile
exactly as cargo asked, so recording never changes outputs, incremental
state, or fingerprints, and the key includes the working dir: a worktree
at another path misses, measured), `-C incremental` accepted (a hit leaves
the incremental dir alone), and the store is off the critical path. The
shim hard-links the outputs into `s/` and a niced, idle-io
`justrust __localcache-finish` copies, hashes, runs the target-path guard
and commits. Off with `JUSTRUST_HOTPATCH=1`. Own cache dir, 10 GB LRU cap.

**Results** (Desktop snapshot unless noted, ABBA, medians, min-max):

| factor | workload | on | off | |
|---|---|---:|---:|---|
| localcache-revert | edit fps_counter.rs, build, undo, build (measured) | 12.00 s (10.6-14.0) | 12.76 s (12.1-15.1) | 0/5 hits, see below |
| localcache-revert-sde | same with `SOURCE_DATE_EPOCH` fixed | **0.70 s (0.68-0.73)** | 10.61 s (10.4-11.3) | 5/5 hits, -93% |
| localcache-revert-small | justrust snapshot, full `test` | **0.88 s (0.80-2.14)** | 2.41 s (2.15-3.70) | 4 of 5 units hit |
| localcache-fresh | fresh target dir, unchanged sources, deps from depcache both sides | **6.27 s (5.4-6.7)** | 66.14 s (59.7-88.8) | 54/54 local units restored |
| localcache-loop | plain reference loop (every build misses) | 8.93 s (8.5-9.2) | 8.70 s (8.4-11.7) | within noise |

Results: `factors-20261011-230158-adeeaebe403f-dirty.json` (revert-sde,
revert-small), `factors-20261011-233833-2b487ca729a9-dirty.json` (fresh),
`factors-20261012-001556-fc0cd2778c87-dirty.json` (loop),
`factors-20261011-221705-a2c6ad1308d2-dirty.json` (plain revert). Example
runs: revert-sde on 20261011-230734816-3809690194 / off
20261011-230713186-3809690629, fresh on 20261011-234637064-336728534 / off
20261011-234507388-336728483.

**Why the plain Desktop revert never hits.** `jcode-desktop-ui`'s build.rs
stamps `JCODE_DESKTOP_BUILD_ID` (build time in ms) into the crate with
`cargo:rustc-env`, read by `env!`. Every build is a new input, so every
result is unique (20 entries, same sources, 20 build ids). It is correct
to miss: the restored binary would report another build's id. Options
measured or considered:

- Set `SOURCE_DATE_EPOCH` by default: makes Desktop hit (revert-sde), but
  changes what the project's binaries say about themselves, and it is a
  build input of every registry build script that reads it (all
  dependency keys would move once). Not done: the user's choice, not ours.
  Mentioned to the project instead.
- Detect volatile inputs and skip storing (shipped): a tracked env value
  that parses as a Unix time from the last ten minutes, or a unit whose
  sources match two stored entries with other env values, is not staged.
  Desktop's UI test unit now stages nothing (`staged:false`), so it costs
  only the lookup.

**Miss-path cost on the loop** (`<run>/localcache.jsonl`, 10 samples):
plan (key, lookup) 14-18 ms after memoizing lib-dir hashes (85 ms before),
stage 17-27 ms. A reflink of the just-written 285 MB test binary took
~170 ms because btrfs flushes dirty pages before cloning, so staging uses a
hard link and the finisher copies it and checks the inode/size/mtime
stamp, dropping the entry if a later build rewrote the file. The
remaining loop delta (+0.23 s median, rustc wall 8.19 vs 8.10 s) is
within the factor's noise. The finisher measured 0.72-0.93 s wall,
0.59-0.79 s CPU per 240 MB UI binary, niced. Disk: 240 MB per stored UI
test binary, ~15 GB after two hours of benchmark edits before the
volatile guard (every edit stores a new result), capped at 10 GB by GC.
Units whose sources are back to a stored state are what the cache is for.
The loop's normal edit is a new state, so the stored copy only pays off on
revert, branch switch back, or a new target dir.

**Correctness.**
- Restored outputs are byte-identical to a compile (justrust test binary,
  `cmp`), and the tests pass on them: 157/157 justrust, 3/3 Desktop
  fps_counter in every hit sample, the same counts as the off side.
- No `Dirty` units: after a hit, a `-v` no-op is all Fresh. Toggling
  `JUSTRUST_LOCALCACHE` on and off in one target dir leaves 48/48 Fresh.
- Key poisoning (`include_str!` data file, a build-script
  `cargo:rustc-env` read by `env!`, `option_env!`): each change misses and
  the test prints the new value, going back to the original inputs hits
  and prints the old one.
- Refused, as designed: integration tests that embed
  `env!("CARGO_BIN_EXE_*")` (the target dir as data), so justrust's
  `remote_agent` test always recompiles.

**Decision: on by default** (`JUSTRUST_LOCALCACHE=0` turns it off). The
win scenarios are large (fresh dir -90%, revert -64% to -93%) and the
miss path costs ~35 ms in the shim, with correctness checks clean. Not
yet measured on the synthetic workload. A fresh worktree at another path
and second checkouts do not hit (absolute paths in local outputs). Making
those hit needs `--remap-path-prefix` for the workspace, which changes
panic paths and debuginfo the user sees: a follow-up to measure.

## 51. Resident build daemon: prebuild on edit cuts the reference loop 8.9 -> 5.1 s; coalescing ties slots (2026-10-12)

`justrust __buildd` (`src/buildd.rs`), one per user over
`~/.justrust/buildd/sock`, spawned like `__daemon` (own process group,
inherited fds closed). Agent builds (`check|test|build|clippy`, no tty)
hand their request to it, and it runs the same `justrust <args>` with the
client's exact env and cwd, so routing, slots, toolchain, recording and the
report are unchanged. Three parts:

- **Coalescing**: identical request (cwd, argv, env minus agent identity
  such as `JCODE_SESSION_ID`/`KITTY_*`, and a source stamp: path, size,
  mtime, inode of every file under the workspace and its path
  dependencies, ~8 ms on Desktop) attaches to the running build and gets
  the full output and exit code.
- **Prebuild on edit**: the daemon watches the sources (inotify) of each
  agent session's last request and starts it 0.4 s after an edit burst,
  niced 10. `test` is prebuilt as `test ARGS --no-run`. Tests never run on
  a guess, and the agent's own request waits for the compile and then runs
  its tests on Fresh units. A new edit, or the agent asking for another
  command, stops an unclaimed prebuild. 3 prebuilds per request between
  asks (bounds builds that write into their own sources).
- **Warm state**: the measured win isn't there. justrust's own pre-cargo
  work is 5 ms plus a 16 ms median systemd scope (p90 42 ms, 467 no-op
  runs). The ~80 ms "overhead" in older notes was jr-cargo vs stock cargo
  noise, not justrust. A jr-cargo daemon mode would have to keep parsed
  manifests and index summaries resident (FINDINGS 38: a fresh process
  costs nothing, the parsing does). Not attempted.

Factors (Desktop snapshot, ABBA), `factors-20261011-234427-dcbed51dadfc.json`,
`factors-20261012-002718-fa7980014c4e-dirty.json`,
`factors-20261012-003954-c998b6a9ce04-dirty.json`:

| factor | on | off | verdict |
|---|---:|---:|---|
| buildd-prebuild, compile-only test prebuild (final), reference loop with 4.6 s think time | **5.14 s** (4.9-5.4), 0 units | 8.87 s (8.8-9.0) | helps, -42% |
| buildd-prebuild, first version (ran tests speculatively) | 4.37 / 4.74 s | 9.26 / 8.79 s | helps |
| buildd-coalesce: A and, 1 s later, B run the identical command | B 8.52 s, CPU A+B 10.4 s | B 8.64 s, CPU 10.5 s | within noise |
| buildd-noop: lone no-op `test` | 0.68 s | 0.65 s | +0.03 s, within noise |
| buildd-loop: reference loop, no think time | 8.92 s | 8.96 s | within noise |
| buildd-prebuild-interference: other agent's wrong-guess prebuild running while B does the loop | B 9.19 s | B 8.84 s | -4%, within noise |

Run ids (first on/off sample): prebuild 20261012-004117749-1180832501 /
20261012-004055505-1180832632, interference 20261012-003028138-1056780716 /
20261012-003018557-1056780260, coalesce 20261011-235557538-455398264 /
20261011-235547557-455398328, noop 20261011-235152778-455398861.

Coalescing ties because slots already coalesce (`join_identical`, FINDINGS
7): B waits for A's slot, then pays a 0.4 s no-op cargo. In real history
identical requests overlap rarely: 12 cross-session and 30 same-session
overlaps in 5,679 agent runs. The daemon keeps it for exact output sharing,
but it is not the win.

The think time is mined from Jcode sessions: the gap between an agent's
last edit and its build call has median 4.6 s, p75 13 s, p25 0 s (2,241
calls). With 0 s (`buildd-loop`) nothing is gained or lost.

Predictor (mined, 3,436 build calls that follow an edit and a previous
build in the same session):

| prebuild | right | prebuilds per edit | wasted per edit |
|---|---:|---:|---:|
| session's last request (shipped) | 53% | 1.0 | 0.47 |
| last 2 | 62% | 1.72 | 1.10 |
| last 3 | 68% | 2.27 | 1.59 |

Wasted CPU: 0.47 builds per edit. A wrong guess usually compiles the same
edited crate for a sibling command, and it is stopped as soon as the agent
asks for something else, so the cost is at most one median agent build
(9.6 CPU-s over 4,896 compiling agent builds, mean 91 CPU-s with cold
outliers) per wrong guess, niced. Measured interference on another agent
is within noise (-4%).

Correctness (`bench/buildd-correctness.py`, prebuild vs a direct build with
the daemon off in its own warm target, exit code and normalized test and
error lines):

    pass: prebuilt=True exit daemon=0 direct=0 same_output=True no_dirty_after=True
    fail: prebuilt=True exit daemon=101 direct=101 same_output=True no_dirty_after=True
    OK

The normalization drops pass counts of a failing run, because those differ
between two direct runs too: likely-failures-first with fail-fast prints
"2 passed" once and "0 passed" the next time. Exit 101 matches on the tiny
crate as well. A daemon that hangs up makes the client build locally (fail
open). Stdout is byte-identical apart from thread ids.

Decision: **on by default** for non-terminal agent builds on Linux, off
under `CI`. Kill switches: `JUSTRUST_BUILDD=0` (no daemon) and
`JUSTRUST_BUILDD_PREBUILD=0` (daemon, no prebuild). Benchmarks pin
`JUSTRUST_BUILDD=0` unless a scenario measures it, so prebuilds of restored
files never overlap samples. Remaining risks: build scripts with side
effects run on a guess (as with any `cargo check` from rust-analyzer), and
a prebuild holds the session's slot until stopped. Next: prebuild the next
command from history instead of the last one (the 68% ceiling of last 3
without the waste), and serve `check` from early rust-analyzer state.

## 52. Windows: recording, sampling, and the depcache on Windows (2026-10-12)

Windows went from plain-cargo passthrough to the same recorder, agent
output, reports, depcache, test pool and pinned toolchains as Linux. One platform layer (`src/sys.rs`) carries every
operation whose Unix and Windows forms differ. Unix keeps its exact calls,
so on Linux nothing changes. Map from each changed path to the check that
covers it (`windows-latest` CI job, "Windows integration paths", unless
noted):

| Path | Windows form | Check |
|---|---|---|
| process exec (shim, cargo, test runner, linker) | spawn, inherit stdio, exit with the child's code, ignore Ctrl+C meanwhile | smoke: check/test/compile error |
| unit and build CPU, max RSS | job object around the child (whole tree), `GetProcessTimes` fallback, peak working set | sampler: `avg_build_cores > 0`, `peak_unit_rss_mb` sane |
| sampler (`procfs`) | Toolhelp snapshot (one per sample), `GetSystemTimes`, `GlobalMemoryStatusEx`, command line via `NtQueryInformationProcess`; no PSI | `psi_unavailable`, `procfs::tests::reads_child_process` |
| locks | `std::fs::File::lock`/`try_lock` everywhere (flock on Unix, as before) | unit tests, status |
| rustc shim, `cargo` proxy | hard link or copy of `justrust.exe` plus a per-file `.justrust` sidecar (symlinks need developer mode) | install/uninstall in CI |
| toolchain paths | `.exe` on every `bin/` tool; host triple `x86_64-pc-windows-msvc`; sha256 via `certutil` | pinned 1.98.1 install and test |
| depcache restore | copy with a fresh mtime (no reflink on NTFS); no xattr hash memo (an NTFS stream would bump mtime) | depcache across checkouts |
| depcache proc-macro key | PE `.rustc` section of the DLL. The whole-file hash missed in every second checkout: the DLL embeds its target dir (PDB path), so serde_derive and its dependents (9 hit/8 miss) | `0 miss` in the second checkout |
| depcache libc family | `msvc`/`mingw` (wine showed the host's glibc loader leaking into the key) | `libc_family_is_the_hosts` |
| sockets (test pool; also compiled for the sync daemon, peer cache, early diagnostics) | AF_UNIX via `uds_windows` (Windows 10 1803+) | test pool in every `test`; the others are not exercised on Windows yet |
| ssh to hosted machines | `NUL` instead of `/dev/null`, no ControlMaster (Windows OpenSSH lacks it) | `ssh_target_is_strict` |
| split: Cargo.toml relative paths | always `/` (a `\` in a TOML basic string is an escape) | `relative_paths` |

Not on Windows, failing open: the resident build daemon (gated to Linux
in `buildd::maybe_run`; its process-group signalling is ported but
unexercised), build slots (need copy-on-write clones, so
builds share the target dir), cgroup priority, wild and hot patching
(ELF), the sync daemon's watcher (pushes ride the per-run rescan), and
automatic remote routing (the machines are Linux). The remote agent and
hot patching compile out.

Validation before CI: cross clippy for msvc, gnu and darwin; the lib
suite under wine (x86_64-pc-windows-gnu, mingw linker) 250/251, the one
left needing a real rustc on PATH; end-to-end under wine with the Windows
Rust 1.98.1 toolchain: `check`, `test` with a failing test shown in full,
`show`, `log`, `runs`, `install`, and the depcache across two checkouts
(17/17 hits, saved 40 s of rustc). Wine quirks that are not justrust's:
rustc ICEs with incremental on (plain `cargo.exe` too), no process CPU
times (`GetProcessTimes` returns 0), no command line query.

Linux unchanged. Recorded and unrecorded builds still share artifacts (a
serde crate after `JUSTRUST_DISABLE=1 cargo build`: 1 local unit
compiled, 2 fresh, 0 Dirty, same as main). Suite A/B, main at merge-base
`dcbed51` vs the port, run twice interleaved on a busy machine (other
agents' benches, 2-8 foreign cores):
`bench/results/20261012-windows-port-ab-{main,win}-{1,2}.json`. Round 1
reads +15..54% slower, round 2 -17..+3%: the order flipped with the load,
every sample of both flagged noisy. A quieter direct measure, no-op
`justrust check` of this repo, 15 interleaved pairs: main 0.158 s median
(0.103 min), port 0.155 s (0.106 min). No regression measurable.

## 53. Remote hosts use 4.5 of 32 cores: profile of every remote run (2026-10-12)

All 351 recorded remote runs (298 on our c7i.8xlarge us-west-2, 53 hosted,
2026-10-09 to 10-12; 183 check, 134 test, 21 clippy, 13 build), mined from
`summary.json`, `samples.jsonl` and `units.jsonl`. Client wall 6841 s,
server wall 4045 s, CPU 18,036 s: **4.46 of 32 cores on average**.

Where the client wall went: compile 3606 s, sync 2556 s, startup 194 s,
lock wait 87 s, test run 72 s. Sync is bimodal: p50 0.17 s, but 54 runs
synced for over 5 s (2361 s in total, worst 359 s, a fresh worktree of
jcode-desktop, 20261011-155126540-3719621). So first sync of a new
worktree costs more than all remote compiling of warm loops.

Time-weighted build cores: under 1 core 43%, 1-2 25%, 2-4 14%, 4-8 9%,
8-16 6%, 16-32 3%. Of 2444 s with any rustc running, exactly one rustc ran
for 1515 s (62%).

| run class | runs | server wall | avg cores |
|---|---:|---:|---:|
| 0 units | 70 | 52 s | 1.3 |
| 1 unit | 88 | 285 s | 1.1 |
| 2-5 units | 59 | 416 s | 3.1 |
| 6-30 units | 73 | 936 s | 1.9 |
| >30 units (cold) | 61 | 2356 s | 6.2 |

Inside one rustc it is barely parallel, even with `-Zthreads=8`:
`jcode_desktop_ui` test link 448 s over 49 runs at 1.35 cores,
`jcode_base` 263 s at 1.98, `justrust` (no -Zthreads) 297 s at 3.62 (its
codegen units). Per-pass for the desktop UI test unit (8.8 s/run):
macro expansion 2.27 s (26%), codegen_crate 2.05 s, mono collector 0.95 s,
link 0.73 s, dep-graph persist 0.70 s. Macro expansion, resolve,
coherence, metadata generation and incremental persist are serial in
today's parallel front end, and they dominate warm loops.

Cold builds (>30 units): 53% of rustc-active time had 2 or fewer rustc
running. The serial tail is a few giant crates: aws_sdk_bedrock 111 s,
jcode_base 105 s, jcode_desktop_ui test 56 s, jcode_tui test 47 s,
jcode_app_core 32 s. Only 22 of 351 remote runs had any depcache hit.

Conclusions for picking the CPU and the work:

1. More cores only help cold, multi-crate and test-suite runs, about 1/3
   of server wall. The other 2/3 is one to three crates at 1 to 2 cores,
   where per-core speed is the only hardware lever (Zen 5 at 4.5-5 GHz,
   c8a/m8azn, over 3.2 GHz Sapphire Rapids). 32 physical cores with no SMT
   (c8a.8xlarge) beats 16 cores + SMT for the cold third too.
2. Bigger software levers than any CPU: (a) sync of new worktrees (2361 s,
   seed from a shared base snapshot or the already-mirrored repo instead
   of uploading), (b) depcache on remote (aws_sdk_bedrock should be a
   download, never a critical path), (c) rustc serial passes in jr-rustc:
   parallel macro expansion is hard, but parallel metadata encoding,
   dep-graph persist and the mono collector are tractable, (d) splitting
   giant crates (finding 47) turns serial crates into parallel units.

## 54. Seeding a new worktree's remote mirror from a mirrored sibling: first sync 45.7 -> 3.5 s (2026-10-12)

FINDINGS 53: first syncs of fresh worktrees/clones (up to 359 s) cost more
than all warm remote compile time. A fresh worktree is a new absolute path,
so rsync sent the whole tree over the WAN even though the machine already
had a near-identical copy.

Now `add_root` (remote_daemon) looks for a related root that has already
been mirrored: the in-memory roots, plus `~/.justrust/remote/mirrored.json`
(new, every root mirrored per backend, newest 64) and the legacy
`prepared.json`. A worktree of the same repo (same `git-common-dir`) wins,
then a clone sharing a root commit. Before the rsync, one ssh command copies
that mirror into the new, still-empty mirror on the machine
(`rsync -a --exclude=target/`, local to the machine). The real rsync then
sends only deltas. Sources only. The machine's `target/` is not copied,
because cargo fingerprints and dep-info are keyed by absolute path, so a
copied target dir would be rebuilt anyway (and the shared depcache/peercache
already makes dependencies cheap). Fail open: no relative found, a missing
source mirror, or a destination that already holds sources means no seed,
and the normal full rsync runs. No protocol change: this is plain ssh and
rsync, so the hosted service is unchanged.

Measured on hosted, `JUSTRUST_REMOTE=force justrust check -p jcode-desktop-model`
in a fresh `git worktree add` of jcode-desktop plus its sibling jcode (2924 files):

| run | seed | sync |
|---|---|---|
| 20261012-050045469-30871 | none (before) | 45.7 s (jcode 37.6 s, desktop 1.6 s) |
| 20261012-050544068-47967 | from ~/jcode, ~/jcode-desktop (daemon restart + agent reinstall in this run) | 12.6 s (seed 2.3 + 1.0 s, rsync 4.2 + 2.1 s) |
| 20261012-050606782-49143 | from seed2 worktrees, warm daemon | 3.5 s (seed 0.3 + 0.3 s, rsync 2.0 + 1.0 s) |

rsync still itemizes every file (worktree mtimes differ), but each one moves as a
near-empty delta. Next step if this matters: also copy mtimes from the
local base, or use `--checksum`-free size-only matching for seeded roots.

### 54b. Checksum on the first sync after a seed, and the daemon's deleted cwd (2026-10-12)

The first rsync after a seed now runs with `--checksum` (`Mirror::rsync_with`).
The seeded copy has the seed's mtimes, so the quick check delta-transferred
every file (2156 + 768 "sent", each costing a block-checksum round trip).
With `--checksum`, both sides compare full-file hashes. Equal files only get
their mtime fixed, so later syncs take the quick path. Same-size edits still
transfer because content is compared, not size alone (unit test
`seeds_from_worktree_then_clone_and_copies_sources_only`).

While measuring I hit a bug: the daemon inherited the cwd of whichever
worktree started it. Once that worktree was deleted, every rsync failed with
`getcwd(): No such file or directory` and remote silently fell back to local.
The daemon now does `chdir("/")` when it starts.

Fresh `git worktree add` of jcode + jcode-desktop, warm daemon, hosted, seed+rsync per root (daemon.log):

| | jcode | jcode-desktop | total | files sent |
|---|---|---|---|---|
| before 1/2/3 | 1.47/1.52/1.56 s | 0.97/1.01/1.00 s | 2.44/2.53/2.56 s | 2156 + 768 |
| after 1/2/3 | 1.16/1.96/2.26 s | 1.55/0.96/1.08 s | 2.71/2.92/3.34 s | 1 + 19 |

Wall time is a wash (hosted WAN jitter plus hashing ~3k small files on both
sides), but the bytes on the wire and the itemized list are now limited to
real differences, and the next sync of the root is a pure mtime quick check.
The remaining ~1 s per root is ssh round trips plus the file-list exchange.
Removing that needs a manifest-based push that skips rsync entirely, which
the daemon already uses for flushes.

## 55. Why remote hosts almost never hit the dependency cache (2026-10-12)

Question: only 22 of 351 remote runs had a depcache hit (53). Measured on the
hosted c7i.8xlarge (us-east-1), fresh target dir and fresh `JUSTRUST_DEPCACHE_DIR`
per run unless noted.

1. **The local depcache on the host works.** It lives on the persistent EBS root
   (`~/.justrust/cache/depcache`, 5.8 GB, survives stop/start, `remote down` keeps
   the disk). `check` of justrust: fresh target, warm host cache: 66/66 hits,
   10.0 -> 4.8 s (runs 20261012-115829195-2345, -115839236-5557 on the host).
   Keys are path-stable (cargo home and target dir are placeholders). Misses in
   hosted history are first builds of each dependency set on that host, plus
   the host being replaced (a new host starts empty).
2. **The shared tier can never hit hosted builds, for two reasons.**
   - Toolchain: the prebuilt store (S3 + CloudFront, the AWS move is done and
     on by default, `depcache_prebuilt.rs`) is compiled with `nightly-2026-10-08`.
     Hosted hosts build with rustup stable 1.99.0. rustc `-vV` is in every key:
     0 of 75 justrust dep keys in the store index (system) vs 32 of 75
     (nightly). The worker's account cache (`CACHE_ENABLED=false`, R2) is still
     off and stays off (compute and cache moved off Cloudflare). The S3 store
     is the shared tier.
   - Profile: jcode sets `[profile.dev] debug = 0`, so every dependency gets
     `-C strip=debuginfo`. The fleet builds default profiles only: 0 of 608
     jcode-tui keys in the index, even on the nightly.
3. Cost model on a server: with the nightly, justrust `check` fetched 1-2 of
   32 available units by default and 32 with `JUSTRUST_PREBUILT_ALWAYS=1`
   (8.9 s vs 9.9 s). The host measured 11 MB/s through CloudFront, so the
   700 ms per-fetch overhead dominates. Follow-up: measure the overhead on
   hosts and lower it there.

Cold `check -p jcode-tui` on the host, 651 units, 575 dependency misses in
every run: system 91.9 s (20261012-120834155-26986), nightly 89.0 s (-121006113-55408),
system 85.2 s (-121135161-80017), nightly + fetch-always 87.8 s
(-121339190-118835). So there is no hit and no win until the fleet publishes debug=0 keys.

Changes:
- justrust: the remote agent builds unpinned projects with the store toolchain
  (`STORE_TOOLCHAIN`, `JUSTRUST_REMOTE_TOOLCHAIN=system` opts out). A project's
  `justrust.toml` or `rust-toolchain(.toml)` or a client `JUSTRUST_TOOLCHAIN`
  wins. The client now forwards `JUSTRUST_TOOLCHAIN` and `JUSTRUST_PREBUILT`.
  The nightly installs in 10 s, once per host.
- justrust-cloud `prebuild/run.sh`: a second `debug=0` pass per profile, plus
  aws-sdk seeds (bedrock and friends, on the cold critical path in 53).
  **Not live yet:** the fleet reads `s3://jcode-justrust-prebuilt-production-280196252242/fleet/`.
  Upload `prebuild/` there (`aws s3 cp prebuild/run.sh .../fleet/run.sh`, same for
  seeds.toml), then rerun the jcode-tui cold check above.
  I left this undone because it is a production change.

## 56. Which part of the toolchain to parallelize: ledger over every recorded unit plus a -Zthreads sweep (2026-10-12)

`bench/pass_ledger.py --filter agent` (2696 agent runs, 23,388 units with
`-Ztime-passes`, 73,712 s run wall). Each unit's passes go into exclusive
buckets, weighted by the share of run wall the unit owned (1/k when k units
ran at once). `solo` = seconds when it was the only rustc running.

| bucket | share s | % | solo s | per run |
|---|---:|---:|---:|---:|
| dependency crates (no passes) | 22,894 | 37.5 | 4,589 | 8.49 |
| local crates without pass data | 11,009 | 18.0 | 8,398 | 4.08 |
| codegen to LLVM IR (main thread) | 5,427 | 8.9 | 3,744 | 2.01 |
| type check | 4,190 | 6.9 | 2,674 | 1.55 |
| macro expansion | 3,058 | 5.0 | 2,606 | 1.13 |
| borrow check | 2,888 | 4.7 | 1,751 | 1.07 |
| mono collector | 2,851 | 4.7 | 2,158 | 1.06 |
| coherence | 1,896 | 3.1 | 1,273 | 0.70 |
| incr persist (serialize_dep_graph) | 887 | 1.5 | 777 | 0.33 |
| name resolution | 886 | 1.4 | 750 | 0.33 |
| linker | 784 | 1.3 | 702 | 0.29 |
| partition_and_assert_distinct_symbols | 650 | 1.1 | 543 | 0.24 |
| metadata encode | 407 | 0.7 | 288 | 0.15 |

Outside rustc, per run: test run 3.33 s, remote sync 0.75 s, build gaps
0.49 s, startup 0.43 s, lock wait 0.33 s.

`bench/threads_sweep.py` (`threads-20261012-051915.json`, laptop, jr
toolchain), median wall per pass at -Zthreads 1 / 8 / 16:

| loop | wall | what moved with threads |
|---|---|---|
| Desktop test edit (reference loop) | 6.30 / 4.66 / 5.02 s | mono collector 0.83 -> 0.50, partition 0.61 -> 0.33, type check 0.24 -> 0.12, persist 0.75 -> 0.57. Macro expansion 0.82 -> 0.61 is noise-level, name resolution 0.51 -> 0.40 |
| jcode_base check | 2.28 / 2.28 / 2.30 s | nothing: every pass is flat |
| jcode_base build | 3.80 / 3.73 / 3.80 s | mono 0.40 -> 0.30, otherwise flat; CPU rises 3.5 -> 5.4 s for no wall gain |

(Thread counts verified per run in units.jsonl `frontend_threads`. But
the Desktop -Z8 column is n=1 clean sample (4 of 5 flagged noisy) and
-Z16 n=2, so the 6.30 -> 4.66 s gain is not established. The jcode_base
rows have n=3-5 and are solid.)

Rerun, Desktop only, n=7 per side (`threads-20261012-055648.json`, load
average 17 on 16 threads from other agents, all samples flagged noisy):
agent wall median -Z1 7.54 s, -Z8 8.11 s; unit wall 7.08 vs 7.66 s; CPU
6.9 vs 9.0 s. Only mono collector (1.17 -> 0.97) and partition
(0.67 -> 0.58) and borrow check got faster; persist, lints and macro
expansion got slower. **The 6.3 -> 4.7 s Desktop gain does not reproduce.**
On a loaded machine -Zthreads=8 costs 2 s of CPU per loop for no wall gain.
Taken together: the parallel front end does not speed up these warm loops,
because their serial passes (macro expansion, persist, resolution) dominate
and the parallel ones are small. That makes making those serial passes
cheaper or parallel (item 3) the rustc work, and -Zthreads alone a dead end
for warm loops. A quiet-machine rerun would still be worth doing.

Ranking for parallelization work:

1. **Dependency crates, 37.5%.** Each one compiles serially, so the lever is
   not threads inside rustc but not compiling them: the depcache and prebuilt
   store (FINDINGS 55: hosted keys never match until the fleet publishes
   debug=0 and nightly keys).
2. **Whole-crate granularity, 18% + the solo column.** "Local crates without
   pass data" are local crates built without `-Ztime-passes` (off by
   default), not a separate kind of crate: by wall since 10-10 justrust
   4305 s, jcode_base 2004 s, jcode_desktop_ui 1820 s, cargo 1635 s,
   jcode_app_core 802 s, jcode_tui 535 s. 76% of their share was solo
   (8398 of 11009 s). The same giant crates: split them (shadow split,
   FINDINGS 47).
3. **Inside rustc, what -Zthreads does not already parallelize in warm
   loops:** macro expansion (1.13 s/run, 2606 s solo, serial by design,
   already cut by the attr-macro cache), incr persist (0.33 s/run, can
   overlap with codegen/link: being implemented as
   `JUSTRUST_RUSTC_OVERLAP_PERSIST` by another session), name resolution,
   misc_checking_3 lints, metadata encode. Codegen to LLVM IR main thread
   (2.0 s/run) is the largest single rustc bucket and is serial per CGU
   assignment. Type check, borrow check, mono and partition already scale
   with -Zthreads on the Desktop loop, but not on jcode_base (flat), which
   suggests its query graph is a single long dependency chain there.
4. -Zthreads 16 is never better than 8 on these loops and costs more CPU.

## 57. Shadow leaf crates as a product (`JUSTRUST_SHADOW=1`): reference loop 7.8 -> 1.8 s; which upstream splits would pay most (2026-10-12)

Productizes 47 (`src/shadow.rs`, opt-in). `justrust test -p <big> --lib --
<m>` where `<m>` is a top-level leaf module (no `crate::`, top-level
`super::`, `#[path]`, child module files, macros, `env!`, feature cfgs) and
the filter cannot match tests elsewhere: when nothing in `<big>` except
`<m>.rs` changed since its last good normal build (mtime/size snapshot),
the tests run in `~/.justrust/shadow/<ws hash>/jr-shadow-<m>`, whose lib.rs
is `#[path = "<real file>"] pub mod <m>;`. Same test names, same target
dir, the user's `[patch]`, `[profile]` and `.cargo/config.toml`. The user's
tree is never written. Anything else: normal build, which refreshes the
snapshot on success. Caveat: the big crate's own uses of the leaf are only
rechecked by the next normal build.

Suite, same binary c9374f1, shadow off vs on (n=6,
`bench/results/shadow-{off,on}-c9374f1.json`):

| scenario | off | on | compile off -> on |
|---|---:|---:|---:|
| desktop-test-edit (reference loop) | 7.76 s | 1.77 s | 7.15 -> 0.54 s |
| desktop-body-test | 8.82 s | 1.57 s | 8.01 -> 0.48 s |

`compare`: -77% ("within noise" only because the first on-sample,
20261012-055159860-324787111, was 103.65 s: the shadow's first build
compiles its own copy of the dependencies, since a one-member workspace
unifies features differently), -82% "faster". Steady-state run ids: off
20261012-054948415-324789782 (7.68 s), on 20261012-055349133-324787401
(1.54 s), 20261012-055402543-324787952 (1.42 s). On the live checkout,
three leaf edits ran in 0.4 s each (3 fps_counter tests). Remaining cost
is routing overhead: startup 0.48 -> 0.92 s (cargo metadata, the snapshot
walk, an extra process). Next: cache metadata by Cargo.toml mtimes.

Candidates (`justrust shadow`, recorded edits plus git churn, because
jcode-desktop-ui's build script watches all of `src/` so rebuild reasons
never name a file): of the UI crate's most-edited files (panel 134
commits, workspace 126, input 36) none is a leaf, and most `panel_*` files
are children of `panel`. Qualifying leaves are small and rarely edited
(commands 5, preview_state 4, memory 3, fps_counter 1). In jcode,
jcode-base has one qualifying leaf (image_normalize). So the shadow helps
the leaf-module test loop but the hot files need real splits.

Upstream splits that would pay most (recorded single-rustc time from 53,
`justrust split` estimates; a note, no changes made to those repos):

1. **jcode-desktop-ui (120k lines, test unit 448 s over 49 runs, ~8.8 s per
   edit).** 214 of 315 Desktop edit runs land here. The hot code is
   `panel*` and `workspace*` (260 commits in 30 days), not leaves: move
   pure state and formatting out of them (`panel` view-model, live tabs,
   voice overlay) into crates below the UI crate so their tests stop
   linking the 8.8 s unit. Also `rerun-if-changed=src` in its build.rs
   reruns the build script on every edit.
2. **jcode-base (134k lines, 257 s).** 11 crates depend on it; every edit
   rebuilds them. `split` names session::persistence, soft_interrupt_store,
   scratch_maintenance, browser (each ~3.5 s per edit), but most files use
   `crate::`, so the first job is untangling config/storage helpers.
3. **jcode-config-types root: 883 s, 808 s of it cascade, 54 runs**, the
   single most expensive edit site in jcode. Splitting `display` out of it
   is the top config move; better, keep often-edited defaults out of the
   crate everything depends on.
4. jcode-tui (225k lines, test 64 s) and jcode-app-core (164k lines, 43 s):
   tui::app::remote::server_events (3092 lines, ~3 s per edit); app-core
   pays mostly for the `docs` build-script input (214 s over 24 runs).
5. justrust itself (251 s): one crate, 300 tests; splitting depcache/remote
   from the CLI would let most edits skip relinking the test binary.

Addendum (factor `shadow`, `bench/factors.py run shadow`, n=6 interleaved,
`bench/results/factors-20261012-060141-2501cadf6e90-dirty.json`): on
0.79 s (0.7-0.8) vs off 7.36 s (6.9-9.9), +6.57 s (-89%), "helps"
(on 20261012-060157748-432792792, off 20261012-060150553-432792924). The
factor has a warm shadow on both sides, so it excludes the one-time
~100 s first build noted above.
Verified on the live Desktop checkout: the shadow run's test lines match
the normal run's exactly (3 fps_counter tests, only the "filtered out"
count differs, 0 vs 1292), and an extra edit to lib.rs falls back
("changed outside fps_counter.rs; normal build", 7.6 s). Leaves found by
`justrust shadow --days 365`: Desktop UI has 12 (performance, commands,
preview_state, fps_counter, memory, clipboard_image, ack, effort,
native_mermaid, persisted_history, test_input, ui_actions), 35 of 2209
recorded edits. jcode-base has 14 (console, output_style,
session_metrics, util, ...), 23 of 1198 edits. About 2% of edits route.

Default-on: not yet. The gain is large when it routes, but (a) it routes
~2% of edits, (b) the first use costs a ~100 s dependency rebuild with
different feature unification, and (c) the parent crate's uses of the
leaf go unchecked until the next normal build, so a passing shadow run
can hide a break in the big crate. Revisit when the feature set is copied
exactly (shared dependency units with the normal build) and a background
`check -p <big>` runs after each routed test.

## 58. jr-rustc: overlap the incremental persist with the link: warm Desktop rustc 4.45 -> 4.20 s (2026-10-12)

Profile of the reference loop's one hot rustc (jcode_desktop_ui test,
warm one-line edit, `-Zthreads=8`, 4.73 s): after codegen, rustc ran
`TyCtxt::finish` (serialize dep graph + query result cache, 0.57-0.73 s)
and only then, with the `GlobalCtxt` freed, joined codegen and linked
(0.5 s). Both only read finished state. jr-rustc 913df80fb7a1 runs them
with `par_join`: one thread persists, the other joins codegen, saves the
work-product index and links. The session directory is renamed after both
finish, and only when no error was emitted (a failed link leaves the
session unpublished, so the next build reuses the previous one: slower,
never wrong). Taken only with incremental on, a parallel frontend
(`-Zthreads` > 1), and a plain exe/metadata output.
`JUSTRUST_RUSTC_OVERLAP_PERSIST=0` turns it off.

Factor `jr-overlap-persist` (same jr-rustc release build both sides, env
switch only, `JR_RUSTC_MODE=release`):

| measurement | on | off | saved |
|---|---:|---:|---:|
| factor wall, n=14 (`factors-20261012-062620-4ef2709df70e-dirty.json`) | 4.61 s | 4.93 s | 0.32 s (6%) |
| rustc unit wall from the same runs | 4.20 s | 4.45 s | 0.25 s |
| direct rustc replay, n=20 interleaved (`rustc-ab-overlap-persist-20261012.json`) | 4.18 s | 4.52 s | 0.33 s |
| first factor try, n=8 (`factors-20261012-060634-...`, machine load 6-12) | 4.95 s | 5.01 s | noise |

Run ids: on 20261012-062640660-661386954 .. 20261012-062843082-661386542,
off 20261012-062634197-661386337 .. 20261012-062847487-661386832. Pass
order confirms the overlap (on: link, serialize_dep_graph, finalize;
off: serialize_dep_graph, finalize, link). The persist itself gets slower
under contention (0.57 -> 0.67 s, both threads compete with the linker),
so the win is about half of link + persist.

Correctness: compile errors byte-identical on vs off; warning output is
the same set but ordered differently from run to run in both modes (the
parallel frontend's lint order, 3 of 3 orders distinct with the switch
off), and the "on by default" note attaches to whichever warning comes
first. `-Zincremental-verify-ich` clean on both sides; error then fix
builds stay incremental (3.4-3.8 s). Full `jcode-desktop-ui --lib` suite
through justrust with the fork: 1303 passed with the overlap on; the one
failure seen (`the_hint_chip_uses_only_spare_tab_space`, an animation
timing test) also failed with it off.

Default on in jr-rustc (no effect on the stock toolchain). Next (step 2b,
resident rustc): reply to cargo after the link and persist in the
background, preloading the next session's dep graph off the critical path
(estimated +0.3 s; the 0.25 s dep-graph and query-cache load at startup
is the remaining target).

## 59. Early exit: return rust-analyzer's errors before rustc (`JUSTRUST_EARLY_EXIT=1`): type-error loop 3.20 -> 0.08 s; 78/79 exits right on 120 agent-weighted mutations (2026-10-12)

Builds on findings 28-29 (`src/early.rs`): the warm rust-analyzer already
printed the first error at ~0.1 s, but the command still waited for cargo.
Now, with `JUSTRUST_EARLY_EXIT=1`, a build run by the build daemon
(`src/buildd.rs`) whose early answer is non-empty and fully coded
(`early::exit_eligible`: hard errors with a rustc `Exxxx` code, in files
changed since the last query, no parser errors) prints the block labelled
`NOT YET CONFIRMED BY RUSTC`, then a marker on stderr. The daemon strips the
marker and sends the client exit 101 at once. Cargo keeps running in the
daemon: the next identical request attaches to it (or replays its result,
`rustc's verdict from the build that continued after the early answer`),
`justrust log` has the full rustc output, and the run's report says
`early: ... (the request returned on them; this is rustc's later verdict)`.
An edit stops the background build like a stale prebuild (seen in the
e2e: `sources changed, stopping build that answered early`, the fix
rebuilt in 2.1 s). Without the daemon, or when the answer is empty,
uncoded, or not ready, nothing changes (fail open). Commits 741619d,
023a708, 181e4a7.

**Speed** (`bench/factors.py run early-exit`, n=8 ABBA, Desktop
`check -p jcode-desktop-ui` after the `desktop-type-error` edit, warm early
daemon on both sides, `factors-20261012-074711-5cad19f84eb6-dirty.json`):

| | median | range |
|---|---:|---:|
| off: early block printed, wait for cargo | 3.20 s | 2.95-3.36 |
| on: return on the early answer | **0.08 s** | 0.07-0.29 |

Every sample's rustc verdict (recorded by the background build) confirmed
the early error (`matched` 1, `early_only` 0 in all 16 runs). The off
column is higher than the suite's 1.96 s (FINDINGS 28) because the factor
runs `check` through the build daemon on a busy machine (all samples flagged
noisy, other agents' builds); the on side does not depend on it.

**Accuracy at scale** (`bench/early_exit/mutations.py`, seed 3,
`early-exit-mutations-20261012-074445.json`). Mutations of the error kinds
agents hit, weighted by the mined first-error mix of the 486 recorded
agent compile-error runs with a location (`bench/early_exit/mine_codes.py`:
E0308 52%, syntax 7%, E0425 5%, E0063 5%, E0433 4%, E0599 3%, E0061 3%,
E0432 2%...), 60 per workspace, at random sites in 6 Desktop and 8 Jcode
crates, each checked with `check -p <crate> --all-targets`:

| | Desktop | Jcode | total |
|---|---:|---:|---:|
| mutations (rustc failed) | 60 (58) | 60 (58) | 120 (116) |
| early exit fired | 41 | 38 | 79 |
| right file, line, code | 38 | 32 | 70 |
| right file and code, line within 5 (same expression) | 3 | 5 | 8 |
| wrong | 0 | 1 | **1** |
| exit on a build rustc passed | 0 | 0 | **0** |
| recall (exits that are right / rustc failures) | 70.7% | 63.8% | 67.2% |
| clean edits (should compile), false exits | 0/12 | 0/12 | **0/24** |
| clean sweep, files / false exits after a clean edit | 257 / 0 | 923 / 0 | 1,180 / 0 |
| rust-analyzer answer, median | 0.25 s | 0.41 s | 0.35 s |
| rustc `check -p`, median where it fired | 6.1 s | 5.1 s | 5.9 s |

The wrong exit: a misspelled type path (rustc E0433 at line 84), which
rust-analyzer reports as two E0599 `no method arg on type Command` two
lines later. Right file, wrong error; the agent's next run gets rustc's
verdict. Misses (37, no exit, the build waits for rustc as before): a
dropped `;` or `)` (19: parser errors have no code and never exit),
E0384 assign-to-immutable (7: rust-analyzer does not report it),
misspelled type paths (6), others 5.

Fixed on the way, found by the first runs (before: Jcode 19 wrong of 72,
Desktop 38 wrong of 46 in a run that also restored files):
- **Per-file baseline.** On Jcode rust-analyzer reports 9 coded errors on
  sources rustc accepts (unresolved import and `!: Provider` through a
  dev-dependency cycle in `jcode-base/src/provider/tests*.rs`). The daemon
  now opens a file with its pre-edit text first and records its errors;
  those never count. Files touched but unchanged (a restore, a checkout)
  are not "changed" any more, only baselined.
- **Never type.** Errors about type `!` are follow-ons of an error
  elsewhere (duckling's review); dropped in `to_diag`.

**Replay on the recorded runs** (`bench/early_exit/replay_savings.py`,
`early-exit-replay-20261012.json`). Per run of the 563 agent runs that
ended in `could not compile` (16,344 s of wall), first error code from its
output; per code the bench's rate of right and wrong exits (codes with
fewer than 3 mutations: `pooled` uses the overall rate, `conservative`
never fires). A right exit saves the wall minus 0.38 s (client plus
rust-analyzer median), a wrong one is charged a whole extra run:

| runs | wall | net saved, conservative | net saved, pooled |
|---|---:|---:|---:|
| all 563 | 16,344 s | 1,911 s (11.7%) | 2,824 s (17.3%) |
| 119 outside bench and scratch copies | 2,105 s | 618 s (29.3%) | 1,144 s (54.3%) |

Wrong-exit cost in the model: 61 s. 444 of the 563 are bench and scratch
runs of the Desktop type-error scenarios (fast 2 s checks), which is why
the real-agent share is higher.

**Ending at rustc's `could not compile` instead** (the first worker's
measurement, handoff): 209 s of 4.54 h (1.3%), and 38 of 563 runs report
further errors after it, which an early return would hide. Not
implemented: too small for an output change, and the rust-analyzer exit
covers the same runs ~2 s sooner.

**Memory**: rust-analyzer 4.8 GB (Desktop) and 5.4 GB (Jcode after a
sweep) per workspace, 11-35 s to ready with its target warm.

Verdict: **opt-in** locally (`JUSTRUST_EARLY_EXIT=1`, which turns early
diagnostics on): 1 wrong exit in 79 and 5 GB per workspace are the
user's to accept, and `auto` early mode only starts daemons for busy
workspaces. Recommended **default on the hosted service** where memory
is the service's: plan in `docs/early-exit-hosted.md` (switch, 6 GB
cgroup cap per tenant workspace, at most 8 per 64 GB VM, idle / LRU /
memory-pressure eviction), not deployed. Next: codes for parser errors
(rustc's `expected ;` is reliable enough to exit on, measure first),
E0384 from rust-analyzer, and buildd under `JUSTRUST_ON_REMOTE_AGENT` for
hosted.

## 60. jr-rustc early reply: answer cargo after the link, persist in the background (`JUSTRUST_RUSTC_EARLY_REPLY=1`): reference loop 4.32 -> 4.10 s, within noise, opt-in (2026-10-12)

Step 2b of the resident-rustc track (58 was 2a). With the overlap of 58,
the hot rustc's link finishes about 0.4 s before the process exits (the
rest of the dep-graph and query-cache persist, plus the session rename
and garbage collection). Direct replay: link done 3.41 s, exit 3.82 s.

Mechanism (jr-rustc 7f5a85180621, 9c3c3ab5531a). The shim opens a pipe
and passes the write end as `JUSTRUST_RUSTC_EARLY_REPLY_FD`, only for
local incremental units and only under `JUSTRUST_RUSTC_EARLY_REPLY=1`.
After a successful link with no errors, rustc runs `finish_diagnostics`
(now idempotent), points fds 1 and 2 at /dev/null so the shim's stderr
relay sees EOF, writes one byte, and keeps persisting. The shim returns
0 without reaping. Before replying, rustc drops a `jr-persisting` marker
in its `-working` session dir, and removes it only after the rename and
GC have released the lock. `prepare_session_directory` waits until no
locked session dir of the crate carries a marker (60 s cap; a dead owner
has no lock and is not waited for). Errors and any other path never
reply early, so the shim falls back to `wait4`. A rustc without the
feature never writes the byte (EOF), and the shim waits normally.

The first version removed the marker before the rename. A next session
started in that window got `WouldBlock` on the source dir's lock, warned,
and started from scratch: 17-19 s instead of 4 s, two of fourteen samples
in `factors-20261012-072936-181e4a7e9e98-dirty.json`. The fix keeps the
marker until unlock.

| measurement | on | off |
|---|---:|---:|
| factor `jr-early-reply`, n=14 (`factors-20261012-080154-9c747bcafaa4-dirty.json`), wall | 4.10 s | 4.32 s |
| same runs, recorded jcode_desktop_ui unit | 3.63 s | 3.85 s |
| direct replay, n=14, under load 10-15 (`early-*.json` in scratch) | 4.54 s done | 5.73 s |
| back-to-back replay (next edit right after the reply) | wait 0.3-0.5 s, build 3.7-4.2 s, incremental | |

Run ids: on 20261012-080405429-1491534431 .. 20261012-080547653-1491534345,
off 20261012-080401328-1491534884 .. 20261012-080551782-1491534835.

Correctness: diagnostics identical with and without (normalized for the
parallel frontend's lint order, see 58). A type error never replies early
(rc 1, full output). `-Zincremental-verify-ich` is clean on a session
that waited. Full `jcode-desktop-ui --lib` suite through justrust with
early reply on, two edits in a row: 1303 passed both times, second edit
incremental (1 unit).

Verdict: opt-in. The saving is real but small (0.2 s, inside run-to-run
noise at n=14), and it only reaches the agent when the persist tail is
not immediately needed. An agent that edits again within ~0.4 s gets it
back as wait time. Costs: an unreaped rustc can outlive the shim by about
0.5 s, and justrust's recorded rustc CPU/RSS is 0 for replied units.
Default-on would need the shim to record usage from /proc before
returning, plus a larger n on a quiet machine.

Side finding: my direct-replay scripts edited the shared bench snapshot
without the bench lock and restored from a copy read at start. That
raced kikazaru's early-exit factor and left a stale injected type error
in `fps_counter.rs`. Replays now edit from a private base copy under the
bench lock (`flock -o ~/.justrust/bench/work/.bench.lock`). Also, cargo
caches `rustc -vV` by the rustc binary's mtime, so a rebuilt
librustc_driver needs `touch bin/rustc` and fresh factor target dirs,
or every unit fails with E0514.

### What a true resident rustc could still win (stop decision)

Warm profile of the same hot rustc after 58 and this section (early
build, `-Ztime-passes`, median of 5, total 4.04 s): serialize_dep_graph
1.02 s and link 0.51 s (overlapped, and off the reply path with early
reply), codegen_crate 0.91 s, macro_expand 0.69 s, resolve 0.41 s,
monomorphization walk 0.39 s, typeck plus borrowck about 0.11 s. Startup
costs that a process kept alive between edits would remove:

| cost | per edit | source |
|---|---:|---|
| dep-graph decode + query-cache open, main thread waits before expansion | ~0.25 s | step 1 self-profile (0.19 + 0.06 s) |
| exec, dynamic loading of librustc_driver | 0.01 s | `rustc -vV`, median of 5 |
| persist tail | 0 | already off the reply path (2b) |

That is about 0.26 s, under the 0.3 s bar set for this track, so the
preloading server is not built. Everything bigger lives inside a session
(expansion, resolution, mono walk, codegen). To reuse it, rustc would
have to keep the `GlobalCtxt` alive across sessions, with its arenas,
interners, `CStore` with ~620 upstream crates' metadata, the resolver
outputs, and the query caches. Then it would have to invalidate exactly
what an edit touches, without the red/green replay from disk. That means
replacing `'tcx` arena lifetimes with a generation scheme, a resettable
`Session` (options, diagnostics, source map), and in-memory red/green
marking. That is a multi-month compiler redesign (the rust-analyzer
model), not a step on this track. Cheaper per-edit wins in the same
profile: macro expansion (0.69 s, the attribute cache in 29 already took
most of it) and body-only invalidation (`docs/integrations.md`).

## 61. jr-cargo fork server (`JUSTRUST_CARGO_DAEMON=1`): no-op `test` 0.24 -> 0.12 s, body-edit `check` 1.62 -> 1.46 s (noise), opt-in (2026-10-12)

Before the first rustc starts, a warm `cargo check -p jcode-desktop-ui`
spends 0.15-0.25 s in cargo: `create_bcx` 142-182 ms (resolve
90-126 ms, `download_accessible` 29-39 ms) plus fingerprinting. None of
it depends on `.rs` contents. jr-cargo now keeps a parked process per
exact invocation that has already run `create_bcx`, and forks it per
request (jr-cargo `src/jr_daemon.rs`, e238aec29..).

Mechanism. Key = hash of the cargo binary (path, inode, mtime), argv,
cwd, and the environment minus caller-identity variables (justrust run
ids, `JUSTRUST_TEST_POOL`, Jcode/terminal ids). The client (`main`'s
first line) connects to `$XDG_RUNTIME_DIR/jr-cargo/<key>.sock`, sends
fds 0-2 (SCM_RIGHTS) and its full environment. The server recomputes a
stamp (stat of `Cargo.lock`, every member, path-dependency and inherited
workspace manifest, cargo config files, rustc and wrappers, plus the
entry names of `src`, `src/bin`, `tests`, `examples`, `benches` of every
path package, for target auto-discovery). On mismatch it answers stale
and exits; otherwise it forks. The child takes over the client's fds and
environment (the config env snapshot reads volatile variables from the
client), times from the request, writes its pid to
`$JUSTRUST_RUN_DIR/cargo-daemon.pid` (justrust samples that tree), runs
the rest of `compile_ws`, and sends its exit code. Client killed: the
child kills its process group. No server, stale, or any error before
the fork: the client runs as plain cargo and, at exit, starts a server
(same argv/env, `setsid`, inherited fds closed), which parks right after
`create_bcx` only if a single thread is left (the idle `futures-timer`
helper allowed), no package-cache lock is held, no build logger, and no
input changed while it loaded. Only `build`/`check`/`test`, never with a
terminal on stdout/stderr or a jobserver in the environment. Children
skip cargo's once-a-day `CARGO_HOME` auto-gc. justrust side: the test
pool socket moved from the runner argument to `JUSTRUST_TEST_POOL`
(`--pool=env`, da4f05e) so cargo's argv is identical every run.

| measurement | on | off |
|---|---:|---:|
| factor `cargo-daemon` (desktop-noop-test), n=12, `factors-20261012-090625-bf4f90e1e837.json` | 0.12 s (0.10-0.16) | 0.24 s (0.17-0.26) **helps** |
| factor `cargo-daemon-check` (desktop-body-check), n=12, same file | 1.46 s (1.1-1.7) | 1.62 s (1.2-1.8) within noise |
| first run, `factors-20261012-085616-da4f05e3eae5-dirty.json`: noop-test n=12 | 0.11 s | 0.23 s |
| same file: body-check n=8 | 1.36 s | 1.59 s |
| same file: reference loop (desktop-test-edit) n=8 | 5.25 s | 5.54 s (noise) |
| justrust's own phase split, loop sample on/off | cargo startup 0.05 s | 0.20 s |
| parked server RSS (per key) | 215-253 MB | |

Run ids: noop on/off 20261012-090626444-2173682125 ..
20261012-090630831-2173682721, check 20261012-090634281-2173682519 ..
20261012-090709144-2173682883, loop pair on 20261012-090202381-2095872373
/ off 20261012-090156671-2095872398.

Correctness (`jr/daemon-correct.py` in jr-cargo): two private copies of
the pinned Desktop snapshot, one with the daemon, one plain, 18 steps
(no-op, body edits, a type error and its fix, a jcode-core edit, adding
and removing a `tests/` file, a manifest edit, warm and edited `test`,
a failing test and its fix). Identical Fresh/Dirty/Compiling sets, exit
codes and stdout (modulo the panicking thread id); a plain cargo run
right after every daemon run is all Fresh (fingerprints on disk are
plain cargo's). Through `justrust test` with the test pool: same test
lines, report shows the rustc unit and its phases.

Verdict: opt-in. It saves the full 0.1-0.15 s cargo startup on every
warm request, a clear win on no-op runs, but on the edit loops it is
2-5% and inside run-to-run noise. Each parked key holds ~230 MB, and an
agent uses several keys (check, test, different `-p`). Default-on wants
(1) eviction across keys (LRU, memory cap), (2) the build daemon
(`src/buildd.rs`) prestarting the server for the predicted next command,
so the first request after a manifest change is also served, (3) the
default jr toolchain shipping this jr-cargo. Also open: fingerprint
calculation (57-95 ms) still runs per request; caching fingerprints of
units whose source mtimes are unchanged in the parked process is the
next cargo-side win.

## 62. After the architecture tracks: justrust defaults vs stock, end to end (2026-10-12)

Whole-result check after findings 48-61, with justrust reinstalled from
main (0f5714b) and its own suite passing (307 tests, local and hosted,
clippy clean).

**System 3, stock cargo + system rustc vs justrust defaults** (pinned
Desktop snapshot, n=6 after one warmup, round-robin,
`versus-20261012-091520-0f5714b38bf4.json`). `versus.py` pins
`JUSTRUST_BUILDD=0`, so this excludes prebuild:

| workload | stock | justrust | ratio | instructions |
|---|---:|---:|---:|---:|
| reference-loop | 6.68 s (5.9-7.0) | 3.88 s (3.6-4.7) | **1.72x** | 0.72 |
| body-check | 2.16 s (1.6-2.3) | 1.53 s (1.2-1.6) | 1.41x, ranges touch | 0.95 |
| noop-test | 0.47 s (0.4-0.5) | 0.29 s (0.3-0.3) | **1.63x** | 0.64 |
| geomean | | | **1.58x** | |

Before the tracks, the same reference loop was stock 6.45 s vs 4.75 s for
the best justrust stack (`versus-20261011-111710-07262f4deb42-dirty.json`,
1.36x). Now it is 1.72x.

**Prebuild on edit, as shipped** (`bench/factors.py run buildd-prebuild
-n 6`, installed binary, 4.6 s mined think time,
`factors-20261012-091523-0f5714b38bf4.json`): on 3.51 s (2.6-3.8) vs off
6.86 s (6.6-7.7), -49%, disjoint ranges. That reproduces finding 51
(5.14 vs 8.87 s) on today's faster baseline. With the versus row above,
an agent that thinks for the median 4.6 s before testing waits about 3.5
s instead of stock's ~6.7 s, about 1.9x.

**No cache invalidation from the new defaults** (real `~/jcode-desktop`,
`check -p jcode-desktop-ui`): `JUSTRUST_BUILDD=0 JUSTRUST_LOCALCACHE=0`,
then defaults, then off again: the second and third runs compiled 0 units
(746 fresh). Switching the daemon or the local cache on or off never
dirties cargo's or rustc's state.

## 63. Cross-crate early cutoff (jr-rustc interface hash + jr-cargo): deep body edit 9.67 -> 5.16 s, 14 of 15 cascade crates skipped (2026-10-12)

Finding 16 showed this is impossible from a wrapper: the SVH changes on
every edit, and a dependent built against the old SVH does not load
(E0460). With both forks it works.

**jr-rustc** (branch `justrust-10x`, b35416a3743d; on for jr toolchains
through the shim's `JUSTRUST_RUSTC_IFACE_HASH=1`, `JUSTRUST_CUTOFF=0` off,
`-Zjr-iface-hash` by hand):

- An *interface hash* of the crate metadata stream, computed while
  encoding: every byte except spans and the source map. Every value
  derived from a byte position (lazy distances, table entries, type and
  predicate shorthands, the alloc index) is hashed in logical form, with
  the skipped span bytes before it subtracted, so spans keep their
  normal, variable-size encoding. Symbol back-references hash the string.
  Dependencies' crate hashes are replaced by their interface hashes.
  Expansion hashes (they hash call-site spans) are skipped. Proc-macro
  crates get no interface hash, so their dependents always rebuild.
- Written next to the `.rmeta` as `<rmeta>.jriface` (32 hex chars),
  before the artifact notification, so cargo sees it with the rmeta.
- Each dependency's interface hash is recorded in the dependent's
  metadata (`CrateRootUnhashed`). The loader then accepts a dependency
  whose crate hash differs when its interface hash matches the recorded
  one (`existing_match`, `crate_matches`).
- A crate loaded against such a newer dependency gets an incremental
  `crate_hash` that folds in the dependencies' actual hashes
  (`CrateMetadata::incr_hash`). Without it the Desktop suite run ICEd
  (`Found unstable fingerprints for module_children(jcode_base::id)`).
  The cause: a glob re-export's spans point into the edited crate's
  file, so decoded spans moved while the crate's SVH, and so every
  incremental query on it, stayed green. With the fold, queries on cut-off
  crates are recomputed in the final crate (cheap: decoding), and the
  cut-off crates themselves are never compiled.
- Foreign spans decoded from a stale crate are clamped to the file length.
- `Generics::param_def_id_to_index` is now encoded in index order. The
  hash map's order differed between a fresh and an incrementally reused
  `Generics`, which changed the interface hash of an unchanged crate
  (found by diffing the dumped streams, `JUSTRUST_RUSTC_IFACE_DUMP`).

**jr-cargo** (`src/compiler/fingerprint/jr_cutoff.rs`, `JUSTRUST_CARGO_CUTOFF=0`
off): a lib/check unit that is dirty only because a dependency rebuilt is
deferred to execution time. If every transitive lib dependency's current
`.jriface` equals the one recorded at its last compile, rustc does not
run: output mtimes are bumped and the fingerprint is written.

Factors (`bench/factors.py jr-cutoff*`, same jr toolchain both sides, off
= both switches off), final build (`factors-20261012-111123-10e8ae34da92.json`):

| factor | workload | on | off | units on/off |
|---|---|---:|---:|---:|
| `jr-cutoff-deep` | body edit in jcode-core (Jcode repo), Desktop UI tests, n=8 | **5.16 s** | 9.67 s | 2 / 16 |
| `jr-cutoff` | body edit in jcode-desktop-ui-core, n=8 | 5.28 s | 5.42 s | 2 / 4 |
| `jr-cutoff-cost` | UI crate body check (nothing to cut off), n=12 | 1.70 s | 1.73 s | 1 / 1 |
| `jr-cutoff-sig` | new pub item upstream (control), n=6 | 2.26 s | 2.10 s | 4 / 4 |

The deep result reproduced in four runs: 10.88 -> 5.48 s
(`factors-20261012-083222-...`), 9.84 -> 5.09 s (`...-090950-...`),
9.74 -> 4.69 s (`...-095018-...`), 9.67 -> 5.16 s. The off side's
jcode_base alone is 3.6 s. On `desktop-upstream-body` the two cut-off
sibling crates are not on the critical path (the UI test crate starts
after ui-core either way), so it stays within noise. The control
(interface change) compiles the same units on both sides.

Cost side. The first version padded spans to a fixed size (positions
then did not move). That grew every rmeta 24-42% (all dependencies
together 866 -> 1825 MB) and cost +0.2 s per check in macro expansion
and resolution, which decode dependency metadata. Logical positions
bring the overhead to 0.1% of rmeta size (22,248 -> 22,266 bytes on a
test file). Metadata generation of the UI crate takes 0.002 s either way.

Correctness:

- `jr/iface-hash-test.sh` (jr-rustc): body edits, added lines, comments
  and panic messages keep the hash. Signature, inline body, generic body,
  macro, const, new item and field type changes alter it. A dependent
  built against the old `up` links against a body-edited `up` and runs
  the new body, with a panic location from the new file. A signature
  change is rejected (E0460). No sidecar without the switch.
- `jr/iface-stress.py`: 25 random rounds of 1-400 inserted lines in 1-3
  bodies, up to 300 leading comment lines, and added pub items. All
  correct.
- `jr/cutoff-real-e2e.sh` (jr-cargo, real jr-rustc): cutoff on two
  successive body edits, binary output reflects the new bodies, rebuilds
  on inline, generic and struct changes, test and check modes, off switch.
  Now includes a glob re-export through a cut-off crate, the ICE
  regression above.
- `bench/cutoff_semantic.sh` (Desktop): a temporary UI test asserts on
  what an upstream function returns. Two cut-off crates every time; the
  test sees each new value and then the old one.
- `bench/cutoff_suites.sh`, jcode-core body edit after a warm build:
  Desktop workspace, 14 units cut off, 1752 ok on both sides in two
  repeats, per-test results identical (two known timing-sensitive tests
  are listed separately; both failed once on one side only and pass on
  rerun). Jcode workspace (8,970 tests, 30 units cut off vs 0, 69 vs 99
  units compiled): the same 12 environment-dependent failures on both
  sides. The 7 tests that differed between the runs pass 5 of 5 in
  isolation on both builds, except one that fails 5 of 5 on both.

Not yet on by default: the jr toolchain with the interface hash is
`jr-b35416a3743d` (installed, never the default, `JUSTRUST_TOOLCHAIN` to
use it). Next: PGO+BOLT build of this branch as the default jr toolchain,
then make the cutoff part of the shipped toolchain.

Run ids: deep on 20261012-111239548-3526197124 .. 20261012-111411908-3526197109,
off 20261012-111229803-3526197310 .. 20261012-111417192-3526197450.

## 64. Test-filter-aware compilation (`JUSTRUST_TEST_FILTER_COMPILE=1`): test binary 242 -> 49 MB, reference loop within noise (2026-10-12)

The libtest filter (`-- fps_counter`) reaches jr-rustc as
`JUSTRUST_RUSTC_TEST_FILTER` (one filter per line, `--exact` supported,
recorded as an env dependency). The harness then lists only the
selected tests. Each unselected test's `static TestDescAndFn` becomes an
`#[allow(dead_code)] const`: it is still type-checked, and the functions
it calls stay "used" (no new dead_code warnings, verified), but it is no
longer a codegen root. Custom `#![test_runner]` crates are left alone.
justrust sets the variable for `test` on jr toolchains when the filter is
plain (positional names, `--exact`; unknown libtest flags disable it).
`jr/test-filter-test.sh`: listing, run, multiple filters, exact, no match,
real dead code still warned, type errors in unselected tests still
reported, 8 -> 0 unselected test symbols, env-dep in dep-info.

On Desktop (`fps_counter`, 3 of 1314 tests) the test binary is 242 MB
instead of 49 MB, and test symbols drop from 42,607 to 106. But the warm
loop does not change: `jr-test-filter` 4.49 vs 4.51 s, n=10
(`factors-20261012-095018-...`, an earlier run 4.20 vs 4.69 s,
`...-090950-...`). In the warm loop incremental codegen already reuses
every unchanged test's codegen unit, and the edited one is redone either
way. The frontend still covers every test, and link only drops
0.23 -> 0.18 s. Opt-in: it rebuilds the test binary whenever the filter
changes, and the build daemon's prebuilds have no filter.

Takeaway: codegen of unselected tests is not where the reference loop's
time goes. The frontend (expansion 0.6-0.8 s, resolve 0.3-0.5 s,
typeck) covers all 1314 tests. A filter-aware *frontend* (skip
`#[cfg(test)]` bodies of unselected tests entirely) is the version that
could pay. It needs to keep their items for name resolution, and to
re-check them in the background build.

## 65. Hosted service: store keys that match real projects, job count, wake and install fixes (2026-10-12)

All on the hosted c7i.8xlarge (us-east-1, 32 vCPU), cold `check -p jcode-tui`
(651 units, 575 dependency units), fresh target and depcache per run,
nightly-2026-10-08.

**Why the store never hit hosted builds (after FINDINGS 55's fix shipped).**
The fleet's debug=0 pass set `CARGO_PROFILE_DEV_DEBUG=0` in the environment.
`CARGO_*` variables reach rustc and are keyed, so those 768 keys matched no
project that sets `debug = 0` in Cargo.toml (0 of 768). Separately, the seed
graph resolves other versions and feature sets than real lockfiles: 77 of
jcode-tui's 625 keys were shared. Fix (justrust-cloud a38d762, 2b deployed to
`fleet/`): debug=0 via Cargo.toml, and the fleet also builds jcode,
jcode-desktop and justrust from their own lockfiles (dependency units only are
published; workspace crates go to the localcache, which is never published).
Store: 3373 -> 7185 manifests. jcode-tui keys in the index: **0 -> 529 of 701**.

**The job cap matters more than the store.** Jcode sets `jobs = 4`. Store
hits are ~0.2 s each, so 605 of them ran 4 wide at 2.1 of 32 cores. The
hosted agent already overrides jobs (`remote_agent::machine_jobs`, shipped
before this session), so real hosted builds already ran at 32 jobs; the
jobs 4 rows below are builds started over plain ssh (as in FINDINGS 55), not
what users see. The override set `CARGO_BUILD_JOBS`, which was keyed, so every
agent-run key differed from the store and from every other machine. 0a7d006
stops keying it. The user-facing win of this session is the jobs-32 store
rows: ~41-44 s -> ~36-39 s with all hits fetched (4-10%).

| run (host, cold `check -p jcode-tui`) | wall | dependency hits |
|---|---:|---:|
| jobs 4, store off (before, FINDINGS 55) | 85.0-91.9 s | 0 |
| jobs 4, store on, cost model | 71.6-85.2 s | 43-46 |
| jobs 4, store on, fetch all | 79.6 s | 529 |
| jobs 32, store off | 41.6-44.5 s | 0 |
| jobs 32, store on, cost model | 37.5-41.3 s | 42 |
| jobs 32, store on, fetch all | 35.7-38.7 s | 529 |

Takeaway: on a server the cost model's 700 ms per-fetch overhead declines 487
of 529 available hits, while fetching them all is ~2-4 s faster at 32 jobs.
Next: per-machine fetch overhead (measure it on hosts, ~0.2 s there), then
fetch-all on hosted. The remaining ~35 s is the three giant local crates
(jcode_app_core, jcode_base, jcode_tui: 16.6 s serial, FINDINGS 53).

**Wakes failed over to local.** About 1 in 3 wakes returned 503: EC2 Instance
Connect answers `ServiceException` for a moment after a start. The worker now
retries (3 tries) and answers 202 booting instead (justrust-cloud d054ace,
deployed, test added). One 503 was still seen afterwards. Not reproduced in 6
more wakes, and the laptop's network was failing DNS at the time.

**Installing justrust on the machine dominated syncs.** Every client binary
change uploaded 11 MB: 40-130 s at the home uplink (90 KB/s measured,
`events.log`: 54-126 s installs). The machine now downloads the matching
release asset from GitHub and installs it only when its sha256 equals the
client's executable (b4aa0c7): 0.39 s on the host, a mismatching hash is
refused. Takes effect from the next release (dev builds still upload).

**Hosted functional matrix (all passed):** check, clippy and filtered test of
justrust; compile error (E0308 in full, exit code) and a failing test (panic
message, `1 of 2 tests failed`, exit 101) in a scratch crate; Desktop
`test -p jcode-desktop-ui --lib -- fps_counter` (37.9 s cold mirror); two
concurrent builds from different workspaces; wake from stopped through a build.
`npm test` in justrust-cloud: 41/41.

Fleet: jcode-desktop's lockfile is stale against jcode HEAD, so `--locked`
failed; run.sh falls back to a minimal lock update. Its `test --no-run` does
not compile at HEAD (project issue). Fleet logs
`fleet/logs/20261012T224351Z.log`, `20261013T002752Z.log`.

Host run ids: 20261012-215912915-1857 .. 20261012-220339511-36302 (before),
20261013-001826 .. 20261013-003715 (after, `~/.justrust/runs` on the host).

### 65b. Acceptance through the agent route (f67c993)

The agent now sets `JUSTRUST_PREBUILT_ALWAYS=1` on the machine. From the laptop,
`JUSTRUST_REMOTE=force justrust check -p jcode-tui` in a fresh `git worktree`
of jcode: 530 dependency hits (529 prebuilt) of 575, build 32.4 s on the host,
38.2 s remote total (sync 3.1 s, the sibling-mirror seed of FINDINGS 54), 41 s
from the agent's seat. Before this session the same shape of build had 0 hits
and 85-95 s over plain ssh, ~41-44 s through the agent (jobs 32, no hits). The
immediate rerun is a no-op in 2.4 s.

### 65c. Repeats through the agent route, and a removed-worktree bug

Fresh `git worktree` of jcode per run, `JUSTRUST_REMOTE=force justrust check -p
jcode-tui` from the laptop (hosted, f67c993 + 2952861):

| run | build on host | remote total | dependency hits |
|---|---:|---:|---:|
| 4 (warm host depcache) | 28.9 s | 39.9 s | 575 of 575 |
| 5 | 19.1 s | 25.8 s | 575 |
| 6 | 18.8 s | 47.3 s (sync 26.3 s) | 575 |
| 7 | 19.2 s | 24.7 s | 575 |
| 8 | 19.2 s | 26.4 s | 575 |

These hit the host's own depcache, filled by the earlier runs, not only the
prebuilt store (the store's 529 hits were measured in 65b: 32.4 s). Store-only
cold on a fresh host stays 32-39 s.

Two runs were contaminated: another agent session set
`[env] LIBRARY_PATH=~/jr-syslibs/...` on the shared hosted machine to link
Desktop, which is keyed, so one run got 0 hits (59.1 s). Hosted machines are
per account and shared by all of its agents; env set there leaks into every
build's keys. Fixed at the source by the other session (justrust-cloud cf119d4
installs those packages in the image).

Bug: removing a worktree that had been mirrored made the next connect or push
rsync a missing dir, which failed the whole link and sent the build local
(`justrust: remote unavailable (rsync ... No such file or directory)`, runs 1
and 3, which then built locally in 32.6 and 28.4 s). 2952861 forgets deleted
roots. Runs 6-8 then removed their worktrees between runs and stayed remote
(`forgot ... (deleted)` in daemon.log).

Release check: v0.1.0-alpha.11 installed via jcode.sh/rust.sh; first remote build: "fetched from its release ... in 0.4s", installed in 0.6s (was 54-126 s by upload).

### 65d. Verification pass: interleaved repeats, a leaked override, and the requirement map

**Interleaved n=3 on the host, alpha.12, agent environment** (`CARGO_BUILD_JOBS=30`,
store toolchain, fresh target and depcache per run, localcache off), cold
`check -p jcode-tui`:

| store | runs (s) | median | dependency hits | exit / warnings |
|---|---|---:|---:|---|
| fetch all | 35.4, 34.5, 34.2 | 34.5 | 529 prebuilt / 575 | 0 / 3 each |
| off | 43.1, 42.5, 43.2 | 43.1 | 0 | 0 / 3 each |

**-20% (8.6 s)**, disjoint ranges, identical exit code and warning count.

The first attempt at this table got 0 hits on both sides: another session had
left `[env] LIBRARY_PATH=/home/ubuntu/jr-syslibs/...` in
`/home/jeremy/.cargo/config.toml` on the shared machine (mirrored-path cargo
config), and `LIBRARY_PATH` is keyed, so every key moved (0 of 701 in the
index; key inputs differed only in that line). I renamed it to
`config.toml.jr-syslibs-disabled` (reversible). Desktop `test -p
jcode-desktop-ui --lib -- fps_counter` still passes remotely without it (17.1 s).
Risk that remains: any cargo `[env]` or user env on a hosted machine silently
disables the store. A keyed-env mismatch should show up in `justrust show`
(follow-up).

| requirement / changed behavior | check | observed |
|---|---|---|
| hosted cold builds faster | interleaved n=3 above | 43.1 -> 34.5 s median, 529 hits |
| store keys match real projects | key inputs diff, index overlap | 529 of 701 keys in index (was 0) |
| `CARGO_BUILD_JOBS` not keyed | unit test `cargo_home_is_not_keyed`; host explain shows no `CARGO_BUILD_JOBS` line | pass; absent |
| fetch-all on the machine | agent-route run from a fresh worktree (65b) | 529 prebuilt hits through `JUSTRUST_REMOTE=force` |
| client can still turn the store off | `JUSTRUST_PREBUILT=off` via agent, fresh crate vs default | off: 3 hit (host depcache only), default: 11 hit |
| store hits give the same build | exit code and warnings, store vs off, n=3 each | equal (0, 3) |
| wake never fails over to local | worker unit test; 3 stop/start cycles through `justrust check` | 3/3 remote, 0 503s (earlier: 1 of 3 wakes 503) |
| machine installs from the release | alpha.11 installed via jcode.sh/rust.sh, first remote build | `fetched from its release ... 0.4s`, install 0.6 s |
| dev builds still upload | wakes with the cargo-installed alpha.12 dev binary | upload 2.8-49.1 s, builds succeed |
| removed worktree keeps remote | 3 runs removing their worktree between builds | 3/3 remote, `forgot ... (deleted)` logged |
| errors and failing tests reported in full remotely | scratch crate (65) | E0308 full, panic message, exit 101 |
| big workspace with sibling path deps | Desktop fps_counter test remotely | ok, 17.1-37.9 s |
| concurrent builds | two workspaces at once (65) | both ok |
| packaging | release workflow, 5 targets; CI on Linux, macOS x2, Windows, MSRV | all green (first green CI in many commits) |

Still open: wake time is dominated by EC2 start (~45 s) plus install for dev
binaries; one 503 from before the worker fix was never explained; Desktop's
`test --no-run` does not compile at its HEAD, so the fleet publishes only its
`check` dependencies.

## 66. Hosted service at scale: idle hosts cost 93% of machine time, the cron was O(hosts), and the fleet cap raced (2026-10-13)

justrust-cloud cb13088..d13fb5e, deployed as worker c37452da. Details and all
tables: `~/justrust-cloud/docs/scaling.md`, load test results in
`docs/loadtest/` there.

**Utilization.** Production D1: 20.5 machine-hours billed for 1.47 h of
builds (233 runs, 7%). Every on-period ended 36-88 min after its last run:
the client's ssh ControlMaster (ControlPersist, plus the daemon's link) keeps
an established ssh connection, which the host's idle script counts as busy.
The control plane now stops a host after 15 min without a connect, charged
run or run event (and no run in flight). Replaying production history: 19.6 ->
10.8 machine-hours, 14 of 233 builds would wait for a wake (10 min: 9.8 h,
22 wakes; 30 min: 13.5 h, 11 wakes).

**Control plane.** `tools/loadtest.mjs` (real worker code, fake EC2/D1/backend):

| | before | after |
|---|---:|---:|
| cron AWS calls, 200 hosts | 201 | 1 (2 at 2000) |
| warm connect AWS calls | 3 | 1 |
| 200 simultaneous first connects, cap 8 | 200 launched | 8 launched, 192 `build_capacity`, 0 AWS calls |

The first cap attempt (count, then launch) let all 200 through: concurrent
requests all read "7 of 8". A single-statement D1 reservation fixed it
(burst test in `npm test`).

Production warm connect (`POST /v1/build/host/connect`, curl, interleaved by
rolling the worker back and forth): **2.10 s -> 1.23 s median** (n=8 / n=16).

**Capacity.** The EC2 vCPU quota is 256 = 8 concurrent c7i.8xlarge. At the
quota, users used to get `build_unavailable` after a failing RunInstances
each. Now: `BUILD_MAX_HOSTS=8`, the newcomer reclaims the longest-idle host
of another account (5 min idle, no run in flight), launch falls back over 5
zones and c7i.4xlarge, and a full fleet answers `503 build_capacity` with
`retry_after`. The client (9ce2d91) prints its own message for it and keeps
the 10 min route backoff. 202s now carry `retry_after_ms` (10 s while a host
is provisioning, was a 2 s poll for minutes).

**Caching: S3 regional vs CloudFront for the store on hosts, rejected.** Cold
`check -p jcode-tui` on the host, 529 store hits, interleaved n=3: CloudFront
43.0/34.8/34.6 s (median 34.8), S3 regional 37.7/37.5/37.1 s (median 37.5).
Small single objects are faster from S3 (0.06 vs 0.12 s), the build is not.
Host run ids 20261013-043603521-10727 .. 20261013-043912220-57910 (on the host).

Next: raise the vCPU quota before the cap binds; a golden AMI (bootstrap is
about a minute on first launch); pre-warm a new host's depcache from the store
for the account's lockfile.

## 67. Prebuilt fetch overhead measured per machine instead of a fixed 700 ms (2026-10-13)

Follow-up named in FINDINGS 55 and 65. The cost model (`worth_fetching`)
charged every store hit 700 ms of fixed overhead, a laptop over home Wi-Fi.
The hosted c7i.8xlarge in us-east-1 measured 19-20 ms per CloudFront
request (5 samples, TLS 18 ms), the laptop 37 ms warm. So on a host the model
declined nearly every hit, and the agent compensates with
`JUSTRUST_PREBUILT_ALWAYS=1` (65b), which also fetches units that are
cheaper to compile on 32 cores.

Change: every manifest request (hit or 404) feeds an EWMA of request latency
(`prebuilt.latency` in the depcache dir). Fixed cost = 2 requests + 30 ms,
clamped 30 ms to 3 s, and the old 700 ms until measured. Unit test covers a
20 ms host flipping a 180 ms unit to fetch, a 200 MB unit still compiled, and
a slow network pulling it back to the 3 s bound. `justrust test` 322/322.

Fleet: uploaded `prebuild/run.sh` to `s3://.../fleet/run.sh` (approved by
the user; only diff vs live was the telemetry/update opt-out from 5d894ca).
`seeds.toml` and `workspaces.txt` already matched. Store: 7185 manifests.

Cold `check -p jcode-tui` on the host over ssh, fresh target + depcache,
nightly-2026-10-08, jobs 30, localcache off (host run ids below):

| mode | wall (3 runs) | prebuilt hits |
|---|---|---:|
| store off (`JUSTRUST_PREBUILT=off`) | 28.2, 38.3, 37.2 s | 0 |
| old model, 700 ms (before this change) | 36.0, 35.7, 36.3 s | 42-43 |
| measured latency (this change) | 34.8, 33.8, 26.6 s | 138, 220, 231 |
| fetch all (`PREBUILT_ALWAYS`) | 34.0, 33.2, 33.4 s | 529 |

The measured model learns within one build (latency 76 ms on its first
manifests, 27 ms after) and lands between: it fetches 230 of 529, skipping
units that are cheaper to compile. Wall differences are inside the noise of
this host (store-off spans 28-38 s), so **no measured speedup**: the change
makes the default model right on servers without the env override, not a
faster build. The critical path is unchanged: aws_sdk_bedrock (9.8 s, the
store has no matching key: the fleet's build of it is a `link` unit under
jcode-desktop's lockfile, while jcode-tui check needs `metadata`), then
jcode_app_core 5.5 s, jcode_base 4.4 s, jcode_tui 3.8 s. Next: publish
`check` (metadata-only) units for the big leaf deps, starting with
aws-sdk-bedrock.

Host runs: 20261013-051819907-233191 .. 20261013-052104092-284623 (this
change), 20261013-051206447-165622 .. -051428313-209773 (before),
20261013-052214469-294010 .. -052321715-316128 (store off).

## 68. jr-rustc parallel upstream monomorphizations: the targeted pass gets 2.6x faster, the loop does not (2026-10-13)

Item 3 of FINDINGS 56 (rustc serial passes). Self-profile on the Desktop
reference loop showed every collector thread blocked ~285 ms in
`upstream_drop_glue_for` at the start of the mono walk: the first upstream
generic computes `upstream_monomorphizations`, which decodes 623 crates'
exported generics one by one. Change (jr-rustc branch
`jr-par-upstream-mono`, d6f55210de1e, not merged into `justrust`): decode
`exported_generic_symbols` for all crates with `par_for_each_in` before the
serial merge (result order unchanged: the merge still walks crates in order),
and start the query during the lints/dead-code `par_fns` in `analysis`, so
it overlaps work that leaves threads idle. Switch:
`JUSTRUST_RUSTC_PAR_UPSTREAM_MONO=0`.

Factor `jr-par-upstream-mono` (release build of the branch,
`JR_RUSTC_DIR=~/jr-rustc-bf JR_RUSTC_MODE=release`, n=8 interleaved,
`factors-20261012-223235-b687ec7aa473-dirty.json`, load average 13-18 from
other agents, every sample flagged noisy): on 7.42 s (5.8-10.0), off 7.33 s
(6.1-14.2), **-0.09 s, within noise**. Per pass, medians over the 8 runs:

| pass | on | off | delta |
|---|---:|---:|---:|
| monomorphization_collector_graph_walk | 0.25 s | 0.64 s | -0.39 |
| codegen_crate | 0.98 s | 1.50 s | -0.53 |
| incr persist (serialize_dep_graph) | 0.79 s | 1.00 s | -0.21 |
| macro expansion | 1.30 s | 1.12 s | +0.18 |
| misc_checking_3 (lints, where the prefetch now runs) | 0.59 s | 0.41 s | +0.18 |
| unit CPU | 8.87 s | 7.73 s | +1.14 |

The targeted wait is gone (walk 2.6x faster, every on sample under every off
sample), but the decode moved into the lints phase and showed up there, and
the run costs 1.1 s more CPU. Macro expansion, which this does not touch,
moved by the same amount as the gain, so the machine noise is the size of
the effect. Not shipped. Worth a rerun on a quiet machine (ideally the
32-core host, where the decode has more idle threads to use) before
deciding; the branch is pushed.

Process note: the first attempt aborted because another agent reinstalled
justrust mid-run (`factors.py` refuses to mix versions). Pin a copy:
`JUSTRUST=<copy of the binary> bench/factors.py run ...`.

### 67b. Why aws_sdk_bedrock still misses the store

The host's local depcache has 10 keys for aws-sdk-bedrock 1.148.0 (one per
command shape and dependency set). None of them is in the store index. The
fleet built bedrock (`check --workspace` of jcode: 8.9 s, published), but it
builds jcode's `origin/HEAD`, and the laptop checkout that agents build had
diverged (17 local commits ahead, 11 behind, 28 lockfile packages added
upstream). A unit's key includes its dependencies' keys, so any drift in
bedrock's aws-smithy dependency graph gives a new key even at the same
bedrock version. Its rmeta is 20.9 MB, and the rlib bundle 118 MB, which is
over the 95 MB bundle cap, so `build` shapes of bedrock could never be
served anyway. Options: build the fleet from more than one recent commit,
or raise the cap for the few crates on the cold critical path. Not done here.

## 68. Jcode account-router validation: oversized project crates (2026-10-13)

Targeted account-router validation in a clean Jcode snapshot continued to
recompile large project crates before running small test selections:

| Run | Unit | Approximate source size | Measured compile time |
|---|---|---:|---:|
| `20261013-010030888-2352953` | `jcode_base` test | 135k lines | 10.45 s |
| Same run | `jcode_base` library | 135k lines | 5.90 s |
| `20261013-010255285-2372233` | `jcode_tui` test | 225k lines | 22.40 s |
| Same run | `jcode_app_core` | 164k lines | 14.46 s |
| Check run, prefix `20261013-010231444` | `jcode_tui` test | 225k lines | 7.85 s |
| Same check run | `jcode_tui` library | 225k lines | 3.26 s |
| Same check run | `jcode_app_core` | 164k lines | 6.94 s |

The targeted TUI test run compiled a binary containing 2,580 tests to execute
one regression. Test filtering reduced execution, not the compilation unit.
These are observed per-unit compile times, not wall-time sums or a measured
speedup from a proposed fix. Smaller project-owned compilation/test boundaries
are a potential follow-up, but no crate restructuring or justrust behavior
change was made during this task.

### 66b. Idle stop observed in production

After the deploy, the cron stopped the hosted machine at 06:00:05 UTC, 15 min
after its last activity (D1 `last_active_at` age 898 s at 05:59:34, `stopped`
with "User initiated" at 06:00:27). Metering stopped with it and resumed at
07:38:24, when a real build (`check -p jcode-provider-anthropic-runtime
--tests`) woke it: 98 min of machine time not billed. Before this change the
same host ran 36-88 min past every last run.

Since the deploy (05:14 to 08:49): 1.66 machine-hours for 0.25 build-hours
over 38 runs, **15% utilization, was 7%** (that includes this session's own
test runs and the 2-hour gap, so it is an early number).

Also seen: one wake (`justrust remote down` then `justrust check`) took 88 s,
82 s of it sync. The machine came back on a new public IP, and the daemon
re-pushed the client binary (`events.log`: patched/installed on the new
address). Not yet explained beyond that.

## 69. Hosted cache hit rate: what misses, and four fixes (fleet shapes, keep-going, binary patch, git seed) (2026-10-13)

**Measured hit rates.** 217 hosted runs in `~/.justrust/runs` (Oct 11-12):
81% of units fresh in the target dir. The dependency cache hit 2% (64 of
2867) on Oct 11 and 52% (6041 of 11597, 1024 from the prebuilt store) on
Oct 12, after FINDINGS 55/65. The localcache hit 4 of 1370 (expected:
those crates are the ones being edited). Telemetry
(`justrust-cloud/tools/usage.py`, now split by where the build ran) agrees:
remote 47% depcache hit, local 37%.

**Where the misses since FINDINGS 65 come from**, by dependency units
compiled in hosted runs:
1. Profiles and target sets the fleet never built: `build --release` of
   Desktop 838 misses (1539 CPU-s, 20261012-175000702-4156431), `--profile
   selfdev` of jcode 448 (20261012-215923969-1322939) and 623, `--tests`.
   The fleet only built `check` and `test --no-run` in the dev profile.
2. Desktop never published past `check`: the fleet image had no
   `libxkbcommon-x11` (`mold: fatal: library not found: xkbcommon-x11`, so
   link failed), its tests did not compile at HEAD, and cargo stops
   scheduling at the first failed workspace crate.
3. Keyed env leaking on the shared machine (FINDINGS 65d), fixed earlier.

**Fixes (justrust-cloud 9fc101c, 44ac5a5; fleet files uploaded to
`s3://.../fleet/`, two fleet runs launched by hand):**
- `workspaces.txt`: `check --all-targets`, `build --release` and
  `--profile selfdev` (jcode) passes for jcode, jcode-desktop and justrust.
- `run.sh`: `--keep-going` (`--no-fail-fast` for `test`, which rejects
  `--keep-going`: the first run lost every `test --no-run` pass to that),
  so a broken workspace crate no longer stops dependency units; the
  hosted image's GL/xcb/xkbcommon-x11 packages.
- Store: 7185 -> 10108 manifests after the first run
  (`fleet/logs/20261013T054303Z.log`).

Key coverage, fresh depcache and target on the hosted machine, store off so
each shape lists every dependency key it needs, compared with the index
before and after the run:

| shape | dependency keys | in store before | after |
|---|---:|---:|---:|
| jcode `check --profile selfdev -p jcode-tui` | 608 | 0 | 529 |
| jcode `build --release -p jcode-tui` | 609 | 0 | 526 |
| jcode `check -p jcode-tui --all-targets` | 608 | 500 | 529 |
| justrust `build --release` | 75 | 0 | 66 |

Cold `check --profile selfdev -p jcode-tui` on the host (jobs 30,
store-toolchain, fresh target and depcache, localcache and peer off),
interleaved: store off 49.4, 42.0 s (575 misses); store fetch-all 38.4,
37.0 s (529 prebuilt hits, 46 misses). Host runs 20261013-085127257-1748,
-085217323-17947, -085256089-27698, -085338426-39277. Desktop shapes still
fail to compile at its HEAD (a syntax error in `panel.rs:4088` upstream),
but with `--keep-going` the dependencies before the failure get published.

**Sync overhead, the other half of hosted wall.** Sync totalled 2271 s
against 2714 s of server build wall (p50 3.5 s, p90 22.5 s, 66 runs over
5 s). Two causes:
- *Installing dev builds of justrust on the machine*: 77 installs, 1115 s
  (`remote/events.log`), each an 11 MB rsync (3.7 MB compressed; rsync's
  block delta matched only 1.3 of 11 MB of a relinked binary). Now
  (b687ec7) the client keeps a copy of what it installed and sends a
  `zstd --patch-from` (level 9, 0.2 s) against it, sha256-checked on both
  ends, full upload as fallback: 1.8 MB for a typical change, 2.4 KB for
  a rebuild with no code change. Verified on hosted: "patched in 1.1 s",
  machine binary sha equal to the client's.
- *First mirror of a repo with no mirrored relative*: jcode-website 320 s,
  gear-perf 250 s, wt-go 174 s. FINDINGS 54 seeds only from a mirrored
  worktree or clone. Now (e9e56e4) the machine also seeds from the
  repo's public origin: a depth-1 fetch of HEAD (or the default branch),
  then the usual `--checksum` rsync. Anonymous https only; a URL with
  credentials is never sent. On hosted, ruff (11k files, 120 MB): bytes
  over the uplink 15.2 MB -> 0.57 MB (seed 4.3 s on the machine); a fresh
  rust-analyzer clone: "seeded from its git origin in 1.98 s", 0 files
  sent.

Left to another session: the prebuilt cost model's fixed 700 ms
default (FINDINGS 67). A machine with no measured latency declines nearly
every unit, so it never makes the request that would measure it.

Second fleet run (`fleet/logs/20261013T090908Z.log`, with `--no-fail-fast`):
the jcode and justrust `test --no-run` passes now complete, and the store
has 10210 manifests. Every jcode-desktop pass still fails on the upstream
syntax error. Only the dependency units compiled before that error get
published.

### 66c. A new user's first hosted build went local: macOS ssh socket limit (fixed in 0.1.0-alpha.14)

Production 08:47-08:50: a new account on a MacBook launched a host (first
launch to readiness 68 s: started 08:47:55, `JCODE_BUILD_READY` 08:49:03),
then the build fell back to local after 136 s: `local_builds` reason
`cannot open an ssh session ... unix_listener: path
"/Users/<17 chars>/.justrust/remote/hosted/cm-<40 hex>..." too long`.
`control_path` used Linux's 108-byte `sun_path`; macOS has 104, and this
home made a 107-byte path. 0a1e758 uses the per-OS limit (falls back to the
runtime dir), test passes on both macOS CI runners. Any Mac user name of 15+
characters was affected. Shipped in v0.1.0-alpha.14; the update endpoint
offers it to alpha.13 clients.

macOS CI had been red since e9e56e4 for an unrelated reason: the git-origin
seed script called GNU `timeout`, which macOS lacks (440bad5). All five CI
jobs green again.

Reverified 2026-10-13 at 10:16 UTC after the first-user report: both Mac
updater requests with `version=0.1.0-alpha.13` return `update: true` and
alpha.14, and the downloaded arm64 Mac, Intel Mac and Linux release
archives match their published SHA-256 files. A real `justrust check` using
the released alpha.14 binary and an isolated long `JUSTRUST_HOME` succeeded
on the hosted service: run `20261013-031625285-3366289`, remote wall 13.97 s,
D1 `build_runs` state `done`, exit 0. Its SSH options use the shortened
`/run/user/<uid>/justrust-cm/%C` instead of the long home path. This is a
Linux acceptance check of the fallback, not a claim that the customer's
Mac has retried successfully. The exact socket regression passed locally
(1 passed, 0 failed, run `20261013-031639400-3367518`), as did `justrust check`
(run `20261013-031632698-3367056`). All cited run summaries are exported.

The customer's last recorded client was alpha.13, so their next step is
`justrust update` and then `JUSTRUST_REMOTE=force justrust check` in the
project. Auto-update normally checks daily, so publishing a fix does not
immediately replace every installed client. macOS automatic routing still
intentionally stays local because the hosted machines target Linux.

### 66d. Expanded hosted acceptance: installer, updater, native Macs, and full tests

2026-10-13, 10:23-10:29 UTC: exercised the actual published installer from
`jcode.sh/rust.sh` into an isolated directory (login and agent-file edits
disabled): it downloads, verifies, installs and runs alpha.14. A separately
downloaded, checksum-verified alpha.13 release binary ran `justrust update`
against the production release endpoint, replaced itself with alpha.14,
and reported up to date on a second update. No normal user installation or
credentials were modified by these two acceptance paths.

Read the actual native Mac logs from release-commit CI run `38374301919`
(4578234): Intel job `115218706991` and arm64 job `115218707112` both execute
`remote_backend::tests::control_path_fits_the_socket_limit ... ok`, with
308 passed, 0 failed and 2 ignored. Their macOS integration, smoke and
release-execution steps pass too. Release workflow `38374301888` built and
packaged both Mac architectures successfully. These are real native CI
observations, not inferred from Linux compilation.

The fresh installer binary's real hosted `test --lib` exposed a separate
test-isolation defect: run `20261013-032421170-3415301` returned 101 with
307 passed and 1 failed. The agent intentionally defaults
`JUSTRUST_PREBUILT_ALWAYS=1`, but the cost-model regression assumed it was
unset. Reproduced locally with that variable (run
`20261013-032558550-3424443`). Factor the policy input explicitly so the
fixture tests normal costs and the forced-fetch override without mutating
process-global environment. The production wrapper still honors the same
variable and preserves the existing fetch policy.

After the fix, the same public installed binary and hosted `test --lib`
path passes all 308 tests, 2 ignored, in 4.82 s: run
`20261013-032804769-3438655`. Production D1 records both the failing run
(`done`, exit 101) and the passing run (`done`, exit 0), verifying error
propagation as well as success. This is a correctness result, not a speedup
claim. Local `check` (`20261013-032741214-3434033`), the override regression
(`20261013-032742619-3434124`), full `test` (326 passed, 2 ignored,
`20261013-032742724-3434190`) and warning-free all-target `clippy`
(`20261013-032803535-3438523`) pass. Reinstalled with
`cargo install --path . --locked`; all cited build summaries are exported.

At 10:41 UTC, post-repair CI run `38383928546` for source commit
`e41edabfa6ef688b29d44b9c3b70cb983112d657` completed successfully on all
five jobs: Linux, arm64 macOS, Intel macOS, Windows and MSRV. Both native
Mac jobs passed lint, tests, smoke, integration and release execution.
Linux additionally passed package validation. This checks the committed
repair on real platform runners, in addition to the installed client's
live hosted success and failure propagation above.

The customer's latest production run remains the original abandoned
alpha.13 attempt. Their Mac and private workspace are not accessible here,
so applying the update and observing their own retry remains externally
blocked. Do not equate our hosted acceptance or native Mac CI with a
successful customer retry. The audit also identified unchecked oversized
runtime-directory fallbacks as separate hardening work, beyond the
ordinary-home-path incident fixed by alpha.14.

### 66e. Alpha.15 release and automatic update acceptance

2026-10-13, 14:20 UTC: prepared alpha.15 at `90e1c3d`, containing the
hosted prebuilt-policy regression repair in 66d. `cargo fmt --check`,
`justrust check --locked` (`20261013-071958930-461076`), full
`justrust test --locked` (`20261013-072000047-461400`, 326 passed,
0 failed, 2 ignored) and warning-free all-target clippy
(`20261013-072021320-464137`) passed through the real hosted service.
These three run summaries are exported. This is release correctness
validation, not a measured compile speedup.

The published alpha.13 and alpha.14 clients already have the same
automatic updater: release installations check after commands/builds,
in a detached background process, normally at most once per day.
Source installations are notify-only. Explicit opt-outs, CI and remote
agents remain excluded. Publishing a release cannot retroactively change
an old client's daily schedule or turn on a customer's deliberately
disabled setting. Verify the normal background path with the actual old
release binaries against production, not just the manual updater.

Alpha.15 was published at 14:25:06 UTC by release workflow `38412590601`
for `90e1c3d`: all five target builds and archive/checksum pairs succeeded.
Clean-commit CI `38412570868` passed all five native jobs (Linux, Intel
macOS, arm64 macOS, Windows and MSRV), including platform integration,
release execution and Linux package validation.

At 14:26 UTC, checksum-verified actual alpha.13 and alpha.14 Linux release
binaries, in isolated writable installations with default update settings,
each received alpha.15 automatically after a normal `justrust update
status` command. The detached workers fetched the real production
endpoint and assets, replaced the executable and wrote the applied-update
receipt. The next command reported the old-to-alpha.15 update. Neither
`justrust update` nor `__bg` was invoked directly, and no check interval was
shortened. Telemetry was disabled throughout, confirming update delivery
does not depend on telemetry consent.

All five production `/v1/release/latest` target responses offered
alpha.15 with `update: true` to alpha.13; all downloaded archives matched
their published checksums and contained the expected executable. The
production installer installed alpha.15. Public `update off`, environment
opt-out and CI checks left alpha.14 untouched without making update
checks. An actual source-built alpha.14 binary fetched the new version,
notified the user and remained alpha.14. The server follows GitHub latest
with a five-minute edge tag cache; no worker deployment or client-setting
override was necessary.

The fresh public installer binary also passed the real hosted workspace
library suite: `20261013-072720834-540166`, exit 0, 309 passed and 2 ignored
(the saved test-result line verifies a nonzero suite even though this
warm run's compact report omitted the count). Reinstalled the local
source build as alpha.15. Customers still must run an eligible client
online, after its daily check becomes due, with a writable executable
directory. This verifies rollout availability and real old-client update
behavior, not that every customer's machine has already applied it.

### 66f. Final whole-release acceptance and requirement traceability

2026-10-13, 14:34-14:39 UTC: reran the published release's public paths
in fresh isolated homes, then used the newly installed release binary on
a clean public clone of the actual alpha.15 tag (`90e1c3d`), not the
concurrently edited working tree. All checks below passed. Test support
scripts only orchestrate the real clients, installer, GitHub artifacts
and production endpoints; no stubbed service or synthetic source replaces
the end-user path.

| Requirement or boundary | Observed public behavior |
|---|---|
| Publish the complete release | GitHub latest is alpha.15; all five downloaded archive/checksum pairs match and contain the platform executable. Exact-tag CI `38412570868` is green on Linux, Intel Mac, arm64 Mac, Windows and MSRV. |
| Offer the release to eligible clients | Fifteen production target/version combinations (five targets times alpha.13, alpha.14 and alpha.15) return enabled=true, the correct alpha.15 asset URLs, update=true for old versions and update=false for alpha.15. |
| Upgrade prior auto-capable releases | Actual checksum-verified Linux alpha.12, alpha.13 and alpha.14 release binaries automatically become alpha.15 after ordinary public commands and detached background work, without manual update or a shortened interval. Telemetry is off throughout. |
| Respect deliberate exclusions | Public update-off, both environment opt-outs, CI and remote-agent mode retain alpha.14 and do not check. A source-built alpha.14 only notifies and retains its executable. |
| Allow deliberate re-enable | Public update-on on the isolated opted-out client starts the real background path and applies alpha.15. |
| Preserve daily scheduling | An old release with a real not-yet-due persisted check schedule remains alpha.14; the schedule is unchanged and no applied receipt appears. |
| Avoid repeated replacements/notices | The update notice appears once, not on the third command. Updating an already-current alpha.15 says up to date and leaves its executable checksum unchanged. |
| Support symlink installations | Automatic update replaces the real target executable with alpha.15 while preserving the symlink. |
| Handle non-writable installations safely | A real non-writable executable directory retains the old executable byte-for-byte, writes no success receipt and schedules a three-hour retry. Scratch permissions are restored afterwards. |
| Reject invalid endpoint input | A malformed target receives HTTP 400 and invalid_request, not an offered download. |
| Serve fresh installs | The actual public installer downloads, verifies and installs alpha.15 in a new isolated destination. |
| Hosted check on the actual release project | `20261013-073555460-622843`: published client, pristine release-tag project, real authenticated hosted service, exit 0. |
| Full hosted test/integration suite | `20261013-073607837-624831`: 326 passed, 0 failed, 2 ignored, exit 0, including the real project's CLI integration tests. |
| Hosted all-target lint | `20261013-073627702-627878`: warning-free, exit 0. All three final clean-tag run summaries are exported. |

Eligibility is a real limitation, not a server setting we can override:
the updater first shipped in alpha.12 (commit `5445c16` is contained in
alpha.12 and later tags, not earlier tags). Alpha.11 and older need the
installer rerun. This is now explicit in the public release notes.
Offline/inactive clients, deliberately opted-out/source-built clients,
non-writable installations and not-yet-due daily checks cannot be claimed
updated. We verified availability across all five targets and real Linux
client application, plus the exact release commit's native Mac/Windows
CI, not every customer's device. No additional runtime change or speedup
claim was needed.

### 66g. Alpha.16: macOS clients install the Linux hosted agent from GitHub

The published Mac client's first connection called `cargo install justrust
--version <client version>` on the Linux host. The crates.io sparse index has
no `justrust` package (HTTP 404), so publishing GitHub assets alone did not
make that path work. Alpha.16 release clients select the Linux host's x86_64
or arm64 asset from the exact same release, verify its archive checksum and
executable version, and replace the agent atomically. Source builds retain
the previous fallback. The exact generated script has executable regression
coverage for success, corrupt checksums and incorrect executable versions.

Hosted validation passed: check `20261013-074043475-677824`, full tests
`20261013-074113970-684828` (329 passed, 0 failed, 2 ignored), and warning-free
all-target clippy `20261013-074132836-689618`. `show` confirms remote on hosted,
and these summaries are exported. A fresh validation session resolved an
initial shared-session ENOENT infrastructure failure without interrupting
other agents. No compile speedup is claimed. See `docs/release-alpha16.md`
for publication and live-service verification, including the boundary that
the customer's own Mac retry is not observable here. Linux hosted builds
still require Linux-compatible projects, not macOS-specific native targets.

Alpha.16 was published at 15:06:34 UTC. All five release builds and all
exact-tag native CI jobs passed, including Intel and Apple Silicon Mac
integration paths. Both live Mac updater targets offered alpha.16 by
15:09 UTC. The public installer and the exact cross-platform template
installed the checksum-verified published Linux binary successfully, the
latter in an isolated HOME on the live account host.

The public release binary then checked an untouched tag archive on hosted:
`20261013-081121855-923025` (12.8s client wall), full 330-test run
`20261013-081134607-924206` (19.0s, 0 failed, 2 ignored), and warning-free
clippy `20261013-081153578-926111` (7.7s). All three summaries are exported.
The fresh checkout hit 66 dependency-cache artifacts during check and 47
during tests, but rebuilding the 47k-line local crate still took 5.3s for
check and about 12.3s in the test build. This records the remaining crate-size
cost rather than claiming a speedup from release correctness changes.

## 2026-10-13: website Trunk builds need the wasm target in the selected compiler

While restyling the justrust product page, normal `trunk build` failed with
E0463 because justrust's selected compiler did not have
`wasm32-unknown-unknown` installed. Turning remote routing off alone did not
help: local default `jr-ac20de5e7921` lacked that target as well. Failed
runs: `20261013-072440930-513193` and `20261013-072656708-535682`.
The installed system Rust 1.98.1 has the wasm standard library. The explicit,
command-scoped workaround `JUSTRUST_REMOTE=off JUSTRUST_TOOLCHAIN=system
trunk build --dist .trunk/hierarchy-preview` passed in 44.11s, without
changing global routing, cache isolation, or compiler pins.

Measured constituent runs: `20261013-072749747-544294` (solar, 32.42s,
31.89s compiling, 11 dependency rebuilds, web_sys 19.1s rustc wall) and
`20261013-072749747-544295` (website, 40.12s, 32.36s waiting for the shared
Cargo build-directory lock, 7.59s compiling). Trunk launches both binaries
against the same target directory, so the second build serialized. No
speedup is claimed. Follow-up: validate requested target support before
selecting a compiler, and investigate target-aware provisioning plus reuse
of Trunk's two-binary build. The unrelated hosted native check and gallery
test passed (`20261013-072347781-500391`, `20261013-072829320-550561`).

## 2026-10-13: hosted dogfooding, native cargo and upgrade-safe agents

The 10x stock-local target is not achieved. Details and pinned results are in
`docs/hosted-speed-20261013.md`. The earlier apparent 54x cold result reused
a warm remote target, not a truly cold hosted compiler.

`623b85e` lazily resyncs requested roots after reconnect and adds phase trace
diagnostics. `3687ab6` pins a reusable executable snapshot, so atomic live
agent replacement cannot break later builds or their recording shims. A real
replacement subprocess runs recorded, raw, recorded offline checks, with zero
units compiled on the final check. All 14 protocol tests passed on hosted run
`20261013-075429794-820147`; check `20261013-075149665-800573` and warning-free
clippy `20261013-075431281-820282` passed. Alpha.16 release owner installed it
and verified full hosted tests (330 passed). No reconnect speedup is claimed.

Cloud `4cd6d38`, deployed worker `beba6b93-5ae1-4cb8-970e-0308c571fb8e`, overlaps
identity lookup with authoritative charging while preserving billing-before-
execution and queued telemetry ordering. All 62 Node and six Python tests passed.
No billing entitlement, tenant isolation or shared-cache publishing rules changed.

Native Ubuntu jr-cargo (vendored libgit2) was built through hosted run
`20261013-074654609-754326` and installed only on our account host. The first
Arch binary could not load libgit2 and was immediately disabled. Both native
and stock report the exact pinned release/channel and base commit. Fixed-client
seven-sample `bench/suite.py compare` reports:

```text
desktop-noop-test  1.35 -> 1.00 s  -26%  within noise
desktop-body-check 2.69 -> 2.54 s   -6%  within noise
```

These are observed medians, not a conclusive end-to-end speedup. Host cargo
startup medians moved 0.34 -> 0.21 s and 0.37 -> 0.23 s. No-op compiled zero
units and body-check compiled one local unit in both measured groups, so
activation did not invalidate dependencies. Real hosted check, library tests
and warning-free all-target lint passed afterwards: `20261013-080759334-903256`,
`20261013-080802295-903631`, `20261013-080803980-903707`. Native fork checksum:
`ab748784c57fc2a3da6f17272be48cfbd7a432f02ee8a864124b73bcedab8531`.

The new stage2 PGO/BOLT compiler finished in 39m52s, then passed interface
hash/link/load checks, test-filter checks and 40 deterministic stress rounds.
These custom compiler harnesses require the local unpromoted compiler, not
normal-build routing off. It has not been installed or promoted to hosted.

Correction to finding 67: the 95MiB bundle cap applies to gzip bytes, and the
unpack limit is380MiB. An 118MiB raw bedrock output alone does not prove it
cannot be served. Matching exact dependency/toolchain keys remains necessary.

## Hosted optimized compiler candidate, 2026-10-13

The PGO/BOLT stage2 b7907f1fe8f7 compiler is provisioned only on our account
host, explicitly pinned in a staged namespace. Its marker has installed_at0,
no active-root entry exists, Ubuntu rustc/clippy and native cargo load, and
older running agents cannot default to the candidate. Full hosted client
tests332 pass at20261013-085654660-1178588, check
20261013-085629718-1176060 and all-target warning-free clippy
20261013-085719414-1181114. Safe staging full-suite acceptance is
20261013-083002649-1025849. No default compiler promotion.

Same frozen client, pinned source snapshots, same native hosted cargo,
five measured edits after one discarded warmup: stock nightly reference
7.777s versus optimized JR6.545s (1.19x). `bench/suite.py compare` reports
desktop-test-edit -16%, faster; desktop-body-check2.700->3.099s +15%,
within noise. Compile phases6.349->5.332s and1.566->1.685s respectively.
All three fps_counter tests actually pass in reference logs. Results:
bench/results/hosted-jr-reference-before-20261013.json and
bench/results/hosted-jr-reference-after-20261013.json. This is hosted versus
hosted, not a new local-versus-hosted10x claim. Target remains unmet.

Local-only unpromoted compiler validation passed18 interface/link/load
checks,12 test-filter checks and40 deterministic interface stress cases.
Live ABI verification and actual887-unit Desktop warmup also passed. The
benchmark excludes this cold warmup, never conflates cold build and reuse.


## Hosted follow-up measurements and truthful reporting, 2026-10-13

Exact commands, controlled comparisons and limitations are documented in
`docs/hosted-speed-20261013.md`. The ten follow-up benchmark files record
explicit opt-ins, deployed admission changes, restored parallel tests,
detached-worker CPU reporting and frontend threading. All ordinary
checks/tests/lints used hosted. The 10x stock-local target remains unmet.

Backend3ff0c61 passed435 tests and deployed version
2cccc5ee-149b-4666-84f0-8b8aa2855321. No billing/identity/quota changes.
Eleven pinned no-op samples1.288->1.240s are within noise. Twenty-one
idempotent admissions0.640->0.370s are not new-charge or compiler evidence,
and after-tail2.783s prevents a clean latency claim. Both health endpoints200.

Native Cargo pool eligibility and report repairs5c30aa5/4cf336e/198209d
passed336 hosted tests. Real full-suite run20261013-092457071-1354392
passed336 across7 binaries,0 compiled56fresh. Pinned five-sample test edit
passes157 cases every time:3.324->2.971s, suite compare -11%, faster.
No-op1.096->0.938s remains within noise. Filters and daemon remain opt-in:
all7-sample comparisons are within noise and actual2/1/3/2 test transitions
pass. The initial report-only change did not restore pooling by itself.

Old daemon run20261013-091246820-1272766255 underreported CPU0.027s
versus6.81s unit and8.03s observed process CPU. b70dbbc uses their maximum,
not overlapping sum, and labels sampled lower bounds.337 hosted tests/lint
pass, actual20261013-093920906-1440678992 reports8.44s sampled CPU.
20261013-100133452-1575817 preserves0 compiled56fresh. Failed intermediate
fixture expectations/compilation are retained, not hidden from evidence.

Five-sample explicit1vs8 threads compare7.201->6.819s reference and
2.962->2.840s body-check, both within noise, so defaults remain unchanged.
Reference frontend2.345->2.311s barely changes. Timing run
20261013-094425981-1461275918 identifies mono graph walk0.840s, macro
expansion0.774s, dep-graph serialization0.717s, symbol partition0.613s and
resolution0.505s. Nested spans must not be added together.


## Guarded sync/run pipeline and measured latency, 2026-10-13

5a211c5 is committed/pushed and installed. Protocolv5 combines source
barrier and build submission on the ordered SSH stream. The agent refuses
all failed-write runs, acknowledges before launching, and retains v1-v4
support. The client never falls back locally after ambiguous submission,
but can do so after a confirmed no-start rejection. Pending cancellation
is latched before PID publication. No billing, quota or identity changes.

Actual subprocess regression rejects failed writes twice without creating
a target directory, repairs them, compiles latest source, detects a real
type error, repairs it, then compiles zero units with fresh artifacts. Twelve immediately
cancelled jobs terminate without leaked sleep processes. Full343 hosted
tests and all-target warning-free lint pass at
20261013-102356238-1713803 and20261013-102404406-1714588. Installed
actual17 protocol tests pass at20261013-102608318-1727805,0compiled56fresh.
First reconnect/upload14.6s is not a warm performance result.

Same frozen source snapshots/account/compiler, seven measured samples
after discarded warmups: bench/suite.py compare reports noop0.962->0.816s
-15% within noise and reference6.586->6.143s -7% within noise. Both exact
result files are hosted-sync-pipeline-{before,after}-20261013.json. No
conclusive end-to-end speedup is claimed. The10x stock-local goal is unmet.

Transport-only curl repeats of an already charged run observed21-sample
fresh0.357s, reused0.197s, fresh-after0.290s. TLS costs0.066-0.082s and
reused samples create0new connections. This is neither a new-charge nor
a compiler benchmark. Artifacthosted-http-transport-repeat-20261013.json
contains only timings and the already-public run ID, never credentials.
Authoritative billing must remain online before every actual build.

## 2026-10-13: native hosted admission pool rejected by measurement

Frozen6c0988c, explicit JR compiler, pinned Desktop/Jcode,11samples per side
after2warmups: noop0.91->1.52s (+67%, slower), body2.56->3.19s (+25%,
slower), reference6.22->6.74s (within noise), per bench/suite.py compare.
Every pool sample verified actual native admission and hosted execution.
Files hosted-http-pool-before/after-20261013.json and curl-recheck are
committed. The correct350-test prototype was removed in ee5699d rather
than carrying an unhelpful dependency stack. Socket0600 stays. Initial
policy-test failure20261013-104430840-1840198 is retained alongside repair
and acceptance.343 hosted tests111516986-2032086 and warning-free lint
111522449-2032855 pass; installed acceptance111621112-2040123 is remote
on hosted with56fresh and zero compiled. No10x or pool speedup claim.

## 2026-10-13: measured faster CPU, original host recovered

Same frozen ee5699d client and explicit JR compiler, seven samples after two
warmups on same preserved account volume: c7i.8xlarge -> m8azn.6xlarge
reference 6.519 -> 4.480 s (-31%, faster), body 2.602 -> 1.986 s (-24%,
faster), noop 0.998 -> 0.749 s (within noise), per bench/suite.py compare.
Actual remote hardware saved separately, benchmark env.cpu is the client.
Candidate 343 tests 20261013-112901865-2119204 and all-target lint
20261013-112905082-2119538 passed. Original type, volume and cost metadata
restored; hosted recovery 20261013-113350378-2148872 passed 56 fresh units,
zero compiled. Hourly machine estimate 73% higher, customer build pricing
unchanged. 10x still unmet. Default-nightly promotion gate follows separately.

## 2026-10-13: faster own dogfood host retained, hosted shortcut fixed

Default nightly, same frozen client and pinned snapshots, seven samples:
reference 7.235 -> 5.208 s (-28%, faster), body 2.428 -> 1.886 s (-22%,
faster), noop within noise. Actual authenticated worker stop/start preserved
M8azn type and volume. Own host only, hourly estimate 73% higher, no fleet,
compiler, customer pricing, authorization or quota change. 10x remains unmet.
08a77e8 fixes local prebuilds bypassing hosted defaults. Do not count the
exported -pre runs as hosted. Full hosted 345 tests
20261013-115014431-2248699 and warning-free lint115017412-2248920 pass.
Installed daemon-enabled defaults115223558-2262223 and115229660-2263025
are actually remote on hosted, 56 fresh units, zero compiled.
JR/M8 phase diagnostic115627127-2282562598: frontend1.539s, codegen0.890s,
incremental0.560s, link0.177s. Profiling warmup115543316-2282562904 reported
887 compiled units, so cache-safe help claim removed rather than assumed.

## 2026-10-13: verified WNAM control-plane placement recovery

- Actual subscription API placement was remote-VIE from SJC despite WNAM D1.
  Old-version rollback stayed slow. Initial billing-batch comparison is
  confounded, not an isolated batch speedup.
- Same source/client, pinned seven samples: noop 1.37->0.68s (-50%, faster),
  reference 5.83->5.16s (-12%, faster). Body 2.46->1.86s is within noise.
  Official suite compare and all trial/rollback/placement files are preserved.
- Backend dde1cef deployed b0d2a727, actual local-SJC under explicit US-west.
  439 backend tests pass. Auth/quotas/pricing unchanged, fallback cancelled
  after genuine hosted acceptance. No fleet/compiler-default change.
- ETXTBSY setup flake fixed in 4292737. Actual 19 protocol tests, 347 hosted
  tests and lint pass cleanly. Exact check 20261013-122843725-2485537,
  protocol 20261013-122844340-2485613, full 20261013-122845620-2485666 and
  lint 20261013-122848384-2476255 exported. 10x unmet, own host cost 73% higher.

## 2026-10-13: current stock-local target check

- Seven round-robin samples, identical pinned edits, same selected tests.
  Current stock rustc 1.98.1 vs hosted default: reference6.846->5.198s,
  1.32x faster; body1.837->1.904s within noise; noop0.462->0.742s slower.
  Three-workload geometric mean0.92x.10x target is not reached.
- All21 measured remote samples have actual hosted markers and exit0.
  Raw versus-20261013-123317-08a77e8-dirty.json and exact exported records
  preserved. Remote CPU/RSS are client-only, not server resource comparisons.

### Hosted child file-creation fidelity (2026-10-13)

- `7096694` preserves caller umask per run child. Real host SSH `0002` versus caller `0022` caused seven unchanged package-security fixtures to reject group-writable directories. Guards and pinned source were not weakened.
- 21 real protocol/subprocess tests cover concurrent0077/0022/legacy children, invalid metadata rejection, metadata stripping, atomic replacement and recorded/raw/recorded artifact reuse (`20261013-125945235-2676755`). Full351 and warning-free all-target lint passed before and after install.
- Installed defaults genuinely hosted: check `20261013-130405588-2703785`, full351tests `20261013-130420296-2705417`, lint `20261013-130423685-2705593` all0compiled56fresh. First connection sync14.16s is disclosed, not a latency win.
- All16 unchanged package/security tests pass `20261013-130424881-2705665`. Full UI changes1295/8/10 to1302/1/10. Remaining viewport assertion fails isolated default and optimizedJR; neither full suite is clean, default compiler unchanged.
- Exact14 acceptance/failure records and details: `docs/hosted-umask-20261013.md`. This is an execution-correctness fix, not a measured speedup or10x claim.

### Current-M8 optimized compiler gate (2026-10-13)

- Same frozen7096694 client, two warmups/seven samples, same M8 host and explicit USwest backend: `bench/suite.py compare` reports reference5.803->5.012s(-14%,faster), body2.437->2.465s(noise), noop1.309->1.487s(noise). Reference compile4.402->3.654s, body compile1.052->1.134s regresses.
- All42 exact measured records report hosted, zero failures and equal3 filtered reference/noop tests. Both raw benchmark files and credential-free network headers/timings are committed. Current ingress KIX/SYD with backend remote-SJC is not the earlier SJC epoch, no cross-epoch compiler-speed claim.
- Default remainsnightly; optimizedJR remains staged opt-in. Full unchanged UI has same1302/1/10 on both and isolated default viewport failure, so neither correctness gate is clean. Details `docs/hosted-compiler-20261013.md`, correctness records `docs/hosted-umask-20261013.md`.10x remains unmet.

## 2026-10-13: six versus eight workspace frontend threads rejected

Same frozen7096694 client, own M8azn host and JR compiler: seven samples per scenario with actual workspace rustc flags checked. Eight-thread recheck versus six: reference5.008->4.968s(-1%) and body2.419->2.581s(+7%), both within noise by `bench/suite.py compare`. Compile medians3.658->3.638s and1.128->1.127s are effectively unchanged. Do not promote a six-thread default.
Both real type-error probes failed as expected and exposed first diagnostics (six1.441s, eight1.492s), not a statistical speed comparison. Actual eight-thread state restored. All30 exact measured hosted records and four raw result files are retained, starting `20261013-131510004-2764782945`; see `docs/hosted-workspace-threads-20261013.md`. The10x target remains unmet.

## 2026-10-13: cloud-worker colocation trial rejected and exact live version restored

Cloud e857502 added explicit USwest placement only,68Node+6Python tests passed. Read-only primary probeLAX/WNAM and immutable binding/runtime equality verified. Same installed7096694 binary hashes equal: seven matched samples original->candidate noop0.673->0.731s(+9%),body1.859->1.910s(+3%),reference5.202->5.260s(+1%), all withinnoise by suitecompare. Actual ingressSJC bothepochs, not the earlierKIX/SYD.
Candidatefa372f4c rejected; exactpriora81f280e restoredhealthy and conditional15minrollback canceled after actualhostedcheck133228254-2861734. Full own351hostedtests/lint passed,0compiled56fresh. Recovery7noop samples0.981s are not a restored-latencyclaim. All49measured+4acceptance exactrecords and raw3results/controlmetadata published, see `docs/hosted-cloud-placement-20261013.md`. No host/compiler/billing/auth changes and10x unmet.

### Bounded hosted provisional first errors, 2026-10-13

- Public57440a9/ba5fc2d/7d4474b and private321324f shipped an opt-in, query-only, resource-bounded hosted analyzer. Same frozen7d4474b SHA, pinned Desktop snapshot and7samples per setting: first visible E0308 median2.192->1.369s,37.5% earlier with nonoverlapping ranges by `bench/suite.py compare`. Full Cargo median2.430->2.400s remains within noise. All7provisional errors confirmed by full rustc/exit101, zero early-only. Not a10x compile win or stock-local comparison.
- Actual startup exposed task-cap EAGAIN (peak256/max232), fixed by service-local Cargo jobs2 without weakening the cap. Eager priming remained unready at227s/6.05GB peak, so it was stopped. Host-only demand loading/four analysis threads and offline host-platform source discovery reached healthy quiescence2.219s with45roots780files, after earlier dependency startup attempts. Local analyzer defaults unchanged.
- Kernel6GiB/no-swap/256tasks/30min/max2service caps verified. Final peak5.48GB and37,120soft MemoryHigh events, zero OOM/hard-task failures. Do not enable by default. Exact analyzer service stopped, no live analyzer children,3defunct entries observed, no private caches deleted. All ordinary admission, identity, quotas and complete Cargo authority preserved.
- Installed default check20261013-140412004-3056545/full355tests20261013-140415130-3056940/lint20261013-140418660-3057272 all truly hosted,0compiled56fresh. Restored passing UI check and3FPS tests pass with query-only mode and no stale error. Full raw comparisons, resource/control evidence and56exact owned exported runs: `docs/hosted-early-20261013.md`.10x target remains unmet.

## Hosted admission phase accounting and rejected SSH HTTPS trial (2026-10-13)

- Nine real CDG admissions took1.077s median HTTP: DNS18ms/TCP162ms/TLS171ms/response733ms. Response wait is not pure billing SQL. First60.314s discarded build is actual idle-host wake and sync, retained with its run id.
- Same frozen7d4474b seven-sample direct/tunnel/direct recheck: noop1.20/1.27/1.14s, body2.54/2.17/2.19s, edited tests5.94/5.58/5.24s. Every suite compare verdict within noise. Corrected27forwardedcharges are actual ATL/HTTP200/hostedexit0; no tunnel default or client feature shipped. Selector-miss and Unix-path assertion mistakes disclosed.
- Opt-in fixed Server-Timing observer e75e03e deployed b5757617 with byte-equal bindings,71Node+11Python tests and actual plain hosted355tests/check/lint passing. Nine live calls show backend charge271ms, parallel identity67ms with0additionalwait, queued97ms, worker total365ms. No auth/quota/TTL/placement/order changes.
- Raw frozen comparisons, all normal charge timings and138exact owned exported runs retained in docs/hosted-admission-20261013.md and bench/results/hosted-admission-control-20261013.json. No cross-ingress speed attribution, no10xclaim.

### Native hosted admission trace acceptance (2026-10-13)

- Client e6a1b5e committed, pushed and installed: JUSTRUST_REMOTE_TRACE=1 requests only existing admission metrics, fixed numeric names and safe status/body framing. Exact default curl argv/key-on-stdin/TLS unchanged, no extra charge or retry. Five regressions include nine actual curl200/402/503 cases,360 hosted tests pass0fail2ignored and warning-free all-target lint.
- Installed trace check20261013-145150385-3352742 reports identity54/charge225/queue67/total292ms,0residualidentitywait;8.160s sync is first new-client daemon reconnect, not compiler time. Plain subsequent check/test/lint all0compiled56fresh. Actual Desktop one-line source acceptance20261013-145211504-3355155 passes3FPS tests and restores exact backup, but12local crates also rebuilt for unrelated user changes:44.5s is not a controlled speed measurement.
- All11exact owned validation records exported, including initial and final hosted gates. Python harness import-root and unrelated peer whitespace staging guards corrected without changing peer files. See docs/hosted-admission-client-20261013.md.10x remains unmet.



## 2026-10-13: HTTP version control does not explain rejected native-pool regression

- Normally billed HTTP2/HTTP1/HTTP2 pinned no-op medians 0.959/0.832/0.841 s, seven measured plus two warmups each. Both suite comparisons within noise. All 27 actual negotiated protocols/status200/hosted exit0/three FPS passes verified; first epoch PDX-only versus later PDX/SJC confounds first pair. Keep curl defaults, no speed claim or billing/instance change.
- Exact ureq3.4.2 already defaults TCP_NODELAY=true. Missing no-delay is not a supported root cause. This curl protocol control does not prove a replacement native library is fixed. Details and sanitized controls: `docs/hosted-http-protocol-20261013.md`.
- Every tracked pinned archive file matched declared fixups (754 Desktop,2110 Jcode), zero mismatches. Pinned 744-fresh/1310-filtered versus live acceptance734/1299 are different workloads, not demonstrated contamination. Installed native trace/default-hosted check `20261013-150525491-3437785` exit0/zero compiled/56 fresh, first reconnect10.2s includes9.530s sync. All 28 exact owned records exported.

## 2026-10-13: Preserve real client-side timing rather than searching remote-only output

- `5bad625` retains bounded optional client milestones and overlapping authoritative admission phases in suite JSON after the timed command. No added HTTP calls, automatic tracing, raw headers, credentials or cache flags. All24 harness tests and real default-hosted two-sample acceptance pass.
- Actual samples `20261013-152106006-3541783284` and `20261013-152106769-3541783225` each zero compiled/744 fresh/three FPS passes. Client charged410.3/364.9ms, server total287/263ms with identity overlapping. This is instrumentation acceptance, not a speed comparison.
- Earlier native/curl/native diagnostic medians.810/1.072/.885s have overlapping three-sample ranges. Its billed builds succeeded, but a read-only verifier wrongly searched remote Cargo output for local-client trace lines. Corrected without rebilling. Default-hosted restoration `20261013-151845550-3526469` passed, cold sync8.763s disclosed. All19 exact diagnostic/warmup/acceptance records exported. Details: `docs/hosted-sample-trace-20261013.md`.

## 2026-10-13: Larger current native-pool recheck remains inconclusive

- Same frozen6c binary/native-curl-native, 11samples each workload: noop.691/.778/.709s, body1.855/1.990/1.839s, reference5.185/5.280/5.221s. Both `bench/suite.py compare` reports classify every row within noise. All99 measured actual hosted exit0, transport provenance and6 client milestones verified. No pool library or transport default restored.
- Peer disclosed possible same-host load22:28:33-22:30:58UTC, covering11 curl reference samples. Controls explicitly flag those records rather than falsely changing original local-noise flags. Peer confirms no live deployment/host type/region/quota/binding changes during the experiment. No causal transport speed claim.
- All121 exact own measured/warmup/recovery/plain-acceptance records preserved. First native warmups18.037/17.057s sync and final daemon restoration10.462s sync disclosed. Final installed plain check `20261013-153339304-3627414`, full360 tests `20261013-153339994-3627461` and strict all-target clippy `20261013-153342838-3627413` all hosted,0compiled56fresh. Details: `docs/hosted-native-current-20261013.md`.10x still unmet.



## 2026-10-13: build hosts moved to us-west-2; same-type warm loop within noise

Launch region us-east-1 -> us-west-2 (justrust-cloud fa581a6, b01662a; worker
fe38bf38, rollback b5757617). Recorded hosts stay managed in their own
region, so the other account's stopped east c7i and its disk are untouched.
Own host: fresh us-west-2a i-0e052af0d41ab457e resized to m8azn.6xlarge, so
type is matched. Old east m8azn i-0a3369f2adbb5d2cf stopped, volume kept,
retagged `build-retired-east` pending deletion.

RTT from the laptop (TCP connect, medians): EC2 endpoint us-east-1 ~146 ms,
us-west-2 ~56 ms, west build host port 22 ~31 ms (min 28).

Same frozen client (sha e2bc9269), nightly-2026-10-08, 2 warmups, 7 samples:
`bench/suite.py compare hosted-region-before-east-20261013.json
hosted-region-after-west-20261013.json`: noop 0.72 -> 0.81 (+11%), body
1.94 -> 1.93 (-1%), reference 5.33 -> 5.37 (+1%), all within noise. The
east baseline overlapped a peer's hosted runs on the same host
(22:28:33-22:30:58 UTC). So the RTT drop (~70-110 ms) does not show
in warm loop wall time: the warm path already holds a persistent SSH
connection and very few round trips per build. No speed claim. The move
pays off for wake/connect and per-request admission, and puts us next to
the regions that have every candidate type (m8azn, c8a, m8a, metal).
us-west-1 is closer (~36 ms) but has no m8azn/c8a.

West acceptance (installed client, hosted): check 20261013-155617354-3849552,
full tests 20261013-155805477-3862970, strict all-target clippy
20261013-155821546-3865896, all exit 0.

### Independent west dogfood acceptance, 2026-10-13 23:01 UTC

- Installed plain default-hosted check `20261013-160058612-3890266`, full tests `20261013-160059348-3890346` (360 passed, 0 failed, 2 ignored), and strict all-target lint `20261013-160102069-3890192` all exit 0, truly hosted, 24 cores, 0 compiled / 56 fresh. Exact three own records exported.
- Regional owner confirmed accepted west M8azn and live counted-reclamation guard. Independent actual-source regional/outage checks passed 11 cases after reproducing a compound AWS + failed database rollback cap leak. Details and attribution: `docs/hosted-west-acceptance-20261013.md`.
- Acceptance completed at 23:01:02 UTC; the owner subsequently announced an exclusive trial window starting at 23:01 UTC. No subsequent host calls or speed claim. Matched warm east/west results remain within noise, and 10x stock-local remains unmet.

## 2026-10-13: c8a.8xlarge vs m8azn.6xlarge (us-west-2, own host, same volume)

Reversible resize of i-0e052af0d41ab457e, same frozen client, nightly-2026-10-08.
Defaults (2+7): `compare hosted-region-after-west hosted-chip-c8a-west`:
noop 0.81 -> 0.83, body 1.93 -> 2.04 (+6%), reference 5.37 -> 5.60 (+4%),
all within noise but every row slower on c8a. Workspace-threads sweep on the
reference loop (2+5, medians):

| threads | c8a.8xlarge (32c, 4.5 GHz, $1.72/h) | m8azn.6xlarge (24c, 5.0 GHz, $2.48/h) |
|---|---|---|
| 1 | 5.97 | 5.63 |
| 8 | 5.57 | 5.35 |
| 16 | 5.64 | 5.34 |

c8a is ~4-6% slower at every thread count; 8 -> 16 threads buys nothing on
either chip, so c8a's extra 8 cores are unused by today's warm loop. The
gap is about the clock ratio (5.0/4.5), as expected for a serial-bound loop.
Per dollar c8a is ~30% cheaper for ~5% more latency. Own host restored to
m8azn. Fleet default (c7i.8xlarge) unchanged. Revisit c8a once the toolchain
uses >8 threads productively.

Desktop unblocked (jcode-desktop 8d2bca1): `origin/main` had misplaced
lines from two commits (`panel.rs` needs_surface_update /
set_surface_focused, `workspace.rs` todo-progress update and its sidebar
test), test initializers missing jcode-sdk's new `pending_soft_interrupts`,
a stale uppercase assertion, and a lockfile that `--locked` rejected
against jcode HEAD. After the fix: `check --workspace --all-targets
--locked` passes, 1778 workspace tests pass. Fleet run
`fleet/logs/20261013T231926Z.log`: all 21 passes ok, including all five
jcode-desktop shapes. Store: 10513 manifests.

### Cumulative acceptance closure: hosted compiler stack faster, 10x still unmet (October 13, 2026)

- Re-ran every mapped requirement through actual installed hosted CLI or the relevant existing component/integration suite. Final plain check `20261013-164407681-4115213`, full tests `20261013-164409873-4115463` (360 pass, 0 fail, 2 ignored), strict lint `20261013-164412791-4115540`, repeated check `20261013-164413643-4115676` all exit0, nightly selected, 0 compiled/56 fresh. `justrust show` confirms remote on hosted.
- New matched, normally admitted seven-sample compiler comparison on unchanged own west m8azn: reference5.32->4.56s, public `bench/suite.py compare` reports -14% faster. Body/no-op within noise. Both use identical official Cargo, but rustc revisions/optimization stack differ. No isolated PGO/BOLT attribution, default promotion or10x claim. Raw before/after and exact run exports are committed.
- Full real pinned Desktop security16 pass on both compilers; full1313 remain1302pass/1same workspace hint-chip failure/10ignored, exit101. Actual filtered3-test harness `20261013-164335712-4110778849` passes and next full run `20261013-164342509-4110778907` correctly rebuilds/restores1313 tests. Full failures were visible and preserved. This is parity, not a clean UI acceptance gate.
- Actual acceptance uncovered missing staged JR on fresh west volume and Arch jr-cargo libgit2 ABI failure. Repaired nondefault stage with hash/loader-verified rustc and portable official Cargo, quarantined incompatible Cargo, then actual JR check/360tests/lint and restored nightly passed. Native fork/bootstrap/test-pool follow-up belongs to host owner, not silently claimed present.
- Fresh stock-local versus hosted round-robin attempted, all42 samples noisy. Peer confirmed local Cargo compile23:38:02 to23:39:53 UTC,769 CPU seconds. Reject raw printed2.05x causal speed claim; preserve raw results and confound controls. Historical stable1.32x is not replaced. Host lease released23:44UTC before owner's separate96-core trial. All18 requirements/results and remaining limitations are in `docs/cumulative-acceptance-20261013.md`.

### Stock benchmark reporting now refuses contended wall-speed claims (October13,2026)

- Real retained42-row stock control had allsamplesnoisy but the old public report printed bold2.05xref and1.26xgeomean. Four scoreboard regressions failed before the fix; a separate cross-resultcompare regression failed too. Existingclean-sample output remains unchanged.
- `bench/versus.py` now marks wall comparisons contended or unverified when sample metadata cannot establish quiet execution. Raw wall medians and instruction measurements remain visible; geomean never silently drops excludedworkloads. Both actual report and compare CLIs were exercised on the original control, all38bench tests pass. Original JSON hashbd8040a8a5abc2ead6ee2d0894b7fa37bbd02408ed04cf75f507071497490b38 unchanged, existingVERSUS.md restored from backup. This is reporting correctness, not a compile speedup.
- No Rust binary, live service, billing, pricing or compiler default changed. Host remains with owner for96-core scaling. Measured candidate run `20261013-164335712-4110778849` and prior cumulative14%reference result remain separate. Next experiments in `docs/cumulative-next-experiments-20261013.md` require real quiet stock control, same-epoch scaling and normal admission. Deferredqueued bookkeeping was rejected because it participates in active-host reclaim protection.

## Whole-result recheck after96-core host epoch, October14 UTC

- All18H/T plus8next-phase requirements now have individual observed outcomes or explicit blocking boundaries, not aggregate-only completion. Fresh actual public report/compare/audit,38Python cases,18interface12filter40stress,cloud78+11 and authoritative billing439+13targeted cases pass. The primary backend feature checkout372pass was excluded because it lacks hosted billing; peer workspace was not switched.
- Actual prior plain hosted test `20261013-164409873-4115463` passed360/0/2 with0compiled56fresh. Named umask, immutable-agent, failed-write/RunSync and cancellation cases and16realDesktop security cases are recorded separately. Same fullUI baseline failure and14%reference gain remain dated24-core observations, NOT fresh96-core acceptance.
- Owner now has96-core/staticCargo host exclusively until explicit release, expected01:45UTC. Own fresh hosted calls0. A45s stock-only quiet gate failed00:07:17-00:08:02, load10.6->10.8 with0foreignrustc; no stock compile or uncontended speed claim. Broader10x goal remains unfinished, continuation confirmed01:55UTC. See `docs/cumulative-requirement-recheck-20261013.md` and the31-row exact-observation artifact.

## 2026-10-14: scaling curve on 96 cores: the warm loop stops at 8 cores, 1.42x

Own hosted host resized to m8azn.metal-24xl (2x EPYC 9R05, 96 cores, no SMT,
5.0 GHz, 377 GiB, performance governor). `bench/scaling.sh` (b2f6b14) run
on the host itself: whole build pinned to cores 0..N-1 with taskset,
CARGO_BUILD_JOBS=N, Desktop frontend threads min(N,16), nightly-2026-10-08,
2 warmups + 5 samples per point. Results
`bench/results/scaling-ip-172-31-32-241-20261014-001706-c*-remote-m8azn.metal-24xl.json`.

| cores | reference loop | speedup | frontend | codegen | link | incr | CPU s |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 6.51 | 1.00x | 2.40 | 1.15 | 1.44 | 0.45 | 6.1 |
| 2 | 5.27 | 1.23x | 2.30 | 0.93 | 0.76 | 0.47 | 5.9 |
| 4 | 4.78 | 1.36x | 2.29 | 0.79 | 0.44 | 0.46 | 5.9 |
| 8 | 4.58 | 1.42x | 2.30 | 0.72 | 0.29 | 0.46 | 5.6 |
| 16 | 4.65 | 1.40x | 2.32 | 0.80 | 0.24 | 0.48 | 7.0 |
| 32 | 4.65 | 1.40x | 2.30 | 0.82 | 0.22 | 0.49 | 8.0 |
| 64 | 4.73 | 1.38x | 2.32 | 0.82 | 0.25 | 0.48 | 7.7 |
| 96 | 5.08 | 1.28x | 2.35 | 1.01 | 0.34 | 0.51 | 14.9 |

Body check: 1.45 -> 1.26 s at 2 cores, flat after (frontend 0.8 s flat).
No-op test: 0.21 -> 0.17 s at 4 cores, flat after.

Reading: the loop's CPU stays ~6 s for a ~4.6 s wall, so average parallelism
is ~1.3 whatever we give it. Frontend 2.3 s does not move from 1 to 16
threads at all: the Desktop crate's frontend is effectively serial even with
the parallel frontend on. Only codegen (1.15 -> 0.72) and link
(1.44 -> 0.22, wild/mold threads) scale, and both saturate by 8 cores.
Incremental persist/load (0.46) is flat. 96 cores is slower than 8 (second
socket, cross-NUMA). So today a per-user 8-16 core VM gets everything this
toolchain can use; many-core shared boxes only pay off after the frontend
and incr stages are made parallel. Next: critical-path profile of the 2.3 s
frontend (self-profile at 16 threads) to find what serializes it.

### Per-pass breakdown on the same host (threads 1/4/8/16)

`bench/threads_sweep.py --toolchain nightly-2026-10-08 --threads 1,4,8,16`
on the metal host (pinned to 32 cores), jcode_desktop_ui unit, n=5,
`bench/results/threads-20261014-004215-remote-m8azn.metal-24xl.json`:

| pass | t=1 | t=16 | scales? |
|---|---:|---:|---|
| macro expansion | 1.27 | 1.30 | no |
| other rustc (unattributed) | 0.57 | 0.57 | no |
| mono collector | 0.58 | 0.43 | a little |
| incr persist | 0.43 | 0.45 | no (jr-rustc overlaps it, stock does not) |
| name resolution | 0.36 | 0.35 | no |
| partition symbols | 0.34 | 0.24 | a little |
| lints (misc_checking_3) | 0.20 | 0.22 | no |
| linker | 0.17 | 0.17 | no |
| type check | 0.17 | 0.14 | barely |
| drop AST | 0.10 | 0.10 | no |
| borrow check | 0.11 | 0.04 | yes |
| unit wall | 4.73 | 4.45 | 1.06x |

About 3 s of the 4.45 s unit is passes that do not use a second thread:
macro expansion alone is 1.3 s, then unattributed 0.57, persist 0.44,
resolution 0.36. The passes the parallel frontend does parallelize (type
check, borrow check) are only ~0.3 s here. So parallelism has to come from
restructuring, in order of size: (1) macro expansion: split proc-macro
execution from rustc's expansion loop and run/cache proc-macro calls in
parallel (attr-macro cache is the existing start); (2) take persist and
drop-AST off the critical path (jr-rustc overlap-persist already does the
first; stock nightly here does not); (3) attribute the 0.57 s unattributed;
(4) crate splitting (shadow leaf crates, 7.8 -> 1.8 s, FINDINGS 57), which
turns one serial crate into parallel units and is the only lever already
measured at more than 2x.

## Hosted shadow forwarding and actual verdict acceptance (2026-10-14 UTC)

- `ab55ea8` shipped/reinstalled public opt-in shadow forwarding, default off and internal child control excluded. Actual red-green hosted whitelist failure175137018 and8-run agent protocol regression175154135 observed, full361hosted tests/lint pass.
- Same new frozen client/96core host normally billed off/on/off reference5.670/1.155/5.556s, but all7fastsamples noisy. No causal speed claim or10x acceptance. Cold first leaf20.38s/98compiled disclosed; extra `--config build.jobs=8` deliberately normal fallback, not shadow tuning. `1b188a7` now labels noisy public suite comparisons uncontrolled,40Python tests pass.
- Actual pinned cases180405935/180407289/180409142/180411023 verify3leaftests,E0308/101,2pass1failure/101,parentnormal3pass;180426280 verifies16securityfixtures. Unfiltered180427353 retains1302pass/1existinghintchipfailure/10ignored/101. Saved source bytes restored. See `docs/hosted-shadow-20261014.md` for full requirement-to-run mapping.
- Corrected genuinely plain installed check180649536/full361tests180650413/lint180653455/recheck180653959 allhosted0compiled56fresh. Accidental local180514589 caused by helper RUN_ID_OVERRIDE is retained/excluded, not a default outage. Current stockquietgate blocked, optimized rustc unchanged, host released01:07UTC for owner-approved resize.

## 2026-10-14: instrumented occupancy map, not a causal critical path

The original `bench/critpath.py` (b146327) incorrectly labeled exclusive
elapsed-event occupancy as CPU work W, the observed event envelope as
mathematical dependency span S, and single-thread occupancy as a proven
critical path. These interpretations are withdrawn. Schema 2 reports only
observed elapsed occupancy and explicitly leaves CPU work, causal span and
speedup ceiling unknown. It also fixes query waits: a `QueryBlocked` interval
now suppresses its enclosing thread, rather than leaving its parent busy.
See `docs/parallelism-map.md` for the mathematical model and acceptance map.

Capture: `bench/critpath-capture.sh` on hosted m8azn.6xlarge,
jcode_desktop_ui after a one-line reference-loop edit, frontend threads 1
and 16, stock nightly-2026-10-08 and staged jr-b7907f1fe8f7. All four raw
captures were reprocessed with `--min-us 50`; checked-in schema-2 summaries
record filtering, missing dependency edges and microsecond endpoint clipping.
Raw captures remain on the host and locally. The profiler materially perturbs
these runs, so neither absolute time nor phase shares are performance proof.

JR b7907f1fe8f7, observed single-nonblocked-thread occupancy in seconds:

| phase | t=1 | t=16 | investigation hypothesis |
|---|---:|---:|---|
| mono collect + partition | 1.171 | 1.363 | independent upstream metadata decode, deterministic merge |
| resolve + HIR lowering | 0.983 | 0.988 | map shared resolver state before choosing module boundaries |
| incr persist | 0.489 | 0.505 | existing overlap path requires a parallel frontend |
| macro expansion | 0.483 | 0.494 | ordered effects and invalidation must be respected |
| typeck + borrowck | 0.358 | 1.172 | profile perturbation prevents scaling attribution |
| lints + dead code | 0.225 | 0.078 | already contains parallel queries |
| incr load | 0.222 | 0.175 | investigate safe prefetch |
| link | 0.173 | 0.165 | dependency on codegen, potential overlap with persistence |
| codegen | 0.065 | 0.065 | labels do not cover every external worker's CPU time |
| total one reported thread | 4.339 | 5.245 | not an inherent serial lower bound |
| total elapsed thread occupancy | 4.682 | 8.715 | not CPU work |
| reported query wait thread-seconds | 0 | 6.153 | not elapsed wall savings available to recover |

Stock t=1 macro-expansion occupancy is 1.300 s versus JR 0.483 s.
This localizes the known attribute-cache effect but is not a new speed claim.
The low-overhead `-Ztime-passes` sweep above shows mono collection 0.58 ->
0.43 s and type checking 0.17 -> 0.14 s from 1 to 16 frontend threads.
The heavily instrumented profiles therefore do not establish a production
contention regression, and cannot establish a 93% inherently serial fraction
or a theoretical floor. The observed core-scaling plateau remains evidence
about today's implementation, not a bound on future compiler changes.

Validation: 15 interval-accounting/schema/CLI/crox regressions passed, and
all four actual captures were successfully reprocessed. End-of-event rounding
corrections were counted explicitly (26-32 clipped endpoints per capture).
No compiler flags, default selection or normal benchmark targets changed
while correcting this analyzer.

## 2026-10-14: reduced-category profiles still need unprofiled controls

`docs/parallelism-light-captures-20261014.md` maps every profiling requirement
to an observation. Fifteen actual pinned runs (nine reference test runs, six
check runs) passed; five profile summaries, raw SHA-256 provenance and fifteen
exact privacy-redacted run exports are retained. Categories were reduced to
`generic-activity,query-blocked`, but clean check controls still observed
1.467 s unprofiled versus 2.238 s profiled at sixteen frontend threads. The
reference controls retain noisy flags. This is not an optimization speed claim.

The first run in each scenario populated a separate target (54 compiles,
833 depcache hits); later runs rebuilt one UI unit with 743 fresh units.
The test workflow reported three passing tests per run. The check workflow
has a different phase mix, with no linking bottleneck. No default compiler,
host type, fleet configuration or permissions changed.

Two harness defects are fixed in 3f87786: source sync now protects nested host
results, including raw profiles, instead of protecting only JSON; and the
launcher's source HEAD is no longer presented as an executable revision.
Actual CLI SHA remains recorded, binary revision is unknown. The provenance
collector explicitly marks the old source label unverified without altering
raw capture records. Fifty-nine Python regressions passed, including actual
rsync retention, collector CLI and source/binary provenance boundaries.

## 2026-10-14: safe sharing needs executable identity, not just metadata

`docs/sharing-rollout-20261014.md` maps the sharing requirements to actual
component observations and remaining rollout gates. The real native-helper
proc-macro proof (`20261013-194627381-739511`) produced different expansions
from byte-identical Rust metadata. Commit `0944f21` therefore hashes complete
executable proc-macro libraries and invalidates the old metadata-only xattr
memo. Check, mutation regressions and strict lint records are committed in
`data/runs/sharing-20261014/2026-10.jsonl`. Transitive native rlib keys are a
separate audit, not covered by this executable-only proof.

Fresh actual shared JR and nightly HTTPS prefixes passed tiny check, tests,
strict lint and expected failure, remained inactive when unpinned, and were
idempotent on repeat installation. Full local consolidated tests passed 380
with three ignored (`20261013-194746505-749789`), final strict lint passed
(`20261013-194811157-758446`); initial invalid fixture failure is retained
(`20261013-193744386-687523`). Local validation was necessary while the
hosted profiling lease was held. These are not hosted acceptance or speed
claims. An exact committed source packaging build is also retained as
`20261013-195503383-785887`.

Actual public registry archive sharing worked, but its complete network
path lost to direct Cargo fetching for this frozen public workload: three
alternating pairs had cold median 1.057 s versus seeded 1.697 s including
download, verification, restore and fetch. The seed transferred 13,340,586
bytes and kept the authoritative sparse index fresh. Bootstrap seeding stays
off by default. Cloud transport evidence is committed in `6f29c51`.

The earlier independent east fleet completed at 02:52 UTC: 211 new public
bundles, zero script runs, index 10,513 to 10,724 lines. It used the older
client/nightly and could read the default public store, so it is not new-key
JR coverage or an independent cold-cache experiment. The new pinned runner
explicitly disables public reads. Native script portability claims in
`docs/c-sysroot.md` are corrected: compiler-only identity is insufficient,
and the tested rootfs candidate remains non-authorizing.

## 2026-10-14: target-named source modules survive bulk source sync

Hosted Cargo integration exposed a mismatch: the source scanner preserved
Git-listed `tests/testsuite/*/target/` modules, but the initial/bulk rsync still
excluded every directory named `target`. Its mirror could therefore be missing
files that the manifest incorrectly considered uploaded. A bounded, hash-checked
manual upload unblocked the experimental Cargo tests while the client was fixed.

The correction allows exact Git-listed source files and symlinks plus their
necessary directories, not whole target trees. Sender-only hiding keeps ignored
untracked files out. Existing receiver protection retains root and nested build
caches, and the 8 MiB per-file cap still applies. The actual rsync regression
also exercises escaped wildcard/backslash paths, stale-source deletion and a
same-size/same-mtime checksum edit.

Forced-hosted check `20261013-202127298-948860` passed. The real rsync regression
passed in `20261013-202136435-948825`; full acceptance
`20261013-202306617-955157` passed 382 tests with three ignored; strict all-target
lint `20261013-202325996-955156` passed without warnings. These four exact
path-redacted exports are under `data/runs/hosted-west-syncfix/`. This is compiled
source correctness evidence, not proof of a newly installed client or a speedup.
Installation and new-root end-user acceptance are a separate rollout gate.

## 2026-10-14: hosted native release binaries for CI

The hosted service already admits runs and provisions tenant SSH connections;
release artifacts use that connection, not a new cloud HTTP artifact API. The
missing CI case was a fresh runner against warm Cargo outputs: timestamp-based
selection omitted binaries older than the invocation. Missing local artifacts
now transfer regardless of age. Native GNU/Linux explicit targets are routed,
while cross/musl/custom/multiple targets remain local (or fail when required).
`JUSTRUST_REMOTE_REQUIRED=1` rejects local routing and infrastructure fallback.
Public compiler pins, flags, release markers and Jcode metadata inputs are
forwarded, without forwarding credentials. A rustup variable suppresses the
store default but is not sufficient with standalone Cargo/rustc. The release
contract therefore uses `JUSTRUST_TOOLCHAIN=<exact version>` and
`JUSTRUST_TOOLCHAIN_REQUIRED=1`. Typed requirement failures cannot be swallowed
by the recorder's ordinary fail-open handler.

Real fixture acceptance `20261013-204341433-1071846` fetched and executed a
stripped x86-64 GNU/Linux binary from `target/x86_64-unknown-linux-gnu/release/`.
After deleting only the local binary, `20261013-204353848-1072682` compiled zero
units, reused one fresh unit and re-fetched the older output in 0.22 s (0.82 s
client wall). Both executions printed the requested version/release/metadata/
epoch values and produced SHA256
`3a94b079651cf0fd1c8130d1fdfcbb4bd5c7f8dba8c0b49272da35b7e3617042`.
The initial smoke used the host's standalone Rust 1.99.0, not a certified project
release pin; merely setting `RUSTUP_TOOLCHAIN` is insufficient on this host.
An official 1.90.0 toolchain was installed on the account's host for subsequent
strict-pinning acceptance, with no change to its default compiler.
Lib tests `20261013-204224122-1065521` passed 360 with three ignored;
lint `20261013-204239912-1066461` had zero warnings. Exact slim exports are in
`data/runs/hosted-release-ci/`. Full suite `20261013-205323034-1131121`
passed 393 tests with three ignored. Exact official Rust 1.90.0 acceptance
`20261013-205452142-1140525` selected the requested compiler, fetched the release
binary and executed all metadata and RUSTFLAGS assertions. A real hosted
uninstalled `0.0.0` pin with auto-install disabled exited 1 without compiling or
falling back. Final lint `20261013-205524366-1143599` passed with `-D warnings`,
and strict requirement integration `20261013-205530034-1144008` passed both tests.
These are correctness and transfer measurements, not a speedup claim.
Follow-up hardening also rejects uninstalled pins when nested Cargo or
`JUSTRUST_DISABLE=1` bypasses recording. Hosted check
`20261013-205722281-1156150`, the expanded integration test
`20261013-205724321-1156248` (system and unavailable pins, recording on/off),
and warning-free lint `20261013-205726886-1156590` passed. The reinstalled client
also rejected an unavailable pin with recording disabled on the real fixture.
The existing slow large-bundle publish fixture dominated the lib test run at
9.9 s; optimizing it is follow-up, not part of this release support change.

See `docs/hosted-release-ci.md` for the command contract and adoption gates.
Git metadata is not mirrored, arbitrary environment variables are not forwarded,
and profile-directory artifact scanning can include unselected binaries. Package
only expected names. Native libraries, toolchain availability and end-user glibc
compatibility still need application validation. No cloud deployment, public
release, API-secret provisioning or billing change was made.

A subsequent real CLI package trial exposed an atomic-reinstall race: the
client's running executable had been unlinked by a concurrent `cargo install`.
`real_cargo()` tried to canonicalize that deleted inode, and optional slot
metadata swallowed the failure before artifact fetching. Cargo discovery now
falls back to the executable path while retaining proxy-name filtering. Artifact
metadata now preserves its causal Cargo stderr instead of using the slot's
fail-open `Option` API. The Linux subprocess regression replaces the running
artifact-server binary before sending its request and uses real Cargo metadata.
Both replacement and diagnostic regressions passed in local run
`20261013-210654240-1225864`; strict release regressions passed in
`20261013-210654403-1226028`; all-target lint with `-D warnings` passed in
`20261013-210654694-1226251`. These checks ran locally with routing, peer cache,
depcache and prebuilt traffic disabled because another agent holds the hosted
benchmark lease until 04:15 UTC. Global reinstall and full application download
retry are deferred to the coordinator's post-lease acceptance gate. No speedup
or completed post-fix hosted application retry is claimed.

## 2026-10-14: final sharing and native real-interface feedback loop

Whole-result follow-through on immutable059674/0a19 now maps18explicitrequirements in docs/sharing-final-whole-20261014.{md,json}. Actual05:36fresh public compiler/profile/check/test matrix again yields8publichits0misses/four2-test passes. Actual05:39–05:40 ordinaryCargo20test workflows cover8native forms plus@file twice:18bypass, genericdep miss/hit,40runtime test results. Real nativeE0308/10120261013-224400547-1905506 followed repaired2tests20261013-224401789-1906049, both cache bypass. Two actual existing-prefix CLI reuse calls and12runtimehelper probes pass, inactiveJRmarker unchanged.

Re-audited actual frozen full410-pass/3ignored raw events:71named sharing/native checks individually mapped, rawSHA bd70ea176517bb21b6003731fcb8e50e49694911e1b3fc1f87d689544a802893. Fresh91named cloud sharing-boundary cases pass.31newexactredactedrows retained, including first acceptance-output-guardfailure with actualclient0 and genuinecompiler101. No aggregate-count substitute for native actualCargo acceptance, no speed claim.

Genuine new-tenant boot, next naturally scheduled fleet runtime receipt and trusted native closure remain explicitly unobserved/blocked. No peer-account host call, default install or mutation during graph lease. Prior actual hosted fullcheck/test/lint and Desktop edit/restoration remain attributed to their exact source, not relabeled as a new hosted run.

## 2026-10-14: installed native-flag safety and fresh public reuse

Guard832e362 and actual shim059674b were installed from exact committed059674 snapshot as0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d. Red native eligibility20261013-215342851-1568478 and real response-file eligibility20261013-215708557-1600890 both exit101 are retained. Early red automatic-update/telemetry/private-daemon network isolation caveat is explicit, not erased.

Actual hosted shim test20261013-220413535-1642867 passes40 invocations:36 native/@ bypasses,4 generic controls,38 compiler calls and2 cache restores, all executable output42. Frozen source guard verifies69 blobs, hosted check/full410tests3ignored/strictalltargetlint/recheck actual IDs220816567-1669580/220822907-1670409/220843215-1671710/220848764-1671939 (all20261013 prefix). Desktop FPS edit/repeat/restoration220940404-1675420/220951644-1676602/220955119-1676852 each3pass, original backup bytes restored.

New installed-safe client fresh public nightly/JR ×default/debug0×check/test all8 hit1/miss0, four runtime2-test workflows pass. Exact26 selected exports and mapped output/log hashes live in docs/sharing-native-safety-20261014.{md,json} and data/runs/sharing-native-flags-20261014. No speed claim or cross-CPU stale-execution claim. Native public authorization remains false and registry eager seeding remains off.

Future-only fleet update completed05:19:45UTC, actual launch-template3 and UPDATE_COMPLETE independently checked; onlyUserData changes, all other actual launch properties identical. Scheduler $Latest/daily09:00UTC unchanged. New opaque runtime receipts and genuine new-tenant boot remain unobserved, not inferred from configuration. Cloud deployment JSON records immutable client/source and runner S3 versions.

## 2026-10-14: native archive identity and actual toolchain helper closure

A real bundled C helper changed behavior from 1 to 2 with byte-identical Rust
metadata. The old metadata-only cache restored output 1. Full code-bearing
extern hashes and memo namespace m3 now produce miss/output 1, miss/output 2,
then unchanged hit/output 2 through the actual shim. Integration run
`20261013-201333279-898502` and strict lint `20261013-201334316-898680`
are exported under `data/runs/sharing-20261014/`. This repairs an observed
collision, not all native build-script closure determinants.

A fresh Ubuntu CDN install at 03:31 exposed an omitted official rustfmt driver
in the short JR ID despite successful core compiler/Cargo loads. Old immutable
bytes were preserved. A corrected full-commit JR ID and bounded publisher/client
helper probes now pass actual glibc 2.39 loader acceptance. Explicit reuse of the
old prefix fails without changing its marker. Exact source ec42e0f and stable
binary SHA2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53
were verified by the actual installer at 03:42:24. Final plain hosted full run
`20261013-204245695-1067158` passed 390 with three ignored. Check/lint/recheck also
passed. Concurrent replacement of the mutable global CLI means the full
sequence cannot be attributed to one frozen digest. The direct installer and
separate stable artifact are verified. No current-global digest is asserted.

Worker 6b86ccf2 enabled fixed-origin future bootstrap sharing without changing
bindings, quotas or defaults. Pinned independent fleet launched at 03:21 but final
publication/fresh reuse and a new tenant bootstrap remain unobserved. Native
rootfs sharing remains unauthorized. Registry archive seeding stays opt-in
because measured full-network median 1.697 s lost to direct cold 1.057 s. No
performance speedup is claimed. Full observations and 43 exact redacted records
are mapped in `docs/sharing-rollout-20261014.md`.

The prospective frozen-client guard then passed actual hosted acceptance at
04:12:42–04:13:16 UTC: known SHA2153a809/ec42 snapshot, 66 matched Git source
blobs, all four commands exit zero, exact snapshot tests 386 passed/three ignored
(`20261013-211249431-1273980`), strict lint and repeated check passed. All
per-step client digest and hosted/compiler-pin metadata guards passed. Receipt
`docs/sharing-frozen-acceptance-20261014.json` and four new exact exports close
future attribution, not the historical mutable-client epoch. Initial check
`20261013-211242111-1273414` had 68 account dependency-cache hits/five misses,
not public-store hits or the 73 hits of an older packaging run. No speed claim.

A distinct empty Cargo home/target/cache consumer subsequently passed check
`20261013-212740419-1382686` and two tests `20261013-212741514-1383002`, but
both compiled memchr 2.7.6 locally with zero public hits. Exact base keys were
absent from the index and direct CDN manifests returned 403 at 04:32 UTC.
This is a safe unpublished-key miss, not final compatible-fleet acceptance.
The independent producer remained running at 04:33 with its previous index
unchanged. Cloud232b53a adds future-only opt-in sanitized stage receipts;
12 executable runner/receipt tests pass, no current-runner deployment occurred.
Full cloud suite's unrelated native-budget failure is retained and assigned.
See `docs/public-sharing-consumer-20261014.md` for source snapshots and limits.

Later actual fleet publication at 04:35:59 UTC added 9,193 bundles (5.62 GB),
zero public script runs, and expanded the index to 19,917 entries. The machine
was observed terminated at 04:38:55. Four independently empty matched memchr
2.8.3 cells (nightly/JR × default/debug=0) yielded eight public hits and zero
registry misses across check/test; every cell passed two runtime tests. Four JR
keys were absent before this producer and present afterward. All eight exact
IDs, fixture/index/log hashes and outcomes are mapped in
`docs/sharing-public-matrix-20261014.json` and its Markdown companion; selected
evidence now totals 51 rows. The outer matrix shell failed only its final guard
filename (`installed.json` rather than `justrust-toolchain.json`); corrected
non-mutating marker read proved installed_at=0, retained without hiding failure.
The later full cloud suite passed174 Node+11 funnel+17 dashboard tests. Future
fleet status/full-JR contract7e66628 is source/schema validated but live safe
client pin is held for a newly identified native-CPU/opaque-response-file cache
eligibility guard. New-tenant bootstrap and universal native closure remain
unobserved/unauthorized respectively. No speed claim.

## 2026-10-14: native-citation integration test rebuild overhead in jcode

Hosted run `20261013-210006749-1169057` tested two new native-citation agent
regressions in 18.07 s, with 17.69 s compilation and 0.11 s tests. The large
`jcode_app_core` test crate took 14.10 s and peaked at 4281 MB. Hosted TUI run
`20261013-210027105-1170331` tested two matching UI regressions in 20.12 s,
with 19.76 s compilation and 0.04 s tests. Its `jcode_tui` test crate took
9.85 s, while the normal `jcode_app_core` dependency took 9.36 s. The report
also identifies `docs` as a build-script input forcing app-core rebuilds.
These are project crate-boundary and embedding opportunities, not a verified
hosted scheduling defect. The source-aware citation task was completed first,
with no unrequested crate split or claimed speedup.

## 2026-10-14: reconnect rsync preserved stale Cargo source timestamps

Routing checks `20261013-205929605-1165789` and
`20261013-210052658-1172116` failed with E0599 for the newly added
`BedrockProvider::model_supports_tools`, taking 3.998 s and 3.332 s while
373 units stayed Fresh. Raw unit data for the initial
`20261013-205602266-1147451` shows `jcode_provider_bedrock` compiled as
`depcache=local-miss`, not a shared dependency-cache hit. The prior read-only
remote diagnostic found identical local/remote source SHA256 and the ungated
method at line 849, but source mtime 03:56:22.914567037 UTC preceded
dep-info/invoked/rmeta at 03:56:22.988778401 UTC. Source ctime/birth
03:59:30.537 UTC matched the reconnect copy: rsync had replaced the bytes
while preserving the older sender mtime. Cargo CheckDepInfo therefore reused
metadata compiled from the previous content.

The repair is confined to source mirroring in `src/remote_sync.rs`: rsync
uses `--no-times --checksum`. Changed bytes get receiver write times, while
identical bytes retain receiver mtimes even when the sender mtime differs.
Mandatory content comparison also catches same-size/same-mtime edits and
avoids resending unchanged files now that sender/receiver timestamps differ.
The legacy seed checksum hint remains accepted, but cannot disable correctness.
The incremental Write-frame path and all target protections remain unchanged.
No billing, tenant isolation, shared-cache identity, or hosted-cache contents
were modified. The concurrent remote_daemon seed toggle was left untouched.

Red regression `20261013-211357060-1279403` reproduced changed source older
than a warm fingerprint. A real dependency-free Cargo fixture in
`20261013-211821139-1313406` also reproduced an incorrect successful Fresh
check after replacing valid code with invalid bytes and preserving an old
mtime, even with checksums enabled. Both regressions now pass. The Cargo
fixture runs locally to its test Mirror, offline, without outer-run recording
variables, so its intentionally failing rustc does not taint the test report.

Final hosted check `20261013-211836530-1314540` passed in 0.899 s. Full
`justrust test` run `20261013-211838055-1315229` passed 397 tests, three
ignored, in 11.887 s. Strict clippy `20261013-211850749-1317741` passed
in 1.269 s without warnings. `cargo fmt` and `cargo install --path . --locked`
succeeded. Installed client SHA256 was
`c63e17951fe267120c4bc0c0279f30c4586fcf2d5fc2d5c79b68ec94c44ce06e`
before and after a real one-line src/runs.rs edit/test workload:
`20261013-211954026-1326445`, 397 passed/three ignored, 11.715 s recorded
wall, 15.4 s hosted round trip including 3.230 s sync. The edited file was
restored byte-for-byte from its backup, not from git. No jcode rebuild or
reload was performed. Nine cited slim, redacted exports are in
`data/runs/hosted-freshness-repair/2026-10.jsonl`. This is a correctness fix,
not a speedup claim. Full rsyncs now hash source contents on both sides.

## 2026-10-14: uniform-cost Cargo scheduler has no demonstrated 24-job benefit

Same immutable client ec42e0f/SHA2153a809, candidate Cargo a45c354/SHA c38c3d63,
and nightly-2026-10-08 ran pinned Desktop OFF/ON/ON/OFF on west m8azn.6xlarge
in a private copied prefix. All 40 measured runs and 16 warmups exited zero;
actual metadata selected the candidate/compiler/local route/24 jobs throughout.
Reference/no-op tests passed three each, edit workflows rebuilt one unit and
no-op rebuilt zero. Representative IDs `20261014-034808753-288986334` and
`20261014-040339460-542771790` are among the 56 exact exported records in
`data/runs/hosted-west-cargo-scheduler/2026-10.jsonl`.

`bench/suite.py compare` reports cached cold 9.45 -> 9.36 s, uncached cold
27.77 -> 28.01 s, reference 4.98 -> 4.96 s, body check 1.32 -> 1.29 s and
no-op 0.18 -> 0.18 s. Four original verdicts are uncontrolled; body check is
within noise. Independent concurrent release work through 03:58:45 UTC means
the earlier body samples also cannot establish a causal gain. All original
flags and eighteen conservative known-confound annotations are retained.
Default-off stays off; no speedup, hardware ceiling or promotion is claimed.

Full results, invocation/hash audit and actual comparison are committed in
`bench/results/scheduler-a45-j24-20261014/`; requirement mapping is in
`docs/cargo-scheduler-abba-20261014.md`. The frozen recorder's live-tree CPU
sum can drop on child exit and inflate derived other-core attribution. Repair
that measurement defect in a new epoch, never rewrite these historical flags.

## 2026-10-14: CPU identity accounting and outer-command verdict repaired

`4809f23` tracks sampled CPU deltas per `(pid, start_ticks)` instead of
subtracting live-tree totals. Four actual hosted regressions failed before
the fix (`20261013-213037568-1395671`) and passed afterward
(`20261013-213336102-1409383`). `e629a00` requires a failing outer exit before
classifying nested compiler probes as a compilation failure. Original run
`20261013-211707833-1300731` remains unchanged: 397 tests passed, exit zero,
but the old report incorrectly classified its intentional failed compiler unit.

Installed clean e629a00 client SHA08d0601aa8bf passed normal hosted
check/full404tests/strictlint/recheck with zero compiled and 62 fresh units;
full run `20261013-213917356-1436666` reported the correct successful verdict.
Real Desktop edit `20261013-214047661-1443635` passed three tests/one compiled
unit; unchanged repeat `20261013-214056105-1444107` compiled zero/746 fresh.
Source was restored byte-for-byte from backup. Actual recorded/unrecorded
minimal checks reused identical rmeta bytes without Dirty or recompilation.
Real nested E0308 output stayed visible in successful outer test run
`20261013-214356941-1470803`; genuine outer failure
`20261013-214408640-1471875` still returned 101 and a compile-failure verdict.

All21 cited records, including red tests and a precompile CLI-harness mistake,
are in `data/runs/hosted-west-measurement/`; detailed requirement mapping and
hash receipt are in `docs/measurement-acceptance-20261014.md` and its result
JSON. Sampling still misses unsampled exit work and cannot establish query
dependencies or a theoretical parallelism ceiling. No historical timing/noise
flags were rewritten and no speedup or scheduler promotion is claimed.

## Final frozen parallelism/measurement requirement rerun (2026-10-14)

Same unchanged two-test fixture: old client2153 run20261013-215604181-1595675
incorrectly reported compile failure despite exit0; corrected08d client
20261013-215604352-1595740 correctly reported success, retaining fullE0308.
Final immutablee629 hosted suite20261013-215956564-1615106 passed404+3ignored,
strictalltargetlint passed, repeatedcheck0compiled62fresh. Actual Cargo20tests
and Desktopedit/repeat/backup-restoration passed. Final receipt/requirement
map: docs/final-parallelism-acceptance-20261014.md; exact18new records appended
to hosted-west-measurement (39total). Prior21summary hashes and56scheduler
records/foursegment hashes verified,86offline guards pass. No speedclaim,
no scheduler promotion, no inferred96-core ceiling. Sharedhost released05:02:02UTC.

## Jcode pinned-todo prompt visibility validation (2026-10-14)

Hosted run `20261013-225238779-1953993` spent 8.14s compiling the
227k-line `jcode_tui` test crate to execute six pinned-todo tests (15.2s
reported build/test wall, 17.8s hosted end-to-end). The report estimated
7.4s oversized-crate waste and 4.1s filtered-test compilation waste. Warm
preparation-only run `20261013-225626778-1975498` passed seven tests with
zero compiled units in 0.1s reported wall (2.6s hosted end-to-end).
This supports keeping focused transcript/viewport test code in smaller leaf
crates as a future project-level improvement, not a compiler or scheduling
speedup claim. No restructuring was performed during the UI bug fix.

## 2026-10-14: optimized BUILD dependency mapping preserves unknown CPU/span

Genuine billed hosted optimized library build `20261013-232242825-2161355`
completed in 77.277 s recorded / 89.474 s client time and downloaded its
22.9 MB artifact. Immutable native-safe 0a19 client and pinned nightly compiler
were unchanged. UI rustc used four frontend threads, 15.909 s wall and
129.698 aggregate CPU seconds. Fresh target, warm shared cache: 887 compiled,
zero Fresh, 726 dependency hits / 107 misses. One foreign rustc was observed,
so this is not uncontended or causal speed evidence.

Public graph/profile/mapping CLIs joined 2,695,071 actual provider intervals to
2,809,610 saved nodes and 20,496,477 edges. All 6,851,280 raw records validate;
114,539 nodes remain untimed. The 177 QueryBlocked records include 68 absent
read edges and two missing caller contexts. Blocked elapsed includes post-wake
lookup, not pure idle. CPU work and execution span remain null: incremental
read dependencies omit continuations, token/OS scheduling and subprocess
causality. No 16/24/96-core recommendation or speedup follows from graph depth.

`8652f30` adds explicit bounded large-capture count limits without raising the
256 MiB input cap or default limits. Actual large default CLIs reject exit 2;
opt-in graph/profile/mapping pass, all 169 bench tests pass, and independent
raw-hash/receipt/output invariants pass. Full requirement mapping, redacted
output hashes and both exact run exports are in
`docs/compiler-query-build-graph-20261014.md` and its result/run namespace.
Failed precompile local harness `20261013-231920710-2139396` is retained, not
misclassified as a hosted failure. No compiler implementation/default/security
changes were made. Account host explicitly released 06:41:46 UTC.

## 2026-10-14: partition existing BUILD activities without inventing CPU work

Offline follow-through on the same optimized BUILD
`20261013-232242825-2161355` now accounts for all 6,851,280 recorded intervals,
including LLVM, LTO, metadata and coordinator activities rather than treating
all query elapsed as frontend work. `2be8d4c` adds a bounded all-kind analyzer:
584 kind/label groups and 517 Rust thread IDs, no crossing/equal intervals.
Group-exclusive elapsed equals per-thread covered unions at 148.396651246
thread-elapsed seconds within a 15.826062441-second observed timestamp window.
This is NOT CPU work: the unit's separately recorded process CPU is 129.698 s.
Waits and descheduling can remain inside exclusive scopes.

Actual public CLI reanalysis reproduces the compact output hash exactly.
All 177 offline tests, eight activity tests and an independent exhaustive
250-forest time-cell reference pass. Default large-input rejection, small
default success, strict/explicit unmapped labels, malformed input, symlinks
and exclusive output preservation are exercised through the real CLI. Six
pinned compiler source hashes and 17 label locations are verified. Inclusive
inventory and independent compact-output review agree. No new compiler build,
host call, installation, default change or speed comparison was made.

The 70.424 s inclusive LTO-wrapper total partitions to 1.441 s exclusive after
recorded children, whereas LLVM optimization and object-emission leaf totals
remain elapsed, not pure CPU. Metadata and ongoing-codegen join are not proven
serial CPU bottlenecks. Query-only and all-kind exclusive definitions remain
separate. CPU work, execution span and the 16/24/96-core ceiling remain unknown.
See `docs/compiler-phase-activities-20261014.md` and its exact output/log/hash
acceptance under `bench/results/compiler-phase-activities-20261014/`.

## Synthetic execution controls were recorded but discarded (2026-10-14)

The comprehensive single-stack suite recorded public optimization settings but
its versus child scrubbed them and the selected private `JUSTRUST_HOME`.
Commit `2172a45` preserves the selected home for justrust stacks and explicitly
forwards only the single-stack suite's allowlisted controls. Stock and ordinary
versus baselines still scrub ambient experimental settings. Hint provenance is
a digest, not its private path; caller routing/run IDs remain runner-owned.

Four of eight tests failed before the fix, including actual Python exec losing
the home. All nine context tests and all 186 benchmark tests pass afterward.
Actual local-only private-prefix check/test/recheck passed with the immutable
0a19 client, c38 Cargo and pinned nightly. Exact run IDs are
`20261014-005414405-3093505455`, `20261014-005414445-3093505147`, and
`20261014-005414740-3093505330`. An earlier check
`20261014-005234369-3084814456` succeeded, then its helper incorrectly expected
an arbitrary JUSTRUST flag in filtered metadata. Retain that harness error,
do not claim metadata supplied it. All four summaries are exported under
`data/runs/synthetic-context-20261014/`. Foreign laptop compilers were present,
so these are functional checks, not timing evidence or queue-reordering proof.
See `docs/synthetic-context-20261014.md`. The separate comprehensive scheduler
trial must establish its own complete matched result before any speed claim.

## 2026-10-14: comprehensive scheduler gate stopped safely on scratch capacity

The unchanged seed-1, 40-point, one-sample OFF/ON gate used a private frozen
0a19 client, c38 candidate Cargo, nightly-2026-10-08 and 24 jobs. All 186
benchmark tests passed before execution. Three successful actual records
verified private Cargo/compiler selection, exact execution controls and
process/summary exit equality: `20261014-081653879-1167169777` (release
pre-build, 121.101 s recorded), `20261014-081855187-1167169410` (first edited
UI release point, 20.508 s recorded), and `20261014-081945795-1167169753`
(next test pre-build, 55.687 s recorded). Executable walls are distinct.

Only one OFF point was measured and marked noisy. ON never began. The first
pre-build observed up to 20 foreign rustc processes. There is no matched
comparison, scheduler speedup, default promotion or core-ceiling result.
The 8-GiB free-space guard stopped both attempts. Standard Cargo refused
cleaning the untagged parent container; only this trial's three tagged,
unmeasured stock-probe output roots were cleaned. Model tables, measured
outputs and existing user/service caches were preserved. Read-only source
inspection did not verify a safe existing authorized filesystem growth path.
No volume, reboot, IAM or general-sudo changes were made. Host released 08:28 UTC.

See `docs/scheduler-comprehensive-2172-20261014.md`, the exact three exported
records in `data/runs/broad-scheduler-2172/`, and hash-audited partial results,
failure logs and execution receipts under
`bench/results/scheduler-comprehensive-2172-20261014/`. Public log redactions
cover operator/home paths and the private SSH address, retaining raw hashes.
Completion of the broad performance gate remains blocked on scratch capacity.

## 2026-10-14: synthetic reports must not turn a noisy partial into a session mean

The unchanged actual one-of-40 noisy OFF payload exposed a second measurement
defect: immutable pre-fix `e1f772d` public `show` printed a bold 20.53 s
expected wait per call, and `compare` printed +0% from that single point.
The report now requires complete point/replicate coverage, explicit quiet
provenance, consistent planned outcomes and compatible workload/source/machine
identities before emitting comprehensive aggregates or comparative ratios.
Raw timings remain visible with noisy/unknown annotations. Missing instruction
counters no longer disappear into a subset median: valid wall reports survive,
but instruction ratios require every replicate's valid counter evidence.

Four tests-first red phases were retained. All 13 reporting regressions and
all 199 benchmark tests pass. Fourteen independent public commands reproduce
the old defect and verify the repaired actual payload, positive clean/renamed
controls, malformed inputs and partial/full counters. Independent review
reproduced the counter fix. Seven measurement functions and the generator,
weights and versus engine are source-audited unchanged. The three actual
compile records and original partial/export hashes are also unchanged.

See `docs/synthetic-report-eligibility-20261014.md` and hash-audited acceptance
under `bench/results/synthetic-report-guard-20261014/`. Positive fixtures are
report controls, not benchmarks. No compiler, account-host, install, storage or
security operation was entered. The complete OFF/ON performance comparison,
CPU work/span and hardware ceiling remain blocked or unknown.

## 2026-10-14: replace the README's mislabeled uv comparison with matched measurements

The October 10 table labeled local justrust timings as the standard toolchain
and compared a 5/574-hit local dependency cache with a 574/574-hit remote cache.
Replaced it with 27 successful measurements on pinned uv `44b2e5877ef7`, three
rotating rounds of stock local, justrust local, and actual hosted execution.

Median agent-visible seconds, stock / local justrust / hosted:

- Fresh workspace check: **81.16 / 34.54 / 14.33**.
- Independent fresh resolver test build (`--no-run`): **85.85 / 33.30 / 15.24**.
- Warm executable body-edit resolver check: **0.956 / 0.849 / 0.891**.

Shared caches persist, with first-round fills retained, not an empty-cache
comparison. Hosted timing includes upload/admission/report delivery. Laptop
load was 11.3 to 26.7, hosted hardware is 24-core AMD EPYC, and toolchains differ
(stock 1.98.1, local jr-ac20de5e7921, hosted 1.99.0). Edit ranges overlap, so no
stable hundredth-second winner or general synthetic-workload speedup is claimed.
A tool-imposed timeout interrupted one stock test build. Successful observations
were preserved, and the interrupted attempt was retried in a new empty target.
All edited sources were restored and compiler identities checked for stability.

Examples of recorded measurements: local check
`readme-uv-1ada85de5f1b4f37908e9f0fee8586d9`, hosted check
`readme-uv-1c34340dc5364e3ea514975db9fb4ac0`.
All 24 justrust recordings, including excluded prewarms, are exported in
`data/runs/xps14/readme-uv-20261014.jsonl`. Full timings, ranges, cache caveats,
source/client identities and retry provenance are in
`bench/results/readme-uv-20261014.json` and
`docs/readme-uv-benchmark-20261014.md`. All 214 benchmark tests, formatting and
diff checks passed. The README Install section was also condensed.

## Windows sampler cycle allocation (2026-10-14)

Jcode v97 Windows x64 alpha17 aborted allocating 30,064,771,072 bytes.
`record::tree_of` had unbounded traversal of historical parent PID links:
cycles from Windows PID reuse append `ProcStat` forever. Its 56-byte x64
layout times a vector growth to 2^29 entries exactly matches the allocation.
The runner snapshot is unavailable, so this is a strongly supported diagnosis,
not a native Windows reproduction. Added unique-PID traversal and rejected
parent-newer-than-child edges, with synthetic cycle/reuse regression tests.
Immediate Windows-only mitigation: `JUSTRUST_DISABLE=1` with `--local` and
system toolchain, bypassing measurement while preserving cargo exit status.
Details: `docs/windows-sampler-allocation-20261014.md`.

Check run `20261014-032322437-1579906`, regression run
`20261014-032341564-1584429` (2 passed), clippy run
`20261014-032428669-1596636`, recorder test run
`20261014-032449198-1599286` (13 passed). Hosted was attempted explicitly but
returned HTTP 404 for `/internal/builds/charge`, falling back to Linux locally.
No Windows execution or performance speedup is claimed. Contention and the
large single crate dominated validation compilation (17.9s check, 46.4s test
compile), unrelated to this correctness repair.
Full library suite `20261014-032511105-1606621`: 380 passed, 3 ignored.
Reinstalled with `cargo install --path . --locked` and exercised the repaired
installed recorder on this repository's `justrust check --local`: successful
report, 62 fresh units, zero recompilation. The five cited validation summaries
are in `data/runs/xps14/windows-sampler-20261014.jsonl`.

## 2026-10-14: larger legacy-host scheduler gate, OFF-only and blocked

The separate existing developer-owned c7i.8xlarge (32 vCPUs) with an existing
500 GiB volume removed the prior storage entry refusal. Effective initial
free space was 229,541,593,088 bytes. Fresh-prefix source/toolchain hashes,
150 GiB entry reserve, 8 GiB per-command reserve and 24 Cargo jobs were checked.
The ordinary tenant host, D1 row, defaults, IAM and service caches were unchanged.

Actual seed-1/40-point OFF execution reached 15 points with 35 completed commands.
Seven measured samples are noisy and three unexpectedly fail tests. ON never
started, so there is no broad comparison, speedup, promotion or core ceiling.
UI `20261014-095558516-43443865`, workspace `20261014-100607224-43443644` and
base `20261014-100809183-43443833` retain exit101 and complete failing output.
The UI failures are panel-cache assertions, not the historical viewport failure.

Point 16 `20261014-101022743-43443971` stopped with its observer/CLI/test group
behind the controlling PTY foreground group. A verified owned-only SIGCONT
immediately restopped. The exact signal/source is unproven. The owned group
was terminated at 10:23:27 UTC, with its 784.933s recorder finalization retained but
NOT counted as a benchmark outcome. Separate headless diagnostic
`20261014-102329731-294199` completed/101, not a pure stdin or compiler gain.

Closing frozen CLI/Cargo/rustc/hint hashes and zero owned commands were verified.
The exact originally stopped legacy instance was restored stopped at 10:27:09 UTC,
with its 500 GiB volume preserved. No termination/deletion or permission changes.
The 11:00 cleanup backstop was canceled only after that observed restoration.

Frozen public show refuses 15/40/noise/outcome aggregation; self-comparison
refuses absent environment provenance, not a fabricated ON result. 199 frozen
Python tests pass. 37 exact exported records include the interrupted recorder
and diagnostic, not 37 samples. Full failure JSONL is compressed and redacted
only for explicit paths/resources/addresses/long bearer tokens. Requirement
mapping and limitations: `docs/scheduler-legacy-attempt-20261014.md`; artifacts:
`bench/results/scheduler-legacy-3f47-20261014/`. All 37 cited exports are in
`data/runs/scheduler-legacy-c7i-20261014/2026-10.jsonl`.

## 2026-10-14: independently bound compiler captures, not a whole-pipeline graph

Source cohort `d3a5319b9` binds selected native Cargo launch declarations to
independent compiler entry and schema-3 query receipts. Actual dirty acceptance
reconciled 1 unit, 1 launch and 7 journals with one entry/query; fresh reuse
reconciled 1 unit and 3 journals with zero compiler launches. Deleted required
evidence, a failed compiler, and a nonprivate sink never passed completeness.
The nonprivate sink still compiled successfully while reporting one lost event.

A real proc macro exposed the original late private argument in process argv
and changed generated output. That transport was rejected before publication.
The replacement reserved-fd-v2 transport adds no argument or environment entry,
preserves foreign descriptors on rejected probes/collisions, and consumes valid
transport only at an explicit unsafe process-main ownership boundary. Rebuilt
native acceptance passed exact full argv, generated output and executable SHA
equality, as well as warm off/on/off executable identity and fresh reuse.

Focused consumer run `20261014-103451429-3329324` passed 28 tests; its export and
six related targeted recorded runs are in
`data/runs/compiler-binding-20261014/2026-10.jsonl`. Final actual acceptance and
artifact fingerprints are in that directory's `native-acceptance.json`, with
procedures and limitations in `docs/compiler-binding-native-results-20261014.md`.
Combined hosted client gates passed 540 tests, check and strict all-target lint.
Capture remains opt-in and scoped: full-pipeline completeness is false,
CPU-work/span are null, wrappers and early-reply cache paths remain unsupported,
and no speedup or thread ceiling follows from these receipts.

## 2026-10-14: jcode v0.98 CI repair exposes oversized-crate edit costs

Hosted run `20261014-122252729-514803` (`check --locked -p jcode-app-core
--tests`, exit 0) spent 13.09s in Cargo, including 7.59s for app-core's test
unit (168k source lines) and 3.52s for base. The corresponding isolated
retention-readiness test run `20261014-122325267-519971` passed 3 tests but
compiled a 1,609-test binary: app-core took 17.8s and base 6.8s. The remote
invocation took 69.7s, of which sync took 43.176s, so compiler timing alone
does not describe the user-visible delay. No before/after speedup is claimed.
These are project crate-boundary costs, not a reason to bypass hosted routing.
The jcode coordinator was notified. Moving helpers between files for LOC
ratchets does not itself reduce the Rust crate's recompilation scope.

Follow-up hosted owned-source validation: run `20261014-123933366-699221`
passed all-target/all-feature checks in 17.9s but reported 12.9s shared Cargo
lock wait during concurrent owner checks. Run `20261014-124321736-736356`
passed 1,249 tests across 16 binaries in 6.2s hosted wall time, including
0.5s downstream rebuild cost for doctor/companion after provider/voice edits.
SDK integration run `20261014-124327969-737152` passed 26 tests in 13.2s hosted
wall time, dominated by two existing 10.00s tests. These point to coordinating
aggregate snapshots and narrowing test fixtures rather than bypassing hosted
routing. No speedup is claimed.

## 2026-10-14: explicit native query-to-task point authority

Query schema 4 and pool schema 2 now bind direct admission, request, read,
result and wait points through reciprocal capture authority. Actual rebuilt
compiler acceptance observed 33,319/33,319 callback points, including 11
registered/resumed wait pairs. Missing or remapped authority and out-of-lifetime
points fail mapping validity without inventing task ownership.

Whole client run `20261014-123649185-667605` passed 559 tests (3 ignored),
with strict all-target lint `20261014-123702967-671342`. The nine exact cited
client/transport runs are exported under `data/runs/query-task-binding-20261014/`.
Actual native acceptance and the requirement matrix are in
`docs/native-query-task-binding-results-20261014.md`.

The 32 paired fixed-fixture controls retained equal executed output and
capture-off/on artifact hashes within each compiler. Capture-on medians were
0.264366 to 0.266704 seconds at one thread and 0.174602 to 0.204570 seconds at
four threads. This is an opt-in mapping capability with measured capture cost,
not a speedup or default promotion. Quiet-machine provenance is unknown, and
no comprehensive session-weighted speed gate was run. Full pipeline mapping,
resource-grant causality and CPU/span ceilings remain unsupported.

## 2026-10-14: targeted allowance prototype has measurable admission cost

A separate experimental `TicketedProxy` now records actual protected request,
selected grant and acknowledgment transitions. The legacy Proxy body is
unchanged, and the experimental protocol is not wired into frontend pools.
Actual owned compiler tests passed all 11 targeted cases, and the rebuilt
compiler passed admission, wait/read/pool, query-task, Cargo authority and
proc-macro neutrality acceptance. Whole-client hosted run
`20261014-132304295-1180347` passed 559 tests, with check
`20261014-132303169-1180201` and strict lint
`20261014-132315425-1180084` also passing.

The matched real-source microfixture pins jobserver 0.1.34. Across 15 rotating
rounds and 180 samples, median one-token/four-token/immediate costs were
286.793/165.949/0.277 ms for legacy, 314.302/201.659/1.244 ms with ticket event
rows disabled, and 311.409/204.504/1.308 ms with event rows enabled. All candidate
medians were larger. Event-row disabling retains IDs and independent counters.
The earlier 0.1.35 baseline is preserved as superseded, not compared against
these candidates. Raw results are in `bench/allowance-tickets/results/`.

This establishes a scoped protocol experiment, not faster compilation or a
default promotion. Callback-in-flight teardown, real helper IO error injection,
Registry/query integration, resource receipt reconciliation and comprehensive
session-weighted performance gates remain open. No full graph or CPU/span
ceiling follows from these results.

## 2026-10-14: slot-free allowance candidate trades idle cost for contention

The evaluated `1233aa16` candidate eliminates the idle path's request-slot
allocation and acknowledges the implicit grant under one protected State lock.
Its native crate passed 133 tests and 5 doctests. Source-matched compiler/query,
Cargo and macro-neutrality acceptance also passed after correcting acceptance
ordering: native crate tests replace the bootstrap private-metadata sysroot,
so the real private-driver callback fixture must run before those crate tests.
No compiler-source fix or metadata substitution was used for that setup failure.
Hosted whole-client run `20261014-134444010-1418389` passed 559 tests, alongside
check `20261014-134442741-1418120` and strict lint
`20261014-134457381-1417956`.

The matched real-source ABBA/BAAB fixture retained 720 samples in 120 batches.
Idle 10,000-pair medians improved from 1.073910 to 0.266289 ms with event rows
disabled and 1.176994 to 0.383702 ms enabled. But natural four-token contention
medians increased from 174.861898 to 189.085572 ms disabled and 170.185051 to
214.829179 ms enabled. Paired blocks were slower in 11/15 disabled and 14/15
enabled comparisons. Forced-pending one-token medians also increased. These
regressions are not averaged away or dismissed as noise; all outliers and
capacity/conservation checks remain in `bench/allowance-tickets/fastpath/`.

The candidate is not promoted as an overall improvement or made the default.
The extra initial State lock on the contended path motivates a separately
measured advisory-idle-hint alternative. Correct native predicates remain the
only admission authority. Full pipeline/resource mapping, fairness and general
compile-speed claims are still unsupported.

## 2026-10-14: advisory hint preserves native authority and lowers idle fixture cost

The `f3849e80` unwired TicketedProxy uses an advisory Relaxed idle hint only to
select whether to attempt the slot-free path. The owning State lock still
checks every grant. Forced stale-true and stale-false tests preserve admission,
registration and conservation. Actual compiler/std/proc_macro rebuild and all
nine native compatibility cases passed in task `326345s2gi`, followed by 136
native crate tests and five doctests. Those compiler cases still exercise the
legacy frontend, not TicketedProxy frontend or query/resource integration.

The matched three-way production fixture retained 1,080 samples in 180 batches.
Idle 10,000-pair medians were 1.073451/0.273216/0.323714 ms with event rows off
and 1.205256/0.359494/0.398261 ms on for original/extra-lock/hint respectively.
Hint beat the original idle path in all 15 paired blocks per mode, but was
slower than the extra-lock candidate in 13/15 off and 14/15 on blocks. Natural
multi-token hint/extra-lock paired ratios were 0.9273 off and 0.9216 on. Against
the original, the corresponding ratios were 0.9966 and 1.0186, so lower aggregate
medians do not establish a robust multi-token win over the original. One-token
and forced-pending tradeoffs remain explicit in the retained raw evidence.

Hosted full run `20261014-140406450-1639310` reported 559 passed after an isolated
retry of an existing parallel ETXTBSY bundle test. A separate deliberate rerun
`20261014-140447126-1646208` passed that test. Check and strict all-target lint
also passed. This supports only a targeted idle-path improvement in the unwired
prototype. Legacy default, full resource mapping and general compile-speed
promotion remain unchanged or unsupported.

## 2026-10-14: explicit selected frontend waits for native allowance after query readiness

The programmatic frontend factory now selects the experimental TicketedProxy
independently of recording. The default factory is absent and retains the legacy
Proxy and original administrative callbacks. Qualified query waits carry checked,
non-reused root/attachment identities only in memory, with phase/attachment
validation and exactly one contextual-or-unit dispatch. No resource wire format,
query schema 4 or pool schema 2 change is introduced.

Final source-aligned task `853801iecj` passed the rebuilt compiler/std/proc_macro,
six selected-driver cases, nine legacy cases, three native compiler crates and
their integration/doctests. The selected actual query fixture positively observed
a ready result while its duplicate consumer remained Pending with return=false.
Only returning a held real jobserver token allowed the helper grant and native
reacquire to complete. Recording off/on preserved full proc-macro-visible argv
and environment fingerprints, executable bytes and output. Default and serial
factory smoke, selected type diagnostics, callback unwind and foreign descriptor
fail-open cases also passed. The initial fixture wrongly assumed two native
tokens rather than sixteen; the corrected fixture holds fourteen real Acquired
tokens rather than fabricating capacity. A native internal test lint was fixed
without changing production code before the final complete rerun.

Hosted whole-client run `20261014-144020738-2003637` passed 559 tests, with check
`20261014-144019067-2003317` and strict lint `20261014-144032083-2003146` passing.
The fixed-fixture ABBA diagnostic retained 64 controls: default-path off medians
were 129.680/126.132 ms at one thread and 93.872/93.361 ms at four threads for
before/after. Capture-on medians were 250.791/219.553 and 211.351/198.412 ms.
The capture fixture saturates read and binding budgets and reports incomplete
coverage honestly. These noisy, narrow controls do not establish zero overhead,
a general speedup, selected-policy performance or a default promotion.

Public graph consumption of the actual selected capture reports one complete
wait lifecycle and valid point authority, but 27,579 of 30,001 points unbound,
full_pipeline_complete=false and CPU/span null. The compiler entry still needs
explicit parent-manifest reconciliation. Wait-ID exhaustion is covered by real
module and representative extracted-latch tests, not an actual compiler forced
exhaustion run. Resource grants, helper IO-error provenance, callback-in-flight
teardown and full semantic/resource mapping remain separate work.

## 2026-10-14: bounded native allowance prefixes reach validated query waits

Resource schema 1 now records selected TicketedProxy grant decisions as bounded,
immutable prefixes. Three independently owned query/pool/resource endorsements
bind exact namespaces without changing query schema 4 or pool schema 2. A closed
source-to-grant-to-Ack-to-actual-acquire-return chain can authorize a query wait
resume, without inferring physical holders, leases or helper demand provenance.

Actual rebuilt compiler task `4999534dv4` and resource acceptance
`690279pgup` passed all six helper/local pregrant/postresume and capacity cases.
Postresume helper and local captures each expose one validated cross-wait edge;
pregrant captures expose none. Capacity 0/1 preserves native progress while
reporting incomplete records and no resource edges. The actual-copy mutation
matrix rejects missing/conflicting endorsements, omitted categories, wrong
sources/references/namespaces, lost inventory and impossible transaction state.
A valid explicit-unbound optional task point preserves the independent resource
chain. Selected six plus legacy nine gates passed task `6544221ke4`; actual
native crate tests `7811838tkb` passed data-structures 159, session 76 and pool
111 tests, plus integration/doctests. Hosted client test run
`20261014-154624496-2726352` passed 587 tests, with check
`20261014-154621239-2725646` and strict all-target lint
`20261014-154639610-2725471` also passing.

Real acceptance caught a publication bug that module checks missed: three
interface callers supplied `.json` to a basename-only writer that appends its
own suffix. Only those three literals were corrected; path safety was not
weakened. Earlier failed compiler warnings and the malformed optional-point
mutation are retained separately from final passing evidence.

Task `84781816zu` retained 64 default-path fixed-fixture ABBA controls against
the frozen prior compiler. Before/after median milliseconds were 143.906/140.511
at one thread and 86.439/89.699 at four threads with capture off, and
224.813/223.176 and 224.152/221.126 with capture on. The four-thread off median
is higher, not averaged away. Quiet provenance is unknown. These controls do
not measure active resource-ledger overhead or establish zero overhead, a
session-wide speedup or default promotion. Resource fixture off/on prepares the
same ledger and checks publication neutrality only. Full lifecycle/resource/
pipeline completeness remains false, CPU/span remain null, and actual rare IO
fault injection and helper-quiescent teardown remain unproven. Detailed checks
and public output are in `docs/native-resource-prefix-results-20261014.md`.

## 2026-10-15: hosted workspace-cache admission and target-bound reuse

Hosted attributed telemetry shows approximately 97.8% observed unit reuse when
Cargo-fresh units are included, versus 2.1% workspace-cache lookup hits. These
are not lifetime or compute-weighted figures. Removed the invalid two-stable-env
variant admission cutoff and added an opt-in exact-target/profile artifact tier
(`JUSTRUST_LOCALCACHE_TARGET_BOUND=1`) without relaxing portable path guards.
Pinned integration retouch changed from 0/3 to 5/5 cache hits. Suite comparison
reports 0.77s to 0.49s (-37%), explicitly uncontrolled/noisy, not a broad speedup.
Hosted reuse run `20261014-175613844-4021580` passed 22 tests with one cache hit;
full hosted gate `20261014-175314579-3984435` passed 600 tests. Installed locally.
Comprehensive baseline timed out at 600 seconds, so the tier remains opt-in.
Details, risks, provenance and incomplete raw unrecorded probe are documented in
`docs/cache-reuse-20261015.md`; cited exported runs accompany the measurements.

## 2026-10-15: measured exclusive frontend CPU remains a partial graph

Opt-in query5/pool3 timing now shares the exact query clock and records worker-local
exclusive CPU segments across task replacement, dispatch, and continuations.
Hosted validation `20261014-181945238-10320` passed 621 tests cleanly, native
crate/integration gates passed 216 tests (28 ignored), and the unchanged real
itoa suite passed all four default/large/timed artifact-neutrality tests.
The cold capture measured 198.099 ms retained observed CPU, including 131.956 ms
opaque CPU (66.61%). Warm measured 38.733 ms, including 1.665 ms opaque (4.30%).
These are scope-qualified observations, not whole-frontend shares or speedups.
Cold task binding still has 392,883 unknown points of 485,881, despite zero
read/binding record loss. Clock alignment and CPU samples do not close semantic
query-call provenance or establish a weighted critical path/thread ceiling.
Reports, exact hashes, acceptance controls and replay commands are in
`docs/frontend-cpu-timing-20261015.md`; four cited client run exports are in
`data/runs/frontend-timing/2026-10.jsonl`. Full CPU work/span remain unknown.

## 2026-10-15: actual cache-result availability graph, not an execution DAG

The opt-in consumer `graph --cache-call-provenance` maps existing original
publication/cache-hit/request events without changing native compilation or
legacy semantic-read metrics. Real retained cold/warm inputs produced 154,017
and 3,580 certified event relations, with 77,734 and 4,458 unresolved requests.
All rows were independently checked against original stream positions and
admitted endpoints. Default reports replay exactly and input hashes are unchanged.
The analyzer retains full private graph stdout outside the source capture and
commits compact reports with exact artifact hashes in
`bench/results/cache-call-provenance-20261015/`.

Hosted check `20261014-184331332-213278`, full test
`20261014-184334962-213547` (630 passed, three ignored), and warning-free clippy
`20261014-184359637-215503` passed. Forty analyzer tests and nine actual retained
acceptance/oracle tests passed, with no acceptance skips. Slim run summaries
are in `data/runs/cache-call/2026-10.jsonl`. Exact artifact paths, SHA-256 values,
reason partitions, controls and reproduction command are in
`docs/cache-call-provenance-20261015.md`.

This closes a useful cache-hit provenance slice, not dispatch/result semantics,
opaque work attribution, weighted critical path, or a 96-core/thread ceiling.
Recorder stream order is not a chain of execution dependencies. No speedup or
full semantic dependency coverage is claimed.

## 2026-10-15: exact lexical dispatch-result availability, performance gate open

Explicit root-6/read-2 capture now retains the existing nonincremental dispatch
return index on its actual lexical request, after normal result conversion.
It records only `resolved_dispatch`, not fresh-provider, race, blocked-wait or
cycle classifications. Strict consumer certification requires a unique prior
publication, exact context, admitted endpoints and complete source authority.
Cache-hit and dispatch-result assessments remain separate event relations,
not whole-job execution edges or a complete semantic dependency graph.

Hosted check `20261014-190255435-340799`, six integration tests
`20261014-190300362-341711`, full test `20261014-190307525-343324`
(644 passed, three ignored), and warning-free clippy
`20261014-190329579-347280` passed. Exported summaries are in
`data/runs/dispatch-result/2026-10.jsonl`. Native primary suites passed
223 tests with 28 ignored, plus one selected short-ice run-make test.
The real-source six-test acceptance passed without skips: cold one/four-thread
captures certified 17,569/17,669 dispatch relations, warm incremental captures
correctly recorded none for this nonincremental-only feature, and mixed timing
certified 17,603. Cold whole-archive and warm object/rmeta/consumer neutrality
passed. Warm whole-archive identity is explicitly not claimed.

The first comprehensive baseline attempt had an incomplete assembled sysroot:
five preparation builds failed with missing std/core before any valid measured
samples, then the run stopped with exit 130. A matching-runtime recovery passed
a std smoke test but the real workspace exposed missing proc_macro, so that
attempt was also stopped. Both failures and their logs remain preserved.

Fresh isolated A2/B2 source reconstructions subsequently built complete native
sysroots, passed proc-macro compile/load, std execution and libtest, and each
passed the pinned 895-unit Desktop release preflight. Independent review
verified source, config, cache, prefix and selected-toolchain identities.
These preflights are setup/correctness evidence, not speed comparisons.

The attested A2 seed-1/40-point baseline completed, but its own eligibility
checks report contention and unexpected test outcomes. For example, run
`20261014-230759120-1788897944` exited 101 with nine failed tests where success
was expected. A zero suite-process exit does not make those samples valid.
The candidate measurement is withheld pending a usable baseline, and the
comprehensive comparison remains blocked rather than replaced by a narrow
fixture. No speedup, zero default overhead, full CPU/span or useful-thread
ceiling follows. See `docs/dispatch-result-provenance-results-20261015.md`.

## 2026-10-16: Desktop scripts invalidate a large UI build unit

Desktop Windows acceptance script-only validation run
`20261016-182406835-2625400` used hosted `justrust test --workspace`:
29.1 s wall, 20.91 s compile, 3.04 s tests, 1,855 passed. The report
attributed 9.6 s to the 125k-line `jcode_desktop_ui` units (6.85 s test,
2.79 s library) and noted that `scripts` is a UI build-script input, so
script edits invalidate the large UI crate and one dependent. It estimated
1.4 s per edit across six recorded runs. Concurrent runtime changes also
invalidated 32 local units, so this run is not an isolated script benchmark.
Potential project-side follow-up: narrow the build script's tracked inputs
or move embedded script data into a leaf crate. No crate restructuring or
speedup is claimed for the Windows release validation task.

## 2026-10-16: Jcode documentation edits rebuild app-core and TUI

During tier-switch validation, hosted run `20261016-182521076-2649361`
(`justrust check -p jcode-tui`) took 4.2s: app-core 2.12s and TUI 1.84s.
The trigger was `edited crates/jcode-app-core/../../docs`, consumed by the
app-core build script. Follow-up test build `20261016-182527652-2650946`
spent 6.5s before failing on a test assertion type error, including 3.15s
app-core and 3.03s TUI compilation with the same docs dependency trigger.
This is a project architecture opportunity: narrow the build-script input or
move embedded docs into a small leaf crate. No restructuring was attempted
while finishing the user's tier-switch task, and no speedup is claimed.

## 2026-10-17: Jcode cache-monitor test build and concurrent lock cost

Hosted run `20261017-032644449-3692707` (`justrust test -p jcode-base
kv_cache_monitor --lib`) passed 12 tests in 13.04s runner wall time. Compilation
took 12.77s, including 8.05s for the 1,815-test base binary and 4.36s for the
base library, after an upstream compaction-core change. The report attributed
12.4s to the 143k-line base crate and its test unit. A project-side leaf test
crate or narrower cache-monitor module boundary could reduce this edit loop,
but no unrequested restructuring was attempted.

Hosted agent regression run `20261017-032743310-3698493` passed its selected
test in 0.16s but spent 11.10s of 11.38s runner wall waiting for the shared
target-directory lock during concurrent swarm tests. It compiled no units.
Coordinate related test requests to reduce queueing where practical. This is
measurement evidence only, not a demonstrated speedup or a billing workaround.

Cache-boundary and cancellation validation also measured hosted check run
`20261017-032709332-3695295` at 27.94s runner wall, of which 20.85s was the
shared target lock and 6.85s compilation. TUI cache-accounting run
`20261017-032758080-3699930` spent 21.64s of 27.85s waiting for that lock.
Final hosted `--lib` cache regression run `20261017-033420733-3765844` passed
97 tests in 12.43s, with 0.39s lock wait, 4.46s app-core compilation, and
6.91s for the 233k-line TUI test unit after an upstream turn_execution edit.
Its tests took 0.38s. These observations favor coordinated test scheduling and
project-side narrower crate boundaries, not more test parallelism within a
locked target directory. No architecture change or speedup is claimed.

Account-status validation provides another measured example: hosted base run
`20261017-035830007-3964345` passed one cache test in 31.8s runner wall,
including 23.62s target lock wait and 7.98s compilation. Subsequent router run
`20261017-035903607-3964344` passed seven tests in 18.6s, with 6.27s lock wait
and 12.01s compilation (6.71s for the 234k-line TUI test unit). Multiple
independent swarms were changing shared upstream crates. Cross-swarm test
scheduling and smaller project test units remain candidate improvements,
not measured speedups. No architecture change was made for this UI task.

## 2026-10-17: hosted tests see the builder's `CARGO_MANIFEST_DIR`

jcode `server::debug_command_exec::tests::debug_tool_selfdev_reload_returns_promptly_for_direct_execution`
fails on hosted (`justrust test`, "Could not find jcode repository directory")
and passes with `--local`. The code resolves its repo via
`env!("CARGO_MANIFEST_DIR")`, which on hosted is the remote builder's path, so
any test that walks up from the compile-time manifest dir to find the checkout
breaks. Fix direction: remap the builder path back to the client workspace
path for compile-time env (or `--remap-path-prefix` plus rewriting
`CARGO_MANIFEST_DIR`), so hosted test behavior matches local.

## 2026-10-17: concurrent quota-refresh validation lock contention

Jcode hosted check `20261017-035643921-3954930` passed in 54.2s wall,
with 43.31s waiting for the shared target lock and 8.20s compilation.
Focused quota tests `20261017-035843112-3965467` passed five tests in 29.5s
wall, including 17.55s lock wait and 8.95s compilation. The remote snapshot
mirror test `20261017-035935829-3969952` passed in 26.0s wall, with 15.27s
lock wait and 7.38s compilation. Concurrent swarms serializing on the same
hosted target dominate these small validation requests. Coordinate related
checks rather than adding competing requests. Large base/app-core test units
also account for 6.9s/8.1s compilation here. No project restructuring or
unmeasured speedup is claimed.

## 2026-10-17: hosted test compiled against stale jcode-base source (unresolved)

Three hosted `justrust test` runs started within 10s of each other on
/home/jeremy/jcode (same agent session) failed with
`E0425: cannot find function request_compression_mode in jcode_base::provider`,
although the function existed locally and a hosted `justrust check` of the
same packages (`20261017-022608522-3089046`) had just passed.

- `20261017-022647728-3097126`: cargo reported jcode-base Dirty
  (`provider/mod.rs` changed, mtime 1s after last build) and recompiled it,
  yet dependents still could not see the new function.
- `20261017-022650638-3097326`: same error.
- `20261017-022657688-3098021`: jcode-base Fresh, same error.
- Local reruns (`--local`) of the same commands passed (272 tests).

No git checkout, stash, or reset of those files happened in the window
(reflog only shows unrelated commits). Sync logs show ordinary 1.3-2.9s
pushes. Hypotheses not yet tested: (1) concurrent hosted runs sharing one
target dir/slot, where a parallel `check` wrote metadata-only rmeta for
jcode-base that a test build then linked; (2) the push for provider/mod.rs
raced the run's barrier. Next step: reproduce by editing a base crate and
launching `check` and `test` concurrently on hosted.

### Follow-up: likely root cause found and fixed in localcache

`localcache::stage` hashed a unit's dep-info inputs *after* rustc finished.
A source written while rustc ran (here: a hosted sync push of
`provider/mod.rs` landing mid-build, mtime 1s after the last build) was
hashed with its new content and paired with outputs compiled from the old
content. Every later build of the new content then hit that entry and
restored the stale `jcode_base` (`20261017-022657688-3098021` shows
`depcache: "local"` for jcode_base, saved 7.8s). Fix: refuse to stage when
any non-target input's mtime is at or after the compile start. Regression
test `source_changed_during_compile_is_not_stored` fails before, passes
after. A scratch 2-crate concurrent repro (8 rounds) did not hit the race,
so the end-to-end trigger is inferred from the run records, not reproduced.

## 2026-10-18: hosted build host disk full from never-evicted mirrored target dirs

The flagship hosted host (m8a.24xlarge, 484G root) hit 100% disk and builds
failed with "No space left on device" while writing target fingerprints. The
remote mirror keeps a `target/` per synced root forever: 373G sat under
/home/jeremy, with 180G in mirrored `~/.jcode/scratch/*` worktrees, many of
which no longer exist locally (deleted review/validation worktrees). Manually
removing 18 stale scratch `target/` dirs freed 67G (100% to 87%). Fix needed
in the client/host: evict remote `target/` for roots whose local directory is
gone or idle beyond N days, plus an LRU sweep with a free-space floor before
each build. Report disk-full as an explicit hosted failure that falls back to
local instead of a cargo I/O error.

## 2026-10-19: hosted `justrust test -p jcode-tui --lib cache` hung >15 min

- Repo: jcode. Run started ~09:59 PT, produced no output, left several `[justrust] <defunct>` children.
- Same command with `--local` finished in 58.5s (137 tests). Hosted `-p jcode-base` run just before was fine (21.8s).
- Suspect: hosted client never reaped/terminated after the remote side stalled; needs a stall timeout + fallback.

## 2026-10-19 15:00 UTC: hosted disk refusal recovered without touching source or caches

Cause: duplicate isolated verification exports each retained their own generated
Cargo target tree. Several compiler snapshots had roughly 17 GiB targets, with
additional old scratch and benchmark outputs on the same 484 GiB root filesystem.
The reported 0.1 GiB free caused admission refusal and an incremental ENOSPC.
Inodes were only 3% used. This was byte capacity, not inode exhaustion.

Read both repository AGENTS.md files, remote command help, the previous hosted
recovery documentation, and `src/remote_evict.rs`/`src/remote_agent.rs` before
maintenance. Existing GC is dry by default, with per-root flock coordination,
process-use checks, canonical target authority, and a two-hour idle grace.
No GC policy, client binary, daemon, billing or isolation setting was changed.

- Mizaru independently removed its completed `vm/target`, `parent-range/target`
  and `staged-range/target` under `/home/jeremy/.jcode/scratch/` at 15:01:28 UTC,
  before receiving coordination. Owner reported about 18.5 GiB reclaimed.
  This investigation independently measured 19,497,697,280 bytes free afterward.
- Rose authorized only inactive generated targets for further cleanup. Puppy
  explicitly confirmed hosted `realloc-verify/target` disposable while retaining
  the source snapshot and its separate local validation. Departed-owner
  `budget1/target` had no active references and its newest artifact was 9,605
  seconds old, exceeding the two-hour grace.
- At 15:04:55-57 UTC, removed only those two hosted target directories under
  `/home/jeremy/.jcode/scratch/`. Used an incident-scoped dry-run/apply helper
  following the stable GC's exact FNV per-root lock convention, acquired both
  exclusive locks, checked canonical real paths, same filesystem, Cargo output
  markers and absence of process cwd/fd/executable/maps references. Only the
  individually identified boot-time systemd, sd-pam and sshd control processes
  had unreadable references, treated consistently with existing GC behavior.
  No unknown unreadable process was ignored. Source manifests and src directories
  remained present and target directories were independently confirmed absent.
- Free bytes immediately before/after this cleanup: **8,196,980,736 ->
  33,567,416,320**, a net **25,370,435,584 bytes (23.63 GiB)** recovered.
  At 15:05:28 UTC, free space was **61,288,054,784 bytes (57.08 GiB)**.
  The later increase includes other or unexplained releases, not this cleanup.
  Active hm-verify, tigress-verify, staged-rev and old running test outputs were
  preserved. No source, account data, private/shared cache or uncertain directory
  was deleted. No worktrees were created and no compiler was installed.

Real authenticated hosted acceptance used a small dependency-free scratch crate:
`check --remote` run `20261019-080256232-1442462` and `test --remote --lib` run
`20261019-080257422-1442533` passed after the owner's initial recovery. After the
two-target cleanup, an edited source forced fresh compilation: check
`20261019-080525656-1460678` and test `20261019-080527350-1460875` both exited 0,
the latter with one test passed. Both `justrust show` reports explicitly said
`where remote on hosted` and reported rebuilding the edited `src/lib.rs`.
Only these four exported run summaries are retained in
`data/runs/xps14-hosted-recovery-20261019/2026-10.jsonl`.

Rose accepted recovery. Resume one substantial hosted validation at a time,
reuse a coordinated target rather than spawning fresh full target trees per
verification export, and get owner confirmation before retiring generated
outputs. Additional owner-cleared staged-rev output was deliberately left alone
once capacity was sufficient. This is incident recovery, not an automatic GC
promotion or a compiler correctness/performance claim.

## Hosted ENOSPC receipts must trigger cross-workspace source retries (2026-10-19)

After hosted disk capacity recovered, the long-lived agent could still reject
healthy builds with three outstanding source-write failures. The agent correctly
retains failed paths across barriers until those operations actually succeed.
The daemon marked each reported path `UNKNOWN`, but a subsequent run only pushed
its requested workspaces. Failed paths in an idle workspace were never resent,
so the connection-wide barrier continued rejecting unrelated builds indefinitely.
Freeing disk capacity alone could not repair that source mismatch.

Before a run, the daemon now retries roots containing `UNKNOWN` entries through
the ordinary source diff/write/delete path, including failed deletes. Unchanged
roots have no additional source transfers. An unavailable local root, changed
non-regular failed source, or unreadable retry remains an error. The agent's
global barrier and launch rejection are unchanged: sending a retry is not proof
of success. Disk-full diagnostics now include up to three actual failed paths,
and the daemon logs the root whose rejected operations it retries. No lease,
tenant isolation, billing, cache deletion or source-authentication rule changes.

Validation used the existing stable target and explicit local mode because the
hosted source-sync outage was the defect under repair:

- Check: `20261019-135357559-419858`, passed in 12.4 s.
- Daemon regression: `20261019-135411129-421191`, passed. Captures the exact
  current write payload and failed delete for workspace A while unchanged B
  emits no writes, preserves rejection on a repeated failure receipt, and
  rejects unavailable/type-changed A rather than authorizing stale source.
- Final full suite: `20261019-135653052-448660`, **745 passed, 3 ignored**,
  24.4 s. Includes a real-agent protocol regression that induces genuine
  ENOSPC through a test-owned atomic scratch symlink to `/dev/full`, without
  filling any filesystem. A's old source stays intact, unchanged B is rejected
  twice even after capacity is healthy, and only the exact A rewrite permits
  B's real Cargo check on that same agent. No restart, `Synced` reset or ignored
  failure is involved.
- `justrust clippy --local --all-targets -- -D warnings`:
  `20261019-135718494-454986`, passed in 22.0 s. `cargo fmt --all` passed.

These four slim summaries are in
`data/runs/xps14-hosted-sync-recovery-20261019/2026-10.jsonl`. This is a
correctness/recovery change, not a measured build-speed claim. Production
installation and a single frozen authenticated hosted canary are coordinated
with the root agent after the fleet is paused and the agent is idle.

Deployment acceptance (2026-10-19, UTC): root approved stable installation after
an authenticated process audit showed no hosted Cargo/rustc jobs. Commit
`376ffaea2` was pushed to `stable`, then `cargo install --path . --locked`
completed from this checkout at 21:01:11. Installed `justrust 0.1.0-alpha.18`
SHA-256:
`1c02f47aeee0759678a58f9b14f1f1cb9ac752163c25206df487b661f86deb74`.
At 21:01:20, the existing graceful version-identity upgrade logged old daemon
3474503's `shutdown requested` and new daemon 473981's startup. It connected to
the current authenticated allocation, `35.89.137.204` (host private address
172.31.32.241), at 21:01:23. No forced kill, source-failure reset command, lease
change or data/cache deletion was used. The earlier incident handoff's public
address was stale, so all operations used the authenticated client route.

The root coordinator authorized raccoon's single check of its frozen owned
`pattern-readyiso` workspace. Run **`20261019-140407801-491668`** exited **0**,
and `justrust show` explicitly reports **`where remote on hosted`**, with zero
compiled units and two fresh units (0.04 s remote Cargo wall time). It did not
fall back locally. The root's authenticated disk measurement at 21:04:41 found
**24,618,397,696 bytes available**. The canary's slim, privacy-redacted summary
is appended to the same scoped evidence file, now containing five records.

This live canary verifies installed stable deployment and the actual hosted
route on an unchanged owned workspace. A graceful upgrade opens a fresh agent
connection, so it does **not** establish that the old live connection replayed
its outstanding failed writes. That exact, same-agent ENOSPC recovery and
fail-closed behavior is established by the regressions above. No additional
build or daemon restart was launched for this acceptance record.

## 2026-10-19: hosted test run hung with no output
`justrust test -p jcode-base -p jcode-provider-openai-runtime -p jcode-app-core -p jcode-tui --lib` (hosted, jcode) printed nothing for 30+ minutes and had to be cancelled. The same tests with `--local` finished in 28s and 55s. Run id unknown (not in `justrust log`). Investigate a remote hang or missing timeout or heartbeat.

## 2026-10-20: hosted disk full again; GC skipped every root as InUse
The hosted build disk measured 100% (4.9 GB free of 484 GB). Every hosted
build failed or fell back locally. The automatic collector ran, but the
daemon log showed all 14 registered roots skipped as `InUse` on every pass
(263 InUse, 1333 Leased and 18844 Recent skips over the evening). No process
actually had a cwd or fd under those roots. The fail-closed check in
`procfs::path_in_use_by_other` counted the account's own non-dumpable session
processes as unknowable live use: `systemd --user`, `(sd-pam)` and the
agent's parent `sshd`, whose cwd/fd/exe return EACCES even to their owner.
The automatic mode was also still `dry`, so even a correct plan freed nothing.

Fix in c390897db: exempt exactly those processes (this process's ancestors,
and the user manager by exact argv, only when unreadable). Every other
unreadable process of the account still blocks collection. The automatic
default is now `on`. On the live machine, the fixed binary's dry plan listed
12 idle registered targets oldest-first and still skipped the 2 roots with
active leases. One `justrust remote gc --apply` freed 138,805,305,344 bytes
(4.9 GB -> 135 GB free, 73% used). Sources stayed intact, and a hosted
`justrust test --test graph_integrity` of committed main passed on hosted
afterwards. The hosted agent now runs the same binary (sha256 2989351da2ba344e...)
and its plan reports no collection needed.

## 2026-10-20: Jcode cloud Git widget build fanout

Hosted run `20261020-001709789-2905971` (`justrust build --profile selfdev
-p jcode --bin jcode`) succeeded in 41.7s end-to-end, with 34.7s build
time and 4.76s artifact transfer (58.1 MB). The report attributed 13.5s
to the oversized `jcode_app_core` (171k lines, 7.07s) and `jcode_base`
(150k lines, 6.46s) crates, despite this task changing TUI/CLI code.
It compiled 49 units (44 workspace, 3 registry, 2 build scripts), with
583 fresh. There were also concurrent unrelated workspace edits, so this
is not a controlled before/after comparison. No project restructuring
was attempted during the widget task.

Follow-up cloud status widget run `20261020-004329529-3007129`
(`justrust test --remote -p jcode-tui --lib info_widget`) passed all 152
selected tests in 8.9s hosted wall time. Tests themselves took 0.22s,
while recompiling the 242k-line `jcode_tui` test crate took 6.45s and
2282 MB peak rustc memory after one regression-test edit. The report
estimated 5.9s oversized-crate waste. This supports splitting frequently
edited UI modules in a separate scoped task, not an unmeasured claim of
a speedup or a reason to restructure during the cloud widget change.

## 2026-10-20: Todo UI validation latency and hosted startup stall

During Jcode lane-status deduplication, hosted run
`20261020-040317149-3824494` passed 4 sidebar tests but spent 19.43s
waiting for the shared build-directory lock and 9.61s compiling the
2842-test TUI binary. Broader run `20261020-040422515-3828522` passed
139 tests (6 ignored), with 15.69s compile versus 0.21s test execution.
These are concurrent-workspace measurements, not controlled comparisons.

A later `justrust test -p jcode-tui --lib todo` started at 11:06:46 UTC
and emitted no output for over two minutes. At 11:08:55 UTC its client
PID 3845451 was blocked in `unix_stream_read_generic` on the shared
`~/.justrust/remote/daemon.sock`; other clients were also waiting. The
request was cancelled without restarting the shared daemon, and final
validation switched to `justrust test --local --profile selfdev`.
The remote startup path needs bounded waits and visible progress before
its first compiler output. No cause or speedup is claimed here.

## 2026-10-22: Cloud handoff regression validation

Hosted run `20261022-111200958-3850282` passed 40 cloud-related TUI tests
in 48.5s end to end. Compilation took 44.43s, versus 0.43s running tests.
The 2866-test `jcode_tui` binary alone compiled for 15.11s, with
`jcode_app_core` at 8.35s and `jcode_base` at 6.49s. The report estimated
26.9s of oversized-crate waste and 7.6s for compiling a broad test binary
to run a small subset (overlapping estimates). A smaller TUI handoff/input
leaf crate could reduce this edit loop, but restructuring was intentionally
left out of the cloud cancellation fix. No speedup is claimed.

## 2026-10-24: Benchmark clean-cloud subscription routing fix

During end-to-end Jcode Cloud benchmark validation, run
`20261023-191330000-3061094` compiled the 1967-test `jcode_base` test
binary to execute five targeted provider regressions. Compilation of that
binary took 23.5s, with 36.6s attributed to recompiling the oversized base
crate across units. The tests themselves passed. Follow-up build
`20261023-191416861-3068827` took 41.31s wall time, including 40.63s
compilation, and rebuilt four registry dependency units (14.6s estimated
cost). `justrust show` explicitly reported local execution because remote
had failed within the prior ten minutes, despite no local flag being used.
The build produced the verified corrected candidate. No restructuring or
speedup is claimed; smaller provider regression targets and clearer remote
fallback diagnostics remain follow-up opportunities.

## 2026-10-24: Background dependency validation loop

Jcode hosted check `20261024-023329221-58932` took 18.4s, including
6.34s waiting for the target lock and 11.74s compilation. Hosted policy test
`20261024-023424414-61021` spent 6.57s on the same lock for 0.28s compilation.
Concurrent agent validation serialized at the build directory, so consolidating
checks into a coordinator-owned queue avoids this measured wait.

Hosted bash suite `20261024-023800501-69851` passed 38 tests and failed three
repository/environment-dependent tests: the cargo-wrapper test could not find
the repository ancestor, and two destructive-gate assertions saw ungated
execution. The same 41 tests passed locally in
`20261024-023843046-71583` (44.7s, 40.84s compilation, 3.14s tests). This
justified a local-only validation exception, not a hosted service outage.
The hosted self-dev build/watcher test then passed in
`20261024-023927825-71573` (13.3s hosted wall time, 9.16s test compilation,
0.58s test runtime). Repository-independent fixtures and smaller app-core
test targets would improve this loop. No restructuring or speedup is claimed.

## 2026-10-24: Cache diagnostic wording regression loop

Jcode hosted check `20261024-134807888-2329881` passed in 2.3s compiler wall
time. Hosted test `20261024-134812180-2329880` passed 23 cache-monitor tests in
13.14s wall time, with 12.88s compiling and 0.01s running tests. The base test
binary contained 1,993 tests. The command omitted `--lib`, unnecessarily building
other targets, estimated at 3.5s by justrust. Future focused invocations should
use `justrust test -p jcode-base --lib kv_cache_monitor`. A one-file base change
also rebuilt the OpenRouter runtime (0.4s). Smaller test/crate boundaries are a
project-level opportunity, not a reason to restructure during this bug fix.
No build speedup is claimed.

The follow-up cache-notice tag run `20261024-140740391-2416527` passed 38
focused TUI tests in 18.8s compiler wall time, including 18.15s compilation
and 0.31s tests. The 2,993-test TUI binary cost 9.66s rustc wall time, with
2,975 MB peak rustc memory. This is another measured candidate for narrower
test/crate boundaries, not justification for an unrequested project split.

## 2026-10-26: Cloud replacement documentation invalidates app-core

Jcode cloud replacement validation on hosted justrust passed check
`20261025-183208193-2818105` in 9.3s total and test run
`20261025-183217453-2818088` in 11.1s total (17 tests, 0.34s test time).
The latter compiled for 8.33s. Its waste report attributes approximately 5.3s
to downstream jcode-tui/jcode rebuilds because the app-core build script treats
the docs directory as an input. A cloud command documentation edit therefore
invalidated app-core and its dependents. Narrowing those build-script inputs or
moving documentation embedding to a small leaf crate is a project-level
follow-up, not part of this command change. No speedup is claimed.

## 2026-10-26: failure-heavy test confirmation, not unexplained shutdown

The clean-room run `20261025-203723357-3339712` took 98.58s. Its initial
libtest output ended after 14.88s, then the pool serially retried failed names
and confirmed apparent flakes. The final confirmation message arrived 83.69s
later. The summary called this `after build`, hiding the actual source of the
wait. The original group had 120 failures. This is distinct from recompilation:
`20261025-203611575-3336121` spent 49.71s of 50.30s compiling.

Commit `7173f6106` keeps selective serial confirmation for small, cheap failure
sets. More than eight failures, or recorded failed-test durations exceeding
twice the first group's wall time, use the existing exclusive whole-group
replay with the original libtest arguments and normal thread count. No tests
are skipped. The replay's exit status and failures remain authoritative.
Whole-group confirmation is announced before the wait. Missing retry outcomes
are no longer described as passes: only explicit successful outcomes earn that
label. Regression coverage checks the threshold and exact original argument,
filtered-count, and failure preservation.

### Measurement and validation

The matched mechanism benchmark uses pre-change source `7d523ddb4` and
candidate source `7173f6106`. The former was archived without a git worktree and
compiled explicitly; binary hashes are recorded in the result files and differ.
An initial shared-target build incorrectly reused the candidate artifact; its
identical hash exposed this, and a forced recompile produced the real baseline.
No measurements from that invalid baseline were used.

`test-retry-storm` pins fixture tree
`10d9c88270859a93825e36a48dd2d27733425ae6`: 32 tests deliberately fail after
200 ms, plus one passing test, with 16 libtest threads. Warmup records failure
history, then five no-edit measurements on each binary exercise the same
split/confirmation path. Every measured run retained exactly 32 failures and
one pass, with zero compilation units.

`bench/suite.py compare bench/results/retry-storm-before.json
bench/results/retry-storm-after.json` reports:

```text
test-retry-storm  6.87  0.85  -6.02  -88%  uncontrolled (noisy or unknown samples)
```

These are observed timings, not a clean benchmark speedup claim: two baseline
samples and one candidate sample exceeded the background CPU threshold.
Ranges were 6.861-6.889s before and 0.843-0.856s after. The quiet subsets had
medians 6.862s (three samples) and 0.848s (four samples), respectively, but the
standard comparator conservatively marks the whole pair uncontrolled.
Representative run IDs are `20261025-221853672-3965309838` and
`20261025-221917158-3969301590`. The earlier 50 ms fixture pair is retained as
`retry-storm-50ms-{before,after}.json`; two short candidate samples were noisy,
which motivated the longer signal. Do not combine the two fixture versions.
This is deliberately a synthetic failure-confirmation test, not evidence that
all builds or all rewrite tests became faster.

The real pinned rewrite snapshot is `c7bda805f8886369571bc41e3309f4fd36c0bebb`.
It differs from the active dirty checkout that produced 120 failures: the pinned
snapshot has three failing tests. Its full library suite is therefore a
real-workload correctness/regression check, not a reproduction of that failure
storm. Debug information is disabled on both sides to bound local artifact
size, with the snapshot's optimization level unchanged. The original installed
client's two warmed runs took 116.12s and 127.99s, with zero compiled units.
The candidate took 163.28s and 122.86s, both marked noisy. All four measured
runs preserved 1932 passing and three failing tests; none compiled a unit.
The changed whole-group confirmation branch was not exercised by these three
failures. `bench/suite.py compare` marks this comparison uncontrolled, not an
attributable improvement or regression. These installed-versus-candidate
numbers are diagnostic only, not an isolated attribution to this patch.

Full current-client validation: hosted `check` (`20261025-221217424-3903668`),
`test -- --test-threads=1` (`20261025-221240428-3906202`, 752 passed, three ignored),
and warning-free `clippy --all-targets -- -D warnings`
(`20261025-221320396-3903667`). An earlier normal parallel full-suite run passed
750 tests after six existing eviction/lock tests required isolated confirmation;
the serial run avoids relying on that masking for the correctness gate. Python
benchmark-harness tests also pass.

Reinstalled the latest shared stable tree with `cargo install --path . --locked`.
The installed client then passed all 12 retry-pool tests on hosted in run
`20261025-222024099-3971224` (12.2s client wall, 0.3s recorded build/test time).

The comprehensive `bench/suite.py run --synth 1 -n 1` baseline hit the execution
environment's 600-second limit after 11/40 points. Its partial results are kept
in `bench/results/test-retry-budget-before.partial.json`; they do not constitute
a completed broad gate and do not support a general session-speed claim.
Hosted execution of the pinned rewrite snapshot was also attempted, but failed
before a build summary with `cannot protect source activity: No such file or
directory`. Source protection was not bypassed. The scoped measurements use
controlled local execution instead.

### Remaining costs: shared build locks and compiler structure

`20261025-031623949-4077709` spent 49.28s of 60.59s waiting on Cargo's build-dir
lock, with only 11.26s compiling. Adjacent recorded integration-test and library
test compilations held the same target directory. This was real compile
contention, not the pool's post-test confirmation delay. Merely moving the wait
to a client queue or skipping a lock cannot safely remove it. The next hosted
change needs bounded target leases, measured artifact seeding/reflinks, exact
snapshot/intent coalescing, cancellation, and quota-aware eviction. No locking
or tenant-isolation guarantees were weakened here.

`justrust split --days 2 --top 5` observed 661 edited runs and 4482s of compilation.
`compiler::frontend::type_check` accounted for 704s over 88 runs. The tool ranked
`nominal_schema` and `root_resolution` as possible extraction sites, but those
are estimates, not measured savings. `nominal_schema` imports `super::*` and
implements parent-owned `PlanState`; moving that file alone is not a valid leaf
crate extraction. The active rewrite also had overlapping uncommitted compiler
work. No active compiler files were changed or automatically split.

## 2026-10-26: Jcode documentation edits and status timeout tests

The apparent 40-second test runs were mostly compilation or Cargo lock waits,
not arbitrary test execution. Historical runs `20261025-215608614-3735981`
(38.85s total, 34.54s compile, 2.03s tests) and
`20261025-185224604-2919084` (42.15s total, 18.00s lock, 23.45s compile,
0.41s tests) illustrate the distinction.

Pinned Jcode snapshot: `146b6f4d84a7e76f212695c89936d69fe36b3e3c`.
`bench/jcode-docs.py` uses isolated source archives, recorded candidate patches,
one warmup and three measured hosted README edits followed by
`justrust test -p jcode-tui --lib justrust`. Every measured sample retained
all 12 passing tests. Results and exact candidate patches are in the five
`bench/results/jcode-docs-*-20261026.json` files.

| Candidate | Median wall seconds | Compile seconds | Test seconds | Compiled units |
| --- | ---: | ---: | ---: | ---: |
| Original baseline | 13.19 | 9.28 | 2.02 | 2 |
| Data leaf, still a normal app dependency | 12.90 | 9.53 | 2.02 | 3 |
| Dependency inversion | 2.98 | 0.00 | 2.01 | 0 |
| Repeated original baseline | 14.52 | 9.46 | 2.02 | 2 |
| Inversion plus injected test timeout | 0.84 | 0.00 | 0.06 | 0 |

`python3 bench/suite.py compare bench/results/jcode-docs-baseline-repeat-20261026.json bench/results/jcode-docs-final-20261026.json`
reports a -13.68s observed median difference, but its verdict is **uncontrolled**
because baseline samples include contention. This is a targeted workload result,
not a general 94% speedup claim. No comprehensive synthetic default-promotion
gate was run. Final measured run IDs are `20261026-014429849-197520119`,
`20261026-014432706-197520302`, and `20261026-014433587-197520393`.

Jcode commits `21a40600d` and `07d9354d0` move bundled document data to a
CLI-only normal dependency, explicitly register it with app-core at startup,
and keep it a dev dependency for app-core tests. The rejected normal-dependency
leaf did not eliminate downstream recompilation. The timeout test still runs
the real subprocess/deadline path, with a 50ms injected deadline instead of
waiting for the unchanged two-second production deadline.

Validation: app documentation tests (3), production registration test (1),
status tests (12), and exact corpus tests (2) passed on hosted. Adding and
removing a documentation file in the isolated snapshot also passed corpus
freshness tests; removal run `20261026-015015947-271744`. Data/app-core strict
clippy passed (`20261026-015017330-271743`). Broader Jcode lint remains blocked
by pre-existing collapsible-if/needless-borrow findings in unrelated root and
tool-display files. The justrust serial suite passed 770 tests, with 3 ignored
(`20261026-014748680-254328`), and the client was reinstalled.

Hosted Cargo lock contention remains unresolved: the inspected 97GB shared
target is on ext4 and its reflink probe fails. Commit `f07a3941c` explains the
safe shared-target fallback instead of silently implying build-slot isolation.
The installed diagnostic appeared in the two validation runs above. No storage
migration, unsafe target copying, or change to cache isolation was made.

### 2026-10-26: hosted Jcode now meets the glibc 2.17 baseline

The explicit Zig target `x86_64-unknown-linux-gnu.2.17` built full-feature
Jcode at `b411a7207` on hosted, with optimization level 3 and incremental off.
Run `20261026-150025317-548870` took 142.88s build wall and 157.8s client
including sync/transfer. The highest imported GLIBC version is 2.17, and the
downloaded binary passed version plus four cloud handoff help commands in the
pinned manylinux2014 container. The prior native-host artifact required 2.39
and failed in the same container. This is a compatibility fix, not a speedup
claim or a full release-matrix promotion.

Warm run `20261026-150308875-562781` compiled no units (611 fresh), reused the
same SHA-256 artifact, and completed in 3.4s client time. It passed the baseline
runtime checks again. Native validation checks ELF GLIBC version requirements
before atomic replacement and before advertising cached artifacts for reuse.
Hosted regression `20261026-145854935-539475` passed 788 tests with 4 ignored.
Commands, hashes, remaining platform gates and the reusable container check
are in `docs/hosted-glibc217.md` and `ci/check-jcode-glibc217.sh`.

## Hosted ARM64 Jcode release compilation (2026-10-26)

Full-feature Jcode at `eaba9240aa4a02401a113debb3cce463bf335074` now cross-builds
on hosted Linux x86_64 using pinned GNU tools, rebuilt std and emulated TLS.
The first run `20261026-151916181-673081` exposed missing cross OpenSSL;
the existing vendored-OpenSSL feature fixes this without disabling PDF/Bedrock.

Run `20261026-152034595-679135` took 139.78s build wall time, 154.31s including
the trial script. Warm run `20261026-152323026-695569` had 630 fresh units,
zero compiled, and 2.2s hosted client time with the same artifact SHA256.
The executable is AArch64 ELF64 with no PT_TLS and requires GLIBC_2.39.
Native execution, Termux compatibility and release publication remain untested.
See `docs/jcode-arm64-hosted-trial-20261026.md` for exact flags and artifact hash.

## Known skipped-work accounting (2026-10-26)

`justrust savings` now sums original compiler/build-script task durations minus
measured cache-hit overhead across retained recordings. It separates dependency,
workspace and build-script hits, deduplicates run IDs, preserves negative savings,
and reports missing recordings, incomplete historical overhead and malformed rows.
`--days`, `--here` and `--json` restrict or export the accounting. New summaries
persist the totals; `show` also reads surviving raw records from older runs.
This metric is task elapsed time, not CPU seconds, wall-clock wait avoided, a
stock-toolchain comparison, or a lifetime/fleet counter. Misses, uploads and other
build costs are not deducted. Original durations can come from other hardware.

The local all-retained report at 15:29 PDT found 376631.77s estimated task time
avoided: 401583.78s original less 24952.01s measured hit overhead. Coverage was
25304/36570 indexed runs, with 11266 unavailable and 322617 hits missing full
historical overhead. These are incomplete historical estimates, not speed claims.
The last-seven-day snapshot at 15:27 PDT was 13590.28s with 7637/10543 runs covered.

Real Desktop test-edit run `20261026-152734282-724189` passed all 3 fps_counter
tests and recorded 29 dependency hits, 56.56s original task time, 27.19s complete
hit-path overhead and 29.37s estimated task time avoided. The build took 34.6s
(50.1s client wall). Concurrent unrelated source changes also rebuilt dependencies,
so this is a workflow/instrumentation check, not an isolated speed benchmark.
The one-line test-message edit was restored byte-for-byte from a backup.
Final hosted regression `20261026-152803445-727731` passed 813 tests (6 ignored);
`20261026-152814413-729091` passed clippy with `-D warnings`. Tests cover real
CLI filtering/deduplication, retained summaries, old/missing timing, negative
estimates, and actual native cache-hit overhead with unchanged executable output.
A local-only stock-path fixture used `JUSTRUST_TOOLCHAIN=system`,
`JUSTRUST_CARGO_FORK=0`, `JUSTRUST_SLOTS=0`, `JUSTRUST_PASSES=0` and one fixed
`CARGO_TARGET_DIR`: recorded warmup `20261026-152957698-740463`, unrecorded
`JUSTRUST_DISABLE=1` replay (Cargo reported Fresh), then recorded replay
`20261026-152958194-740614` (zero compiled, one fresh). This isolates recording
from toolchain and slot changes; local execution was required to compare the
same on-disk Cargo artifacts. No comprehensive performance claim is made.

## Jcode complete-blocker edit loop (2026-10-26)

Hosted run `20261026-175614870-1640997` (`justrust test -p jcode-app-core
-p jcode-tui -p jcode completely_blocked`) passed five matching tests. The client
reported 55.4s wall time, including 4.48s source sync. Compiler accounting showed
30.22s waiting for the shared target-directory lock and 19.43s compilation.
Build slots were disabled because the reflink probe failed. `justrust show`
confirmed `where remote on hosted`, 32 rebuilt local units and 597 fresh units.
The app-core test unit took 12.25s attributed build time (19.3s unit wall), 49.0s
CPU and 4653 MB peak memory. The report estimated 17.9s downstream rebuild cost
from shared upstream changes and 10.9s oversized app-core cost. No crate split
was attempted because it is outside the blocker change. Warm follow-up run
`20261026-175747020-1658922` passed eight UI blocker tests with no compiled units
in 2.9s client wall time, mostly its 2.14s sync. These are observations, not a
controlled speedup comparison. Investigate the hosted reflink probe/slot fallback
to avoid shared-target lock serialization before restructuring Jcode crates.

## Hosted ARM build-std sources are host-local (2026-10-27 UTC)

Jcode native-handoff GitHub Actions run `40429328779` failed because the
submitting runner installed nightly-2026-10-08 with rust-src, but the hosted
machine auto-installed only rustup's minimal compiler profile. Cargo rejected
the missing `lib/rustlib/src/rust/library/Cargo.lock` before compiling any unit.
Installing rust-src on the runner does not provision the executing host.

Fix `a7e7d02cb` provisions matching rust-src while configuring native GNU ARM64
build-std on the executing machine. It resolves the selected compiler's sysroot,
uses its exact rustup distribution toolchain name, skips an already complete
installation, and verifies Cargo.lock after installation. Owned/custom compiler
sysroots without sources fail with an explicit diagnostic instead of borrowing
stock sources. Regression fixtures cover exact selection, idempotence, install
failure and rejection of owned/custom source provisioning.

Hosted validation passed: check `20261026-183824528-1929852` (7.4s build,
56.5s client), full test `20261026-183926987-1936533` (821 passed, 6 ignored,
23.7s build/test, 25.2s client), clippy with `-D warnings`
`20261026-183952204-1936531` (7.5s build, 9.0s client). Reinstalled locally.
These are correctness observations, not a controlled speedup claim.

Jcode commit `30b451186` synchronizes the workflow checkout and provenance pin
to the fix, with all 25 native-handoff Python tests passing. Its real dispatch
`40434055378`, `lane=linux-arm64`, `sign_macos=false`, succeeded. Hosted logs
at 01:43:41 UTC explicitly show `downloading component rust-src`; the release
build finished in 159.5s (178.8s hosted client wall), returned one 52.5 MB
artifact, and provenance sealing passed. Same-run artifact `12703870249` was
downloaded on `ubuntu-24.04-arm`; native architecture/provenance checks and
binary smoke passed at 01:46:49 UTC. The entire run succeeded and sign-macos
was skipped. No release publication, signing change or secret change occurred.

## Expensive failure confirmations serialized a 2,861.5s test call (2026-10-27)

The reported operation is exactly `20261027-093514646-611705`,
`cargo test --no-fail-fast` in the rewrite checkout. Its hosted summary records
`remote.client_wall=2861.465543838s`, `sync_ms=60027.733419`, and
`wall=2798.482s`. This is the complete suite command, not an individual test
or a "fail-fast" phase. The run reported 4,843 passes and 178 failures.
Only 115.44s was attributed to rustc, with 2,304.35s labeled build gaps and
376.76s attributed to tests. The gaps are not proof that the host was idle:
test-pool confirmation work is not all represented by those top-level phases.

The output explicitly shows exclusive whole-group confirmations for
`full_programs`, `vec_algorithms`, `projected_callable_bounds`,
`generated_doc_metadata`, `payload_enums`, `projected_reference_returns`,
`projection_defaults`, and `temp_places`. Every expensive failure group
previously acquired a writer-preferring exclusive gate immediately, blocking
other tests and serializing even deterministic failures. Cheap failure sets
already had a shared quick-check path.

Expensive groups now replay their original arguments and libtest parallelism
under the shared gate first. If every initially failed name fails again, retain
the original failure and diagnostics without taking the exclusive gate. If any
failure disappears, retain the existing isolated confirmation. Crashes with no
named failures still go directly to isolated replay. The bounded shared-holder
regression test verifies that a repeatable expensive failure completes without
waiting for unrelated work, and the existing large-group test verifies original
filters, exit status, filtered counts, and isolated fallback.

Pinned `bench/suite.py compare` results, local controlled scheduler execution:

- `20261027-retry-contention-before.json` versus `...-after.json`, three samples:
  median **3.666s -> 0.852s**, displayed **-77%**. Every sample preserves all
  **72 failures and eight passing controls**. Representative recorded runs:
  `20261027-165801153-1971556209` and `20261027-165818795-1993697720`.
- `20261027-shared-confirm-before.json` versus `...-after.json`, one sample:
  retry-storm **0.853s -> 0.916s**, preserving 32 failures and one pass.
  Pinned rewrite library suite **129.528s -> 150.128s**, preserving 1,932 passes,
  three failures and one ignored test. Runs `20261027-165543813-1939540911`
  and `20261027-170113601-1993693849`. No improvement is established for this
  real-workload diagnostic, and the observed regression must not be omitted.

All comparisons are explicitly **uncontrolled (noisy or unknown samples)**:
other agents were using the machine, with different contention on each side.
The contention fixture isolates the removed serialization mechanism, not a
prediction that the original 47-minute invocation will improve by 77%.
The exact full invocation was not rerun against the actively edited rewrite.
The comprehensive session-weighted synthetic comparison was not run, so this
is a scoped retry-scheduling fix, not a general justrust speedup claim.

Hosted correctness: check `20261027-165253796-1937504`, test-pool regression
suite `20261027-165402408-1937440` (13 passed), full suite
`20261027-165441404-1956988` (822 passed, six ignored), warning-free clippy
`20261027-165452950-1958972`, and release build
`20261027-165505143-1956983` all passed. Benchmark infrastructure's 33 Python
tests and `cargo fmt --check` passed. Exported run summaries accompany the
four committed benchmark result files.

Reinstalled with `cargo install --path . --locked`. A subsequent forced-hosted
contention smoke attempt (`20261027-170626197-2067747668`) was blocked before
the build by `cannot protect source activity: No such file or directory`.
There was no summary and no hosted performance result. Activity protection
was not bypassed or weakened. Earlier hosted source tests/builds above passed,
but acceptance of the newly installed client on this fixture is blocked by
that separate source-root registry problem.

The first push was rejected because October's exported JSONL crossed GitHub's
100 MB limit. Split its first 20,000 complete lines into
`2026-10-archive-1.jsonl`, keeping later lines in `2026-10.jsonl`. Concatenation
was byte-for-byte identical to the pre-split backup. Export deduplication
already reads all JSONL siblings, so no run records were dropped or duplicated.

### Hosted follow-through: benchmark source placement (2026-10-27)

Resolved the installed-client smoke blocker without changing GC or tenant
security. The daemon's authenticated sync log identifies the actual cause:
`target GC enrollment refused ... root outside authorized source scope` for
the fixture under `~/.justrust/bench/work`. The receipt registry intentionally
refuses any source root with a `.justrust`, `.cargo`, `.git`, or `target` path
component. Failure to register then correctly prevents the build, although the
follow-on missing-receipt error obscured the original reason.

The suite now selects `~/.cache/justrust-bench/work` for `--remote` snapshots,
retaining the previous default for local runs and preserving explicit
`JUSTRUST_BENCH_WORK` overrides. This fixes benchmark placement rather than
relaxing the protected-state exclusion. Three regression tests cover default
selection, overrides, and actual command-mode wiring. All 36 suite Python tests
pass. Existing wrappers that assign `suite.WORK` and call `run_scenario`
directly retain their own workspace handling.

Installed-client forced-hosted acceptance passed with all 72 expected failures
and eight passing controls, no local fallback. Three quiet samples in
`20261027-retry-contention-hosted-acceptance.json`:

- `20261027-171019802-2089987823`: 2.445s client / 0.825s host.
- `20261027-171022269-2089987488`: 1.439s client / 0.825s host.
- `20261027-171023737-2089987142`: 1.450s client / 0.823s host.

After the harness fix, running the same command with **no workspace override**
also passed: `20261027-171117187-2095552310`, saved in
`20261027-retry-contention-hosted-default.json` (1.43s client, 0.82s host).
`justrust show` confirms `where remote on hosted`, eight binaries with exactly
nine failing tests and one passing test each. These are acceptance observations,
not hosted before/after speed comparisons.

Investigated the earlier noisy rewrite comparison rather than attributing its
regression to the fix: its longest **passing** test,
`bounded_panic_family_exact_grammar_and_diagnostics`, increased from 116.613s
to 138.887s, compared with a 20.600s overall increase. Neither run logged an
expensive whole-group isolated confirmation. This is consistent with changed
contention in the identical pinned compiler test binary, not proof of a cause.
The prior caveat remains: no full-suite speedup or comprehensive synthetic
promotion is claimed. The installed hosted retry path is now validated, and the
previous source-placement acceptance blocker is closed.

### Hosted TUI test stall: project lock inversion and hidden warnings (2026-10-28)

At 01:55 UTC the active Jcode TUI library run
`20261027-185045233-2600539` had one test process alive for over four minutes,
with 197 threads: 96 Tokio workers and 96 waiting test threads, plus service
threads. The host had about 366 GiB available memory and load below one.
This was a blocked test process, not evidence of insufficient hosted CPU or RAM.

A bounded full-binary reproduction under an unprivileged debugger established
an environment/render-state mutex inversion. The app test
`test_remote_hydrated_catalog_restores_missing_direct_bedrock_route` held the
test environment lock through `with_temp_jcode_home`, then waited for the render
lock in `create_test_app` / `clear_test_render_state_for_tests`. Meanwhile,
`justrust_status_is_below_todos_and_never_wraps` held the render lock and waited
for the environment lock in `with_pinned_todos_config`. That repair belongs in
Jcode's test harness, not the cloud scheduler or tenant resource limits. The
original invocation was left intact while the separate reproduction was bounded
and stopped. No user data or tenant isolation settings were changed.

By 02:03 UTC both the local caller and remote agent had exited, leaving the
recorder reparented to PID 1. After checking both parent PID and exact executable,
sent SIGTERM to that orphan only. Its deadlocked test child exited, but the
recorder incorrectly began failure confirmation before exiting. The recorder
and original child were both gone by 02:04 UTC. This additionally exposed a
service lifecycle bug: disconnected agents must not orphan build recorders,
and cancellation must not initiate test retries. Follow-through is tracked
separately from the project's mutex-order repair.

Recovered the completed host summary into the original local run directory
after its caller disappeared, preserving the local hosted metadata, then
exported it. The canceled original ran for 783.61s, including 765.63s attributed
to its test binary. This is a censored hang observation, not a comparable
before/after benchmark sample.

There was also a justrust diagnostics defect: `test_pool::execute` retained
every output line until the child exited, hiding libtest's long-running test
warnings precisely when a child could not exit. Commit `78b364990` emits
attributed process-start diagnostics and standalone libtest timeout warnings
while the process is alive, without publishing provisional failures or replaying
the warning in the final block. Normal authoritative results remain buffered.

Hosted regression run `20261027-190148464-2677748` passed all 824 tests, with
six ignored: 12.1s host wall, 4.33s test execution, 13.9s client hosted wall.
Focused pool tests, check, and clippy also passed in runs
`20261027-190025678-2669713`, `20261027-185917324-2663293`, and
`20261027-190037104-2671086`. These are correctness and diagnostic observations,
not a benchmark speedup claim or a comprehensive default-promotion comparison.

Jcode fix `56f901a32` is pushed to its primary branch. The pinned-todo helper
now acquires environment before render state, matching app construction.
Shared app fixtures use two Tokio workers rather than expanding each runtime
to all 96 hosted CPUs, retaining multithreaded runtime semantics. The bounded
cross-thread regression passed, and temporarily restoring the reverse lock
order triggered its ten-second failure deadline. Restored-code hosted check
passed in `20261027-190842537-2713718`; all five pinned-todo tests passed in
`20261027-190851878-2714766`; the originally blocked Bedrock test passed in
`20261027-190639874-2703249`.

The full unsharded serialized TUI run `20261027-190642838-2703531` finished its
tests in 28.27s: 3,070 passed, three failed, 19 ignored. Those three failures
were independently reproduced with the original fixture runtimes: Muse
subscription success notice, structured session-preview alignment, and prompt
shimmer movement. The full TUI suite is therefore **not green**; eliminating
the deadlock does not establish correctness of those unrelated assertions.
Default-parallel runs also finished, but exposed additional shared-state races.

Installed-client forced-hosted acceptance `20261027-190917542-2717063` exercised
a genuine 63-second Rust test and a quick passing control. Exactly one libtest
timeout warning reached the caller at 61.52s while its process was still alive;
both tests then passed, with caller exit zero at 64.65s. This validates live
delivery across the actual hosted transport, not just an in-process sink stub.

Service follow-through `a6cfa5dc3` is pushed and installed. Linux detached
recorders receive SIGTERM when their owning agent dies, including the
parent-death setup race. Recorder cancellation is tracked before cargo exits
and prevents new pool work and failure-confirmation retries. Reaped cargo PIDs
are cleared before waiting on pooled tests, avoiding signals to recycled PIDs.
Subprocess regressions exercise owner death and cancellation of running tests.
Final hosted check, recorder tests, and warning-free clippy passed in
`20261027-191349143-2744389`, `20261027-191352602-2744973`, and
`20261027-191357642-2745358`. The final committed-source full hosted run
`20261027-191550739-2756664` passed 827 tests, with six ignored, in 8.0s host
wall and 20.0s client wall (including 11.19s synchronization).

An installed-client cancellation acceptance canceled a single-test hosted
63-second fixture immediately after its process-start diagnostic. The caller
exited on SIGTERM, the remote child was gone, and a later process inspection
found no fixture recorder or test left running. Exactly one test start and no
confirmation retry were observed. An initial two-test variant incorrectly
expected one process start despite normal sharding; both children still exited,
and the corrected single-test acceptance passed. Final installed hosted run
`20261027-191513371-2753567` additionally verified warning persistence in
`output.jsonl` before the test process exited. These fixes do not require a
cloud infrastructure deployment or a change to tenant resource limits.

## 2026-10-30: Query-retention frontier and uncontrolled Desktop pair

The scalar retention experiment now passes 66 shared-cache no/yes/no transitions
across 22 cases against fresh native references (264 compiler invocations with
incremental fingerprint verification and exact diagnostics/runtime). The empty
lint-owner setup candidate separately passes 22 cases, 88 native invocations.
These are finite correctness gates, not full-IR retention or speedup evidence.

The comprehensive scalar baseline stopped at 22/40 points with seven failures,
including pinned tool-description assertions, context-sensitive and UI failures.
A named full-test baseline also failed because DEPCACHE=0 correctly disables
the wrapper that its pinned test unconditionally unwraps. Neither was accepted.
The named harness now rejects unexpected failed tests and historical comparisons
containing them, instead of printing a note and accepting a timing result.

A prospective check-only Desktop pair completed three measured body edits per
mode with one frozen compiler/client and only the scalar flag changed. Off run
`20261029-220342124-3169100682` is among the baseline samples; on run
`20261029-220833457-3235606297` is among the candidate samples.
`bench/suite.py compare` observes 2.79s off versus 3.17s
on (+14%), explicitly **uncontrolled** because all six samples were noisy.
The first baseline sample had four foreign rustc processes and 7.67 other cores.
This does not establish a causal slowdown or improvement. No default promotion.
Exact reports and limitations are in `docs/query-retention-benchmark-20261030.md`;
62 private experiment runs were exported to the separate
`data/runs/xps14-query-retention-20261030/` directory.

### Local daemon lookup is source validation, not prebuild waiting

Installed timing correction `bbf63226e` separates recursive source validation
from prebuild coordination. Unchanged runs `20261029-221029310-3292520` and
`20261029-221030605-3292774` each kept 62 units fresh, but full process walls
were 1.213/1.276s. Source validation accounted for 951.4/1044.1ms and prebuild
coordination for 0.0ms. The old log incorrectly included scanning in its
"waiting for a compile-only prebuild" label. This proves the next local
bottleneck, not a before/after speedup. Hosted strict lint passed after install
(`20261029-221012298-3290569`). See `docs/daemon-lookup-attribution.md`.

### Candidate-free daemon execution leaves freshness with Cargo

Default-off `75e009bb0` avoids a source stamp only under an atomic globally-empty
active/finished-state reservation. Unstamped children never authorize attachment
or replay. Controlled same-binary local Clippy medians were 751.823 ms off and
173.682 ms on (three samples each, machine contention disclosed), with actual
edit/failure/backup-restore, disconnect cancellation and overlapping agent /
prebuild refusal verified. See `docs/daemon-native-empty.md` and its raw evidence.

The installed executable passed full hosted tests (843 passed, 12 ignored,
`20261029-225443553-3674420`) and strict all-target lint. Isolated installed
Clippy runs `20261029-225910643-3719519` and `20261029-225910808-3719614`
took 164.025 / 136.757 ms full wall with 62 fresh units and zero compilation;
daemon logs showed native execution and zero source validation. The first
isolated toolchain installation/rebuild took 57.554 s and is explicitly excluded
from warm observations. Shared-daemon unrelated candidates correctly retained
validation, so this is not a universal fast path. It remains default off, not a
whole-command result cache or comprehensive speedup claim. Exact installed
identity, remote-backoff/reconnection limits and cleanup are recorded in
`docs/daemon-native-empty-installed.md`.

### Native freshness can skip unrelated coordination entries

`8705f9420` adds separate default-off
`JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE=1`. Under the reservation lock it rejects
same-key active/finished entries and the exact shared same-session wrong-guess
predicate. It does not claim disjoint source roots, which the regression in
`3e1631580` proved insufficient. Native Cargo still runs and its unstamped output
cannot be attached to or replayed.

The installed binary passed actual concurrent Cargo socket acceptance: an
independent sleeping build script survived another session's native check and
was stopped only when its own requesting socket disconnected. An actual edited
compile failure (`20261029-231844899-3903294`) returned 101; byte-restored source
compiled successfully (`20261029-231845124-3903387`). Final hosted tests passed
847 cases with 12 ignored (`20261029-231720182-3886462`), plus strict all-target
Clippy. Two real shared-daemon warm local lint observations were 281/258 ms
with 62 fresh units, no compilation and zero source-validation time
(`20261029-231730330-3890942`, `20261029-231730605-3891080`). These are installed
observations under contention, not a comprehensive matched speedup or default
promotion. See `docs/daemon-native-no-candidate-installed.md` and retained raw
acceptance fixtures/logs for the exact contract, rebuild exclusions and hashes.

## 2026-10-30: matched Blaze warm-runtime comparison and item retention

Final `br34` passes all five stack contracts and 25 identical executed edits.
Seeded warm-store medians: owned off 1.318050s, earlier three experiments
1.263141s, Blaze Cranelift 0.143916s, all four experiments 1.284972s,
all four plus existing hotpatch 1.073436s. All five timed hotpatch edits
actually patched and executed fresh markers. Different frontends/backends,
unknown machine noise, and repeated fresh Blaze store failures remain explicit
limits. No general speed claim or default promotion.

Native default-off per-item dead-code query retention passes 224 differential
compiler invocations. Real body edits show 3211 ordinary plus 180 allow/expect
named disk loads and five ordinary same-value provider executions. Reuse is
proven, saved time is not. Supported `bench/suite.py compare` full-test gate:
retention 2.57 -> 2.60s, +1%, uncontrolled; hotpatch-only 2.78 -> 2.73s, -2%,
uncontrolled. Each measured sample passes 157 tests. Example runs:
`20261030-025226633-1068258516`, `20261030-025255895-1077024379`,
`20261030-025802164-1135394810`, `20261030-025813885-1141148463`.
All calls exported. Full methodology, failed attempts, artifact identities and
portable acceptance audits: `docs/blaze-runtime-results-20261030.md`.
All 570 benchmark Python tests pass. Comprehensive session-weighted gate and
the maximum reusable fraction remain unresolved. Experimental defaults stay off.

Follow-up `br35` passes six stacks and 30 matching edits, including explicitly
selected Blaze LLVM from an identical warm-store seed. LLVM median 0.401728s
and owned-off 3.503743s are descriptive only: load was about 42.9 on 16 logical
CPUs. Existing hotpatch really patched all five edits, yet its median was
3.862631s, slower than all-four-on 3.633667s. Do not omit that counterexample
or mix absolute times with br34. Earlier fresh-store LLVM failures remain.
All 40 additional owned calls are exported under xps14-runtime-br35 directories;
the portable `validate-complete.py --case br35` audit passes all 60 records.
See the follow-up section of `docs/blaze-runtime-results-20261030.md`.

## 2026-10-30: instrumented actual edit remains compiler-bound

Six four-test workflows pass and restore all 63 pinned source files. Edit runs
`20261030-035237630-1591346` and `20261030-035258029-1595722` explicitly disable
and enable native procedural macro caching, respectively. Macro expansion is
291.104 versus 161.620ms, but total harness wall is 1.332673 versus 1.351485s.
Result-cache persistence is 106.866 versus 146.550ms. This single instrumented,
uncontrolled pair shows no total speedup. Nested time-pass spans are not additive.
All six runs exported and exact phase/raw evidence retained in
`docs/owned-runtime-frontier-20261030.md`. Compiler work remains the primary
target, not startup or the few milliseconds of test execution. Cached-family
external-input correctness and deterministic declarative macro reuse are
separate pending gates. No default promotion or Blaze-beating claim.

Follow-up actual-family gate reproduces stale derive and attribute expansion
outputs after std environment/file and tracked environment/path changes.
The original function-like-only passing fixture was an uncached control, not
a safety gate for these families. Automatic proc-macro caching is now disabled
in the shipping shim, requiring explicit expert opt-in. The explicit on-mode
phase diagnostic above is not an eligible correctness-preserving optimization.
Local safety validation: check, four shim tests, warning-free clippy, full
847 passed / 12 ignored, reinstall, and recorded/unrecorded/recorded rmeta
identity sharing. Full test run `20261030-042612239-1866782`; compatibility runs
`20261030-042837633-1891779` and `20261030-042837808-1891834`. Hosted validation
was paused under the earlier maintenance coordination. Details and exact
acceptance logs: `docs/macro-cache-safe-default-20261030.md`. No speed claim.
