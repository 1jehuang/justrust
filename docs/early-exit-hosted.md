# Early exit on the hosted service: default plan

Status: plan, not deployed. Measurements: FINDINGS 59. Client code (MIT,
this repo): `src/early.rs`, `src/buildd.rs`. Anything multitenant
(per-account limits, admission, metering) belongs in `~/justrust-cloud`
(see [remote-strategy.md](remote-strategy.md), licensing), never here.

## What runs where

A hosted run is `justrust <sub>` executed by `remote_agent` on the
account's build VM (`build_hosts.js`: one c7i.8xlarge per account, 32 vCPU,
64 GB, 200 GB gp3, stops after 30 idle minutes). Early exit needs two
resident processes on that VM, both per workspace path:

- the early daemon (`justrust __early-daemon <root>`) with its
  rust-analyzer: **4.8 GB** (Desktop, 780 files) to **5.4 GB** (Jcode, 1,446
  files, after a full sweep), 11-35 s to ready with its target dir warm;
- the build daemon (`justrust __buildd`, one per user): keeps cargo running
  after the early answer, so the next identical request and `justrust log`
  get rustc's verdict. ~20 MB.

Today hosted runs set `JUSTRUST_ON_REMOTE_AGENT` (`route::ON_AGENT`), which bypasses `buildd`
(`maybe_run` returns `None`), so an early answer cannot end the request
there. Step 1 below removes that.

## The switch

1. **Client (this repo).** On the agent: let `buildd` serve requests that
   carry `JUSTRUST_ON_REMOTE_AGENT` (`route::ON_AGENT`) (they never route again: the flag stays set on
   the child). `remote_agent` relays frames unchanged, so the early `X 101`
   reaches the laptop as the run's exit, and the laptop's report says
   "returned early". The verdict replay works the same way: the next
   identical hosted request attaches to the finished build.
2. **Default.** On the hosted VM only: `JUSTRUST_EARLY=auto` and
   `JUSTRUST_EARLY_EXIT=1` in the build environment `remote_agent` sets
   (`start_run`), not on laptops. Memory there is the service's, not the
   user's: the 64 GB VM runs one account's builds, and a warm Desktop
   `check` peaks at ~2.8 GB of rustc.
   Local stays opt-in (a laptop's 4.8-5.4 GB per workspace is the user's
   memory, and `auto` only starts a daemon after 3 agent builds in an hour
   with 8 GB free).
3. **Per-request opt-out** stays `JUSTRUST_EARLY_EXIT=0` in the client env
   (forwarded like every other variable).
4. **Kill switch** in `~/justrust-cloud`: a worker env flag
   (`EARLY_EXIT_DEFAULT=0`) the VM bootstrap reads into the agent's
   environment, so a bad rust-analyzer release is one deploy away from off.

## Memory budget per tenant workspace

| item | budget | basis |
|---|---|---|
| rust-analyzer per workspace | 6 GB hard cap (cgroup `memory.max`), 5 GB expected | measured 4.8 GB Desktop, 5.4 GB Jcode after a full sweep |
| rust-analyzer's own target dir | 2-4 GB disk | proc macros and build scripts only |
| early daemons per VM | at most `floor((64 GB - 16 GB) / 6 GB)` = **8** workspaces | keep 16 GB for cargo/rustc (Desktop peak 2.8 GB build, several concurrent agents) |
| buildd | 1 per VM | ~20 MB plus the builds it would run anyway |

The cgroup cap is the hard guarantee: rust-analyzer OOM-killed inside its
own cgroup means `ready: false` and the build waits for rustc (fail open),
never a failed build. Each daemon runs in `systemd-run --user --scope -p
MemoryMax=6G` (bootstrap in `build_hosts.js`); the client already handles
a dead daemon.

## Eviction

In order, cheapest first:

1. **Idle**: a daemon exits after `JUSTRUST_EARLY_IDLE_SECS` without
   queries. Hosted: 30 min, matching the VM's idle stop (laptop default
   3 h).
2. **Count**: starting a 9th daemon on a VM stops the least recently
   queried one (LRU over `~/.justrust/early/*/sock` mtimes, done by the
   starter before `spawn_daemon`).
3. **Pressure**: when `MemAvailable` < 12 GB (PSI `memory some avg10` > 10
   as the early signal), stop the least recently queried daemon, repeat
   every 10 s. Builds always win over early answers.
4. **VM stop**: daemons die with the VM; the first agent build after a
   restart starts them (auto mode), first early answer ~15-35 s later.

Restart cost is small next to a cold VM: rust-analyzer's target dir is on
the persistent volume, so a restart is the 11-35 s load, not a rebuild.

## What it buys and what it risks

Per the mutation bench and replay (FINDINGS 59): the rustc-confirmed early
answer arrives in a median ~0.2-0.6 s instead of the full `check`, on the
mutations where it fires; wrong answers are the right file with an extra or
different error, and the agent's next run gets rustc's verdict. Hosted
checks are where the gap is largest (Jcode `check -p` 10-36 s on a laptop,
less on 32 cores but still frontend-bound and single-crate).

Not covered and still to measure on a VM before turning it on: rust-analyzer
latency on c7i (the laptop numbers are from an Ultra X9 388H), memory with
several agents' worktrees of one repo (each worktree is its own workspace
root, so each gets its own 5 GB; share by path remapping later).
