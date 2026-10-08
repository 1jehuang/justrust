# Findings: where agent build time goes

Data: `~/.jcode/sessions` on one developer machine (16-thread Core Ultra X9,
62 GiB). Covers Jcode and Jcode Desktop work from late August to early October
2026. Collected with `justrust history`. "Foreground" means the agent was
blocked waiting on the tool call. Calls over 30 minutes (hung processes) are
excluded.

## Headline numbers

| Repo                     | Foreground cargo calls | Agent-blocked time | p50   | p90    |
|--------------------------|-----------------------:|-------------------:|------:|-------:|
| Jcode + Jcode Desktop    | 3,470                  | 52.3 h             | 16 s  | 134 s  |
| Jcode Desktop only       | 2,557                  | 41.1 h             | 18 s  | 148 s  |

By kind (Desktop): test 27.7 h, build 8.7 h, check 4.7 h, fmt 0.1 h.

## 1. Test calls are 90% compile, 10% tests

For test calls whose output included libtest results:

- Wall time: 21.0 h
- Actually executing tests: 2.1 h (10%)
- Compiling, linking, and overhead: 18.9 h (90%)
- Median tests run per call: about 20 (p90 about 1,400)

Agents typically ask for about 20 tests and pay for rebuilding and relinking
the whole test binary to get them.

## 2. One crate dominates

`cargo test -p jcode-desktop-ui [--lib]` alone accounts for 18 h, almost half
of all Desktop wait time. `jcode-desktop-ui` has 143k lines and 941 `#[test]`s
in one crate. Its debug test binary is about 670 MB.

Measured a controlled single-line edit in `fps_counter.rs`, using
`cargo test -p jcode-desktop-ui --lib --no-run` with mold and the parallel
frontend already enabled:

| Scenario                           | Wall   |
|------------------------------------|-------:|
| No-op (nothing changed)            | 0.5 s  |
| Body edit (first, upstream dirty)  | 32.3 s (jcode-base 10.1 s + desktop-ui 21.7 s) |
| Edit adding an item                | 9.9 s  |
| Revert edit                        | 9.5 s  |

So a best-case one-line change in this crate costs about 9 to 10 s. The
historical p50 is 24 s and p90 is 163 s, meaning most real calls pay well
beyond that best case: upstream crates rebuilding, cold incremental caches,
flag or target-dir changes, and lock waits.

## 3. A fifth of the time ends in a compile error

581 foreground calls (11.7 h, 22%) ended with `error:` output. Those calls
waited on full test or build pipelines just to learn about a type error that
`cargo check` (or a warm rust-analyzer) could have reported sooner.

## 4. Smaller effects

- Cargo lock contention: 107 calls, 0.8 h. Parallel agents in the same
  checkout serialize on `target/`.
- 1,300 background cargo calls are not counted above. They still use CPU and
  the cargo lock, which slows down foreground calls.
- `target/` in Jcode Desktop is 466 GB.
- Test execution is rarely the bottleneck: p50 0.5 s, p90 about 20 s per call.

## 5. Observed: a dependency edit forces a full rebuild of the downstream crate

Run `20261007-214608994-2456373`. A one-line test-body edit in
`jcode-desktop-ui` took **46.3s**, against 9.5s for the same edit earlier.
`jcode_base` (a path dependency in `~/jcode`, edited by another agent at the
same time) had changed and rebuilt in 10.5s. Because its metadata changed,
`jcode_desktop_ui (test)` reused no codegen units: `codegen_crate` took 23.6s,
monomorphization 9.4s, and LLVM 11.9s, using 118s of CPU.

Implication: parallel agents editing shared upstream crates turn cheap
incremental builds into near-full rebuilds for everyone downstream. Candidates
are isolated per-agent build dirs pinned to a stable upstream snapshot, and
sharing or avoiding codegen when only an upstream body (not its API) changed.

## Implications for justrust

1. Optimize compile and link for test binaries, not test execution.
2. Answer with check-speed diagnostics first. Only build test binaries once
   the code type-checks.
3. Huge single-crate test binaries are the main cost center. Build and link
   only what the requested tests need, or keep the test binary hot-patchable.
4. Keep incremental state warm and isolated per agent so calls hit the 10 s
   best case, or better, instead of the 24 s median.

## 6. Jcode Desktop compiles gpui twice: build vs test (gpui `test-support`)

Measured 2026-10-07 on a scratch copy of `~/jcode-desktop` (HEAD 9a92572,
separate `CARGO_TARGET_DIR`s under `~/.jcode/scratch/gpuifeat`). Benchmark:
delete the `gpui-*` fingerprint dirs (forces gpui and its dependents to
rebuild), then `justrust build -p jcode-desktop -p jcode-desktop-ui --lib`
followed by `justrust test -p jcode-desktop-ui --lib -- fps_counter`.

