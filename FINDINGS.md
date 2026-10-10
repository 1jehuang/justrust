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
parallel seeder (run 20261007-225143060-2962538). Raw copy of the
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

## Splitting jcode-desktop-ui: harness crate pilot (2026-10-07, hibiscus)

Desktop commit 751f1cd moves the GPUI-free harness cluster (harness*.rs,
remote, managed_cloud, managed_cloud_parity, remote_targets, platform: 7.3k
lines, 94 tests) from jcode-desktop-ui (142.5k lines, 1668 tests) into
crates/jcode-desktop-harness. The UI re-exports the modules at their old
paths. Hot reload: the new crate is an rlib linked into the UI cdylib, so the
host/UI ABI is unchanged. Ctrl+R on the running host built and activated UI
generations 3 and 4 (main) and 3 (single-panel).

Module map (crate:: edges between the 70 top-level modules): workspace (48.6k
lines with its #[path] children) and panel (42.8k) are a mutual hub that
depends on almost everything. input (6.8k) has back-edges to panel/workspace.
The clean leaves without gpui are harness+transports (5.5k plus 1.5k, used by
12 modules, only out-edge was build_info::VERSION), diff_model (2.5k, 62 tests,
no deps), learning (1.6k, 34 tests, no deps), diff (1.1k, 32 tests, no deps),
updates (1.8k, deps harness+build_info), global_voice_input (1.2k),
accounts (1.2k, deps harness+platform), remote_targets, pdf_render, todoist,
preview_control, render_stats. Churn over two weeks (593 file-touches in
ui/src): workspace 44, panel 40, input 19, harness cluster 44, voice files 50,
login/account files 59.

One-line edit benchmark (insert `let _probe = N;` in a test fn body, `justrust
test`, restore from backup; median of 3; "others" = cores used by other
processes during the run). The before runs used the shared target dir, the
after runs justrust slot 1 (both warm incremental).

| edit in | command | before compile / wall | after compile / wall |
|---|---|---|---|
| harness.rs | before `-p jcode-desktop-ui --lib -- harness::`, after `-p jcode-desktop-harness --lib -- harness::` | 9.99s / 13.5s (runs 19.7, 10.0, 8.9; others 14, 6.5, 3.2) | 1.25s / 4.2s (runs 1.69, 1.17, 1.25; others 5-7). 2.56s of the wall is the tests themselves |
| fps_counter.rs (UI) | `-p jcode-desktop-ui --lib -- fps_counter` | 14.75s (runs 21.6, 14.8, 9.5; others 14, 11, 5) | 8.02s (runs 9.3, 8.0, 7.7; others 5, 2, 3) |
| harness.rs, then UI test | `-p jcode-desktop-ui --lib -- fps_counter` | same as row 1 | 10.2s compile / 10.9s wall (harness 0.5-1.1s + UI 8.7-9.5s) |

Run ids: before 20261007-223543659, -223606014, -223621987 (ui), -223648196,
-223711646, -223725262 (harness); after -225722087, -225732166, -225740763
(ui), -225751905, -225756538, -225800732 (harness crate), -225815555,
-225826549, -225837526 (harness edit, UI test).

Reading:
- The win is for edits inside the extracted crate tested in that crate:
  about 8x less compile (8.9-10s to 1.25s) and a 1.2 GB smaller rustc. The
  harness test binary is small, so the frontend (0.3s) no longer pays the UI's
  2.4s macro_expand and 1s incremental cache save.
- Edits in the UI are not measurably faster. The UI lost 5% of its lines, so
  the expected gain is ~0.3-0.4s, below the contention noise here (quiet runs
  7.7-9.5s both before and after).
- A harness edit followed by a UI test is not faster, and cannot be: the
  harness rmeta changes, so the UI recompiles (the harness crate adds 0.5-1s).
  Agents get the speedup only when they run the harness crate's own tests,
  which hold all 94 harness tests.
- Expected value: the harness cluster was 44 of 593 two-week file touches
  (7%). Saving ~8s each is small in aggregate. The pattern is what matters.

Recommended next splits (same recipe: move, re-export, test-support feature
for cross-crate test helpers, inject UI-only constants through a setter):
1. Pure-data leaves with no crate:: deps: diff_model + diff (3.6k lines, 94
   tests), learning (1.6k, 34 tests), todoist, preview_control, pdf_render.
   Trivial, but low churn, so mostly a UI frontend shrink.
2. accounts + updates (3k lines, 57 tests) onto the harness crate once
   build_info's VERSION is reachable (already via client_version()).
3. The big lever is panel/workspace (91k lines, 1000+ tests, 84% of churn).
   They depend on each other, so a split needs a trait or a shared state
   crate first. Splitting gpui view code by feature (voice: ~6k lines across
   panel_voice*/workspace_voice/global_voice*; login/accounts UI ~8k) into
   crates that depend on a small `jcode-desktop-ui-core` (theme, render_stats,
   text_selection, scrollbar, image_cache, config) would let the most-edited
   features compile alone. Precondition: break the panel <-> workspace cycle
   (workspace::* used from panel, panel::Panel from workspace).
4. Cheapest per-edit win without any split: run the narrow test crate. A
   `justrust test -p jcode-desktop-ui -- fps_counter` still builds a 1574-test
   binary. justrust's waste line already says so.

## 8. Shared dependency cache (depcache, 2026-10-07, mushroom)

Goal: fresh target dirs, scratch checkouts, and build slots stop recompiling
the same registry/git dependencies. Code: `src/depcache.rs`, hooked into the
shim (`src/shim.rs`). On by default for non-local units, `JUSTRUST_DEPCACHE=0`
disables it. Store: `~/.justrust/cache/depcache/` (override with
`JUSTRUST_DEPCACHE_DIR`).

### sccache does not work across target dirs

sccache 0.18 with `RUSTC_WRAPPER=sccache`, `cargo check` of justrust into two
fresh target dirs: **0 hits / 86 misses** (also 0 with `SCCACHE_BASEDIRS`
listing both target dirs). Its hash includes the absolute `-L dependency=`,
`OUT_DIR`, and `CARGO_TARGET_DIR`, which differ per target dir. Rejected.

### Design

- Key, two levels: base key = rustc `-vV` + args with `<target>/<profile>`
  replaced by a placeholder + content hash of every `--extern` file (memoized
  in an xattr by inode/size/mtime) + linker content + native lib dirs inside the
  target + `CARGO_*`/`RUSTC_*`/`OUT_DIR` env. Result key = base + content of
  every dep-info source (including `OUT_DIR` generated files) + every
  `env-dep` value.
- Hit: reflink the outputs into `--out-dir`, rewrite `.d` paths, replay rustc's
  stderr (artifact notifications keep pipelining working), exit 0. Miss: compile
  normally, then store only if rustc exited 0.
- Units with `-C incremental` (local crates), `--emit` paths, `-o`, or
  `--print` are never cached. Any cache error compiles normally.

### Measurements (jcode-desktop, `justrust check -p jcode-desktop-ui`, fresh CARGO_TARGET_DIR)

| case | run id | wall | depcache |
|---|---|---|---|
| cache off | 20261007-231843110-3108683 | 181.7s | off |
| cold, empty cache | 20261007-232144859-3139425 | 128.3s | 0 hit / 833 miss |
| second fresh target dir | 20261007-232353322-3168951 | **35.8s** | 833 hit / 0 miss |
| scratch checkout (pincheck), cache on | 20261007-232541361-3194212 | **29.2s** | 795 hit / 30 miss |
| same checkout, cache off | 20261007-232610551-3208355 | 118.1s | off |

- The machine was busy throughout (load 7 to 28, up to 38 other rustc during
  the cold runs), so the off/cold numbers are noisy; the off vs cold1 gap is
  contention, not the cache. The hit runs are dominated by the 49-51 local
  crates (jcode_base 9.0s, desktop-ui 3.9s, jcode_protocol 4.3s): 833 hits took
  2.4s of rustc wall in total, max 0.16s (aws_sdk_bedrock), versus ~1129s of
  original rustc time.
- justrust itself: deps-only cold check 5.1s -> 1.4s (52/52 hits); `build`
  10.5s -> 4.3s, and the resulting binary and `justrust test` work.
- Store and miss overhead: hashing and storing added nothing visible to the
  cold run (it was faster than the cache-off run under lower contention).
- Disk: 1.4 GB after the Desktop dependency set (check mode), 2.9 GB after the
  pincheck checkout too (different features for 30 units plus its own path
  crates are not cached; the growth is mostly rmeta for differently-featured
  units). Reflinks on btrfs share extents with the target dirs. No eviction
  yet.
- Correctness: after a hit build, `JUSTRUST_DEPCACHE=0 cargo check` on the same
  target dir is fresh (740 fresh, only the desktop-ui unit rebuilt for an
  unrelated `.git/index` change), so restored artifacts do not dirty cargo.
  `.d` files are byte-identical after path substitution. Restored rmeta are
  identical to a fresh compile except for embedded absolute target paths
  (240/762 rmeta mention `OUT_DIR` or target paths; the same 240 differ between
  any two uncached target dirs, so this is not cache-induced).

### Risks / follow-ups

- Restored binaries embed the target-dir path of the build that produced them
  (debuginfo, `include!(concat!(env!("OUT_DIR"), ...))` file names). Harmless
  for check/test; panics and backtraces in deps may name another target dir.
- No size cap or eviction yet (`rm -rf ~/.justrust/cache/depcache` is safe).
- Build-script *execution* is not cached, only its compilation; cargo still
  runs every build script in a fresh target dir (the 8.9s of gaps in the hit
  run). Addressed in 8c.

### 8a. Size cap and LRU eviction (2026-10-07, dep cache worker)

Code: `src/depcache_gc.rs`, `justrust cache [--prune|--clear]`.

- LRU clock: a hit or a repeated store touches `o/<key>/meta.json`. After each
  recorded run, at most once per 10 minutes (one `stat` of `gc.stamp`
  otherwise), a detached, `nice 19` `justrust cache --auto` evicts the least
  recently used entries until the cache is under 90% of the cap. Default cap
  30 GB, `JUSTRUST_DEPCACHE_MAX=50G|800M|0` (0 = no cap). Evictions are logged
  to `gc.jsonl`.
- Concurrency: one collector at a time (`gc.lock` flock). Entries are renamed
  into `trash/` before deletion, entries used in the last 10 minutes are never
  evicted, and an entry touched between scan and rename is put back. Manifest
  lines for evicted outputs are dropped. A restore that loses a race fails open
  (compiles normally, overwriting partial outputs).
- Stress test, worst case: a scratch copy of the cache with a loop moving 3
  random entries to trash and deleting them every 0.1s (900 entries evicted)
  during a fresh-dir Desktop check (run 20261007-234551593-3434267): exit 0,
  774 hit / 51 miss, and a following `JUSTRUST_DEPCACHE=0 cargo check -v` on
  that target dir reported 732 Fresh, only the desktop-ui unit Dirty (same as
  without eviction). Same with `justrust cache --prune` at a 1.2 GB cap in a
  0.4s loop (1569 evictions, grace 0): all units compiled, cargo fresh after.
- Cost: `justrust cache` on the real 4.1 GB / 1643-entry cache: 0.2s.
  `--prune` of a reflinked copy down to 2 GB evicted 1319 entries in 1.0s.
- Hit runs (`bench/depcache-fresh.sh 3`, fresh `CARGO_TARGET_DIR`, warm cache,
  825/825 hits every run):

| | runs | walls | median | load | hit restore sum |
|---|---|---|---|---|---|
| before | 20261007-233243709-3309755, -233310202-3317914, -233338842-3327229 | 26.1, 28.3, 23.9s | 26.1s | 12.5-14.8 | 1.4-2.6s |
| after | 20261007-234647588-3448800, -234726717-3459509, -234820383-3471186 | 38.8, 52.8, 31.7s | 38.8s | 21-30 | 2.1-10.0s |

  The "after" runs ran at twice the machine load (4-6 other rustc), so the wall
  difference is contention, not the change: the only per-hit addition is one
  `utimensat`, and the median per-hit restore stayed at 1.1-1.7 ms. No speedup
  is claimed for this step; it bounds disk use.

### 8c. Cached build-script runs (2026-10-08, pig + depcache follow-up)

Code: `src/buildscript.rs`. Follows c3c9903 (never store outputs that embed a
foreign target path). This step caches build-script *runs* for registry/git
packages. When the shim compiles or restores a build script, it moves the
binary to `.jr-real-<name>` and puts a `/bin/sh` wrapper in its place. The
wrapper runs `justrust __build-script`, or the real script directly when that
justrust binary is gone. Cached runs replay stdout/stderr and restore
`OUT_DIR`. They are only stored for scripts with `rerun-if-*` directives, exit
0, inputs outside the target dir, and no binary `OUT_DIR` file naming the
target dir. Off with `JUSTRUST_BUILD_SCRIPT_CACHE=0`.

Review fixes before shipping: (1) the base key now includes a system stamp
(mtime of `/etc/ld.so.cache` and the package database: pacman, dpkg, rpm,
nix). A system upgrade misses, so pkg-config/cc probing scripts that only
declare env vars re-probe in a fresh dir. (2) `rerun-if-changed` symlinks
are signed by target content, not the link text. (3) A script killed by a
signal now kills the wrapper with the same signal (it used to exit 128+n).
The tests cover fail-open (wrapper with justrust missing or with a disabled
justrust: same stdout, stderr, args, cwd, exit code), non-cacheable runs (missing
input, input inside target, non-UTF-8 stdout, symlink in OUT_DIR), directory
inputs, and the cross-target-dir round trip.

Correctness on Desktop (`justrust check -p jcode-desktop-ui`, fresh dirs):
- cold run 20261008-205843070-3935519: 74 scripts run, gaps 6.58s. Second
  dir 20261008-205907582-3955347: 45 hit / 29 miss (the misses have no
  `rerun-if` or are local), gaps 0.86s.
- Fingerprints: on the cached dir, a second `justrust check` was 742 fresh /
  0 compiled, and plain `/usr/bin/cargo check -v` (no shim, no depcache) was
  742 Fresh, 0 Dirty. Swapping the binary does not dirty cargo, because cargo
  fingerprints the build-script *source* and its `output`, not the binary.
- OUT_DIR diff against an uncached dir (`JUSTRUST_BUILD_SCRIPT_CACHE=0`), with
  target paths normalized: 76 out dirs, identical file lists. The one content
  diff is local `jcode-desktop-ui/out/changelog.md` (embeds a build
  timestamp, never cached). The `output` files of aws-lc-rs, aws-lc-sys, and
  onig_sys differ only in line order (parallel cc output), and libm
  in `CFG_CARGO_FEATURES` order (`["arch","default"]` vs reversed: HashMap
  iteration order in the script's own run, not the replay: the cached
  copy is a verbatim earlier run of the same script).
- `justrust test -p jcode-desktop-model` in the cached dir: 140 passed.
- `cargo clean -p aws-lc-sys` removes the whole `build/<pkg>-<hash>` dir
  (wrapper and `.jr-real-`), and the next check rebuilds and re-wraps it.
  Slots copy whole `build/<pkg>-<hash>` dirs (`cp -a`, hidden files
  included), so wrapper and real binary always travel together. A wrapper
  copied into another dir finds its real binary through `dirname "$0"`.
  Eviction of a script entry only causes a miss (the real script runs).

Hit runs, `bench/depcache-fresh.sh 1` alternated on/off, 833/833 rustc
depcache hits each:

| | runs | walls | median | gaps |
|---|---|---|---|---|
| script cache on | 20261008-210127927-4050724, -210205780-4063343, -210246936-4077015 | 14.8, 16.6, 16.2s | **16.2s** | 0.75-0.83s |
| `JUSTRUST_BUILD_SCRIPT_CACHE=0` | 20261008-210143096-4056390, -210222703-4068925, -210303453-4082648 | 22.4, 23.9, 27.4s | 23.9s | 5.78-6.92s |

Load rose from 5.7 to 10.2 during the series (0 to 2 other rustc), so each
"off" run had equal or higher load than the "on" run before it. The gap
seconds are the robust number: 5-6s of build-script time removed from every
fresh-dir build (aws-lc-sys alone 10.8s CPU, onig_sys 2.6s). The wall
median went from 23.9s to 16.2s (-32%).

Remaining risk, accepted: a registry script that declares `rerun-if` inputs
but also reads undeclared state (a system file not covered by the package
stamp) replays stale output in a fresh dir. Cargo makes the same assumption
in an existing target dir.

## 9. gpui `test-api`: one GPUI build for Desktop build and test (2026-10-07, rose)

Follow-up to section 6. Fixed at the source in the gpui fork, commit
`1cfbc5d` on github.com/1jehuang/gpui main: a new API-only feature `test-api`
(TestAppContext, `#[gpui::test]`, TestPlatform, run_until_parked,
debug_bounds, layout stats, FakeHttpClient, proptest re-export).
`test-support` is now `test-api` + `leak-detection` + wayland/x11, so
existing users are unchanged. gpui_apple, gpui_linux, gpui_macos,
gpui_platform and gpui_windows forward `test-api`.

cfg audit (131 `feature = "test-support"` sites in gpui, all moved to
`test-api`): every one is either an API item that a real app never calls or a
branch only reachable with a test dispatcher (`dispatcher.as_test()` returns
None for real dispatchers, `GpuiMode::Test` is only set by test contexts).
Executor branches: with a real dispatcher, `BackgroundExecutor::new` and
`ForegroundExecutor::new` take the same PlatformScheduler path as before,
including the profiler's foreground runnable counter. The one real per-frame
cost, `debug_selector` recording into `debug_bounds`, now only records once a
test app exists in the process (`fast::test_api`, an AtomicBool set by
`GpuiMode::test()`). With `test-support` it always records, as before. Leak
detection stays only in `leak-detection`/`test-support`. Platform crates'
`test-support` sites (macOS `set_framebuffer_only(false)`, render_to_image)
stay on `test-support` and are not reached by `test-api`. gpui's own 388 lib
tests pass with `test-support`; `cargo check -p gpui` passes with no
features, `test-api`, `test-support`, `bench-support`, `test-api,profiler`.
`script/check-upstream`: only the 2 violations that predate this change.

Desktop change (gpui rev 1cfbc5d with `features = ["profiler", "test-api"]`
in `[workspace.dependencies]`, the `gpui` dev-deps drop `test-support`,
`hdrhistogram` dev-dep `default-features = false`, `GPUI_REVISION` bumped).
Unit graph (`__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS=nightly cargo
build|test --unit-graph -Z unstable-options`, compared by package, features,
profile minus name, and transitive deps): build 960 units, test 960, 958
shared. gpui, gpui_linux, gpui_platform, gpui_wgpu, gpui_macros,
gpui_thinking_orbs all identical. The only differences are jcode_desktop_ui
(lib vs test) and jcode_desktop_harness (its own `test-support`, a small local
crate, separate issue). Release unit graph: gpui has `test-api` and no
`leak-detection`, i.e. no runtime change per the audit above. All Desktop
workspace tests passed (1731/1732 on the first run; the one failure,
`the_hint_chip_uses_only_spare_tab_space`, passed alone and in two full
reruns of the 1404 UI tests, so it is flaky under load, not caused by this).

Benchmark (scratch copies under ~/.jcode/scratch/gpuitestapi, separate target
dirs, `JUSTRUST_DEPCACHE=0 JUSTRUST_SLOTS=0`: depcache otherwise restores gpui
and hides the cost; delete `gpui-*` fingerprints, then `justrust build -p
jcode-desktop -p jcode-desktop-ui --lib` and `justrust test -p jcode-desktop-ui
--lib -- fps_counter`):

| run | before build | before test | before total | after build | after test | after total |
|---|---|---|---|---|---|---|
| 1 | 45.3 | 50.4 | 95.7 | 58.1 | 9.8 | 67.9 |
| 2 | 82.1 | 99.4 | 181.5 | 59.2 | 8.9 | 68.1 |
| 3 | 59.3 | 85.3 | 144.5 | 68.8 | 10.2 | 79.0 |

Medians: before 144.5s (test 85.3s), after 68.1s (test 9.8s), about 76s
(53%) saved per gpui-invalidating build+test cycle. The test step drops from
~85s to ~10s because it compiles 1 unit instead of 8. Other rustc on the
machine at start: 0-2, but the build column varies 45-82s for identical work,
so the totals are noisy; the test-step drop is far outside the noise.

Status: landed in Desktop as d206de0 (2026-10-08). The running hosts were
restarted onto gpui 1cfbc5d with Desktop's built-in `--restart-all-worker`
(8 windows restored), and a Ctrl+R build-reload on the new host activated a
fresh UI generation. Build and test now share every gpui unit (unit graph:
961/961, 958 shared, the 3 differences are only the local crates' own test
vs lib units). Workspace tests: 1731 of 1732 pass;
`bundled_github_applet_renders_valid_documents` failed once under load and
passes alone (also flaky under LLVM per the Cranelift section).

Caveat: `test-api` pulls the optional `proptest` dependency into normal and
release builds. It is compiled but not used at runtime.

## 10. Build slots follow-ups: refresh, GC, joining identical builds (2026-10-07, cow)

### Refresh stale slots (commit 6725be0)

Before a slot run, units the shared `target/<profile>` built since the
slot's last refresh are reflink-copied in. A unit is the granule: its
`.fingerprint/<name>-<hash>` dir, its `build/<name>-<hash>` dir, and every
`deps/` file carrying the hash (checked on the Desktop: all 28k deps files
and all build dirs map to a fingerprint hash). Artifacts are copied first,
fingerprints last, with mtimes preserved, so cargo's fingerprints decide
freshness exactly as for a seeded slot. Copied: units missing in the slot,
and units whose newest fingerprint file is newer in shared AND whose hash
files differ (identical state is skipped, copying it would only bump
mtimes). A unit the slot built more recently is never touched. Runs only
while a non-blocking shared flock on the shared dir's `.cargo-build-lock`
succeeds (cargo holds it exclusively while building; verified that a
running cargo blocks it and a reader holding it shared blocks cargo), and
only when the shared dir mtimes moved since the last refresh. Scan cost on
the Desktop: 0.15s for 9.4k fingerprint dirs (36k files).

| case (Desktop, `justrust check -p jcode-desktop-ui --lib`) | time |
|---|---|
| slot missing all 210 gpui fingerprints, no refresh, depcache off (run 20261007-233515707-3342162) | 23.2s, gpui rebuilt 12.8s |
| same, no refresh, depcache on (run -233601632-3346804) | 23.3s, depcache 0 hit / 6 miss |
| same, with refresh (run -233544850-3345335) | 1.0s: 236 new + 1 rebuilt units (540 files) copied in 0.8s, 0 compiled |
| real stale slot 2 (34 missing, 2 rebuilt units, run -233326676-3325406) | refresh 0.2s, then only the 4 local crates compiled |

Scratch project (serde/regex, then rayon+syn added to Cargo.toml, a
Cargo.lock-bump analog): stale slot 6.6s cold / 0.4s with depcache hits
vs 0.0s after a 26-unit refresh in 0.1s. depcache and refresh are
complementary: depcache needs the exact rustc invocation to have been
cached; refresh also skips cargo and build-script work.

### Automatic GC (commit 6bb60d4)

After a slotted run, a detached `justrust slots --gc-root` runs at most
hourly per workspace. Removes free (not flocked) slots idle over 7 days,
then least recently used free slots until btrfs exclusive bytes fit 40 GB
(`du -sb` off btrfs). Slots used in the last hour are never reaped for
budget. `justrust slots` shows exclusive/total size and age.

`btrfs filesystem du` costs 3.8s per 170 GB Desktop slot and does not
parallelize (13.4s for 4 at load 33), so the listing uses sizes cached by
the last GC (34 ms vs 13.4s); `--du` re-measures. Current Desktop: 4 slots,
0.0 / 1.8 / 7.3 / 11.9 GB exclusive (21-31 GB total), well under budget.
The hook adds nothing measurable to a run (93 ms no-op check with GC spawned).

### Joining identical concurrent builds (commit 84dda77)

Each slot records its current request key (cwd, cargo args before `--`,
compile-relevant env) in `<n>.req`. A new run whose key matches a slot that
is flocked right now waits for that flock (max 300s) and then runs cargo in
that slot, so cargo still checks freshness and the joiner sees its own
test output. The wait is shown as a `slot_join` finding.

Desktop identical pair, one-line edit, `justrust test -p jcode-desktop-ui
--lib -- fps_counter`, B starts 0.3s after A (bench
~/.jcode/scratch/slotbench/join.sh), quiet machine (0 other rustc):

| | pair wall | A+B CPU |
|---|---|---|
| parallel slots (JUSTRUST_JOIN=0) | 9.9 / 9.4s | 20.4 / 19.3s |
| joined | 9.3 / 9.0 / 9.0s | 9.8 / 9.7 / 9.7s |

Runs: parallel -235008527/-235008828, -235027474/-235027776; joined
-234959162/-234959463, -235018471/-235018775, -235036897/-235037198.
B finishes about 0.4s after A instead of compiling in parallel. On a quiet
16-core machine the wall gain is small (0.4s) because one crate's codegen
does not saturate the cores; the CPU halves, which matters on this shared,
usually loaded machine (section 7 measured 15s vs 10-12s at load 20+).
Conservative: only a held slot with the exact key, and the joiner still runs
cargo, so a file edited in between is still rebuilt correctly.

### 8b. Embedded target paths in restored artifacts (2026-10-08, dep cache worker)

Question: can a restored artifact behave differently from a fresh compile
because it embeds the first producer's target dir?

Evidence, scanning every cached output (2546 files, 1643 entries):
- 41 of 2546 (15 rlib, 26 rmeta, 0 `.so`, 0 build-script executables) contain
  a target-dir path at all. All 82 occurrences are the full path of an
  `OUT_DIR` source file that the unit `include!`s (serde/serde_core
  `private.rs`, thiserror, ref-cast, pulp `x86_64_asm.rs`, crunchy `lib.rs`,
  mime_guess, aws-types `build_env.rs`, unicode-general-category, libsqlite3-sys
  `bindgen.rs`). 0 occurrences are anything else (checked against each entry's
  dep-info inputs).
- Those file paths come from `SourceFile` names in rmeta and from panic
  locations / line tables (`file!()`, `Location::caller`). Their content is in
  the result key (dep-info inputs are content-hashed), so the restored code is
  the same code. Only the *name* differs: a panic message or backtrace frame
  inside, say, serde's generated `private.rs` names the other target dir. It
  never reads that file at runtime. Desktop uses `debug = 0` for non-local
  packages, but a test with `-C debuginfo=0/1/2` shows the same set (full file
  paths only, never a bare dir).
- What *would* change behavior: a dependency keeping `env!("OUT_DIR")` (or
  another target path) as a runtime value, e.g. opening a generated file at
  runtime. Then a restored artifact would read another build's directory,
  which may have been deleted. None of the cached Desktop or justrust deps do
  this, but nothing prevented it.
- Tracked env with target paths: `OUT_DIR` (30 entries) and
  `MIME_TYPES_GENERATED_PATH` (3). Both are normalized in the key, and the
  above scan covers what they are compiled into.
- `--remap-path-prefix` was considered and rejected: it changes the bytes of
  every output and would have to be applied identically to uncached builds
  (or every unit differs between cached and uncached builds), and it does not
  remap `env!` values (verified: `env!("OUT_DIR")` stays absolute while
  `file!()` is remapped), so it would not fix the dangerous case anyway.

Fix (`target_refs_are_inputs` in `src/depcache.rs`): at store time, every
occurrence of the target dir in a non-dep-info output must be immediately one
of the unit's own input file paths (exact path, followed by a non-path byte).
Otherwise the unit is not stored and always compiles normally.

- Cold store into an empty cache with the guard (run
  20261008-000652409-3638276, 159.9s at load ~30): 833 misses, 833 stored, 0
  rejected, so the hit rate is unchanged on Desktop.
- Guard cost: scanning all 927 MB of Desktop outputs takes 0.52s CPU in total
  (release microbench), spread over the cold misses. Hits do no extra work.
- Hit runs with the guarded cache (`JUSTRUST_DEPCACHE_DIR=<fresh cache>
  bench/depcache-fresh.sh 3`): 44.3, 41.8, 30.8s (median 41.8s), 833/833
  hits each, hit restore sum 1.9-2.6s, median 1.1-1.4 ms per hit (same as
  8a). Runs 20261008-000941671-3672951, -001026422-3695394,
  -001108803-3705552. Load average was 27-31 (vs 12-15 for the 26.1s
  baseline), so wall times are contention-bound; the guard cannot affect hit
  runs.

## 11. Splitting jcode-desktop-ui, part 2: leaf crates and the view cycle (2026-10-08, cricket)

Follows the harness pilot above. Desktop commits, each verified separately
(`justrust test --workspace` 1746 passed / 13 ignored every time, `justrust
check --workspace --all-targets`, offline screenshot, Ctrl+R on the running
host activated a new UI generation, release-desktop.py MANIFESTS updated):

- 51adb8d: new `crates/jcode-desktop-model` (diff_model + its tool-result
  parser, diff, learning, todoist, pdf_render: 6.1k lines, 160 tests, no
  GPUI). `accounts` (1.2k) joins jcode-desktop-harness. `updates` stays in
  the UI: its Linux path uses jcode-desktop-api's LaunchMode, and the api
  crate depends on gpui, so moving it would put GPUI in the harness crate.
- 12f4118: breaks the panel <-> workspace cycle (below). No crate change.
- c3fd05c: AGENTS.md tells agents to test the smallest crate containing the
  change and records the "panel/input must not use workspace" rule.
- 3d6e312: new `crates/jcode-desktop-ui-core` (theme + palettes, config,
  render_stats, text_selection, prompt_background, scrollbar, image_cache,
  animation_clock, transition, pulse_text: 6.1k lines, 70 tests). Test-mode
  `cfg(test)` behavior (config persistence stubs, user-config bypass, fixed
  animation epoch, theme lock, render counters) moved behind a
  `test-support` feature the UI enables only from dev-dependencies, the same
  pattern as the harness crate. GPUI via `gpui.workspace = true`, so it
  shares the test-api GPUI from section 9.

jcode-desktop-ui is now 121.9k lines (from 142.5k before the pilot).

### One-line edit benchmark (median of 3, `let _probe = N;` in a test fn)

| edit in | before: `-p jcode-desktop-ui --lib -- <mod>` | after: `-p <new crate> --lib -- <mod>` |
|---|---|---|
| diff_model.rs | 8.60s compile / 9.2s wall (8.11, 8.60, 8.70; others ~2 cores) | 0.65s / 1.1s (0.57, 0.65, 0.74; others 5-10 cores) |
| theme.rs | 8.01s / 8.5s (9.49, 8.01, 7.83; others ~2 cores) | 1.49s / 2.1s (1.10, 1.52, 1.49; others 6-12 cores, 2-15 other rustc) |
| accounts.rs | (UI, same ~8s class) | 1.02s / 1.4s (1.02, 1.00, 1.03; quiet, others ~2 cores) |
| fps_counter.rs (stays in UI) | 8.0s (hibiscus, quiet) | 8.36s / 8.9s (8.25, 8.61, 8.36; others 2-5 cores) |

Run ids: diff before 20261007-230932200, -230940912, -230950204; diff after
-232606302, -232607336, -232608528; theme before -235629590, -235639641,
-235648217; theme after 20261008-001122633, -001124328, -001126488; accounts
-235232963, -235234407, -235235782; fps_counter -235206178, -235214927,
-235223983. A first run under heavy load (others 11-15 cores, 20-28 other
rustc, load average ~25) measured 5.4-6.8s for accounts and 13-54s for
fps_counter; those runs are discarded as contention.

Reading: edits in an extracted crate, tested in that crate, are 6-13x
faster to compile (0.6-1.5s vs ~8s). UI edits are unchanged within noise
(8.0 -> 8.4s) even though the UI lost another 14% of its lines. The UI
crate's per-edit cost is dominated by frontend + codegen of the crate as a
whole, not by the extracted leaves. ui-core costs more than model (1.5 vs
0.65s) because it is GPUI code, with gpui generics to monomorphize.

### The panel <-> workspace cycle, concretely

Module-graph SCC over the 70 top-level UI modules (each module with its
`#[path]` children; edges are `crate::<module>` references outside
`#[cfg(test)]` items). Before 12f4118, one SCC held 86.1k lines: workspace
(40.6k), panel (36.5k), input (6.4k), applet_runtime, applet_view,
applet_surface. Inside it the forward edges are large (workspace uses
panel 34x and input 10x; panel uses input and the applet modules). The
back-edges were few and small (~20 call sites):

| back-edge | items | fix |
|---|---|---|
| panel -> workspace | actions OpenResume, OpenAppletShowcase, ToggleOnboardingSimulator, OpenChangelog, ClosePanel, PublishDesktop, OpenAccounts, change_review::{Open,Close}ChangeReview | moved to new leaf `ui_actions.rs`, names unchanged (`actions!(workspace, ...)` keeps the namespace), re-exported from workspace |
| panel, input -> workspace | `resume::is_resume_command` | moved to `commands` |
| panel -> workspace | `restart::spawn_worker` | new leaf `restart_spawn.rs` (spawner, detach, flags) |
| panel, input -> workspace (test) | `panel_cache_tests::record_render` | `render_stats::test_renders` |
| input -> panel | `shortcuts::background_tool_bindings` + its 3 actions | `ui_actions` |
| input -> panel | `pretty_provider_name` (pure string fn) | `accounts` (harness crate) |
| applet_runtime -> workspace | `applets::SHOWCASE_ID` | const moved into applet_runtime |

After: no SCC contains workspace, panel or input. The only remaining
cycles are theme <-> config (now inside ui-core) and markdown <-> diff_view
<-> html_preview <-> diff_block (5.6k, rendering, one call site each way).
Test code still uses `Workspace::for_test` from panel tests (69 sites);
that is fine within one crate but must become a test-support helper when
panel moves out.

### Plan for the feature-crate split (not done: needs its own session)

The big lever is still moving the most-edited view code out of the UI
crate. Churn over two weeks: voice files 50 and login/account/usage files 67
of 631 .rs touches in ui/src. Sizes (non-test): voice 7.8k lines in 11
files, login/accounts/usage 9.1k in 11 files. The blocker is no longer the
module cycle but Rust's crate rules:

1. Almost all of that code is `impl Panel { ... }` or `impl Workspace
   { ... }` blocks in `#[path]` child files (panel_voice*.rs, panel_login*.rs,
   panel_usage.rs; workspace_voice.rs, workspace_global_voice*.rs,
   workspace_account_sign_in.rs, sidebar_account*.rs). Inherent impls cannot
   live in another crate, and they read private fields freely (panel_voice.rs
   touches 41 distinct `self.` fields, workspace_account_sign_in.rs 47).
2. So a feature crate must own its state and views: e.g. `VoiceState` (the
   Panel's voice fields) with methods taking `&mut VoiceState` plus a small
   trait the host implements for what it needs from the panel (bridge send,
   session id, notify). Panel keeps a `voice: VoiceState` field and forwards.
   Same for `AccountSignIn` on the workspace side.
3. Order that keeps every step shippable:
   a. Leaves first, no Panel/Workspace involvement: global_voice_input,
      global_voice_overlay, global_voice_session, panel_voice_tag
      (1.9k) and login_input, panel_login_catalog, panel_login_status (1.1k)
      move into `jcode-desktop-voice` / `jcode-desktop-accounts-ui` crates
      depending on ui-core and harness. Mechanical, like ui-core.
   b. Extract the voice field group from Panel into `VoiceState` inside the
      UI crate (pure refactor, tests unchanged), then move it and
      panel_voice*.rs into the voice crate behind a `VoiceHost` trait.
   c. Repeat for login (panel_login*.rs) and workspace_account_sign_in.
   d. Before moving panel itself, turn `Workspace::for_test` uses in panel
      tests into a test-support constructor or move those tests to the UI
      crate's integration layer.
4. Expected payoff: per-edit compile for voice/login work drops from ~8s to
   the ~1.5s class (the ui-core number for GPUI code of that size), for
   ~19% of UI churn. UI-only edits stay ~8s until panel/workspace
   themselves shrink.
5. Hot reload: every new crate is an rlib linked into the UI cdylib, so the
   host/UI ABI is unaffected. Globals keyed by TypeId (Sounds, Runtime,
   StatusAccounts, ...) stay in one crate instance per generation, so moving
   them is safe as long as the host never holds one.

Process notes: the shared tree had another agent's uncommitted gpui rev bump
twice during this work. Ctrl+R then fails with "plugin GPUI revision differs
from host" until the host restarts. Commits were staged by writing HEAD
plus only my hunks into the index (`git hash-object` + `update-index`) so
the other agent's in-flight Cargo.toml/Cargo.lock edits stayed out. The
UI test suite has two pre-existing flaky tests
(`workspace::tests::the_hint_chip_uses_only_spare_tab_space`,
`applet_runtime::tests::bundled_github_applet_renders_valid_documents`),
about 1 failure in 5 full UI runs, reproduced on a clean d206de0 worktree.
The offline screenshot sometimes captures the 3s startup beta toast
(same on d206de0 and after, 1 of 2 runs each); that is screenshot timing,
not a layout change.

## 12. Splitting jcode-desktop-ui, part 3: voice and login leaves (2026-10-08, cricket)

Step 3a of the plan in section 11, plus the first slice of 3b. Desktop
commits, each verified with `justrust test --workspace` (1746, then 1747
passed / 13 ignored), `justrust check --workspace --all-targets`, an offline
screenshot, and a Ctrl+R reload that activated a new UI generation
(release-desktop.py MANIFESTS and AGENTS.md's narrow-crate list updated):

- 1430ed1: new `crates/jcode-desktop-voice` (global_voice_input,
  global_voice_overlay, global_voice_session, panel_voice_tag -> `tag`:
  1.9k lines, 27 tests) and `crates/jcode-desktop-accounts-ui`
  (login_input, panel_login_catalog -> `catalog`, panel_login_status ->
  `connection`: 1.1k lines, 10 tests). All seven were real leaves: their
  only crate deps were `theme`/`render_stats` (ui-core), `harness`/`platform`
  (harness crate) and catalog -> connection (moved together). The UI
  re-exports them at their old paths (`crate::global_voice_*`,
  `crate::login_input`, `panel::voice::tag`, `panel::login::{catalog,
  connection}`), so no caller changed. Neither crate needs a
  `test-support` feature: the moved code has no `cfg(test)` behavior beyond
  its own tests. gpui via `gpui.workspace = true`, libc/zbus/async-io only on
  Linux, with the UI's libc features.
- 5bf7aff (3b, first slice): Panel's voice fields were already grouped
  in `voice: VoiceState` (panel_voice.rs), so the "extract VoiceState"
  half of 3b had already been done. What remains is moving the logic behind a
  `VoiceHost` trait. This commit moves the parts with no panel state into
  `jcode-desktop-voice::chrome`: shortcut keycap and tooltip, pill status
  labels (`short_voice_status`, `global_pill_*`), and the idle
  microphone/shortcut crossfade view `VoiceSwap` with its 2 tests.
  panel_voice.rs: 2085 -> 1840 lines.

### One-line edit benchmark (median of 3 after one warmup, `let _probe = N;` in a test fn)

| edit in | compile (median) | wall | runs (compile s) |
|---|---|---|---|
| UI `panel_voice.rs`, `-p jcode-desktop-ui --lib -- panel::voice` (old home of tag.rs) | 9.41s | 10.2s | 8.66, 21.08 (8 other rustc), 9.41 |
| voice `tag.rs`, `-p jcode-desktop-voice --lib -- tag` | 1.39s | 2.0s | 1.63, 1.39, 1.19 |
| accounts-ui `catalog.rs`, `-p jcode-desktop-accounts-ui --lib -- catalog` | 0.97s | 1.5s | 0.97, 1.02, 0.84 |
| voice `chrome.rs` (GPUI views), `-p jcode-desktop-voice --lib -- chrome` | 3.9s, contended | 5.2-6.8s | 4.89, 3.19, 4.02 (others 15.3 of 16 cores, 56-64 other rustc; own CPU 2.3-2.4s) |

Run ids (all 20261008-): UI 225901370-402380, 225910853-403300,
225932795-406079; tag 225654565-372432, 225656959-372833,
225659035-373219; catalog 225804731-391461, 225806260-393194,
225807855-394959; chrome 231001940-579130, 231009186-581497,
231014935-583831 (earlier contended set 230724158, 230731419, 230737731:
3.91, 3.24, 3.20).

Reading: edits in the new crates are 7-10x faster to compile than the same
code in the UI (about 1.0-1.4s vs 9.4s), the same class as ui-core and
model in section 11. The chrome number is inflated by contention (CPU use
of the build itself was about 2.4s, versus 11-12s of CPU for a quiet UI
edit). justrust startup was also 1.8-2.6s under that load, versus 0.45s
quiet. jcode-desktop-ui is now about 119k lines.

### Left for step 3b and later

- The rest of panel_voice.rs (1.8k), panel_voice_overlay.rs (0.9k) and
  panel_voice_pills.rs (0.7k) are `impl Panel` blocks. They read
  `self.voice.*` plus a few panel members (`input`, `session_id`,
  `status_line`, `submit_or_queue`, `supports_voice`, `terminal`,
  `todoist`, `orchestration`, `preview_state`, `is_side_document`,
  `is_pending_session`). That is a small, explicit surface for a
  `VoiceHost` trait. `Phase`, `VoiceTrace` and `VoiceState` can move as
  soon as their private-field access from Panel tests (`panel.voice.error
  = ...` in panel_voice_overlay.rs tests) goes through methods.
- Login (3c) and `Workspace::for_test` in panel tests (3d) are unchanged.

Process notes: during this work another agent released Desktop 0.5.1
(version bump in every manifest, including the two new ones, which it
picked up on its own) and the main Desktop instance exited. The 5bf7aff
reload was verified on the single-panel host (generation 3), and 1430ed1
on both hosts (main generation 4, single-panel generation 2).

## 13. Machine-wide build priority (src/sched.rs, 2026-10-08, jaguar)

Problem (index.jsonl, 477 runs over 48h): runs over 60s were 12% of calls
but 59% of wait time. The 30-200s tail of one-unit `test -p
jcode-desktop-ui` runs was mostly contention: other agents' builds used
9-15 of 16 cores, up to 58 foreign rustc, and the edit build got 0.5-1 core.

Mechanism: every recorded run moves itself (over the user bus, `busctl
StartTransientUnit`, ~40ms) into `app-justrust.slice/justrust-<id>.scope`
before starting cargo, so everything cargo starts is inside it.

1. v1, cgroup `cpu.weight` = 1000 * 60 / (60 + CPU-s used) (least attained
   service). **No effect.** This machine runs sched_ext `scx_lavd`, which
   ignores cgroup weights, nice and `cpu.max`. Synthetic check: a 4-thread
   probe next to a 48-thread hog got 4.4s of CPU at weight 10 and 4.9s at
   weight 1000; a 200% quota let a 16-thread hog use 67 CPU-s in 6s. Desktop
   probe runs 20261008-231039903 .. -231731304 vs off runs: the same.
   The weights stay in (they work under the normal fair scheduler).
2. v2 (c068015), affinity. lavd honors `sched_setaffinity`: the same probe
   got 13.3s of CPU when the hog was pinned to 8 CPUs, versus 3.6s. So while
   another justrust build is young (< 200 CPU-s) and actively running, a
   build that has used more than 200 CPU-s pins all its threads to the
   slower half of the CPUs (by `cpuinfo_max_freq`: here E and LP-E cores
   8-15, keeping P cores 0-3 and E cores 4-7 free). It gets every CPU back
   within 0.5s once no young build runs. Idle young scopes (a `justrust run`
   server) do not count.

Benchmark `PROBE=desktop bench/sched-contention.sh 4 2 "off on off on off on"`:
two cold `justrust check -p jcode-desktop-ui` loops (fresh target dirs,
depcache and build-script cache off) as other "agents", and as the probe a
one-line test-fn edit in fps_counter.rs + `justrust test -p jcode-desktop-ui
--lib -- fps_counter` in a scratch copy. Same load in each mode, alternated.

| probe wall, 12 runs each | median | mean | max |
|---|---|---|---|
| `JUSTRUST_SCHED=0` | 24.5s | 25.4s | 53.2s |
| sched on (affinity) | **12.9s** | **12.9s** | **16.0s** |

Off: 9.3, 9.6, 11.2, 12.9, 14.1, 22.5, 26.4, 32.8, 33.9, 37.0, 41.5, 53.2.
On: 10.3, 10.8, 11.3, 11.5, 11.9, 12.9, 12.9, 13.3, 13.7, 14.3, 15.4, 16.0.
Run ids 20261008-234357414 .. -235809675. The quiet-machine number is
about 9s, so contention cost drops from about +15s median / +44s worst to
about +4s / +7s.

Cost to the long builds: the loads yielded 47-55s each while probes ran.
No clean wall comparison (the bench kills loads at mode switches), but
loads in "on" rounds reached 867-879 of 887 units in 115-122s, while "off"
loads finished all 887 in 121-147s: no visible slowdown, because the
probe only needs 1-4 cores and the loads keep 8.

Open: the threshold (200 CPU-s) and the half split are guesses that fit
this machine. A per-build jobserver (fewer rustc threads for old builds)
would also cut the 50+ runnable threads that make even the reserved cores
contended under the fair scheduler.

## 14. Jcode TUI repo: parallel front-end was bypassed, giant crates (2026-10-09)

The Jcode TUI repo (`~/jcode`) had `scripts/rustc-parallel-frontend`, but
only `scripts/dev_cargo.sh` exported it as `RUSTC_WRAPPER`. Agents call
`justrust` (and plain `cargo`), which skip dev_cargo.sh, so jcode-base
(139k lines), jcode-app-core (160k) and jcode-tui (221k) compiled with a
single-threaded front-end. Fixed in jcode 04c7d2b04 by setting
`build.rustc-wrapper` in `.cargo/config.toml` (CI and Windows set
`CARGO_BUILD_RUSTC_WRAPPER=""`).

Measured on the XPS (16 threads, others using 2-4 cores):

| Scenario (`check -p jcode-tui`)                  | 1 thread | -Zthreads=8 |
|--------------------------------------------------|---------:|------------:|
| Non-incremental after an upstream edit (x2)      | 44.0 / 45.3s | 24.0 / 23.3s |
| Incremental, new pub fn in jcode-base (x3)       | 9.2-10.1s | 10.5-11.0s |
| `test -p jcode-base --lib --no-run`, same edit (x2) | 13.9 / 15.5s | 13.3 / 13.5s |

So the threaded front-end halves full rebuilds of a big crate (app-core
18 -> 10s, base 11 -> 6s, tui 10 -> 5s) but does nothing for warm
incremental edits, which stay at ~3s per giant crate in the chain. That
floor is crate size, not flags. New finding `giant_crate` (local crate
>= 40k lines in `src/` costing >= 5s of the run) tells the agent to split,
with the crate's line count and seconds; it also flags single-threaded
front-ends. Cost estimate assumes split-out crates of ~20k lines.
First real report: run 20261009-005101282-1501305, jcode_base 138k lines,
41.0s, ~35s attributable.

## 15. `justrust split`: which code to move out, from recorded edits (2026-10-09)

`giant_crate` and `upstream_cascade` said "split", not what. `justrust split`
answers that from this workspace's recorded runs plus a regex module graph:

1. Each run's `Dirty <pkg>: the file X has changed` lines locate the edit
   (module file, crate root, or build-script input) and the run's cost is the
   edited crates plus their dependents' unit time (dev-dependency edges are
   ignored: they only affect the crate's own tests and otherwise create false
   cycles such as jcode-base -> openrouter-runtime -> jcode-base).
2. Every crate's module tree is parsed from `mod` items (with `#[path]`), and
   `crate::`/`super::`/`self::`/child paths resolve to the deepest module they
   name. `pub use` re-exports are not edges. Moving module M forces out every
   module that references it, transitively; above 35% of the crate it is
   reported as entangled with the references to cut.
3. Inherent `impl T { .. }` blocks for a type defined in a module that stays
   are a hard blocker (Rust only allows them in the defining crate), so those
   candidates are marked "needs prep".
4. The saving replays the runs whose edits in that crate all fall inside the
   moved code: the crate costs only its moved share of lines, dependents that
   do not use the moved code (word search, including `crate::` through glob
   re-exports like `pub use jcode_base::*`) drop out.

First run on ~/jcode (96 runs with edits, 2105s in edited crates and
dependents, last 30 days), 0.5s wall. Correction: 19 of those runs (and
more in other sections of the report) were jcode-desktop runs, because the
run filter matched `/home/jeremy/jcode` as a string prefix of
`/home/jeremy/jcode-desktop`. Fixed to a path-component match; with it the
report has 77 runs and 1918s. Numbers below are from the first run unless
marked.

- Biggest costs are not modules at all: jcode-config-types' single 1.7k-line
  lib.rs (39 runs, 566s cascade into 17 dependents), jcode-base's root file
  (127s), jcode-build-support (180s), and `docs/` as a build-script input of
  jcode-app-core (169s). Git hunk headers name the churned items in a root
  file (AgentsConfig 10 edits, WebSearchConfig 7, DiffDisplayMode 6) and which
  dependents use them; for config-types nearly every dependent uses them, so
  moving items does not help, the fix there is fewer dependents per item.
- `jcode-base::external_auth` (1030 lines, nothing else in jcode-base
  references it, checked by hand) was ranked first at ~24s over 2 runs,
  sparing 10 provider/runtime crates. Both of those runs were jcode-desktop
  runs (`check -p jcode-desktop-ui` at 31.8s and 17.7s, Desktop building
  jcode-base as a path dependency), so the ranking came from the prefix bug.
  After the fix no jcode run edited it and it is no longer listed.
- Needs prep: `config::config_file` (~31s), `provider::startup` (~16s), and
  three `tui::app::*` files (13-19s each) are all `impl Config`/`impl
  MultiProvider`/`impl App` blocks split across files, so moving them first
  needs free functions or an extension trait.
- Entangled: `session::persistence`, `provider::openrouter`, `auth::cursor`
  drag along 83% of jcode-base; the report lists the referencing modules.

Takeaway: in this repo the cascade cost is dominated by small, widely used
root files (config-types, build-support) and a build-script input, not by
modules inside the giant crates. Splitting the giant crates still matters for
their own incremental floor (section 14).

Caveat: the savings are a model, not a measurement. The skipped dependents
follow from cargo's rules, but the new crate's cost is assumed to equal its
share of the old crate's lines, and users are found by word search.

Validated on `jcode-base::external_auth` in a scratch worktree at jcode
04c7d2b04 (moved to `crates/jcode-external-auth`, `crate::` -> `jcode_base::`,
`pub(crate)` -> `pub`, app-core re-exports it under the old path, workspace
checks, the 10 moved tests pass). Same body-only edit, `check --workspace`,
three runs each:

| | wall | units rebuilt | jcode_base | provider/runtime crates |
|---|---|---|---|---|
| before (runs ...011923788, ...011935867, ...011948463) | 11.6 / 12.5 / 11.5s | 17 | 3.0-3.5s | 10 rebuilt |
| after  (runs ...012002880, ...012011969, ...012019539) | 8.8 / 7.1 / 7.4s | 7 | not rebuilt | none rebuilt |

- The predicted set of spared crates was exact. Cargo's rebuild reasons
  before: jcode-base, the 10 provider/runtime crates (anthropic, antigravity,
  copilot, cursor, gemini, grok-build, openai, openrouter, doctor) plus
  tui-permissions, then app-core, tui, jcode. After: only jcode-external-auth,
  app-core, tui, jcode, the crates the report listed as users. The new crate
  costs 0.1s.
- Saving: ~4.1s per edit (35%), measured. The report's ~24s over 2 runs
  (~12s each) is not comparable: those were Desktop runs (see the
  correction above), so the per-edit figure was replayed from a different
  workspace's unit times. The estimate replays old unit times in any case,
  so it overstates savings after the toolchain gets faster; the structure
  (what stops rebuilding) is what to trust.
- One manual step the report does not mention: the moved tests used
  `jcode_base::storage::lock_test_env`, gated on jcode-base's `test-support`
  feature, so the new crate needs it as a dev-dependency feature.

### 15b. Second real split (Desktop) and a calibrated saving model

Split `jcode-desktop-ui-core::scrollbar` (481 lines) into
`jcode-desktop-scrollbar` in a scratch worktree of jcode-desktop 400400d
(sibling `jcode` symlink for the `../../../jcode` path deps). `check
--workspace --all-targets` ok (one pre-existing `pulse_text` warning), the 4
moved gpui tests pass. Same body-only edit, `check -p jcode-desktop-ui`, 3x:

| | wall | rebuilt (cargo) |
|---|---|---|
| before (...144526866, ...144530399, ...144533461) | 3.0 / 2.9 / 3.2s | ui-core, voice, accounts-ui, ui |
| after  (...144548605, ...144551673, ...144554725) | 2.6 / 2.9 / 2.7s | scrollbar, ui |

Spared set predicted exactly again (ui-core, voice, accounts-ui). Measured
saving ~0.3s/edit, i.e. not worth doing; the old report said ~3.5s/edit.

Why the old estimate was 3-10x high: it replayed each historical run's own
unit times. 2 of the 5 scrollbar runs were on a machine with 14 of 16 cores
taken by other builds (60+ foreign rustc), where ui-core took 2.8-3.2s
instead of ~0.5s, and test units (`ui_core (test)`) were counted as if a
split would spare them.

Model now: per package, the median over quiet `check`/`clippy` runs (other
processes under a quarter of the cores; check because it is the edit loop
and test/build units do codegen, 2-3x the cost), counting only lib/bin units
of packages cargo marked dirty. Validation via the hidden
`justrust split --runs <ids>`, which replays given runs against the current
checkout, with typical costs taken from the workspace's own history:

| candidate | old estimate | new estimate | measured |
|---|---:|---:|---:|
| jcode-base::external_auth | ~12s/edit | 7.7s/edit | 4.1s/edit |
| desktop ui-core::scrollbar | ~3.5s/edit | 0.7s/edit | 0.3s/edit |

Correction: a first version of this table showed 3.8s and 0.5s. That was
circular: the typical costs were computed from the same benchmark runs the
estimate was checked against. With costs from history only, the estimate is
still about 2x high on both. Cause, from the per-run data: history mixes
single-threaded and threaded-front-end runs (jcode-base check 2.5-15s across
the switch in section 14), and the cascade crates (10 provider runtimes) have
few quiet check samples, so their medians come from test/build runs.
Restricting to threaded runs gives 6.0s, but then most of the provider crates
have no samples at all. The estimate improves as history accumulates under
the current toolchain; it is a ranking signal, not a forecast.

Also fixed: the root package's sources included every member crate under
it (and `target-release/`), so `jcode-desktop` was listed as a user of
anything any member used. The predicted users now match the crates that
still rebuilt in both splits (jcode: jcode, app-core, tui; Desktop:
jcode-desktop-ui). They are not the set of manifests to edit: in the jcode
split only app-core needed the dependency, and tui and jcode reached the
module through app-core's `pub use jcode_external_auth as external_auth`.
The report now says so.

Remaining limits: two validation points only, both overestimated about 2x
(the Desktop one is also inside the run-to-run spread of 2.6-3.2s walls).
Which crates stop rebuilding was exact both times; the seconds are not. The
saving is a per-package median, so a candidate seen in few quiet runs
inherits whatever runs exist; and the moved share is
by lines, which ignores that a small crate has fixed overhead (~0.1-0.2s
per new crate in both splits).

## 16. Skipping downstream rebuilds after body-only edits: measured, not viable as a wrapper (2026-10-09)

Question: can justrust skip rebuilding dependents when an upstream edit
leaves the public interface unchanged, with a background full build as the
correctness check?

Size of the prize (~/jcode, 77 recorded runs with edits): 930s of 2199s wall
(42%) was local units rebuilt only because a dependency rebuilt. Per crate,
a cascade-only rebuild costs: jcode-base median 6.1s, jcode-app-core 4.1s,
jcode-tui 3.1s, jcode-protocol 1.9s (check runs).

What a wrapper can see. Scratch workspace `up -> down -> top`
(`~/.jcode/scratch/rmeta-exp`), rustc 1.98.1, `cargo check`:

| Edit in `up`                         | `up` rmeta bytes changed |
|--------------------------------------|-------------------------:|
| non-generic fn body                  | 32 (crate hash only)     |
| private fn body                      | 32                       |
| `#[inline]` fn body                  | 32                       |
| generic fn body                      | 1106 (MIR is exported)   |
| comment that shifts lines            | 1129 (spans)             |
| pub field type                       | 34                       |

The 32 bytes are the crate hash (SVH, `rustc -Z ls=root`), which covers
private items too. So:

1. A byte hash of the rmeta changes on every edit, and the SVH is not an
   interface hash. A body-only edit and a pub field type change both differ
   by just the SVH, so the wrapper cannot tell them apart from the outside.
2. Skipping is not even mechanically possible: the downstream rmeta embeds
   the upstream SVH. Keeping `down`'s old rmeta and compiling `top` against
   the new `up` fails with E0460 ("found possibly newer version of crate `up`
   which `down` depends on"). Rewriting SVHs would be required, and that is
   exactly where upstream ran into ICEs and miscompilations: DefIds shift
   when items are added, invalidating artifacts that were not rebuilt.
3. rustc's own incremental cache already absorbs part of it: in the scratch
   workspace a 9k-line dependent rechecks in 0.6s after an upstream body edit
   vs 1.75s cold; in jcode a cascade recheck of jcode-tui is ~3s.

Upstream status: this is the Rust project's "Relink, don't rebuild" (RDR)
work (compiler-team MCP 790, project goal 2025h2, rust-lang/rust#143249 for
comment-only changes). The 2025h2 goal was marked "will not complete"; it is
listed again on the 2026 Fast Builds roadmap with a 5-10x target for
body-only changes.

Verdict: no-go as a justrust wrapper feature. A correct version needs a
rustc change (an interface hash that excludes bodies, private items and
spans, plus SVH-stable DefIds), which fits the vendored/forked toolchain
plan in docs/toolchain.md and should track upstream RDR rather than
duplicate it. Until then the levers that work today are the ones in 14 and
15: fewer dependents per hot file (`justrust split`) and the threaded
front-end for the rebuilds that remain.

Real-project confirmation (jcode 04c7d2b04, scratch worktree, run
20261009-011923788-1604530): a body-only edit (`&& 1 > 0` inside
`can_prompt_for_external_auth`) rebuilt 16 dependent crates, every one with
cargo reason "the dependency ... was rebuilt", 11.6s wall. The E0460 failure
of a forced skip was reproduced only in the scratch workspace; forcing it on
jcode would mean planting stale rmeta files in a target dir, which proves
nothing more than the minimal case.

## 17. `justrust split --apply`: deterministic splits in the user's own tree (2026-10-09)

Why: AI agents split crates by hand, turn by turn. Two recorded Desktop
split sessions: tigress (2 crates, 7 modules, ~2h) made 87 tool calls (71
bash, 9 compiles), 7.9M prompt tokens processed (97% cache reads), 45k
output, peak context 141k; cricket (74 min) made 192 tool calls, 30.5M prompt
tokens, 161k output, peak 271k. Most turns were mechanical (grep users, read,
sed paths and manifests, git). Each turn resends the growing context, so cost
scales with turn count, not with judgment.

What it does (`src/split_apply.rs`), in the working tree the user is in:

- Refuses unless `git status --porcelain` is empty (untracked included), so
  HEAD is the pre-split state. One commit afterwards, with the user's git
  identity, naming the pre-split commit; undo is `git revert` or
  `git reset --hard <pre>`.
- Refuses what it cannot do safely: modules other modules reference (the
  closure from section 15), inherent impls of types that stay, crate-root
  uses, test-only modules, feature-gated code, optional deps, glob
  re-exports of the module, and crate-identity macros (`include_*!`,
  `CARGO_MANIFEST_DIR`, `CARGO_PKG_NAME`, `CARGO_CRATE_NAME`,
  `module_path!`), whose values silently change with the crate. Warns
  when log/tracing macros will change target.
- Rewrites: moved files (`crate::` -> old crate, `super::` chains that leave
  the subtree, tracking inline `mod {}` blocks and skipping strings and
  comments; top-level `pub(super)` -> `pub`); users (`old::m` paths, `use
  old::m;` -> `use new as m;`, multi-line `use old::{..}` groups, names the old
  root re-exported from the module); glob re-exporters (`pub use old::*`) get
  `pub use new as m;` so `crate::m` keeps resolving.
- Manifests with toml_edit: package fields, lints and dependency specs
  copied from the old crate (only crates the moved code names), the old crate
  by the spec its users use, workspace members and
  `[workspace.dependencies]`, user deps in their existing style.
- Verifies: `check -p new --all-targets` with two rule-based fixes (missing
  dependency of the old crate; old crate's `test-support` feature for moved
  tests), formats only touched files that were rustfmt-clean at HEAD (stdin,
  so `mod` declarations are not followed), `check --workspace
  --all-targets`, and `test -p new`, which must run at least as many tests
  as the moved files declared. Any failure restores every touched path from
  HEAD and removes the new crate dir.

Checks:

- `bench/split-apply-e2e.sh` (fixture workspace, real cargo and git, 15
  checks): success path (clean tree, one commit on the pre-split commit,
  tests and app output unchanged, emptied dir removed), dirty-tree refusal
  (tree untouched), root re-exported name rewritten in a dependent, a
  behavior change caught by tests (tree byte-identical by sha256, HEAD
  unchanged), refusals for an entangled module and `CARGO_PKG_NAME`.
- Real jcode (local clone at 04c7d2b04): `--apply
  jcode-base::external_auth` succeeded unattended in 4.5 min (cold clone
  build), 10/10 moved tests, 24 caller tests in `jcode` pass. Same edit,
  `check --workspace`, 3x: 10.9/11.0/10.5s before, 8.6/7.2/7.6s after (the
  hand split in section 15 measured 11.9 -> 7.8s).
- Real Desktop (clone at 400400d with a committed sibling jcode): `--apply
  jcode-desktop-ui-core::scrollbar --name jcode-desktop-scrollbar`
  succeeded unattended in 1.7 min, 4/4 gpui tests, diff of 7 files and
  rustfmt-clean (the 6 files `cargo fmt --check` flags were already
  unformatted before). `check -p jcode-desktop-ui`: 2.8/2.7/2.9s before,
  2.2/2.3/2.7s after.
- Refusal on the user's real ~/jcode-desktop, which has 3 uncommitted files:
  refused with the list, tree untouched.

Bugs the real runs found and fixed before this commit: `super::` inside
inline test modules was rewritten as if it escaped (scope tracking added);
module depth was taken from file paths instead of the module tree; users
reached only through a glob re-exporter got a needless dependency (and
`add_user_dependency` failed on them); `use old::{.., m, ..}` became `use
new;`, losing the name `m`; whole-directory rustfmt reformatted files the
split never touched.

What an agent now does for a ready candidate: `justrust split` shows
`apply: justrust split --apply <crate>::<module> --dry-run`, the agent
commits its work, runs it, and reads one result line. The report's `apply:`
hint appears only for candidates with no blockers.

## 18. One feature set per workspace: no dependency rebuilds when switching packages (2026-10-09)

Code: `unify_features` in `src/record.rs`. Bench: `bench/feature-switch.sh`.

### Why the depcache held so many variants

`justrust cache` showed 21.8 GB in 9,106 entries. Grouping entries by cargo's
own output file name (which already encodes version, features, profile, and
dependencies), 9.7 GB were extra entries for the same output under different
keys, and most of the rest were genuine feature variants (`aws_sdk_bedrock`
11 builds, `gpui` 17, `reqwest` 32, `syn` 29).

The cache key itself is stable. With `JUSTRUST_DEPCACHE_EXPLAIN=<dir>` (dumps
the normalized key inputs of every unit), two fresh target dirs and a
separate checkout of Desktop produced identical keys for all 643 crates and
833/833 hits. The duplicates came from builds that really were different:
cargo resolves features per command, so `check -p jcode-desktop-model` builds
`subtle` with no features and `mio` without `log`, while `-p
jcode-desktop-ui` builds them with `default,i128,std` and `log`. Each
different set means a different `-C metadata`, different `--extern` hashes
for everything downstream, and new entries.

### Fix

Every recorded cargo run sets `CARGO_RESOLVER_FEATURE_UNIFICATION=workspace`
(cargo's `-Zfeature-unification`): features are resolved once for the
whole workspace, so every `-p` selection compiles the same build of each
dependency. The nightly gate is opened with cargo's channel override, not
`RUSTC_BOOTSTRAP`: rustc reads `RUSTC_BOOTSTRAP`, so cargo fingerprints it,
and setting it rebuilt 304 units of desktop-ui once (measured). The override
is read only by cargo. Off with `JUSTRUST_UNIFY_FEATURES=0`, and an explicit
`CARGO_RESOLVER_FEATURE_UNIFICATION` (for example `selected`, cargo's
default) wins.

### Measurements

`bench/feature-switch.sh`: fresh target dir, private depcache, `check -p`
each package twice through the list. Machine busy (other agents building).

Jcode Desktop (`model, harness, ui, desktop`):

| | pass 1 compiled / deps | pass 1 wall | pass 2 |
|---|---|---|---|
| unify on | 166+329+392+8 = **895** / 764 | 91s | 0 compiled |
| unify off | 114+395+642+8 = **1,159** / 971 | 121s | 0 compiled |

Jcode (`jcode-base, jcode-tui, jcode-app-core, jcode-protocol`):

| | pass 1 compiled / deps | pass 1 wall | pass 2 |
|---|---|---|---|
| unify on | 418+230+0+0 = **648** / 525 | 110s | 0 compiled |
| unify off | 412+354+85+85 = **936** / 737 | 179s | 0 compiled |

With unification, once any package is built the others need no further
dependency work (jcode-app-core and jcode-protocol: 0 compiled, 0.3s,
instead of 85 units and 34s/14s). That is 23% and 31% fewer units compiled
for the same four commands, and one depcache entry per dependency instead of
one per feature combination.

On the warm main checkout, switching `model -> harness -> ui -> desktop ->
model -> harness -> ui` compiled 0 units on every step (0.4-0.5s each).
Before, `check -p jcode-desktop-model` after `-p jcode-desktop-harness`
rebuilt 27 units and the reverse 131.

Correctness: `justrust test -p jcode-desktop-model` (140 passed) and
`justrust test -p jcode-protocol` (89 passed) with unification on. The
first unified run in an existing target dir rebuilds dependencies whose
feature set grows (one time, up to ~130 units in Desktop); after that,
plain cargo without unification and justrust with it disagree on features,
so mixing them in one target dir rebuilds the differing dependencies.

Note: in this repo, `justrust build --release` goes through a build slot,
so `target/release/justrust` is not updated. Use `JUSTRUST_SLOTS=0` when the
binary itself is the output.

## 19. Split recommendations at build time, from incrementally stored evidence (2026-10-09)

`giant_crate` and `upstream_cascade` told the agent "split, run `justrust
split`" without saying what. The specific answer needs the workspace's edit
history, a module graph and, for root files, git churn, and `justrust split`
recomputed all of it per call: 1.37s on ~/jcode (re-reading 1,370 run
summaries, re-parsing ~1,100 source files, 4 `git log`s).

Every piece is now stored by whatever already has it (`src/split_index.rs`):

| Piece | Written by | Store |
|---|---|---|
| Edit record: changed files, local unit times, quiet/check | every recorded run, local and remote, when its summary is written | `split/edits.jsonl` (append; old runs back-filled once from `summary.json`) |
| Per-file parse facts: lines, `mod` items, written module paths, types, inherent impls | any module-graph build; only files whose size/mtime changed are re-parsed | `split/facts-<ws>.json` |
| Root-file churn (git hunk headers) | report/refresh, keyed by HEAD and day | `split/churn-<ws>.json` |
| Per-file hint: the concrete recommendation for edits to that file | niced, detached `justrust split --refresh-hints` after builds with edits (at most once a minute per workspace, flock'd), and every `justrust split` | `split/hints-<ws>.json` |

A finishing build reads only the hints file and attaches the hints for the
files it edited (14 us measured), and the costliest of `giant_crate` /
`upstream_cascade` shows it instead of the generic text. Path resolution
was split from extraction (`written_paths` stores paths as written,
resolution runs against the module tree), so facts are cacheable per file.

Measurements on ~/jcode (release build):

| | before | after |
|---|---:|---:|
| `justrust split`, warm | 1368 ms | 196-274 ms |
| `justrust split`, empty `split/` (back-fill + parse) | 1368 ms | 587-967 ms |
| `--refresh-hints` (background, niced) | n/a | 156 ms |
| hint lookup at the end of a build | n/a | 14 us |

Regression: on a frozen copy of the run history (1,402 summaries), the
`justrust split --days 365` report is byte-identical before and after for
both ~/jcode (161 lines) and ~/jcode-desktop (63 lines), cold and warm.

End to end, in ~/jcode: appending a comment to
`crates/jcode-base/src/config/config_file.rs` and running `justrust check -p
jcode-app-core` (6.8s, run 20261009-213927583-3383129) printed:

> jcode-app-core rebuilt only because jcode-base changed (edited
> crates/jcode-base/src/config/config_file.rs) (3.2s). Specifically, edits to
> jcode-base::config::config_file cost ~17.3s each here (2 recorded runs).
> Moving jcode-base::config::config_file (863 of 134594 lines) into its own
> crate that depends on jcode-base would stop them rebuilding jcode-base and
> 13 dependents, saving ~15.8s per edit. First turn its `impl Config` blocks
> (the type stays in config) into free functions or an extension trait ...

The same build appended its edit record and triggered the background
refresh (hints file rewritten 0s after the run).

Hint kinds: a movable module (saving, moved lines, spared dependents,
remaining users, and either the `--apply` command or the blocker to fix
first), an entangled module (share of the crate it would drag along and
the references to cut), a crate root file (most edited items and how many
dependents use each; says when moving them would not help), a build-script
input (embed in a leaf crate or load at runtime).

Limits: hints are as fresh as the last refresh (at most a minute behind the
latest build plus the refresh time); the first build in a workspace with no
`split/` state shows the generic advice and starts the refresh. The savings
model is unchanged from 15b (ranking signal, about 2x high in two
validations).

## Remote sync daemon protocol v2 (ed048bb): sync cost unchanged, remote post-build tail found

Warm `justrust remote check` on c7i.8xlarge us-west-2, same machine, before
(v1 daemon) and after (v2):

| workload | v1 | v2 |
|---|---|---|
| ~/justrust no-op | 0.6s (sync 30-33 ms) | 0.6s (sync 29-31 ms) |
| ~/justrust one-line edit | 0.6-0.8s (sync 32-49 ms) | 1.1s (sync 30-31 ms) |
| jcode-desktop-model no-op | 0.6-0.7s (sync 47-49 ms) | 0.6-0.7s (sync 46-50 ms) |
| jcode-desktop-model one-line edit | 0.6-0.7s (sync 79-81 ms) | 0.7-0.9s (sync 79-86 ms) |

Sync is unchanged. The ~/justrust edit total moved because of the machine,
not the daemon: run on the machine directly, a `justrust check` after
`touch src/remote_watch.rs` takes 1.04s with the v1 binary (213cd03^), with
213cd03 and with v2 alike, while plain `cargo check` takes 0.50-0.55s. An
strace (run 20261010-044440144-6736 on the machine) shows the last rustc
exit at +0.82s and justrust exiting at +1.11s: about 0.29s after the build
plus ~0.06s before cargo starts, in the remote justrust wrapper, not in sync.
Since all three binaries show it today, something on the machine changed
since the v1 measurement (cause not identified yet). Worth chasing next.

Daemon ping during a 2,102-file cold mirror of a jcode clone: 0.2-0.4 ms
(previously it shared the one state lock with rsync and pushes).

## Hosted backend end to end (2026-10-09, local wrangler dev + real AWS dev stack, c7i.4xlarge 16 vCPU us-east-1)

Client `src/remote_hosted.rs` against the build-host API (`/v1/build/host*`).
146 ms round trip from the laptop (us-east-1, vs 29 ms to the us-west-2 aws
backend), so small edits cost more than on aws.

| step | time |
|---|---:|
| first `remote up`, server creating the machine | ~75 s (server: 84-119 s launch to ready) |
| `remote up` with the host running (fresh key + connect + new master) | 4.6-4.7 s |
| `remote down`, then `remote up` (wake, new public IP pinned) | 43-46 s |
| `remote check` ~/justrust, cold machine | 11.9-12.5 s (sync 2.9-5.7 s) |
| `remote check` ~/justrust, warm no-op | 0.7 s (sync 71 ms) |
| `remote test` ~/justrust, 157 tests | 10.3-11.5 s |
| `remote check -p jcode-desktop-model`, cold, two roots | 25.4-25.6 s |

Server bugs found by the run and fixed by the server side: c7i.8xlarge over
the 16 vCPU on-demand quota (503, client guidance correct), a readiness
marker broken by systemd `$(...)` expansion (wake stuck at booting, client
timed out cleanly at 360 s), and systemd-tmpfiles resetting /home's ACL mask
(fixed with jcode-mkdir creating /home/<u>).

## 20. Benchmark suite on pinned snapshots, and 0.5s lost at every build exit (2026-10-10)

`bench/suite.py` replaces ad-hoc single-scenario scripts as the basis for
speed claims: pinned `git archive` snapshots (jcode-desktop 7d38fb7 + jcode
abff94eeb, justrust f97fcc2), private target dirs, a unique edit per
iteration, 1 warmup + 5 measured runs, phases read from each run's
summary.json, JSON results in `bench/results/`. See `bench/README.md`.

Baseline (f97fcc2, `bench/results/20261010-005520-f97fcc2d7490.json`),
median wall as the agent sees it: reference loop `desktop-test-edit` 8.06s,
body-edit test 8.05s, body-edit check 2.54s, first type error 2.04s, no-op
test 0.53s, upstream ui-core body 9.54s, ui-core new pub fn (check) 3.04s,
jcode-core body 15.05s, justrust small-crate test 3.03s.

The first run showed every agent-visible wall time on a 0.5s grid (x.03 /
x.53). Agent-visible wall minus recorded wall was median 0.27s, max 0.52s:
`sched::Scope::finish` joined the cgroup weight thread, which slept
`UPDATE_EVERY` (0.5s) between checks. Fixed in c219b49 (the thread waits on a
channel with a timeout and wakes on drop). After
(`bench/results/20261010-010147-c219b496046c.json`): overhead median 0.034s,
max 0.082s. desktop-body-check 2.54 -> 2.27s, desktop-type-error 2.04 ->
1.78s, no-op 0.53 -> 0.47s (all with non-overlapping ranges); the long
scenarios moved by about -0.4s, within their run-to-run spread.

Where the reference loop goes now (7.59s): startup 0.48, frontend 3.83,
codegen 1.11, incremental persist 0.77, link 0.47. jcode-core body edit:
16 units, frontend 10.6s, the worst warm case.

Cold baseline (`bench/results/20261010-011659-cold.json`, 1 warmup + 3
runs, fresh target dir each, check -p jcode-desktop-ui, 887 units):
depcache on 16.1s (15.8-16.4), every cache off 70.4s (70.2-70.9). The
desktop app itself kept 7-8 other cores busy, so all samples are flagged
noisy. A first attempt measured 0 depcache hits: the suite forced
`JUSTRUST_PASSES=always`, and units compiled with -Ztime-passes are never
stored. The suite no longer forces pass timings for cold scenarios.

Suite noise check (A/A): re-running six warm scenarios on the unchanged
build (`bench/results/20261010-012107-ba5914b-repro.json` vs
`20261010-010147-c219b496046c.json`, about 20 min apart) moved medians by
-9% to +2%, and one scenario (desktop-upstream-sig, -8%) had disjoint
ranges. So `compare` now also needs the median to move more than
`--floor` (default 10%). With it, the A/A pair reads all "within noise"
and the sched fix still reads faster on body-check (-11%), type-error
(-13%) and no-op (-11%). Exit overhead (agent wall minus recorded wall),
the direct measure of that fix: 0.271/0.515s med/max before, 0.034/0.082s
after, 0.036/0.069s on the A/A rerun. The rerun also exposed a flaky
justrust test (ETXTBSY exec race in buildscript tests, fixed ba5914b); the
suite now records a failing test instead of aborting.

## 2026-10-10: slot refresh copied a half-written shared target (jcode)

Symptom: `justrust test -p jcode-tui --lib -- <filter>` in ~/jcode failed with
197 errors starting `E0463 can't find crate for jcode_app_core` /
`jcode_tui_permissions`, three runs in a row, while `cargo test` on the same
tree passed. Each run printed `slot 0: copied 0 new and 8 rebuilt units (21
files) from the shared target dir`.

Context: a second justrust run in a `git worktree` of the same repo was using
`CARGO_TARGET_DIR=~/jcode/target` (the shared target) at the same time, and
another agent session owned slot 1. The slot refresh copied a partial set of
rlibs/rmetas from the shared target mid-rebuild, leaving slot 0 with metadata
for crates whose artifacts were missing.

Recovery: one plain `cargo test` rebuilt the shared target; the next justrust
run copied 47 new + 10 rebuilt units and passed.

Fix idea: refresh only from a shared target that is not locked by a running
cargo (respect `target/debug/.cargo-lock`), or verify copied rmeta/rlib pairs
exist before trusting a refreshed slot and fall back to a rebuild.

### Root cause and fix (2026-10-10)

Not a half-written copy. The lock check was already right: cargo 1.98 holds
`.cargo-build-lock` exclusively for the whole build (seen with `lslocks`),
so the refresh's shared flock cannot be taken during a build. The real
cause is that **unit hashes do not depend on the checkout path**: two
copies of a workspace in different directories produce identical
`.fingerprint/<pkg>-<hash>` names (verified with a 2-crate workspace copied
to two dirs). The worktree at `~/.jcode/scratch/base2-wt` built with
`CARGO_TARGET_DIR=~/jcode/target`, so its `jcode-app-core-463b...` and
`jcode-tui-permissions-d481...` (different sources) landed in the shared
dir under the same names as ~/jcode's. Refresh saw them as newer and copied
them into slot 0 with fresh mtimes, and cargo then trusted them for ~/jcode's
sources. The extern rlibs loaded fine on their own (checked with rustc),
but the set came from another source tree, so `jcode_tui` failed with E0463
at the crates' first use. Six runs in a row failed in slots 0 and 1
(runs 20261010-025433901-1482017 .. -025705623-1502688).

Fix (src/slots.rs):

- Refresh never copies workspace-local units. Locality comes from the
  unit's dep-info: rustc writes workspace sources as relative paths and
  everything else as absolute ones. It is decided per package, so a
  member's run-build-script units are skipped too, and only the first 4 KB
  of one `.d` per package is read. Refresh exists for dependency changes.
  The slot always builds its own members, which cargo's fingerprints
  handle correctly.
- Safety net: after a failed slot run whose output has E0463/E0460 for
  crates the slot has units for, those units' fingerprints are deleted and
  justrust says so, so the next run rebuilds them instead of failing the
  same way again.

Checked on ~/jcode: plain `cargo check -p jcode-tui-permissions` rebuilt 25
shared units (registry and members). The next `justrust check` refreshed
9 units in 0.2s, all registry crates (azure_core, reqwest, ...), and none
of the members (jcode-base, jcode-tui-permissions stayed slot-built). It
then compiled 0 units.

## 2026-10-10: `justrust clippy` reported 0 compiled units

cargo runs workspace members under clippy as `$RUSTC_WORKSPACE_WRAPPER
$RUSTC <args>`, and clippy-driver compiles in-process, so the shim never ran
and the report said "0 compiled, compile 0.00s, startup 1.36s" while cargo
printed `Checking justrust` (run 20261010-061717076-2544870). Pointing the
wrapper at the shim is not an option: cargo hashes the wrapper path into
unit hashes (two wrapper paths gave different `.fingerprint` names in a
scratch workspace), which would stop sharing artifacts with plain `cargo
clippy`. The summary now reconstructs these units from the sampled
clippy-driver processes (200 ms resolution, no pass timings). Same edit
after the fix: "3 compiled (3 local), compile 1.99s" (run
20261010-062530043-2617015).
