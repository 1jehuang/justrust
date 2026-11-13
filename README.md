<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/justrust-icon-dark.svg">
  <img src="assets/justrust-icon.svg" alt="justrust: a halftone gear" width="140">
</picture>

# justrust

**Rust compile as a service.**

A faster, vertically integrated Rust build toolchain optimized for agents.

[Website](https://jcode.sh/rust) · [Install](#install) · [Use](#use) · [Remote builds](#remote-builds) · [Goal](#goal) · [Findings](FINDINGS.md) · [Changelog](CHANGELOG.md)

</div>

---

justrust vertically integrates and optimizes the Rust build toolchain to make
the entire edit, check, and test loop faster. Use it locally or with our remote
build service.

[uv benchmarks](docs/readme-uv-benchmark-20261014.md), median of three rounds. Factors are relative to Stock Cargo.

| Operation | Stock Cargo (local) | justrust (local) | justrust (remote) |
|---|---:|---:|---:|
| Fresh checkout, `check --workspace` | 81.2 s | **34.5 s (2.35×)** | **14.3 s (5.66×)** |
| Fresh checkout, `test -p uv-resolver --no-run` | 85.8 s | **33.3 s (2.58×)** | **15.2 s (5.63×)** |
| Warm one-line body edit, `check -p uv-resolver` | 0.96 s | 0.85 s (1.13×) | 0.89 s (1.07×) |

Your coding agents spend valuable time waiting for Rust checks, builds, and
tests. Running those operations locally can slow down your machine, drive up
CPU usage, temperature, and RAM consumption, and fill your disk with build
artifacts. Our remote service offloads compilation, test execution, and build
artifact storage to our machines, freeing up yours.

Each build command also returns actionable diagnostics, showing agents where
time went, why crates rebuilt, and which bottlenecks to address, so they can
proactively optimize builds within your codebase.

## Install

```sh
curl -fsSL https://jcode.sh/rust.sh | sh
justrust agents
```

Linux and macOS, x86_64 and aarch64. Installs to `~/.local/bin` and starts
sign-in. `justrust agents` configures your coding agents ([manual setup](docs/AGENTS-snippet.md)).
From source: `cargo install justrust --locked` (Rust 1.91+).

Prebuilt installs update automatically. Opt out with `justrust update off`;
disable anonymous telemetry with `justrust telemetry off`.
[Send feedback](docs/feedback.md) with `justrust feedback "Your suggestion"`.
Add `--kind bug|idea|question|infra|infra-urgent` to classify it.

## Experimental native compiler

The stable client can also explicitly run our owned, **Blaze-derived resident
compiler** with `justrust native test --lib -- TEST_FILTER` (or native `build`,
`check`, `run`, `status`, `stop`, and `toolchain`). Build/install its separate
`jr-native` executable using `toolchain/resident/build-jr.sh TOOLCHAIN_DIRECTORY`.
This is opt-in, local, and not yet a general rustc replacement. Ordinary justrust
commands retain the Cargo/hosted path. See [native setup and exact Blaze techniques](docs/stable-native-engine.md)
for pinned toolchain requirements, private state, compatibility and limitations.

## Use

```sh
justrust check -p my-crate
justrust test -p my-crate --lib some_module::
justrust build
```

`check`, `test`, `build`, `clippy`, and `run` take cargo's arguments and
return its exit code. Errors and failing tests stay in full, followed by
a concise result and timing report. Use `justrust log` for the full output
and `justrust show` for the timing breakdown, including an end-to-end
waterfall of everything you waited for (routing, hosted admission, source
sync, cargo phases, artifacts). `justrust profile --days 7` totals that
waterfall across runs to show where agent wait goes.

Other cargo commands (`cargo fmt`, `cargo add`, `cargo tree`) stay plain cargo.

### Human dashboard, explicitly interactive

The normal justrust install includes a terminal dashboard in the same binary:

```sh
justrust interactive dashboard
```

Only the `interactive` namespace enters a terminal UI. There is no automatic
TUI detection, and redirected stdin or stdout is rejected immediately. Existing
build, status, runs, and show commands retain their agent-friendly behavior.
For automation use `justrust status --json`, `justrust runs --json`, and
`justrust show ID --json` instead.

The dashboard shows active builds observed by this client, recent local runs,
account-private uploaded recordings, and your signed-in account's build allowance.
It uses the same credentials as `justrust login`, without operator access or a
separate installation. Local history works while signed out or offline. Hosted
refresh failures keep the last successful data visible with its age and error.

Use **↑/↓** to select a run, **Enter** for details, **l** for the last 2,000 local
log lines (up to 1 MiB), **a** for full account status and refresh errors,
**r** to refresh, and **q** to quit. **Esc** returns from details, with
**Page Up/Down** for scrolling.

Press **s** for the filter menu: **↑/↓** picks a filter, **←/→** changes it, and
**s** or **Esc** closes it. Filters combine, and the Runs card's top border
always states which are on, with the changed ones highlighted.

| Filter | Choices |
|---|---|
| Project | all projects, or any project with recorded runs (most runs first) |
| Command | check, test, build, release (a release-profile build), clippy, run, other. The kind bar shows how many runs each choice would list. |
| Where | anywhere, remote only, local only |
| Result | all, failed, passed, failed on your code, failed on the toolchain or infrastructure |
| Search | command, project path and run ID (Enter keeps it, Esc cancels) |

Shortcuts outside the menu: **c** / **Tab** steps the command kind (back with
**C** / **Shift+Tab**), **w** steps where, **f** toggles failures, **h** toggles
the project you launched from, **/** searches, and **x** or **Esc** clears all.

The Runs card says what happened instead of a bare exit code: `compile error`,
`tests failed`, `your code failed`, `toolchain fault`, `infra fault`,
`interrupted` or `unknown cause`. The cause is the class the recorder stored. A
failure with no recorded class shows its exit code. The last column is how long
ago the run started. Enter still shows the run ID and full details.

**Overall profile** adds up the wait of the listed runs and shows each part
as a gauge with its share and total: compiling, running tests, source sync,
artifacts back, hosted overhead, cargo startup, build directory lock, the
justrust client, and other. Timings come from run summaries on this machine:
account-only records are counted on the card's border as runs without timings.

**Compile profile** splits the compiler's own time for the same runs by what
was compiled: workspace crates, test targets, dependencies, proc macros, build
scripts, and cache restores. It sums unit times, and units run in parallel, so
its total is compiler time and exceeds the compiling share of the wait.

Both profiles follow the filters, so choosing a project or a command kind
shows that slice. They sit side by side at 110 columns or more and stack
otherwise. Terminals shorter than 24 rows hide both, and narrow terminals
shorter than 40 rows show only the overall profile.

**Versions** shows what is running on each side: this client (and whether it
is a published release or a source build), the hosted service deployment, the
compiler the service names for hosted builds with its protocol, and what the
build machine itself reports (toolchain, rustc, agent). The border says `up to
date`, `update available`, `ahead of release`, or `update required` when the
client cannot drive the hosted toolchain. A build machine on a different
toolchain from the service is flagged. It sits beside the account card at 110
columns or more, below it otherwise, and is hidden on narrow terminals shorter
than 30 rows. **v** opens the full list. It is refreshed every minute and on
**r**.

Builds in progress lead the Runs card as `building` rows with where they run,
the time so far, the estimate and the crates compiling. Rows alternate
background shades. Wall time is colored by length: green under 10 seconds,
plain under a minute, amber under five minutes, red beyond.

The cards follow Jcode's card style: a title and status on the top border,
label and value rows, and `▰▱` gauges.

Account records that carry no command are counted as `other`, never guessed.
Local history is a bounded recent window (up to 200 index entries); the account
list contains up to 50 uploaded runs. Older account records may show unknown
results until opened. Hosted uploads contain summaries, not full output logs.
The dashboard is read-only: it does not start/cancel builds, change routing,
subscribe, spend credits, or modify your account.

### Known work avoided

`justrust savings` totals recorded cache hits across retained run history.
Use `--days 7` for the last week, `--here` for this directory and its children,
or `--json` for structured totals. New run summaries also include the accounting.

The metric is **estimated task time avoided**: the original elapsed durations
of skipped compiler and build-script jobs minus measured cache-hit overhead.
Dependency hits include local, remote, peer, and prebuilt cache tiers without
double counting. Workspace cache hits and build-script hits are counted separately.
Negative savings are retained when restoring costs more than the original job.

This is not CPU time, user wait saved, or a stock-toolchain comparison. Concurrent
jobs are summed, original durations can come from different machines, and misses,
uploads, process startup, recorder work, and other build overhead are not deducted.
Older recordings lack full key-computation overhead and are marked incomplete.
Missing recordings are reported, not treated as zero. Totals cover retained
history on this client, not lifetime or fleet-wide savings. No extra compilation
or baseline replay is performed by this command.

## Remote builds

```sh
justrust login     # approve in the browser
justrust upgrade   # activate a remote build plan
justrust check
```

Signed in, builds use hosted machines by default on Linux, and your machine
does not compile. If a hosted build cannot run, justrust does not quietly
compile locally instead: it says what went wrong, how to report a service
fault (`justrust feedback --kind infra-urgent "..."`), and how to build here
when nothing else works (`--local="why"`). `justrust local on` brings back
the router and automatic local fallback, `justrust local off` refuses every
local compile. `--remote` and `--auto` (remote only when predicted faster)
choose per command.

Local builds are free. Hosted plans start at $20/month for 5,000 builds,
with the first month free and a card required. See [pricing](https://jcode.sh/rust#pricing).
For agents without a browser, `justrust login --no-wait` prints a sign-in link.
For CI, set `JCODE_API_KEY` from your [account](https://jcode.sh/account).
More setup details: [remote builds](docs/remote.md).

## Goal

**The shortest possible path from a source edit to a trustworthy test result.**

justrust is building one integrated system across the entire Rust build stack:
Cargo, rustc, code generation, linking, and test execution, backed by shared
artifact caches and our own build infrastructure. We optimize the whole loop,
not just individual compiler stages.

Today, that includes our Cargo and rustc forks, prebuilt dependencies, shared
caches, hosted builds, and test scheduling. The next steps are keeping compiler
state alive between edits, avoiding unnecessary downstream recompilation, and
updating test binaries without a full relink.

The goal is for agents to spend their time editing and reasoning, not waiting
for builds. We will fork tools and run infrastructure wherever that makes the
loop faster, while preserving build correctness and useful diagnostics.
We vertically integrate the entire toolchain end to end. Our forks are
independent development lines that can have completely different internals,
replace components, or merge them into one system. External correctness is
the compatibility contract: Rust semantics, existing projects, build scripts,
proc macros, platform ABIs, and trustworthy build and test results. Internal
upstream compatibility, upstreamability, small patches, and easy rebases are
not goals. Upstream is a source of useful fixes and ideas, not a limit on the
designs we can measure and ship.
See the [toolchain plan](docs/toolchain.md) for the roadmap.

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). The client is MIT. Vendored toolchain sources retain
their component licenses and third-party notices. See [LICENSING.md](LICENSING.md).
