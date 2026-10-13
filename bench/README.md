# Benchmarks

Three layers, from "what agents actually wait for" to "why":

| | tool | question | output |
|---|---|---|---|
| System 1 | `bench/trend.py update` | Are real agent builds in Jcode, Desktop, justrust getting faster than before justrust? | `bench/trend/TREND.md`, `daily.csv`, `reuse.csv` |
| System 2 | `bench/factors.py run` | What is each factor (depcache, slots, linker, ...) worth on its own? | `bench/FACTORS.md`, `bench/results/factors-*.json` |
| System 3 | `bench/versus.py run` | How much faster is justrust than the stock toolchain, with and without its caches? | `bench/VERSUS.md`, `bench/results/versus-*.json` |
| Suite | `bench/suite.py run` | Did this commit make the standard scenarios faster or slower? | `bench/results/*.json`, `compare` |

System 3 and the suite also run the **synthetic agent workload**
(`--synth SEED`, see `bench/synth/README.md`): points sampled from how
agents really call cargo (mined from Jcode sessions: command, what was
edited before it, where), turned into concrete edits on the pinned
snapshots. Its headline is the expected wait per agent call. Prefer it for
any claim about agents in general; the named scenarios stay for targeted
questions and for comparing with older results.

## System 1: rolling log, `bench/trend.py`

Mines Jcode session history (`justrust history --dump`) for every foreground
`check`/`test`/`build`/`clippy` call an agent made in the three repos, with
the wall time it was blocked, from September 2026 (before justrust) onward.
Baseline = everything before 2026-10-07, when agents switched to justrust.
Also reads `~/.justrust/runs/*/summary.json` for the reuse ledger: units
compiled vs fresh, depcache hits and the rustc time they saved, build-script
replays, slot refreshes. Run `bench/trend.py update` now and then and commit
`bench/trend/` (aggregates only). Real usage, not controlled: the work mix
changes, so read the same-command table and System 2 for causes.

## System 3: stock toolchain vs justrust, `bench/versus.py`

```sh
bench/versus.py run                   # every warm workload on every stack (add --cold for fresh-dir)
bench/versus.py run reference-loop -n 8
bench/versus.py run --stack jr-rustc:RUSTC=$HOME/jr-rustc/build/host/stage1/bin/rustc
bench/versus.py report                # rebuild VERSUS.md from the newest result
bench/versus.py compare A.json B.json # same stacks, two dates or justrust versions
```

One scoreboard: the same agent workloads (reference loop, check, first
type error, deep edit, no-op, small crate, cold checkout) run on three
stacks, round-robin with a rotating order on the pinned snapshots:

- `stock`: `/usr/bin/cargo` + system rustc, no justrust anywhere.
- `jr-nocache`: justrust with every cache off. `stock -> jr-nocache` is
  what the owned toolchain, forks and integration buy without caching.
- `justrust`: defaults, with a private depcache warmed by the warmup round.

An extra `--stack NAME:K=V,...` is a candidate component (a forked rustc,
another linker) on top of justrust defaults.

Besides wall time, every run counts **user-space instructions retired by
the whole build process tree** (`perf_event_open` with inherit, both
hybrid core PMUs). Run to run, instruction counts vary by about 0.1%,
where wall time varies by 5 to 20%, so a forked compiler doing 3% less
work shows up as `instr 0.97` even when wall ranges overlap. Caveat: work
moved out of the build's tree (a resident daemon, a remote host) is
invisible to the count, so read it together with wall time.

## System 2: one factor at a time, `bench/factors.py`

```sh
bench/factors.py list                 # factors, workloads, on/off switches
bench/factors.py run                  # every factor not marked slow (hours: each new config builds cold once)
bench/factors.py run --all            # plus feature-unify, pinned nightly, cranelift
bench/factors.py run mold -n 8        # one factor
bench/factors.py report               # rebuild FACTORS.md from the newest results
```

Each factor is switched off alone with everything else at the default
("leave one out"), on the suite's pinned snapshots, on a workload where it
matters (warm edit loop, cold fresh target dir, package switch sequence, or
two concurrent agents). On and off runs alternate in ABBA order. Loop
variants keep their own warm target dir keyed by the build configuration;
cold caching factors use a private depcache dir. Every sample carries the
reuse ledger (compiled, fresh, depcache hits and saved rustc seconds,
build-script hits, slot-refreshed units). Add a factor whenever a new
optimization lands: give it an off switch and a workload.

## The suite: `bench/suite.py`

Use this for any speed claim. It runs the agent edit loop on **pinned source
snapshots**, so numbers taken on different days and justrust versions can be
compared.

Only one benchmark runs at a time: suite, factors and versus all edit the
same extracted snapshots, so each takes an exclusive lock
(`~/.justrust/bench/work/.bench.lock`) before touching them and a second
run waits ("another benchmark holds the snapshot lock"). Do not bypass it
with `BENCH_NO_LOCK=1` unless the runs use different snapshots.
A script that takes the lock itself (`flock` on that file) and then
runs a bench script must export `BENCH_LOCK_HELD=1`, or the child waits on
its parent forever.

