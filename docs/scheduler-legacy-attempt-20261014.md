# Legacy-host comprehensive scheduler attempt, 2026-10-14

## Result

**The full OFF/ON comparison is still blocked. There is no speedup, default promotion, work/span result, or hardware core-ceiling conclusion.**

The larger existing developer machine removed the previous disk-entry blocker. The real unchanged seed-1 workload reached **15 of 40 OFF points**, with **35 completed preparation/previous/measured commands**. Seven measured samples were noisy and three unexpectedly failed tests. ON never started. The 36th command stopped in its owned process group and was not accepted as a measured outcome.

Evidence: `bench/results/scheduler-legacy-3f47-20261014/`, with 37 actual records in `data/runs/scheduler-legacy-c7i-20261014/2026-10.jsonl`. The extra records are the interrupted recorder's termination finalization and a separate headless diagnostic, not two additional benchmark points.

## What ran

- Separate, existing, initially stopped, developer-owned EC2 `c7i.8xlarge`, west region, 32 vCPUs. It is not the normal tenant build host and does not use its D1 row, quota, shared/private service caches or cache-publish role.
- Existing 500 GiB volume and effective root filesystem of 518,933,680,128 bytes. Initial free space was 229,541,593,088 bytes. The fresh-prefix entry guard required 150 GiB free, and each actual invocation checked an 8 GiB reserve.
- Fresh private prefix, pinned git-archive sources and isolated cache/target directories. No current service cache was copied, resized, deleted or replaced.
- Same fixed 24 Cargo jobs, frontend toolchain selection, Cargo fork enabled, Cargo daemon and shadow disabled. Scheduler OFF was verified at every executed preparation and measured boundary. ON was planned but never entered.
- Harness `3f47eadb35019943dd506d8d2f6ab69ea4cadd96`, seed 1, 40 points, one repetition, unchanged generator/weights and pinned desktop/Jcode/justrust snapshots. Execution is local mode on the normally AWS-billed developer machine, not ordinary hosted-tenant acceptance.
- CLI `059674bf76fa218ba7e475279a86b4b6caf32aac`, SHA256 `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`.
- Cargo candidate `a45c3546671925c24649e451a673aa71b1385d0d`, SHA256 `c38c3d63e82c74fc4ba9e2978ad74ccad7f74aa21dec6d2bf978e0c4dcf1e13d`.
- Nightly 2026-10-08 rustc SHA256 `66877d44efc77e5d5478c756abf48574c5ce4d41a1102518b20e2ff3f3a0ba56`.
- Hints SHA256 `29bb6618b1ca614cfed13c3bd77777954f2380f757aeeb2fd799abe2040e44d8`. The earlier offline model is not proof that those costs generalize.

Start/staging and closing hashes match. Actual metadata selected the private candidate Cargo, pinned rustc, 24 jobs and local route. All 35 completed receipt exits equal their actual summaries, and all 15 measured outcomes join to those records. `commands.json` separates roles, exec wall and recorder wall, and records original metadata/summary/output hashes.

The stock probe used this machine's rustup stable Cargo/rustc 1.99.0. Its binary hashes are retained in `owner/stock-probe-runtime.txt`. Those probes are not a stock-versus-justrust speed comparison. They and this host differ from the previous m8azn epoch, so observations cannot be merged across those epochs.

## Failures and suspension are retained

- UI measured run `20261014-095558516-43443865`: recorded 1,301 passed, 2 failed, 10 ignored, exit 101. The raw failures are `running_tool_row_pulses_do_not_rebuild_the_panel` and `working_session_animation_ticks_do_not_rebuild_workspace_or_panel`. These are not the previous viewport assertion.
- Workspace measured run `20261014-100607224-43443644`: recorded 5,581 passed, 14 failed, 43 ignored, exit 101. Full raw output includes missing `sqlite3`, archived-tree Git-repository assumptions, login-executable discovery, tool-description expectations and other runtime assertions. Recorded aggregate counts are recorder observations, not a reconciliation of all interleaved named assertions.
- Base measured run `20261014-100809183-43443833`: exit 101, preserved separately. All three were planned normal successes, not expected compile-error points. All retain `compile_failed=false`.
- In-flight point 16, `20261014-101022743-43443971`: `test -p jcode-tui --lib`. Before shutdown it had only live metadata, no after-exec receipt, and no accepted sample. Its observer, CLI and test process were `T/Tl`, with `do_signal_stop`, in background PGID 43441 while the controlling PTY's foreground PGID was 43348.

One owned-only SIGCONT at 10:19:27.933 UTC immediately returned to stopped states. This supports a terminal job-control hypothesis but **does not establish SIGTTIN, SIGTTOU or the precise originating test**. An exact pinned-source search found no direct SIGTSTP/SIGSTOP/killpg literals, which does not exclude indirect library or terminal-ioctl behavior.

