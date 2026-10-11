# justrust

Fast Rust compile for coding agents.

Coding agents run `cargo check` and `cargo test` hundreds of times a day, and
spend most of that time waiting on the compiler rather than running tests.
justrust aims to make the agent edit, check, and test loop as fast as possible.
Every optimization has to be justified by measurements from real agent sessions.

## Status

Alpha (0.1.0-alpha). Linux only: on macOS and Windows the binary builds and
`justrust check|test|build|clippy|run` fall back to plain cargo without
recording. Output formats, the data layout under `~/.justrust`, and
environment variables may change between releases. Remote builds are
experimental and off unless you configure a machine. See
[CHANGELOG.md](CHANGELOG.md).

## Install

Prebuilt binary (Linux and macOS, x86_64 and aarch64), no compile:

```sh
curl -fsSL https://jcode.sh/rust.sh | sh
```

It installs to `~/.local/bin` (override with `JUSTRUST_INSTALL_DIR`), verifies
the sha256, and runs `justrust login` when interactive (`JUSTRUST_NO_LOGIN=1`
skips it). From source:

```sh
cargo install justrust --locked
```

Or from a checkout: `cargo install --path . --locked`. Requires Rust 1.91 or
newer (edition 2024). Then point your agents at it with the snippet in
[docs/AGENTS-snippet.md](docs/AGENTS-snippet.md).

## Quickstart: remote builds in 3 steps

```sh
curl -fsSL https://jcode.sh/rust.sh | sh   # install (runs `justrust login`)
justrust login                             # approve in the browser: 25 free builds
justrust check                             # slow builds now go to a build machine
```

Agents without a browser: `justrust login --no-wait` prints the link for the
user and returns at once. Builds stay local until it is approved. After the
free builds, `justrust upgrade` ($10/mo, 500 builds). Out of builds, offline,
or signed out: every build runs locally, with one line saying why.
Signed in (API key or subscription), check, clippy, and test run remote by
default. The agent picks per command: `--local`, `--remote`, or `--auto` (the
router: remote only when predicted faster), or `JUSTRUST_REMOTE=off|force|auto`.

CI and headless machines: create an API key at https://jcode.sh/account and
export it as `JCODE_API_KEY`. It takes precedence over the saved login, so no
`justrust login` step is needed.

MCP: `justrust mcp` is an MCP server for the Jcode account (status and builds
left, sign-in link, API keys, billing links). Add it to an agent with
`claude mcp add justrust -- justrust mcp` (or `{"command": "justrust",
"args": ["mcp"]}` in any MCP client). Billing changes are links for the
human, never actions the agent can take.

## What exists so far

- **Agent interface.** `justrust check|test|build|clippy|run` takes cargo's
  arguments and prints only what an agent needs to act on, followed by a
  one-line verdict. Every run is recorded.
- **Build recorder.** Every recorded `cargo` invocation gets a full profile of
  where its time went: cargo startup, lock waits, each rustc unit (with
  frontend, codegen, link, and incremental-cache split), build gaps, test
  execution, CPU, memory, and machine-wide stalls.
- **`justrust split`** says which code to move into its own crate to stop
  rebuild cascades, from the recorded edits in this workspace and its module
  graph, with the estimated saving and the references or impls to fix first.
  The same analysis runs incrementally: every build appends what it saw to
  `~/.justrust/split/`, a niced background refresh keeps per-file hints
  current, and a build that edits an expensive file names the specific move
  in its waste report (`JUSTRUST_SPLIT_HINTS=0` disables the refresh).