```sh
bench/suite.py list                    # scenarios
bench/suite.py run                     # every warm scenario, 1 warmup + 5 runs each
bench/suite.py run desktop-test-edit -n 10
bench/suite.py run --cold              # also the fresh-target-dir scenarios (slow)
bench/suite.py compare bench/results/A.json bench/results/B.json
bench/suite.py show bench/results/A.json
```

How it works:

- **Executable identity.** Set `JUSTRUST=/absolute/path/to/justrust` (or
  `JUSTRUST_BIN`) to select a frozen client. Missing, non-executable, empty,
  or conflicting explicit selections fail before installing toolchains.
  Results record `justrust_binary_sha256` outside timed samples and reject
  a changed executable before writing completed suite/versus results.
  `benchmark_repo_rev` is the harness checkout, not the installed client.
  `justrust_binary_revision` is only a declared `JUSTRUST_BENCH_REV`, never
  inferred from that checkout. The legacy `justrust_rev` label remains for
  older consumers, so use the SHA for exact client identity.

- **Pinned sources.** `git archive` of fixed commits of Jcode Desktop
  (+ sibling Jcode) and justrust, extracted once to
  `~/.justrust/bench/work/`. Bumping a pin in `SNAPSHOTS` makes old results
  incomparable. `compare` warns when they differ.
- **Private target dirs** next to each snapshot, so other agents' builds and
  slots never warm or cool the bench state.
- **Unique edits.** Every iteration inserts a fresh one-line edit (for example
  `std::hint::black_box(<n>u64);`), so rustc never reuses a state it has seen.
  Files are restored from a saved pristine copy, never via git.
- **Data from justrust itself.** Each run uses a known run id, and the
  suite reads `~/.justrust/runs/<id>/summary.json` for phases (startup,
  compile, tests), the per-pass split of local crates (frontend, codegen,
  incremental, link), units, and CPU. `JUSTRUST_PASSES=always` is set so
  every crate has the split, except in cold scenarios: units built with
  `-Ztime-passes` are never stored in the depcache. Cold scenarios with the
  depcache on run one warmup first so the cache is filled.
- **Noise.** A sample is `noisy` if another rustc was running at its start,
  or if other processes averaged more than `BENCH_NOISY_CORES` (default 4)
  cores. The table shows the median of all samples and of the quiet ones.
  `--quiet-wait 120` waits for a quiet machine before each sample.
  `compare` only says faster/slower when the min..max ranges do not overlap
  and the median moved more than `--floor` (default 10%: an A/A rerun of
  the same build drifted up to 9%). It also prints justrust's exit
  overhead (agent-visible wall minus recorded wall), which has no compile
  noise.
- **Interpretation choices** (guesses, revisit if wrong): the scenarios are
  the edit shapes agents make most in Jcode Desktop (test body, non-test
  body, dependency body, new pub item, deep sibling-repo edit, type error),
  timed as wall clock seen by the agent. The suite does not cover
  concurrent agents (`sched-contention.sh` does) or remote builds.
- **Results** go to `bench/results/<time>-<rev>.json` with machine,
  toolchain, and pin info, plus the run id of every sample. Commit them.

Scenarios (warm unless marked):

| scenario | what it measures |
|---|---|
| `desktop-test-edit` | **the reference loop**: one line in a `#[test]` fn in `jcode-desktop-ui` (118k lines), `test --lib -- fps_counter` |
| `desktop-body-test` | one line in non-test code of the same crate, same test command |
| `desktop-body-check` | same edit, `check` only |
| `desktop-type-error` | edit that adds a type error, `check`: time until the agent sees the error |
| `desktop-type-error-early` | same, with early diagnostics (`JUSTRUST_EARLY=1`, rust-analyzer daemon started untimed). Read the `1st err` column |
| `desktop-noop-test` | no edit: fixed overhead of `test` (should compile 0 units) |
| `desktop-upstream-body` | body edit in `jcode-desktop-ui-core` (a dependency), UI tests |
| `desktop-upstream-sig` | new pub fn in `jcode-desktop-ui-core`, `check`: metadata change cascade |
| `desktop-jcode-core-body` | body edit in `jcode-core` in the sibling Jcode repo, UI tests |
| `justrust-body-test` | small-crate loop: body edit in this repo, full `test` |
| `desktop-cold-check` (cold) | fresh target dir, depcache on: a new agent checkout |
| `desktop-cold-check-nocache` (cold) | fresh target dir, all justrust caches off |

## Focused scripts

These measure one mechanism each. Keep using them for that purpose:

- `edit-loop.sh`: per-run `justrust show` breakdown of the small-crate loop.
- `sched-contention.sh`: edit loop while cold builds run (the scheduler).
- `depcache-fresh.sh`: fresh-target-dir hits from the shared dependency cache.
- `feature-switch.sh`: dependency rebuilds when switching packages.
- `split-*.sh`: crate split before/after.
- `cranelift/`: cranelift backend comparison.
- `early_accuracy.py`: early diagnostics vs rustc (clean-sweep false positives, mutation corpus). Untimed.
- `ra_spike.py`: raw rust-analyzer LSP latency on the Desktop snapshot.