Cause: both Desktop manifests enable gpui's `test-support` only under
`[dev-dependencies]`. With resolver 2 the test unit graph turns on
`test-support` (= `leak-detection` + `backtrace` + `proptest` +
`collections/test-support` + `http_client/test-support`). Unit graph diff
(`cargo build/test --unit-graph -Z unstable-options`, comparing units by
features, profile and transitive deps): build 950 units, test 959, 927 shared,
32 test-only. The expensive test-only ones are gpui, gpui_linux, gpui_wgpu,
gpui_platform, gpui_thinking_orbs, sum_tree, zlog, ztracing, plus
jcode_sdk/jcode_desktop_api/jcode_desktop_ui and proptest's deps. A second
split comes from the `hdrhistogram = "7.6"` dev-dep (default features
`serialization`, `sync`) versus gpui's `default-features = false` one.

| run | build | test | gpui-family units (build+test) |
|---|---|---|---|
| before1 | 53.3s | 61.5s | 3+3 |
| before2 | 73.3s | 103.9s | 3+3 |
| before3 | 93.0s | 97.5s | 3+3 |
| after1 | 72.8s | 10.9s | 3+0 |
| after2 | 85.3s | 25.0s | 3+0 |
| after3 | 89.3s | 13.7s | 3+0 |

"after" = gpui `test-support` in normal `[dependencies]` of both crates and
`hdrhistogram` dev-dep with `default-features = false`. The unit graph then
differs only in the jcode_desktop_ui test unit itself. Medians: before
73.3s + 97.5s = 170.8s, after 85.3s + 13.7s = 99.0s, about 72s (42%) saved
per gpui-invalidating build+test cycle. The test step drops from about 98s to
about 14s. The machine was shared (other agents compiling), so absolute
times are noisy (build 53-93s for the same work). The test-step drop is far
larger than the noise. Run ids: before1 20261007-223529986-2792438 /
20261007-223623262-2799898, before2 20261007-224549993-2900285 /
20261007-224703347-2909978, before3 20261007-224847324-2927272 /
20261007-225020339-2939751, after1 20261007-224420268-2886179 /
20261007-224533055-2898124, after2 20261007-225158015-2964597 /
20261007-225323336-2976054, after3 20261007-225348582-2979157 /
20261007-225517874-2988351.

Not applied, because no option was safe:

- Enabling `test-support` in `[dependencies]` also enables it for release and
  shipped builds. `[target.'cfg(debug_assertions)'.dependencies]` does not help.
  Cargo evaluates target cfgs against the target platform, not the profile, so
  the unit graph of `cargo build --release` still had `test-support` (checked).
- It changes runtime behavior of the dev app (Ctrl+R hot reload and the
  running Desktop use dev builds): `leak-detection` takes the
  `EntityRefCounts` write lock and inserts into a HashMap on every
  `Entity`/`AnyEntity` creation, clone and drop, and `LeakDetector::drop`
  panics with "Exited with leaked handles" if any entity handle is alive when
  the App is torn down. It also adds `debug_bounds` and `debug_selector`
  bookkeeping each frame (Desktop uses `debug_selector` in about ten places),
  the test scheduler branch in `ForegroundExecutor::new`, and turns off the
  profiler's foreground runnable counter only for test dispatchers.
- Making tests drop `test-support` is not possible. 134 Desktop test files use
  `#[gpui::test]`, `TestAppContext`, `run_until_parked` and `add_window_view`,
  which only exist with that feature.
- A Desktop cargo feature (say `dev-tools = ["gpui/test-support"]`) that Ctrl+R,
  the launcher and agents always pass would unify the graphs, but every build
  path (rust-analyzer, `desktop_selfdev`, scripts, CI) must agree or it
  reintroduces the split and an ABI split between host and plugin. Ctrl+R
  builds `-p jcode-desktop -p jcode-desktop-ui` so the host and plugin share
  one GPUI feature set. It also carries the same behavior change above.

The safe part on its own: the `hdrhistogram` dev-dep with
`default-features = false` only removes the hdrhistogram/crossbeam_channel
test-only units (small, under 1s). It is not worth a separate change because
the gpui split dominates.

Possible real fixes, all upstream in the gpui fork (github.com/1jehuang/gpui):
split `test-support` so the test APIs (`TestAppContext`, `gpui::test`,
`TestPlatform`) compile always (or behind a no-runtime-effect feature),
while `leak-detection` stays opt-in. Or make leak detection a runtime toggle.
Then Desktop could enable the API-only feature in `[dependencies]` with no
runtime change and the 32-unit split disappears. For justrust: a finding
that detects "the same package compiled with different feature sets in check,
build and test" (dev-dependency feature unification) and names the
dev-dependency causing it would have found this immediately.

