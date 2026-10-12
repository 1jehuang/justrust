<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/justrust-icon-dark.svg">
  <img src="assets/justrust-icon.svg" alt="justrust: a halftone gear" width="140">
</picture>

# justrust

**Rust compile as a service for coding agents.**

Faster `check` and `test` on your laptop or on our build machines,
with output an agent can act on.

[Website](https://jcode.sh/rust) · [Install](#install) · [Remote builds](#remote-builds) · [How it is faster](#how-it-is-faster) · [Findings](FINDINGS.md) · [Changelog](CHANGELOG.md)

</div>

---

Coding agents run `cargo check` and `cargo test` hundreds of times a day and
spend most of that time waiting on the compiler. justrust takes cargo's
exact arguments, makes the loop faster (locally and remotely), prints only
what an agent needs, and records where every second went. Every
optimization ships only with before and after measurements, all in
[FINDINGS.md](FINDINGS.md).

On [uv](https://github.com/astral-sh/uv) (674 units, measured 2026-10-10,
laptop under typical multi-agent load vs a 16 vCPU build machine):

| | local cargo | justrust remote |
|---|---:|---:|
| fresh checkout, shared cache | 77.7 s | **22.9 s** (3.4x) |
| `test -p uv-resolver --no-run` | 73.6 s | **38.5 s** (1.9x) |
| cold `check --workspace` | 70.3 s | **52.2 s** (1.3x) |
| one-line edit, `check -p uv-resolver` | | **1.0 s** |

Remote builds leave no `target/` and use no rustc RAM or CPU on your machine.

## Install

```sh
curl -fsSL https://jcode.sh/rust.sh | sh
```

Prebuilt for Linux and macOS (x86_64, aarch64); a few Linux-only features
are listed under [Status](#status). Installs to
`~/.local/bin` (`JUSTRUST_INSTALL_DIR` to change it), verifies the sha256,
and runs `justrust login` when interactive (`JUSTRUST_NO_LOGIN=1` skips it).

What the installer sends: a random install id, the OS and CPU
architecture, and whether the install succeeded (or which step failed), to
build.jcode.sh. `justrust login` sends the same id with its sign-in flow id
so a sign-in can be matched to its install. Nothing else is sent before you
sign in, and no IP address is stored. `JUSTRUST_NO_TELEMETRY=1` or
`DO_NOT_TRACK=1` turns it off. Signed in, hosted builds also report each
remote run (command, exit code, time) and why a build stayed local.
From source: `cargo install justrust --locked` (Rust 1.91 or newer).

Then tell your agents to use it:

```sh
justrust agents      # adds a short block to Claude Code, Codex, Gemini CLI, and Jcode instructions
```

Or paste [docs/AGENTS-snippet.md](docs/AGENTS-snippet.md) into an
`AGENTS.md` yourself. Or give your agent this prompt:

```text
Install justrust with `curl -fsSL https://jcode.sh/rust.sh | sh`, run `justrust login --no-wait`,
give me the link it prints, and use `justrust check` / `justrust test` instead of cargo from now on.
```

## Use

```sh
justrust check -p my-crate
justrust test -p my-crate --lib some_module::
justrust test -p my-crate -- --nocapture
justrust log --grep warning     # full saved output of the last run
justrust show                   # where the time went
```

`check`, `test`, `build`, `clippy`, and `run` take exactly cargo's
arguments and return cargo's exit code. Output keeps errors and failing
tests in full and the first 3 warnings (`JUSTRUST_MAX_WARNINGS`), drops
`Compiling`/`Running`/`Finished` lines and passing tests, then ends with a
report: verdict, where the wall time went, why each crate rebuilt, and
`waste` lines for fixable inefficiencies.

```text
justrust: ok: 3 tests passed in 0.5s
justrust: 16 more warnings hidden, 1664 tests filtered out
justrust: full log `justrust log 20261007-202534713-2006572`, timing `justrust show 20261007-202534713-2006572`
```

```text
justrust: FAILED: 1 of 3 tests failed in 0.1s (0.1s compiling)
justrust: FAILED to compile (1 error) in 0.0s
```

Other cargo commands (`cargo fmt`, `cargo add`, `cargo tree`) stay plain cargo.

## Remote builds

```sh
justrust login     # approve in the browser: 25 free remote builds, no card
justrust check     # now runs on a build machine
```

Signed in, `check`, `clippy`, and `test` run remote by default. Choose per
command with `--local`, `--remote`, or `--auto` (remote only when predicted
faster), or `JUSTRUST_REMOTE=off|force|auto`. Offline, out of builds, or
signed out, the build runs locally and prints one line saying why.

- **Agents without a browser:** `justrust login --no-wait` prints the link
  for the user and returns at once. Builds stay local until it is approved.
- **CI:** create an API key at [jcode.sh/account](https://jcode.sh/account)
  and export it as `JCODE_API_KEY`. No login step needed.
- **Pricing:** after the free builds, `justrust upgrade`: from $20/month for
  5,000 remote builds (2,500 more per $10 of plan), $0.10 each after that.
  Local builds are always free. See [jcode.sh/rust](https://jcode.sh/rust#pricing).
- **MCP:** `justrust mcp` serves the account (status, builds left, sign-in
  link, API keys, billing links) to agents:
  `claude mcp add justrust -- justrust mcp`. Billing changes are links for
  the human, never actions the agent can take.

A sync daemon pushes edits on save, so a warm remote build pays only
milliseconds of sync. Details, the legacy own-AWS-machine backend
(`justrust remote up`), and measurements: [docs/remote.md](docs/remote.md).

## How it is faster

All of these are on by default unless marked opt-in, fail open to plain
cargo, and have an off switch.

| What | Effect (measured) | Off / on |
|---|---|---|
| **Prebuilt dependency store.** Popular crates.io crates compiled by our fleet, fetched only when downloading beats compiling here | cold check of 105 crates: 19-26 s vs 34-35 s (FINDINGS 44) | `JUSTRUST_PREBUILT=off` |
| **Shared dependency cache.** Content-addressed rustc outputs reused across target dirs, checkouts, and agents (sccache shares 0 of these) | cold Desktop check from cache: 53.3 s -> 14.8 s (FINDINGS 37) | `JUSTRUST_DEPCACHE=0`, `justrust cache` |
| **Test pool.** Test binaries in parallel, last failures and slow tests first, overlapped failures confirmed alone, fail-fast honored | first failure 14.4 s -> 7.0 s on Desktop (FINDINGS 34) | `JUSTRUST_TEST_PARALLEL=0` |
| **Per-agent build slots.** Each agent gets its own reflinked target dir, so nobody waits on cargo's lock | no lock waits between parallel agents | `JUSTRUST_SLOTS=0`, `justrust slots` |
| **Build priority.** Short edit builds get the fast cores when cold builds compete (cgroup weights, CPU affinity under sched_ext) | the 30-200 s contention tail (FINDINGS 13) | `JUSTRUST_SCHED=0` |
| **wild linker** for local crates, after `justrust toolchain wild` | link 0.92 s (mold) -> 0.35 s (FINDINGS 36) | `JUSTRUST_LINKER=off` |
| **jr-rustc proc-macro cache**, with a jr toolchain installed | Desktop loop 8.65 -> 5.70 s (FINDINGS 30) | `JUSTRUST_CACHE_PROC_MACROS=0` |
| **jr-cargo** (cargo fork: mimalloc, parallel manifest parsing) | no-op `test` 0.39 -> 0.22 s (FINDINGS 38, 39) | `JUSTRUST_CARGO_FORK=0` |
| **Early diagnostics** (opt-in). A resident rust-analyzer prints type errors while cargo runs | first type error in ~0.1 s (FINDINGS 28) | `JUSTRUST_EARLY=1` or `auto` |
| **`justrust split`.** Which module to move into its own crate to stop rebuild cascades, from your recorded edits; `--apply` does it | per-build hints name the move | `JUSTRUST_SPLIT_HINTS=0` |

## Recording and profiling

Every `justrust` build is recorded: cargo startup, lock waits, each rustc
unit (frontend, codegen, link, incremental cache, per-pass timings), build
gaps, test execution, CPU, memory, and machine-wide stalls.

```sh
justrust runs             # recorded runs
justrust show             # full breakdown of the latest run
justrust show <id> --json # machine-readable
justrust status           # what is compiling right now, and time left
justrust install          # optional `cargo` proxy so plain `cargo` calls are recorded too
```

```text
Where the wall time went
  cargo startup     0.45s    5%  █                        resolve, fingerprints, planning
  compiling         8.97s   95%  ███████████████████████  at least one rustc running
  running tests     0.01s    0%                           test binaries

  UNIT                                 SHARE    WALL     CPU   RMETA  PEAK MB  SPLIT (frontend/codegen/link/incr)
  jcode_desktop_ui (test)               9.0s    9.0s   10.3s       -     2260  4.4 / 1.6 / 0.7 / 1.1

  Slowest rustc passes in jcode_desktop_ui (test)
    macro_expand_crate                              2.40s
    codegen_crate                                   1.55s
    incr_comp_persist_result_cache                  1.07s
```

<details>
<summary>How recording works, and where the data lives</summary>

- cargo runs with `RUSTC` pointed at a justrust shim. `RUSTC` is not part of
  cargo's fingerprint, so recorded and unrecorded builds share artifacts:
  switching between them rebuilds nothing.
- The shim times each rustc with `wait4` (wall, user and system CPU, peak
  memory, including the linker) and records when `.rmeta` became available.
  On a pinned nightly every local crate gets `-Ztime-passes`. On stable it
  is added only when the unit already runs with `RUSTC_BOOTSTRAP=1`, since
  rustc's incremental cache keys on that (`JUSTRUST_PASSES=always|off`).
- A sampler reads `/proc` every 200 ms: the build's process tree, machine
  CPU, memory, pressure stalls, and competing rustc processes.
- Recording fails open. Overhead on a no-op build is within noise.

```text
~/.justrust/                  (JUSTRUST_HOME to move it)
  index.jsonl                 one summary line per run
  runs/<id>/meta.json         command, cwd, agent session, environment
  runs/<id>/summary.json      computed breakdown (what `show` prints)
  runs/<id>/units.jsonl       one record per rustc invocation, with pass timings
  runs/<id>/output.jsonl      every output line with a timestamp
  runs/<id>/samples.jsonl     200 ms CPU, memory, and pressure samples
  runs/<id>/processes.json    every process the build spawned
```

`JUSTRUST_DISABLE=1` turns recording off. `justrust history` mines past
[Jcode](https://github.com/1jehuang/jcode) sessions for cargo calls, and
`justrust export` writes slim, path-redacted run summaries to `data/runs/`.

</details>

## Toolchains

```sh
justrust toolchain                         # what builds here use, what is installed
justrust toolchain pin nightly             # latest complete nightly, writes justrust.toml, installs it (~25 s)
justrust toolchain pin nightly-2026-10-08  # or exact; X.Y.Z and beta-YYYY-MM-DD work too
justrust toolchain unpin
justrust toolchain wild                    # wild linker, pinned release, checksum verified
```

Toolchains come straight from static.rust-lang.org into
`~/.justrust/toolchains/<id>`, checksum-verified, no rustup. A pinned
project's builds use that cargo, rustc, clippy, rustfmt, and rustdoc.
`JUSTRUST_TOOLCHAIN=<spec>` overrides the pin for one run,
`JUSTRUST_TOOLCHAIN=system` turns it off. A missing toolchain installs on
first use (`JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` to never download during a
build); if that fails the build uses the system toolchain and says so.

**jr-rustc**, justrust's rustc fork: `justrust toolchain jr-install <build dir>`
packages a build as `jr-<commit>`. With nothing pinned, builds use the
newest jr toolchain, and every incremental crate gets
`-Zcache-proc-macros`, which reuses derive and attribute macro output when
the macro's input tokens are unchanged (724 to 1 `#[gpui::test]` expansions
per Desktop edit). A proc macro whose output depends on something other
than its input (a file it reads, an env var) is not re-run when only that
changes. Dependencies keep their cache keys.

**wild**: once installed, local crates that link (test binaries, bins,
cdylibs) link with [wild](https://github.com/wild-linker/wild) through a
`-B` linker directory, which works with `cc`, `clang -fuse-ld=mold`, and
rustc's bundled lld alike. Link args are untracked, so switching it
rebuilds nothing. If wild fails, the original linker runs with the same
arguments (logged in `~/.justrust/linker-fallbacks.jsonl`).

## Status

Alpha (`0.1.0-alpha.7`). Output formats, `~/.justrust`, and environment
variables may change between releases ([CHANGELOG.md](CHANGELOG.md)).

- **Linux:** everything.
- **macOS:** everything except what has no macOS counterpart: CPU priority
  between concurrent builds (cgroups), pressure-stall reporting (reports
  say "not measured" instead of showing zeros), the wild linker and hot
  patching (ELF only), and automatic remote routing (the build machines
  are Linux, so a Mac builds locally unless `JUSTRUST_REMOTE=force`).
  Process sampling uses libproc and Mach, dependency-cache restores and
  build slots clone with APFS `clonefile`, and the sync daemon watches
  with kqueue.
- **Windows:** recording, agent output, reports, the depcache (plain
  copies with a fresh mtime, no hash memo since NTFS has no xattrs that
  leave mtime alone), the test pool, pinned toolchains, and the build
  daemon. Build CPU comes from a job object around cargo,
  processes from Toolhelp. Not on Windows: per-agent build slots (they
  need copy-on-write clones; builds share the target dir), cgroup priority, pressure-stall
  reporting, wild and hot patching (ELF only), the sync daemon's file
  watcher (pushes fall back to the per-run rescan), ssh connection reuse
  (Windows OpenSSH has no ControlMaster), and automatic remote routing
  (the build machines are Linux).

## Where this is going

justrust is going all in on optimal compile times: one tool that owns the
whole path from source to test result, including a vendored and forked
toolchain and our own servers. An idea is dropped only when measurements
show it is not faster. Plan: [docs/toolchain.md](docs/toolchain.md).

| | |
|---|---|
| Done | recorder, agent output, pinned toolchains, depcache, public prebuilt store, test pool, build slots and priority, wild, jr-cargo, jr-rustc with the proc-macro cache, hosted remote builds |
| In progress | hermetic C sysroot for `-sys` crates ([docs/c-sysroot.md](docs/c-sysroot.md)), hot patching of test binaries (prototype: link 0.54 -> 0.20 s, FINDINGS 35) |
| Next | resident compiler in jr-rustc (query cache in memory between edits), streamed diagnostics, body-only upstream edits that do not invalidate downstream codegen |

The reference loop, a one-line edit in `jcode-desktop-ui` (122k lines) and
3 tests, is 7.6 s today (`bench/suite.py run desktop-test-edit`). Estimated
end state with a resident compiler, Cranelift, and hot patching: about
0.4 s, with the first type error in 0.1 to 0.2 s. These are estimates;
each step is measured before and after in [FINDINGS.md](FINDINGS.md).

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). MIT, except the self-hostable cache
server in `server/` under FSL-1.1-ALv2. See [LICENSING.md](LICENSING.md).
