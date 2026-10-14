# Benchmark terminal and generated-status boundary

## Scope and shipping decision

The earlier comprehensive scheduler experiment is still ineligible: OFF 15/40, ON 0, noisy and failed samples, and one interrupted sample. Nothing in this follow-up supplies an OFF/ON comparison or a compiler speedup. The legacy machine was restored stopped with its volume preserved before this local work started.

The only launcher change shipping here is that its **generated benchmark script** captures the payload exit code immediately, prints that same `EXIT` value, logs the finish time, and exits with the captured code. The original local fixture printed `EXIT 87` but returned 0 because its last command was `date`. The changed script prints `EXIT 87` and returns 87. Root replay against the exact original source also observed failure 23 and signal status 143 both incorrectly returned 0, while success 0 passed. The final script preserves all three 0/23/143 controls.

`on-remote.sh start` is an asynchronous dispatch interface when used with real tmux. A successful dispatch is not a successful payload. The synchronous fake-SSH fixture executes the generated script to observe its return value; its launcher return code is not evidence that real remote `start` now waits for or reports the benchmark result. Read the retained `EXIT` line for the completed payload.

No `setsid` wrapper, new session mode, signal normalization, timeout option, hosted credential, global CLI, cache, quota, permission or billing change is installed. The ordinary tmux launch and keepalive paths remain unchanged.

## Actual local terminal observations

All fixtures create their own session and PTY or an isolated private tmux socket. They do not acquire or signal the user's controlling terminal or ordinary tmux server.

1. In a fixture-owned background process group, a process with descriptor 0 already set to `/dev/null` can still receive **SIGTTIN 21** when it opens and reads its controlling `/dev/tty`. This directly distinguishes stdin EOF from controlling-terminal isolation.
2. A real GNU `timeout` fixture retains a controlling terminal and its payload can stop in the background group. That is a local mechanism demonstration, not attribution of the earlier EC2 stop signal.
3. A tested `setsid --wait` recipe gives the payload no controlling terminal and preserves a normal exit 23 and deliberately inherited pipe descriptors. It also has distinct observed signal-status behavior: the tested utility returned 15 after a child SIGTERM, not the shell's conventional 143. No normalization or equivalence is claimed.
4. In an actual private tmux server, the simple detached recipe stayed alive until natural payload completion. But after an early `kill-session`, the detached payload was still alive. Only that handshake-identified owned child was subsequently cleaned up by the fixture.

**The naive detached default is rejected.** Removing the controlling terminal is insufficient if supervised teardown can leave a workload running. No unsafe recipe is silently promoted just to make the terminal test pass. The existing timeout's child coverage is not weakened with `--foreground`.

## Requirement-to-observation map

| Requirement | Actual check | Outcome and scope |
|---|---|---|
| Preserve a payload's failure status after logging | Original versus changed generated-script execution | `EXIT 87` / script 0 becomes `EXIT 87` / script 87 |
| Retain ordinary success and failure cases | Generated-script exit controls and log assertions | Actual 0/23/143 payload statuses match both script exit and retained `EXIT` lines |
| Separate stdin from controlling terminal | Real fixture-owned PTY/background-group read with fd 0 null | SIGTTIN 21 observed; prior stdin guard is not a terminal-isolation fix |
| Test the real timeout grouping boundary | Actual GNU timeout payload in the fixture session | Stopped payload observed without weakening timeout options |
| Verify a detached recipe's natural lifecycle and inherited context | Real setsid execution, pipe descriptors, and private tmux | No controlling TTY and normal exit 23 observed; natural lifecycle passes |
| Check the likely unsafe early-teardown edge | Actual private tmux `kill-session`, then bounded owned-PID liveness check | Detached payload survives, candidate rejected and exact owned cleanup performed |
| Preserve existing launcher/evidence interfaces | Shell syntax, existing remote-evidence-sync tests, and whole Python suite | Root 11 focused tests and all 231 benchmark tests pass, with 54 benchmark Python/shell source blobs unchanged during validation |
| Avoid misattribution and accidental live changes | Source diff and resource-scope audit | No real SSH/auth/provider operation, no session-mode promotion, no full-workload success claim |

The transport fixture is intentionally synthetic, while the generated shell script, local process execution, kernel terminal behavior, GNU timeout, setsid and private tmux checks are real. These are not hosted-service acceptance or a rerun of the stopped EC2 workload. Supervisor cleanup only signals current un-reaped children of its sole-threaded Linux subreaper, then adopts and reaps descendants. Every retained root supervisor outcome has an empty remaining-child list. Private tmux payload cleanup opens a stable Linux pidfd before checking the handshake PID's birth identity and signals only that verified handle, with no numeric-PID fallback. Retained early-kill and natural-exit records show the handle closed and no remaining live payload; only early-kill uses `pidfd_send_signal`. Platforms lacking the required APIs explicitly skip those two private-tmux tests. All 11 focused cases ran without skips on the recorded machine.

## Evidence and remaining gates

Exact source hashes, named local outcomes, original-red and changed-green observations, final test logs, path-redaction provenance and checksummed artifacts are published under `bench/results/benchmark-terminal-boundary-20261014/`.

A robust headless launcher still needs explicit supervised termination and deadline proof, including descendant behavior. The cause of the actual legacy stop signal remains unknown. Archived workload prerequisites and all unexpected failures also remain unresolved. Any future parallelism claim requires a fresh eligible full same-mode OFF/ON pair on unchanged snapshots, generator and weights. Do not merge old samples with changed launch context or treat these local diagnostics as CPU work/span or a core-utilization ceiling.
