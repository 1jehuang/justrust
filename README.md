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

## Use

```sh
justrust check -p my-crate
justrust test -p my-crate --lib some_module::
justrust build
```

`check`, `test`, `build`, `clippy`, and `run` take cargo's arguments and
return its exit code. Errors and failing tests stay in full, followed by
a concise result and timing report. Use `justrust log` for the full output
and `justrust show` for the timing breakdown.

Other cargo commands (`cargo fmt`, `cargo add`, `cargo tree`) stay plain cargo.

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

Signed in, `check`, `clippy`, and `test` use hosted build machines by default
on Linux. Choose `--local`, `--remote`, or `--auto` (remote only when predicted
faster). If hosted builds are unavailable, justrust falls back to local builds.

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

See [CONTRIBUTING.md](CONTRIBUTING.md). The client is MIT, and the self-hostable
cache server in `server/` is FSL-1.1-ALv2. Vendored toolchain sources retain
their component licenses and third-party notices. See [LICENSING.md](LICENSING.md).
