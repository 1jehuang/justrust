# Noninteractive benchmark stdin boundary

## Observed improvement

`versus.run_counted` now gives an executed benchmark child read-only `/dev/null` on descriptor 0. An inherited open pipe or noncontrolling PTY therefore produces immediate EOF instead of blocking a child that reads stdin. Standard output and standard error remain captured in the existing log. Process groups, environment forwarding, jobserver descriptors, `wait4`, timing, resource accounting, generator, weights and noise eligibility are not changed by this patch.

Root replayed the exact new tests against the original `ef31947` runner in a separate Git archive without modifying the working source. Four input-boundary cases failed at their explicit 3-second watchdog. Exit-status and signal controls passed. With the patch, all six real fork/exec/wait4 cases pass, and the whole current benchmark suite passes all 220 tests. This is an execution-boundary correctness improvement, not a compiler speedup.

## Requirement-to-observation map

| Requirement | Actual execution check | Observed result |
|---|---|---|
| An open inherited pipe cannot hold a benchmark on stdin | `test_open_pipe_becomes_empty_non_tty_stdin` keeps the input pipe open while the child executes `os.read(0, 1)` | EOF, `isatty(0) == False`, normal result schema and nonnegative resource fields |
| Noncontrolling PTY input is also noninteractive | `test_pty_becomes_empty_non_tty_stdin` retains both PTY endpoints | EOF and non-TTY stdin |
| The temporary `/dev/null` descriptor does not leak through exec | `test_temporary_null_descriptor_is_closed` deliberately makes the temporary descriptor inheritable | Executed child observes `EBADF` on that descriptor |
| Opening `/dev/null` directly as fd 0 works despite Python's close-on-exec default | `test_null_open_returning_zero_keeps_stdin_open` forces the open to return 0 | Executed child can read EOF from descriptor 0 |
| Existing context and output survive | Both input probes compare child/driver process-group IDs and the environment sentinel, and inspect stderr | Same process group, same sentinel, stderr marker retained |
| Nonzero and signal exits are not rewritten | `test_real_exec_failure_status_and_output_are_preserved`, `test_signal_exit_is_preserved` | Exit 23 with stderr retained, and signal status -15 |
| Existing Python benchmark integrations do not regress | Full `unittest discover -s bench -p 'test_*.py' -v` | 220 tests passed |
| Verification does not overwrite unrelated changes or claim old samples use the new runner | Source hashes before/after testing and a separate committed-source audit | Exact tested source, scoped source-only patch, no reuse of pre-patch samples as a paired result |

The exec tests disable access to instruction counters in the test harness only. They still run the production fork/exec/log/wait4 path with real children. The guard itself does not alter counters. These checks do not constitute exhaustive jobserver-token verification.

## Deliberate limit: controlling terminal is unchanged

The PTY fixture is **noncontrolling**. Redirecting stdin does not call `setsid`, remove an existing controlling terminal, or change process groups. A descendant can still open `/dev/tty` or issue terminal ioctls and encounter job-control signals. The actual repeated process-group suspension from the legacy-host trial has **not** been causally diagnosed or repaired by this patch.

The earlier trial remains OFF 15/40, ON 0, with noisy samples, unexpected archived-workload failures and an interrupted unfinished sample. It has no eligible comprehensive comparison, speedup, CPU work/span or hardware-ceiling result. Its original stopped EC2 state and existing volume were restored and verified before this guard was published. See `docs/scheduler-legacy-attempt-20261014.md`.

A future comprehensive attempt must separately validate its headless/session boundary and archived-workload prerequisites, then run a fresh unchanged OFF/ON pair. Do not combine old samples with this changed execution context.

## Evidence and packaging

Root baseline-red, final focused-green and final whole-suite logs, source hashes, explicit path-redaction provenance and artifact checksums are in `bench/results/benchmark-stdin-boundary-20261014/`. The original full receipt remains in private scratch. Public logs retain every line except operator/scratch-path replacement. This patch is Python-only: no compiler selection, global CLI installation, EC2 configuration, tenant quota, billing or shared cache was changed. Tests require Unix `fork` and `wait4`, and skip on platforms without those interfaces.
