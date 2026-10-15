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

## Measurement policy: comprehensive first, focused when diagnosing

The primary score is **mean agent-visible wait across the session-weighted
synthetic workload**, not `desktop-test-edit` or any other single scenario.
For general speed claims and default promotion:

```sh
bench/suite.py run --synth 1 -n 1       # baseline and candidate, separately
bench/suite.py compare A.json B.json   # matched synthetic results
bench/versus.py run --synth 1 -n 1      # stock vs caches-off vs default
```

Keep the seed, point count, generator/weights fingerprint, snapshots, and
execution mode fixed. Inspect command/change breakdowns for regressions as
well as the aggregate. Do not refresh weights between baseline and candidate.
Repeat measurements when noise or sampling makes a conclusion uncertain.

Focus experiments with a named suite scenario (`bench/suite.py list`), a
component factor (`bench/factors.py list`), or an additional candidate
`--stack NAME:K=V,...` in versus. Such results support scoped claims only,
not a claim that justrust as a whole got faster. The historically named
`reference-loop` / `desktop-test-edit` is one warm filtered-test diagnostic.

The synthetic workload is broad, not universal: supplement it with relevant
cold-build, contention, hosted-mode, and correctness checks. See
`bench/synth/README.md` for coverage gaps. If the comprehensive comparison is
blocked, say so explicitly rather than substituting one convenient loop.
Real-usage trends remain the reality check, not a controlled attribution test.

## Explicit native compiler identity (local synthetic runs)

Use `suite.py run --toolchain SPEC --toolchain-identity PRIVATE_JSON` when two
owned compiler builds report the same upstream `rustc -vV`. This opt-in requires
an already registered toolchain in an isolated `JUSTRUST_HOME`, local `--synth`,
and no `--resume`. It does not change the legacy result format without the flag.
The bounded (64 KiB) JSON manifest has exactly `schema: 1`, `toolchain: SPEC`, and
`artifacts`. The latter contains all six roles `rustc`, `cargo`, `driver`,
`rustdoc`, `clippy-driver`, and `cargo-clippy`, each with exactly `relative_path`
and a lowercase 64-digit `sha256`. Executables use `bin/<role>`; Cargo may use
`bin/cargo` or `bin/jr-cargo`; the driver names its `lib/librustc_driver*.so`.
All resolved files must remain inside the selected installed/staged prefix.
Manifest paths stay private. Results expose role hashes, marker hash, and hashed
invocation records, not the manifest's machine-local paths.

Keep A and B in separate private homes and separate `JUSTRUST_BENCH_WORK` and
`JUSTRUST_VERSUS_WORK` directories, even with the same declared SPEC. Do not
share target/dependency/workspace caches between different compiler bytes.
`justrust toolchain jr-install FULL_SYSROOT --cargo-from nightly-2026-10-08
--no-default` registers a complete matched compiler/helper prefix in the private
home without changing machine defaults. Copy the trusted official Cargo donor
into that private home's toolchain registry first. For donor `bin/cargo`, set
`JUSTRUST_CARGO_FORK=0` on both runs. Do not substitute stock helpers into a
partial native prefix or invent an installation marker.

The runner hashes declared compiler/driver/helper files before and after each
invocation, and audits every pre-build, switch build, and measured build's
`meta.json`: explicit toolchain, local execution, and exact selected compiler and
Cargo paths. Missing records, fallback, substituted binaries, or loader overrides
(`LD_LIBRARY_PATH`, `LD_PRELOAD`) reject the demanded run. Results retain overall
before/after identity plus per-invocation audit hashes. This proves **declared
prefix files and recorded selection**, not mapped-memory identity or that a
compiler executed during a no-op/cache hit. Clippy helper hashes likewise do not
prove a particular helper was loaded.