## Cranelift backend for Desktop debug/test builds (2026-10-07)

Availability: the system toolchain (Arch rust 1.98.1, no rustup) ships no
`codegen-backends/` dir and no `rustc-dev`, so a cranelift backend cannot be
built or loaded for it. The only practical route is a rustup nightly with
`rustc-codegen-cranelift-preview`, installed user-local under
`~/.jcode/scratch/cranelift/{rustup,cargo}` (rustc 1.101.0-nightly
1d81eb4ad 2026-10-07). Implications: a different rustc means separate
fingerprints and a separate cold dependency build (~3 min, ~4 GB per target
dir), nightly drift, and `-Zcodegen-backend` needs nightly anyway.

Setup: rsync copies of ~/jcode and ~/jcode-desktop in scratch, one
CARGO_TARGET_DIR per variant, all on the same nightly so only the backend
differs. Variants: `llvm`; `clif` (RUSTFLAGS=-Zcodegen-backend=cranelift for
every crate, so gpui and the other opt-level 2 crates lose their
optimization); `cliflocal` (wrapper adds the flag only for non-CARGO_HOME
crates at opt-level 0, chained in front of Desktop's parallel-frontend
wrapper). Scripts: `bench/cranelift/`. Command: `cargo test -p
jcode-desktop-ui --lib`.

Wall seconds (medians; machine shared, rustc counts from other agents 0-34):

| step                                  | llvm | clif (all) | cliflocal |
|---------------------------------------|------|------------|-----------|
| cold `--no-run` (740 units)           | 182  | 203        | 195       |
| one-line edit + fps_counter tests     | 7.0  | 7.5        | 7.3       |
| one-line edit, recorded by justrust   | 8.8  | -          | 8.3       |
| full desktop-ui codegen, quiet (x3)   | 20.0 | 16.8       | 16.8      |
| full codegen, recorded, quiet         | 26.6-32.2 | -     | 23.9-24.7 |
| full lib test suite run (1655 tests)  | 16-28 | 45-80     | 13-16     |

Recorded run ids (full codegen = delete desktop-ui incremental dir in the
scratch target and touch lib.rs): llvm 20261007-225541367-2992294 (32.2s,
codegen 17.2s, cpu 113s) and 20261007-225637578-2998150 (26.6s, codegen
14.9s, cpu 103s); cliflocal 20261007-225613624-2995425 (23.9s, codegen
11.8s, cpu 81s) and 20261007-225704287-3001214 (24.7s, codegen 11.7s, cpu
84s). Edit loop: llvm 20261007-225745049-3005459 / -225801786- /
-225819136- (codegen 1.08-1.44s), cliflocal 20261007-225753010-3006812 /
-225810648- / -225827972- (codegen 1.16-1.43s).

Findings:
- Codegen of jcode_desktop_ui drops about 25% (15-17s to 11.8s) and CPU
  about 20% with Cranelift, peak RSS ~400 MB lower. That only matters when
  the whole crate regenerates (upstream jcode crate changes, flag changes).
- The small-edit loop is unchanged (codegen ~1.2s either way, inside noise):
  the loop is dominated by frontend and macro expansion, not codegen.
- An upstream body edit in jcode-sdk did not force full codegen in any
  variant (8-17s, noisy), incremental reuse already covers it.
- Cranelift for everything is a loss: gpui and friends lose opt-level 2, so
  the test suite runs 2-4x slower (45-80s vs 16-28s) and cold builds are not
  faster. Keep deps on LLVM.
- Correctness: all variants pass the 1655 tests in most runs. The same
  handful of timing/layout tests (bundled_github_applet_renders_valid_documents,
  live_tabs_switch_rows_and_stay_detached_from_panels,
  the_hint_chip_uses_only_spare_tab_space,
  precise_horizontal_scroll_moves_rendered_panels_smoothly) fail
  intermittently under load in all three variants including llvm, and pass
  in isolation. Flaky, not miscompiles. Failures cluster under clif(all)
  because its slow gpui makes timing tests slower.

Recommendation: not worth it now. The win (about 4-8s on a full
desktop-ui regeneration, nothing on the common edit loop) requires
switching agents to a nightly toolchain, which costs a ~3 min cold rebuild,
another ~4 GB target per variant, and nightly churn, and it conflicts with
the "recording must never change build results or caches" rule if justrust
toggled it. No justrust hook added. Revisit if Arch or stable ships the
cranelift component, or if full-crate regenerations become frequent.

## 7. Per-agent build slots (2026-10-07)

Problem: cargo holds an exclusive flock on `target/<profile>/` for the whole
build (`.cargo-lock`, `.cargo-build-lock`, `.cargo-artifact-lock`; all three
block). Two agents in one checkout fully serialize. Cargo 1.98's
`build.build-dir` does not help: with a separate build-dir per run but a
shared target-dir, the second build still blocked on "artifact directory"
(22.4s then 44.3s for two justrust builds).

