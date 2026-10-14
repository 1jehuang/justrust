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

On [uv](https://github.com/astral-sh/uv) (674 units, measured 2026-10-10,
laptop under typical multi-agent load vs a 16 vCPU build machine):

| Operation | Standard toolchain (local) | justrust (local) | justrust (remote) |
|---|---:|---:|---:|
| Fresh checkout, shared cache | 77.7 s | Not measured | **22.9 s** |
| `test -p uv-resolver --no-run` | 73.6 s | Not measured | **38.5 s** |
| Cold `check --workspace` | 70.3 s | Not measured | **52.2 s** |
| One-line edit, `check -p uv-resolver` | Not measured | Not measured | **1.0 s** |

![Build time comparison for uv. Standard local: 77.7, 73.6, and 70.3 seconds. justrust remote: 22.9, 38.5, and 52.2 seconds. justrust local has not been measured.](assets/build-benchmarks.svg)

Lower is better. Missing measurements are not estimates. These results compare
a laptop under load with a hosted build machine, not toolchain-only performance.

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

Prebuilt for Linux and macOS (x86_64, aarch64). The installer places
justrust in `~/.local/bin` and starts sign-in. `justrust agents` adds usage
instructions to your coding agents, or copy the [AGENTS snippet](docs/AGENTS-snippet.md).

From source: `cargo install justrust --locked` (Rust 1.91 or newer).
Prebuilt installs update automatically. Disable updates with
`justrust update off`, and anonymous usage telemetry with `justrust telemetry off`.

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
See the [toolchain plan](docs/toolchain.md) for the roadmap.

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). MIT, except the self-hostable cache
server in `server/` under FSL-1.1-ALv2. See [LICENSING.md](LICENSING.md).