At 10:23:27 UTC only the verified experiment group received TERM followed by CONT. It disappeared, and the shell/keeper exited. The recorder finalized interrupted point 16 with exit 101 and 784.933 seconds elapsed. That is interrupted recorder data, **not a measured outcome or valid benchmark duration**, and is excluded from the 15-point partial.

A separate bounded command without a PTY and with stdin DEVNULL then completed, still with exit 101, as `20261014-102329731-294199`. Recorded counts were 2,484 passed, 4 failed, 18 ignored. Full raw failures include onboarding Git-tree assumptions. This changed more than one execution-context property and did not validate a pure stdin fix, a compiler gain or a full-workload repair.

All five nonzero records have their entire path/address/resource-redacted output JSONL compressed in `failed-output/`. No failing assertion or compiler error was intentionally omitted. Successful raw logs remain in private evidence and the preserved volume, with their original hashes in `commands.json`.

## Requirement-to-observation map

| Requirement | Actual check and observation | State |
|---|---|---|
| Sufficient effective scratch capacity | Guest `df`/block layout, 150 GiB entry reserve, 8 GiB per-call guard. Reached 15 points without the old storage refusal | Observed for this attempt, not a bound on full-workload peak |
| Exact compiler/Cargo/CLI and flags | Loader probes, pinned source/binary/archive hashes, all 35 actual receipt/meta joins, closing hashes | Passed within recorded boundaries |
| Preserve generator, seed, points and weights | Frozen `3f47` archive and workload identity in staging/identity logs | Passed; no failed point replaced or filtered out |
| Use real public build/check/test interfaces | 35 completed calls including release builds, checks, filtered/full tests, plus interrupted call and diagnostic | Executed, with failures preserved |
| Match outcome to measurement | All 15 sample wall/exit/compile_failed fields join exactly to recorded outcomes | Passed integrity, three normal-success plans failed |
| Clean full paired comparison | Seven noisy samples, 15/40 OFF, ON 0 | Blocked, no ratio or weighted aggregate |
| End-user report refuses invalid claims | Frozen public `suite show` refuses aggregate and annotates raw details. `suite compare` self-refusal has no invented environment/ON provenance | Passed refusal, not an OFF/ON comparison |
| Full harness regression check | Actual frozen 3f47 Python suite: 199 tests passed | Passed harness tests, not compiler performance acceptance |
| Preserve interrupted execution | Original before-exec, stopped-state receipts, continuation, controlled shutdown and finalized record all retained | Passed; no sample/outcome fabricated |
| Bound lifetime and credit burn | Mode deadline 10:59, actual external 11:00 backstop, early owned-only shutdown | Passed; ended early on observed liveness failure |
| Restore initial resource state | AWS stop requested 10:26:28 UTC, independently queried stopped 10:27:09 UTC; original 500 GiB volume retained | Passed; no termination or volume deletion |
| Keep tenant security/defaults intact | Separate legacy namespace, no D1/default/IAM/cache-publish changes, no tenant host mutation | No changes made; legacy bootstrap is not tenant security proof |
| Preserve honest CPU/scaling scope | CPU lower-bound/exec CPU definitions separated; compiler DAG work/span and core ceiling remain null | No unsupported scaling conclusion |
| Preserve public privacy and auditability | Byte/SHA manifests, explicit path/resource/address/token redaction, 37 exported IDs and complete compressed failure output | Audited before publication |

## Preparation, lifecycle and limitations

The first SSH transport was terminated at its tool-imposed 600-second limit during stock-probe preparation. It had zero measured receipt rows. The empty receipt and full preparation log were retained. After observing no active owned compiler, the same frozen launcher was restarted detached, and the existing generator journal handled interrupted probe preparation. No measured sample was discarded or resumed into a completed result.

The legacy guest's existing sudo marker was used by its keeper. Its historical volume is unencrypted. Neither is claimed to match the normal tenant's no-general-sudo/encrypted bootstrap. No permissions were broadened, credentials or publication role attached, or current host changed.

The quiet wait precedes point preparation, not the measured command after preparation. Own preparation can leave a high load average, and seven samples remain conservatively flagged noisy. Those flags were not relaxed. Sequential OFF then ON would also retain a cache-warming caveat even if the pair completed.

Before EC2 stop, no owned commands were active and closing binary/hint hashes matched. Stop was verified at 10:27:09 UTC, then the scheduled 11:00 cleanup backstop was canceled. The retained volume, target data and journal were not deleted. Future retries must recover that journal or use a new private prefix, address the headless launcher boundary and dataset runtime prerequisites, and measure a fresh full pair. A stdin-only boundary guard does not prove removal of a controlling terminal.

Original logs are preserved privately. Published files are lossless except the explicit path, resource-ID, non-loopback address and long-bearer-token redactions described by the raw-source manifest. Original `lsblk` trailing spaces and the observation log's final blank line are deliberately retained, so the whole-artifact `git diff --check` flags those three raw-log lines. Source, documentation and findings whitespace checks pass separately. There is still no accepted broad speedup or basis to choose a 96-core server from this attempt.
