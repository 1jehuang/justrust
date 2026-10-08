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
  run).

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

Status: the gpui commit is pushed. The Desktop pin bump is NOT landed: the
running Desktop host (pid 2174267, built on gpui 424044a) refuses a UI plugin
built against another GPUI ("plugin GPUI revision differs from host"), so
landing it breaks Ctrl+R hot reload for every agent until the host is
restarted. The ready change is in
~/.jcode/scratch/gpuitestapi/desktop-test-api.diff (also the "after" copy's
Cargo.toml, Cargo.lock, crates/jcode-desktop-ui/Cargo.toml,
crates/jcode-desktop-api/src/lib.rs). Land it together with a planned host
restart.

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
