# justrust

An attempt at a faster all-in-one Rust compiling solution for coding agents.

Coding agents run `cargo check` and `cargo test` hundreds of times a day, and
spend most of that time waiting on the compiler rather than running tests.
justrust aims to make the agent edit, check, and test loop as fast as possible.
Every optimization has to be justified by measurements from real agent sessions.

## Status

Early. What exists so far:

- **Agent interface.** `justrust check|test|build|clippy|run` takes cargo's
  arguments and prints only what an agent needs to act on, followed by a
  one-line verdict. Every run is recorded.
- **Build recorder.** Every recorded `cargo` invocation gets a full profile of
  where its time went: cargo startup, lock waits, each rustc unit (with
  frontend, codegen, link, and incremental-cache split), build gaps, test
  execution, CPU, memory, and machine-wide stalls.
- **`justrust history`** mines past [Jcode](https://github.com/1jehuang/jcode)
  sessions for cargo calls. See [FINDINGS.md](FINDINGS.md).

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
cargo install --path .
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

## Direction

1. **Measure.** Record every agent build (this), then build a replay benchmark
   of real agent edit loops.
2. **Check before test.** Report errors from a fast `check` instead of waiting
   on test-binary codegen and linking.
3. **Run only the affected tests.** Avoid rebuilding and linking huge test
   binaries for a handful of tests.
4. **Keep the compiler warm.** Use a persistent build daemon so incremental
   state and metadata do not restart from zero on every call.
5. **Share artifacts.** Use a content-addressed cache across checkouts,
   worktrees, agents, and machines.

## License

MIT