Design (src/slots.rs): agent-mode `check`/`test`/`clippy` get
`--target-dir <target>/justrust-slots/<n>`. `build`/`run` and the plain
`cargo` proxy keep the shared dir (scripts expect binaries there,
`JUSTRUST_SLOTS_ALL=1` opts in). Skipped when CARGO_TARGET_DIR,
CARGO_BUILD_TARGET_DIR, or a target-dir argument is set. `JUSTRUST_SLOTS=N`
caps the pool (default 4), `JUSTRUST_SLOTS=0` disables. A session
(JCODE_SESSION_ID, else Unix sid) sticks to its slot. A busy slot is skipped
via a non-blocking flock. Pool full means the shared dir, never waiting.
`justrust slots [--clean]` lists or removes free slots.

Seeding: parallel reflink copy (6 `cp --reflink=always` workers) of
`.fingerprint` first, then `build`, `deps`, `examples`, and `incremental`
dirs modified in the last 3 days (129 of 1970 dirs, 11 of 181 GB on the
Desktop). mtimes are preserved, so a seeded slot is fully fresh: 740 fresh
units, 0 compiled, 1.6s (run 20261007-222839390-2662980). Disabled when the
filesystem cannot reflink.

First-use seed cost on the Desktop (load avg 20-39): 13.8s, 25.8s, 37.1s
sequential cp (runs -224514294-2896271, -224733055-2917773); 6.4s with the
parallel seeder (run -225143...: slot 3). Raw copy of the
deps/.fingerprint/build trees: 8.8s with one cp, 5.0s with 6. Seeding is paid
once per slot, then sticky reruns are free (0.8s no-op check).

Disk: `btrfs filesystem du` per slot after some use: 0.26, 1.6, 1.7, 8.3 GB
exclusive. About 125 GB is shared extents with target/debug, so 4 slots cost
about 12 GB, not 4x150 GB. Exclusive use grows as the slot rebuilds crates
the shared dir has not.

Concurrency, two sessions, one-line edit in fps_counter.rs, warm slots
(bench script ~/.jcode/scratch/slotbench/conc.sh):

| pair | shared dir (JUSTRUST_SLOTS=0) | slots |
|---|---|---|
| A test + B test (same command) | pair wall 11.5 / 9.9 / 11.6s; B lock wait 11.0 / 9.1 / 11.2s | pair wall 15.4 / 14.7 / 15.0s; lock wait 0.3-0.9s (package cache) |
| A test + B check -p desktop-ui --lib | B wall 17.7 / 13.6 / 12.4s (lock wait 8.7 / 10.8 / 9.0s), pair 18.0 / 13.9 / 12.7s | B wall 3.4 / 3.2 / 3.5s (no lock wait), pair 10.8 / 10.1 / 11.6s |

Run ids: shared mixed -225747943 / -225748251, -225816867 / -225817168,
-225840955 / -225841256; slots mixed -225805987 / -225806288,
-225830853 / -225831154, -225853719 / -225854020; identical pairs
-225606396 .. -225710013.

Reading:
- Different commands (the real multi-agent case): the second agent goes from
  about 14s median to 3.4s, and the pair finishes in 10.8s instead of 13.9s
  (median). Lock wait goes from about 9-11s to 0.
- Identical commands at the same moment: the shared dir wins (10-12s vs
  15s), because B waits and then finds A's work done, while with slots both
  compile the same crate in parallel on a busy 16-core machine. That
  case only arises when two agents run the exact same build at once. Each
  agent still sees its own state, not one that another agent's edit just
  churned.
- The remaining 0.2-0.9s waits are cargo's CARGO_HOME "package cache" lock,
  which slots cannot remove. The lock_wait finding now names the lock and
  only suggests JUSTRUST_SLOTS when it was the build directory.
- Live use: within minutes another agent (hibiscus, ui crate split) was
  given slot 1 automatically. A plain `cargo check` through the proxy at the
  same time still waited 11.4s on the shared dir (run -225913558-3020615),
  which is exactly the cost slots remove for agent runs.

Risks / follow-ups:
- Stale slots: a slot seeded long ago does not get dependency rebuilds that
  later happen in the shared dir (Cargo.lock bump, gpui rev). It rebuilds
  them itself once. Re-seeding a slot when the shared dir has many newer
  fingerprints would be cheap with reflinks and is not implemented yet.
- Disk: exclusive bytes per slot grow with divergent rebuilds. No automatic
  GC. `justrust slots --clean` removes free slots.
- `target/debug/<bin>` from agent test/check runs no longer appear in the
  shared dir. Test binaries live under the slot. Builds and runs are not
  slotted, so the Desktop launcher and Ctrl+R are unaffected.
