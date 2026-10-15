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

> **Clean-room rewrite branch:** the installed service described below is the
> stable implementation. This branch now includes `jr-rust`, an experimental
> compiler for a small Rust subset that emits native Linux x86-64 executables
> directly, plus the `jr-prototype` graph-runtime fixtures. See
> [COMPILER.md](COMPILER.md) for supported syntax, commands and limitations, and
> [ENGINE.md](ENGINE.md) for the graph runtime. It cannot compile general Rust
> projects yet. Do not install it over the stable client.

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

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). The client is MIT, and the self-hostable
cache server in `server/` is FSL-1.1-ALv2. Vendored toolchain sources retain
their component licenses and third-party notices. See [LICENSING.md](LICENSING.md).