Hashing is outside measured intervals but warms compiler/driver OS page caches.
Compare only A/B runs using the same attestation protocol; `suite.py compare`
rejects attested versus legacy-unattested results. A cold target directory is
**not a cold OS page cache**. Keep normal seed/40-point/weights/source/mode gates,
inspect all regressions, and do not overlap measurements with compiler bootstrap.

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
bench/versus.py run --stack jr-rustc:RUSTC=$HOME/justrust/toolchain/rustc/build/host/stage1/bin/rustc
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

Hosted provisional-error experiments use `--remote --hosted-early` with a
named compile-error scenario, for example `desktop-type-error`. Prewarm the
remote analyzer through an admitted build with a machine-local bounded launcher.
The suite only queries it, never starts local rust-analyzer as a substitute,
and requires shown diagnostics confirmed by the final rustc result. Full cargo
exit remains measured. This is not an early-exit or successful-test speed claim.

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

- **Hosted timing diagnostics.** With `--remote`, explicitly set
  `JUSTRUST_REMOTE_TRACE=1` to retain existing fixed numeric client trace
  timestamps in each sample's optional `remote_trace.client_elapsed_ms`.
  Newer clients also report `remote_trace.admission_awaited_ms`: identity,
  charge, residual identity wait, queue, and total. These are awaited paths,
  not isolated database execution times. Identity overlaps charge, so do not
  sum all fields. Parsing happens after the timed command exits, adds no
  requests, and never stores raw headers or credentials. Missing, duplicate,
  or invalid fields are omitted. Tracing is not enabled automatically.

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
| `desktop-test-edit` | historical reference-loop diagnostic: one line in a `#[test]` fn in `jcode-desktop-ui` (118k lines), `test --lib -- fps_counter`, not the overall score |
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

## Experimental Cargo scheduling

`python3 bench/scheduler-sweep.py` runs the pinned suite in OFF/ON/ON/OFF
order with a single experimental Cargo binary. Set an isolated `JUSTRUST_HOME`
containing an already-installed compiler prefix and the candidate `bin/jr-cargo`.
Never use the production `~/.justrust` prefix for this experiment. For example:

```sh
JUSTRUST_HOME="$HOME/bench-scheduler/home" JUSTRUST="$HOME/bench-bin/justrust" \
  python3 bench/scheduler-sweep.py --toolchain nightly-2026-10-08 \
  --out-dir bench/results/scheduler-abba --jobs 24 --samples 2 \
  desktop-cold-check desktop-test-edit desktop-body-check desktop-noop-test
```

The initial experiment uses uniform costs, not unrecorded package hints. Each
segment is an actual `suite.py run`; the final `off.json` and `on.json` are
compared by `suite.py compare`. All samples and contamination flags are retained.
The harness hashes justrust, rustc, Cargo and the installed toolchain marker,
rejects changing identities, and preserves partial completed segments. Every
measured run's actual invocation record must select that private Cargo/compiler,
the requested nightly and job count, and the controlled local route. Missing
records or silent fallback prevent a completed comparison, even if the expected
candidate files remain on disk. It records
`JUSTRUST_CARGO_CRITICAL_PATH`, not private hint paths or arbitrary environment.
This is controlled host-local scheduling evidence, not hosted transport latency,
a proof of a causal compiler DAG, or a reason to promote the default-off scheduler
before real public-interface correctness and performance acceptance.

## Committed evidence correspondence

`python3 bench/audit_hosted_evidence.py` audits the committed October 13 hosted
and cumulative documents/results against committed exported run summaries. It
checks full run IDs (including `-pre`), dictionary-key references, suite and
round-robin samples, compiler/host identity, exits, arguments, test counts and
compiled/fresh units where reported. A missing reference or mismatch exits
nonzero. It reads Git and JSON only, with no network or builds.

This checks reporting fidelity, not current runtime correctness or speed. Real
hosted public-interface and pinned end-user acceptance remain separate gates.
`python3 -m unittest discover -s bench` covers both the benchmark runner and
negative audit regressions. See `docs/cumulative-acceptance-20261013.md`.

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
