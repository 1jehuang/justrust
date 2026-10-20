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

> **Note:** `main` is the clean-room compiler rewrite (`jr-rust`), which supports
> only a small Rust subset so far ([COMPILER.md](COMPILER.md)). The client and
> service described below are released from the
> [`stable`](https://github.com/1jehuang/justrust/tree/stable) branch.

justrust vertically integrates and optimizes the Rust build toolchain to make
the entire edit, check, and test loop faster. Use it locally or with our remote
build service.

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
compilation, code generation, linking, and test execution, backed by shared
artifact caches and our own build infrastructure. We optimize the whole loop,
not just individual compiler stages.

The goal is for agents to spend their time editing and reasoning, not waiting
for builds. We are rewriting the internals from scratch, rather than embedding
Cargo, rustc, or their vendored implementations, while preserving build correctness
and useful diagnostics. We use the standard Rust toolchain to build the new
implementation and keep utility dependencies minimal. External correctness is
the compatibility contract: Rust semantics,
existing projects, build scripts, proc macros, platform ABIs, and trustworthy
build and test results.

## Design

The new compiler is one demand-driven graph, not a pipeline of phases. File
discovery, parsing, name resolution, type and borrow checking, lowering, code
generation, linking and tests are all designed as nodes that produce and consume immutable,
typed facts. A node runs as soon as the facts it reads exist, not when a global
phase ends, so code generation for one function can overlap checking of an
unrelated one. A node that is missing an input suspends only the branch that
needs it, never a worker thread. Success is released only after every required
check passes.

Nodes are keyed per file, per item, or per generic instance, so equivalent work
shares one node and is computed once within a build. Because every compiler
fact lives in the graph, with no side tables or global state (enforced by
[GRAPH-INTEGRITY.md](GRAPH-INTEGRITY.md)), the same identities are meant to
become the units for incremental rebuilds, persistent and remote caching,
and scheduling across cores and machines. Today reuse is within a single build
only, a few stages (parsing, resolution) are still whole-program nodes being
split, and cross-build persistence and shared caches are planned, not built.
See [ARCHITECTURE.md](ARCHITECTURE.md) and [ENGINE.md](ENGINE.md) for details.

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). The compiler is source-available under
FSL-1.1-ALv2: use it for anything except a competing product or service, and
each release becomes Apache 2.0 after two years. Contributions are accepted
under Apache 2.0. Third-party sources keep their original licenses. See
[LICENSING.md](LICENSING.md).