- **`justrust history`** mines past [Jcode](https://github.com/1jehuang/jcode)
  sessions for cargo calls. See [FINDINGS.md](FINDINGS.md).
- **Pinned toolchains** (phase 1 of [docs/toolchain.md](docs/toolchain.md)).
  `justrust toolchain pin nightly` writes `justrust.toml` and installs the
  latest complete nightly (rustc, cargo, std, rust-src, clippy, rustfmt,
  Cranelift, rustc-dev, llvm-tools) straight from static.rust-lang.org into
  `~/.justrust/toolchains/<id>`, checksum-verified, no rustup. Every build
  in that project then uses it. See [Pinned toolchain](#pinned-toolchain).

## Agent interface

```sh
justrust check -p my-crate
justrust test -p my-crate --lib some_module::
justrust log --grep warning     # full saved output of the last run
justrust show                   # where the time went
```

Output keeps errors and failing tests in full and the first 3 warnings
(`JUSTRUST_MAX_WARNINGS`). It drops `Compiling`/`Running`/`Finished` lines and
passing `... ok` test lines, then ends with a verdict:

```text
justrust: ok: 3 tests passed in 0.5s
justrust: 16 more warnings hidden, 1664 tests filtered out
justrust: full log `justrust log 20261007-202534713-2006572`, timing `justrust show 20261007-202534713-2006572`
```

```text
justrust: FAILED: 1 of 3 tests failed in 0.1s (0.1s compiling)
justrust: FAILED to compile (1 error) in 0.0s
```

To make agents use it by default, add the snippet in
[docs/AGENTS-snippet.md](docs/AGENTS-snippet.md) to your global or project
`AGENTS.md`.

## Recording builds

```sh
justrust install          # puts a `cargo` proxy in ~/.local/bin (must precede the real cargo on PATH)
cargo test -p my-crate    # runs exactly as before, then prints one summary line
justrust runs             # list recorded runs
justrust show             # full breakdown of the latest run
justrust show <id> --json # machine-readable summary
```

Or without the proxy: `justrust cargo test -p my-crate`.

Example from a one-line edit in a 143k-line crate:

```text
justrust: 9.5s · startup 0.5s · compile 9.0s | top jcode_desktop_ui (test) 9.0s (frontend 4.4 codegen 1.6 link 0.7) | cpu 11s (1.2 cores)
```

`justrust show` expands that into:

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
    ...
```

### How it works

- The `cargo` proxy runs the real cargo with `RUSTC` pointed at a justrust
  shim, tees its output byte for byte, and keeps exit codes, colors, and
  progress bars unchanged. Non-build subcommands (`metadata`, `fmt`, `--version`)
  are passed straight through.
- The shim times each rustc invocation with `wait4` (wall, user and system
  CPU, peak memory, including the linker) and records when `.rmeta` became
  available for pipelining. When the unit already runs with `RUSTC_BOOTSTRAP=1`,
  it also adds `-Ztime-passes`. rustc's incremental cache does not key on
  `-Ztime-passes`, but it does key on `RUSTC_BOOTSTRAP`, so the shim never
  changes the bootstrap mode on its own. Set `JUSTRUST_PASSES=always` to force
  pass timings everywhere, which costs one cold incremental rebuild per mode
  change.
- A sampler reads `/proc` every 200 ms for the build's process tree (CPU, RSS),
  whole-machine CPU, available memory, pressure-stall info, and rustc processes
  from other builds that compete for the CPU.
- `RUSTC` is not part of cargo's fingerprint, so recorded and unrecorded
  builds share artifacts. Switching between them does not trigger rebuilds.
- Recording fails open. If anything goes wrong, the real cargo runs unchanged.
  Measured overhead on a no-op build is within noise (about 10 to 30 ms).

### Data layout

```text
~/.justrust/                  (override with JUSTRUST_HOME)
  index.jsonl                 one summary line per run
  runs/<id>/meta.json         command, cwd, agent session, environment
  runs/<id>/summary.json      computed breakdown (what `show` prints)
  runs/<id>/units.jsonl       one record per rustc invocation, with pass timings
  runs/<id>/output.jsonl      every output line with a timestamp
  runs/<id>/samples.jsonl     200 ms CPU, memory, and pressure samples
  runs/<id>/processes.json    every process the build spawned
```

Environment: `JUSTRUST_DISABLE=1` turns recording off, `JUSTRUST_QUIET=1`
hides the summary line, and `JUSTRUST_PASSES=off|always` controls pass timings.
Linux only for now.

## Pinned toolchain

```sh
justrust toolchain pin nightly             # latest complete nightly, writes justrust.toml, installs (~25 s, 1.7 GB)
justrust toolchain pin nightly-2026-10-08  # or an exact one; X.Y.Z and beta-YYYY-MM-DD work too
justrust toolchain                         # what builds here use, what is installed
justrust toolchain unpin                   # back to the system toolchain
justrust toolchain remove nightly-2026-10-08
```

```toml
# justrust.toml (found in the build directory or any parent)
[toolchain]
channel = "nightly-2026-10-08"
```

Recorded builds (`justrust check|test|build|clippy|run`, the `cargo` proxy)
run the pinned cargo with the pinned rustc behind the recorder, and the
toolchain's `bin/` first on `PATH`, so clippy, rustfmt and rustdoc match.
Other cargo commands through the proxy use it once it is installed.
`JUSTRUST_TOOLCHAIN=<spec>` overrides the file for one run and
`JUSTRUST_TOOLCHAIN=system` turns it off. A missing toolchain is installed
on first use (`JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` to never download during a
build). If that fails the build runs on the system toolchain, says so, and
does not retry for 10 minutes. On a pinned nightly, every local crate gets
rustc's per-pass timings (no `RUSTC_BOOTSTRAP` cache flip needed).

## wild linker

```sh
justrust toolchain wild     # download wild (pinned release, sha256 checked) into ~/.justrust/toolchains/linkers
```

Once installed, every local crate that links (test binaries, bins, cdylibs)
links with [wild](https://github.com/wild-linker/wild) instead of the
project's linker. Measured on the Desktop UI test binary (FINDINGS 35): the
link itself 0.92 s with mold, 0.35 s with wild. The shim adds
`-B ~/.justrust/bin/ld` to those rustc runs. gcc and clang take `ld`,
`ld.mold`, `ld.lld`, ... from a `-B` directory before their own, whatever
`-fuse-ld` says, so it works with `cc`, `clang -fuse-ld=mold` wrappers and
rustc's bundled lld alike. Link args are untracked by rustc and cargo, so
switching it on or off rebuilds nothing. Dependencies are untouched.
If wild fails, the linker the driver asked for runs with the same
arguments (logged in `~/.justrust/linker-fallbacks.jsonl`), so a wild bug
costs one extra link, never a failed build. `JUSTRUST_LINKER=off` disables
it.

## Direction: all in

justrust is going all in on optimal compile times. It will become one
all-in-one solution that owns the whole path from source to test result, and
we will do whatever it takes to get there: vendor and fork the toolchain, and
run our own servers. Maintenance cost is accepted. An idea is dropped only
when measurements show it is not faster. Detailed plan:
[docs/toolchain.md](docs/toolchain.md).

What justrust will own:

- **A vendored toolchain.** rustc (a pinned nightly, then our own fork),
  LLVM, cargo, the standard library, Cranelift, a linker (wild, mold), the C
  toolchain and the system libraries that `-sys` crates link against,
  rust-analyzer, and the test runner. One exact toolchain on every machine
  means we control every flag, and cache keys match everywhere.
- **Prebuilt artifact servers.** A cargo-like registry that serves compiled
  dependencies (rlibs, proc macros, build-script outputs) for that exact
  toolchain, so a fresh checkout downloads its dependencies instead of
  compiling them.
- **A remote compile service.** Server-grade CPUs with warm incremental state
  and the shared cache. justrust picks remote or local for each build,
  whichever its measurements say is faster: remote for cold builds, full-crate
  rebuilds, and big test suites, local for small edits.
- **A resident compiler.** A rustc that stays running between edits and
  keeps its incremental state in memory, recompiling only the items that
  changed.
- **Hot patching.** When an edit only changes function bodies, patch the
  running test binary instead of relinking it.

### Theorized end-state gains

These are estimates, not measurements. Each phase gets measured before and
after, and the results go in [FINDINGS.md](FINDINGS.md).

The reference loop is a one-line body edit in `jcode-desktop-ui` (122k
lines), then running 3 tests (run 20261007-235214927-3498261):

| phase | today | end state | how |
|---|---:|---:|---|
| cargo startup, fingerprints | 0.33 s | 0.02 s | daemon keeps the build graph in memory, file watcher instead of a rescan |
| macro expansion | 2.70 s | 0.05 s | resident compiler re-expands only the edited item |
| resolve, typeck, borrowck | 1.96 s | 0.10 s | only the edited body and its dependents are checked again |
| incremental load and save | 0.85 s | 0 s | state stays in memory, written to disk in the background |
| codegen | 1.45 s | 0.10 s | one small codegen unit, Cranelift |
| link | 0.63 s | 0.05 s | hot patch, or incremental linking (wild) |
| other | 1.06 s | 0.05 s | |
| **total** | **9.0 s** | **~0.4 s** | **about 20x** |

Time to the first type error drops from a full build to about 0.1 to 0.2 s,
because diagnostics stream out before codegen starts.

Other scenarios:

| scenario | today | end state | how |
|---|---:|---:|---|
| edit in a shared upstream crate (finding 5) | 46 s | ~1 s | a body-only change upstream does not invalidate downstream codegen |
| fresh checkout or new agent target dir | 182 s cold, 16 s with the local depcache | ~5 s | prebuilt dependencies downloaded, local crates from the remote cache |
| full rebuild of the UI crate | 20 to 32 s | ~5 s | remote server with many cores, Cranelift for local crates |
| full Desktop test suite (1,655 tests) | 16 to 28 s run time | ~3 s | sharded across remote cores |

Taken together: in the measured history, agents in Jcode Desktop were
blocked on cargo for 41 h over about six weeks, with a median call of 18 s.
If the median call becomes about 1 s and the slow tail mostly moves to fast
remote machines, that waiting drops by about 90 to 95%, to roughly 2 to 4 h.
It also removes the indirect costs: agents stop batching edits to avoid
builds, parallel agents stop slowing each other down, and a fifth of build
time no longer goes to learning about a type error.

### Order

1. **Measure.** Record every agent build (done), then replay real agent edit
   loops as a benchmark.
2. **Pinned toolchain, managed by justrust**, plus nightly flag experiments
   (`-Zcache-proc-macros`, `-Zshare-generics`, dependencies as one dylib,
   Cranelift for local crates, wild).
3. **Share artifacts.** Local depcache (done), then prebuilt artifact servers
   and a hermetic C sysroot.
4. **Remote compile service** that is chosen per build when it is faster.
   Experimental. Working: `check`, `clippy`, and `test` route to a remote machine when
   measured faster (a machine in your AWS account via `justrust remote up`,
   or any ssh host via `justrust remote use ssh`), with a sync daemon that
   pushes edits on save. Cold Jcode Desktop `check` 44 s vs ~180 s local.
   Hosted builds with a Jcode subscription (`justrust remote use hosted`)
   are built client side, waiting on the server
   ([docs/remote.md](docs/remote.md)).
5. **Our rustc fork**: resident compiler, streamed diagnostics, body-only
   upstream invalidation.
6. **Hot patching** of test binaries.

## License

MIT, except the self-hostable server in `server/` under FSL. See
[LICENSING.md](LICENSING.md).
