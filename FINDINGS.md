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

## Remote vs local on uv, for the /rust landing page (2026-10-10)

uv at 44b2e58 (674 units, 758 lockfile crates). Local: this laptop, 16
threads, load 7 to 13 from other agents. Remote: hosted dev build host
(c7i.4xlarge, 16 vCPU, us-east-1) through local justrust-cloud and backend.

| case | local | remote (client wall) |
|---|---:|---:|
| cold `check --workspace`, no depcache | 70.3 s, peak 3.4 GB RSS, 563 CPU-s, target 1.8 GB | 52.2 s incl. 4.5 s first sync |
| fresh target dir, depcache warm on that machine | 77.7 s (local depcache cold for uv: 5/574 hits) | 22.9 s (574/574 hits, saved ~171 s rustc) |
| `test -p uv-resolver --no-run` | 73.6 s, peak 4.1 GB, target grows to 4.2 GB | 38.5 s |
| one-line edit, `check -p uv-resolver` | | 1.0 s |

Local footprint of remote builds: 0 bytes of target, no local rustc. The
remote numbers are on a 16 vCPU machine; production default is c7i.8xlarge
(32 vCPU) once the quota increase lands.

Over 272 recorded Jcode Desktop builds over 20 s on this laptop: median peak
build RSS 4.3 GB (p90 5.9, max 8.5), median peak 11 cores, median 237 CPU-s.
Local target dirs here: jcode 744 GB, jcode-desktop 1.1 TB.

## 2026-10-10: pinned toolchain (phase 1) vs system rustc

`justrust toolchain` now installs dated toolchains from static.rust-lang.org
into `~/.justrust/toolchains/<id>` (commit 2e5721d). nightly-2026-10-08 with
rustc, cargo, std, rust-src, clippy, rustfmt, cranelift, rustc-dev and
llvm-tools: 23.4 s to download, verify and unpack (9 components in
parallel), 1.66 GB on disk. Cranelift loads (`-Zcodegen-backend=cranelift`
hello world runs).

Reference loop, same pinned snapshots, own target dir per toolchain
(`bench/suite.py run --toolchain`), 5 samples each:

| scenario | system 1.98.1 | nightly-2026-10-08 | compare |
|---|---:|---:|---|
| desktop-test-edit | 8.49 s (8.15..10.73) | 7.44 s (6.76..8.78) | -12%, within noise (ranges overlap) |
| desktop-body-check | 2.58 s (2.43..3.05) | 2.44 s (2.02..2.75) | -6%, within noise |

Results: `bench/results/20261010-190616-5b0be04862d3.json` (system),
`bench/results/20261010-192507-5b0be04862d3-dirty-nightly-2026-10-08.json`
(the dirty flag is an uncommitted README edit only). Frontend 4.27 -> 3.59 s
and incr 0.86 -> 0.71 s moved most, which fits a newer rustc, but the
samples overlap, so no speedup is claimed. The cold dependency build on the
new toolchain took 155 s (run 20261010-190717445-4098100643, 887 units),
and a later warmup rebuilt 662 units in 64 s (run
20261010-191741408-4098100974): switching toolchains costs one cold build
per target dir.

Takeaway: phase 1 does not slow the loop down, and it delivers what phase 2
needs: one exact nightly on every machine, unstable flags without
`RUSTC_BOOTSTRAP` (the shim now gets per-pass timings for every local crate
on a pinned nightly), Cranelift, and rustc-dev. The wins come from the
phase 2 flag experiments on this toolchain.

## 21. Two benchmark systems: rolling agent log and one-factor-at-a-time (2026-10-10)

**System 1, `bench/trend.py`** (committed log in `bench/trend/`): every
foreground check/test/build an agent ran in Jcode, Desktop, and justrust,
from Jcode session history (September onward, so it includes the time
before justrust) plus justrust run records. `justrust history` had two
bugs that made early weeks look like 0.1s medians: queued self-dev builds
and reload acknowledgements counted as blocking calls, and `justrust ...`
commands were dropped (fixed 6987dbd, with a test). Median blocked wall,
baseline (before 2026-10-07) vs 2026-10-04..10: Desktop 26.6s -> 21.8s
(-18%), Jcode 35.3s -> 25.5s (-28%), justrust 8.5s -> 6.3s (-26%); `check`
-47 to -54% in all three. Not uniformly better: same-command
`test -p jcode-desktop-ui --lib` 24.2s -> 31.6s (653 vs 54 calls, not
controlled). Reuse ledger for W41: 86% (Desktop) to 95% (Jcode, justrust)
of units were not recompiled; Desktop depcache hits replaced about 727 min
of rustc. Raw agent data: the work mix changes, so it shows the trend,
not causes.

**System 2, `bench/factors.py`**: each factor switched off alone on the
pinned suite snapshots, on/off interleaved ABBA, private target dirs and
caches, reuse ledger per sample. First run
`bench/results/factors-20261010-194503-f7c1b3e840c7-dirty.json` (the
"dirty" is another agent's uncommitted depcache work in the tree, not the
bench). Machine busy with other agents, so the cold rows are flagged noisy.

| factor | workload | on | off | saved |
|---|---|---:|---:|---:|
| depcache (+script cache) | fresh target dir, `check -p jcode-desktop-ui` | 18.6s (18.4-18.7) | 104.1s (100.2-123.8) | 85.5s (-82%), 833 units restored |
| build-script cache | same, depcache warm in both | 19.6s | 27.7s | 8.1s (-29%), 45 scripts replayed |
| feature unification | fresh dir, `check -p` model, harness, ui | 97.9s, 887 units | 122.8s, 1151 units | 24.9s (-20%), 264 fewer units |
| slots | agent B's reference loop while agent A builds | 10.6s | 13.8s | 3.3s (-24%): 4.2-4.8s lock wait gone |
| parallel front-end | reference loop | 8.67s | 8.71s | none |
| parallel front-end | jcode-core body edit, 16 crates | 16.3s | 16.5s | none |
| mold vs lld | reference loop | 9.26s | 9.37s | none (link pass 0.8 vs 1.4s) |
| incremental (CARGO_INCREMENTAL=0 off) | reference loop | 9.04s | 25.20s | 16.2s (-64%) |
| pinned nightly vs system stable | reference loop | 8.08s | 8.78s | 0.7s (-8%), below the 10% floor |
| cranelift (all crates, nightly both) | reference loop | 8.25s | 8.05s | none |

Notes:
- The first depcache run lost one sample to a mid-run justrust reinstall
  (new depcache key scheme, ab89431/0ab7874: 833 misses). Rerun
  `factors-20261010-204006-c3c63f0156d5-dirty.json` via
  `bench/factors-isolated.sh` (frozen justrust copy, private
  JUSTRUST_HOME) is the row above; the second attempt had aborted on
  another reinstall, which the new guard caught.
- Parallel front-end: switch verified (frontend_threads 8 vs none), and
  it does not help the warm incremental loop: the edited crate's frontend
  is mostly incremental query reuse. Its measured win is on full
  re-checks (FINDINGS 14: 44s -> 24s). So it matters for cold or
  upstream-cascade builds, not the reference loop.
- mold: switch verified (ld.mold vs collect2+ld.lld). The link pass is
  0.65s shorter but within the loop's noise. The system default here is
  already lld, not GNU ld.
- Cranelift: no gain on the warm reference loop (1 unit, mostly frontend
  and incremental); the first cranelift build was 121s cold. Pinned
  nightly ran 8% faster with disjoint ranges, but under the 10% floor.
- Bottom line: the measured wins are reuse (depcache 82%, incremental
  64%, build-script cache 29%, slots 24%, feature unification 20%). The
  CPU-side knobs (parallel front-end, mold, cranelift, nightly) are each
  within noise on the warm loop. The warm loop (~8-9s) is now bounded by
  one crate's incremental frontend + persist, so the next big step needs
  less frontend work per edit (crate splits, resident compiler), not
  faster codegen or linking.

## 22. Test execution: one slow test sets the suite's wall time (2026-10-10)

Across 1193 recorded test runs on this machine, compile is still most of
the wait (jcode-desktop: 94 min compile vs 9 min running tests; jcode: 95
vs 16). But the jcode-desktop-ui lib suite (1290 tests) runs 10-38s, and
its wall time was bounded by single tests: under load,
`startup_each_native_submission_paints_the_newest_row` took 31.6s (9.2s
alone), 35 tests over 2s were 43% of summed test CPU. Running slowest-first
with one process per slow test did not help (19-22s vs 19-21s, load 44):
the suite is CPU-bound, so the fix is cheaper tests, not scheduling.

Causes, from a samply profile and reading the tests:

- `simulate_input` dispatches one keystroke and one full window render per
  character. A test typing 20 prompts of ~210 chars rendered ~4200 times
  over a growing transcript. Fix: `test_input::commit_text`, one IME commit
  through the same input handler (9.15s -> 0.17s).
- Scroll tests travelled 500+ rows in 180px wheel steps, each settled for
  up to 16 frames. Long strokes reach the same end state (7.13s -> 0.35s;
  mutation-checked that a wrong target still fails).
- Real `thread::sleep` for 2x tween durations, and PDF pages rasterized at
  2400px (~450ms of pdftoppm each) in tests that only check layout.

Result, interleaved A/B of the old and new test binaries on the same
machine (load 26-40), whole suite: old 17.3/18.8/16.9/12.2s, new
10.7/11.0/10.2/8.1s, median 16.9s -> 10.5s (-37%). Runs
20261010-203941085-1026350 (baseline, 9.15s slowest) and
20261010-204556033-1118868
(after). jcode-desktop commit a780ae3.

justrust now records per-test durations on every `test` (255d152): a cargo
`runner` (`justrust __test-runner`) adds libtest `--report-time` with a
process-local `RUSTC_BOOTSTRAP`, so artifacts stay Fresh. The report lists
the slowest tests when one is >=1s and >=10% of the test phase, and
`slow_tests` names the slowest one.

Remaining, measured alone after the fixes (load 16): pending_editor 2.6s
(~2.2s of it real `thread::sleep`, because motion tweens read
`Instant::now`; an injectable animation clock in jcode-desktop-motion would
remove it and the other 2x-duration sleeps in workspace.rs), tail_glide
tests 1.9-2.7s (rendering plus ~1s of 8ms frame sleeps the glide relies
on), everything else under 1.7s. tall_preview went 1.16s -> 0.07s.

## 23. Local and remote dependency caches combined (peer cache, 2026-10-10)

Code: `src/peercache.rs` (dfae1ad and follow-ups), depcache schemes 3 to
5, `docs/remote.md` "Peer dependency cache". Bench: `bench/peer-cache.sh`
on a pinned Jcode snapshot (abff94eeb, `check -p jcode-base`, 376
dependency units, nightly-2026-10-08), laptop (16 threads, load 5 to 15
from other agents) and the AWS c7i.8xlarge in us-west-2 (29 ms, ssh
throughput 15 to 20 MB/s down, 6 to 25 MB/s up).

### Why the two caches could not be combined before

Measured by building the same snapshot on both machines with
`JUSTRUST_DEPCACHE_EXPLAIN` and diffing the key inputs, then fixing each
cause and measuring again:

| cause | effect | fix |
|---|---|---|
| gitignored `Cargo.lock` (Jcode ignores it) was not mirrored | the machine resolved newer versions (itoa 1.0.18 vs 1.0.17): **remote builds compiled different code than local ones** | mirror `Cargo.lock` even when ignored (remote_sync) |
| system rustc differs (Arch 1.98.1 vs rustup 1.99.0) | no key can match | pinned toolchain (`justrust.toml`) on both sides |
| `--extern` keyed by whole-file content | rlib object code (sysroot paths in debuginfo) and proc-macro `.so` (gcc 16 vs 13, CRT) differ per host and cascaded up the graph: 24/56 keys equal | key `--extern` by crate metadata only (rmeta, rlib `lib.rmeta` member, `.rustc` section), scheme 3 |
| exact host glibc in proc-macro/bin keys | 2.44 vs 2.39: no proc macro shared | key the libc family, record the needed `GLIBC_x.y` (from `.dynstr`) and refuse to restore where the host is older |
| registry source paths in metadata (`/home/ubuntu/.cargo` vs `/home/jeremy/.cargo`) | rmeta differs, SVH differs | agent builds with `CARGO_HOME` at the client's path (registry/git symlinked to its own) |
| `CARGO_HOME`, `CARGO_TARGET_DIR` keyed | cargo passes them to rustc only when set: one side set them | not keyed (values are placeholders anyway), scheme 4 |
| generated `OUT_DIR` sources `include!`d (serde, serde_json, toml, ring, aws-lc) | the absolute target dir is in the metadata: 58 of 314 crates differed **even between two local target dirs** | compile dependency units with `--remap-path-prefix=<target>=/justrust-target`, scheme 5: 314/314 equal across target dirs |

After the fixes, keys of independently built caches: two local target
dirs 314/314 crates equal. Laptop vs machine 288/314: the 26 left are the
aws-lc/ring/sqlite `-sys` chain. Their build scripts compile C with the
host gcc, and the static archives are keyed by content.

A test binary for `jcode-protocol` linked from 28 rlibs fetched from the
machine (other gcc, other glibc) builds and passes 89/89 tests.

### Results (wall as the agent sees it, bench/peer-cache.sh)

| case | wall | dependency units |
|---|---:|---|
| remote-cold (both caches empty) | 61.5 to 64.5 s | 0 hit / 376 miss |
| local-cold (peer off) | 62 to 72 s | 0 / 376 |
| local-pull (local empty, machine warm, fetch on miss) | 66.3 s | 63 peer / 313 miss |
| remote-pull (machine empty, laptop warm) | 54.2 to 60.0 s | 88 to 101 peer |
| synced (after a remote build, background sync only, peer off at build time) | 41.1 to 57.9 s | 349 hit / 27 miss |
| floor: fresh target dir, local cache fully warm | 14.5 to 18.2 s | 376 / 0 |

Readings:

- **Background sync is what works.** After one remote build, the daemon
  pulled 376 entries (350 MB) in about 90 s, and the next cold local build
  hit 349/376 (-30 to -40% wall here). That is about as good as the
  cross-machine key overlap allows. The 27 misses are the `-sys` chain plus
  `aws_sdk_bedrock` (15 s alone), which keeps the wall well above the 15 s
  floor.
- **Fetch on miss is about break-even on this link** (local-pull 66 s vs
  62 to 72 s cold). The cost model declined 170 to 296 entries: the median
  declined unit is 480 KB and compiles in 0.09 s, which loses to a 5.8 MB/s
  measured single-stream link. The big ones that won were fetched.
  Fetching into a busy laptop also competes with the rustc processes it
  replaces for the link and the disk.
- **remote-pull saves 5 to 10 s** (88 to 101 entries) on a machine with 32
  cores, where compiling is already cheap.
- The remaining gap to the floor is the 26 `-sys`-chain crates. Making
  their C builds hermetic (one vendored C toolchain, toolchain.md step 3)
  would close it.

### Verification: paired A/B, cache filled only by the background sync

Clean protocol (`peer-ab2.sh`): wipe the machine's cache, build there once
through a fresh private home, wait for that home's daemon to sync, then
alternate cold (empty cache) and synced (a copy of the sync-filled cache)
local builds in fresh target dirs, peer fetching off. The sync pulled all
376 entries (350 MB) in about 30 s.

| pair | cold | synced | synced hits | load |
|---|---:|---:|---|---|
| 1 | 78.0 s | 60.9 s | 290/376 | 12 / 15 |
| 2 | 75.4 s | 43.8 s | 349/376 | 12 / 10 |
| 3 | 66.1 s | 45.1 s | 349/376 | 10 / 9 |

Median 75.4 s -> 45.1 s (-40%), every pair faster. Pair 1 got fewer hits
because the private home's daemon was still syncing while it was copied.
The cache directory had grown to 487 keys by then, so the copy caught part
of an in-flight install.

Two earlier runs of this A/B found real bugs, now fixed: a remote build
with `JUSTRUST_PEER_CACHE=0` journaled nothing, so the sync had nothing to
offer (0 entries pulled, fixed in 036f9cb). The sync also dropped journal
entries beyond the first 400 of a window instead of deferring them
(1fd90a2).

Bugs found and fixed on the way: a stale peer key set ruled out every
entry the machine compiled after the daemon connected (0/376 hits). The
daemon inherited `JUSTRUST_PEER_CACHE=0` from the build that started it.

## 24. jr-cargo: justrust's cargo fork, first change (2026-10-10)

Forked cargo at f3865b2a4, the cargo that the pinned nightly-2026-10-08
ships (`~/jr-cargo`, github.com/1jehuang/jr-cargo, branch `justrust`).
`justrust toolchain cargo-fork <binary>` installs a build as the
toolchain's `bin/jr-cargo` (refuses a binary built from another cargo
commit); builds with that toolchain run it, `JUSTRUST_CARGO_FORK=0` uses
stock cargo.

Where a no-op cargo run goes (Desktop, `test -p jcode-desktop-ui --lib
--no-run`, warm, `CARGO_LOG_PROFILE` + samply): 0.39 s wall. Full
workspace resolve against the registry index 110 ms (only to confirm
Cargo.lock), targeted resolve 300 ms, reading 713 dependency manifests
for downloads 136 ms, fingerprints ~60 ms, job queue 25 ms.

Change 1, resolve stamp: after a full resolve, store a hash of everything
it reads (lock file, member, path-dependency and [patch] manifests and
their workspace roots, cargo config files, version, flags, resolver env)
in `CARGO_HOME/.justrust-resolve/`. When it matches, the lock file is the
previous resolve and only the targeted resolve runs. Workspaces with
`[replace]` or unused patches always resolve fully, so their warnings stay
exact (`replace::overriding_nonexistent_no_spurious` caught that). The
first version missed unused-patch manifest edits
(`patch::update_unused_new_version`). Cargo testsuite: 4045 passed, the
same 2 environment failures as stock.

| Desktop, warm, 10 alternating runs | stock | fork, stamp off | fork, stamp on |
|---|---:|---:|---:|
| `cargo check -p jcode-desktop-ui` no-op | 0.435 s | 0.437 s | **0.367 s** |
| `cargo test --lib --no-run` no-op | 0.413 s | 0.482 s | **0.402 s** |
| justrust reference loop (test-fn edit), 10 each | 7.39 s (6.9-9.7) | | 7.55 s (6.7-8.4) |

So the stamp saves ~70 ms per cargo invocation, which is invisible in the
7.5 s reference loop (within noise).

Rerun with the channel-matched build (`build-jr.sh`, the binary actually
installed), raw cargo, 12 alternating runs each, 0 units rebuilt in any
run: `check -p jcode-desktop-ui` stock 0.327 s, fork with the stamp off
0.329 s, fork with the stamp on **0.271 s** (-17%). `test --no-run` was
noisy in this round (busy machine, 0.4-1.5 s spread; medians 1.29 / 1.45 /
1.07 s), so the earlier table is the cleaner number for it.

System 2 (`bench/factors-isolated.sh run cargo-fork cargo-fork-loop`,
`bench/results/factors-20261010-234515-fb4b860a68dd-dirty.json`), through
justrust with the pinned toolchain, on/off interleaved: no-edit `test`
0.49 s vs 0.52 s (10 each, -5%, within noise: justrust's own startup and
the test run dominate), reference loop 7.77 s vs 7.64 s (8 each, within
noise). So at the agent level the first fork change is not yet
measurable; raw cargo `check` is where the 17% shows. The point of this step is ownership:
the fork now builds, installs, and is selected by justrust, so the bigger
cargo changes (in-process fingerprints, a resident cargo, replaying a
recorded unit graph) have somewhere to land.

Pitfall found: a fork built without `CFG_RELEASE_CHANNEL` reports channel
`dev`, and cargo's defaults depend on it (`-Zembed-metadata=no` only when
cargo and rustc are both nightly). That changes every unit's fingerprint
("the config settings changed"), so switching between stock and fork
rebuilt all 887 units each time. `build-jr.sh` builds with the stock
cargo's release and channel; after that, switching rebuilds 0 units.

## 25. Remote builds ran on 4 of 32 cores (2026-10-11)

Of 1225 runs in 24 h, 42 ran remote (952 s of 26,000 s wall), and the
remote shared dependency cache served 0 hits (it is off by default,
`JUSTRUST_REMOTE_CACHE`, while the backend moves to AWS; the cloud worker
has `CACHE_ENABLED=false`). The first routed `test -p jcode-tui --lib` on
the c7i.8xlarge (run 20261011-003340339-57893, 194 s) never had more than 4
rustc units at once and averaged 3.0 of 32 cores: Jcode's
`.cargo/config.toml` sets `build.jobs = 4` for 15 GB laptops, and the remote
agent inherited it. That run was only observable because remote runs now
ship their run directory back (protocol v4 `RunFiles`).

The agent now sets `CARGO_BUILD_JOBS` to the machine's cores, capped at
one job per 2 GiB (30 on the c7i.8xlarge) unless the client passed `-j` or
`CARGO_BUILD_JOBS`. Cold `check -p jcode-tui` on the machine, fresh target
dir, all caches off: `-j4` 85.8 s (20261011-074216045-122932), `-j30`
40.6 s (20261011-074341880-137389), 2.1x.

## 26. System 3: stock cargo+rustc vs justrust, head to head (2026-10-11)

New `bench/versus.py`: the same agent workloads on `stock` (/usr/bin/cargo,
system rustc 1.98.1), `jr-nocache` (justrust, every cache off) and
`justrust` (defaults), round-robin on the pinned snapshots, with
user-space instructions retired by the whole build tree as a low-noise
second metric (spread 0.0-1.8% across runs; wall varies 5-50%).

First full result (`bench/results/versus-20261011-010303-084664294e8c-dirty.json`, n=5):

| workload | stock | jr-nocache | justrust | instr jr/stock |
|---|---:|---:|---:|---:|
| reference-loop | 10.28s | 10.40s | 11.56s | 1.03 |
| body-check | 2.61s | 3.02s (slower) | 3.05s (slower) | 1.05-1.06 |
| first-error | 2.12s | 2.60s | 2.38s | 1.06 |
| deep-edit | 16.50s | 19.26s | 23.99s | 1.04 |
| noop-test | 0.52s | 0.55s | 0.56s | 1.06 |
| small-crate | 2.15s | 2.25s | 2.27s | 1.02 |
| cold-check | 79.05s | 81.71s | **19.17s (4.1x)** | 0.13 |

- **The only win over stock today is caching on cold builds (4.1x).**
  Without caches justrust does 2-6% *more* work than stock on every warm
  workload. Geomean of warm + cold: jr-nocache 0.91x, justrust 1.08x.
- Where the extra work is (follow-ups `versus-20261011-010853-*`,
  `versus-20261011-011620-*`, `versus-20261011-012455-*`):
  - Not the toolchain: `JUSTRUST_TOOLCHAIN=system` has the same 1.05-1.06
    instruction ratio as the pinned nightly.
  - Not feature unification: `JUSTRUST_UNIFY_FEATURES=0` is identical.
  - `-Ztime-passes` on local crates (pinned nightly default) costs about
    3 points: body-check 1.06 -> 1.03 with `JUSTRUST_PASSES=0`.
  - The rest is fixed per-invocation overhead: on a no-op `test`,
    `JUSTRUST_DISABLE=1` is 2.230G instructions (same as stock), a
    recorded run is 2.353G (+0.12G, +5%, +0.05-0.1s wall): the rustc
    shim per unit, the stderr pipe threads, summary writing.
- The incremental loop is a regression we pay on every agent call:
  making the recording path free (and time-passes off by default, or
  sampled) is worth 3-6% on every warm workload before any fork lands.

## 27. jr-rustc build config: release and PGO vs the official nightly (2026-10-11)

Same rustc commit on every side (jr-rustc at the pinned nightly's commit,
CI LLVM, which upstream already ships PGO+BOLT optimized). `bench/versus.py`
with every justrust cache off and `RUSTC` pointed at each compiler; `instr`
is user-space instructions retired by the whole build tree relative to stock
(system rustc 1.98.1), the nearly noise-free column.

| compiler | reference-loop | instr | body-check | instr |
|---|---:|---:|---:|---:|
| official nightly (PGO+BOLT) | 9.13 s | 0.95 | 2.75 s | 0.95 |
| jr-rustc release (thin LTO, cgu 1) | 9.12 s | 1.05 | 2.75 s | 1.12 |
| jr-rustc + PGO (trained on our workloads) | 9.27 s | 1.00 | 2.90 s | 1.06 |

Results: `versus-20261011-021709-bde782c2cedb-dirty.json` (release vs
nightly) and `versus-20261011-024221-bde782c2cedb-dirty.json` (all three).
Wall times overlap within noise. Instructions show the real gap: release
does 10-18% more work than the official build, PGO closes about half of it
(5-11% left). The earlier 31% (finding in `bench/FACTORS.md`, jr-rustc row)
was the dev build (incremental, 256 CGUs).

PGO for a stage 1 compiler needed a bootstrap patch (upstream applies PGO
and BOLT link flags only to a stage 2 dist). Training (`pgo-train.sh` in
jr-rustc): cold and incremental check and test of the Desktop UI crate plus
cold builds of justrust, 407 s, one merged 45 MB profile. The rest of the
gap is BOLT on librustc_driver, which the official build has (its driver
carries `.bolt.org.text`), next: `bolt.sh` in jr-rustc.

Then BOLT (`bolt.sh`, llvm-bolt built from the in-tree LLVM, opt-dist's
flags, instrumentation mode since there is no `perf` here) and frame
pointers off (the `compiler` bootstrap profile turns them on, the official
driver has none), with PGO and BOLT retrained on that build:

| compiler | reference-loop | instr | body-check | instr | first-error | instr |
|---|---:|---:|---:|---:|---:|---:|
| official nightly | 5.33 s | 0.86 | 1.73 s | 0.93 | 1.32 s | 0.93 |
| jr-rustc PGO+BOLT, frame pointers on | 6.97 s* | 0.99 | 2.11 s* | 1.04 | 1.89 s* | 1.05 |
| jr-rustc PGO+BOLT, frame pointers off | 5.56 s | 0.90 | 2.04 s | 1.03 | 1.69 s | 1.03 |

(*) from the earlier run `versus-20261011-040904-d150dca4c0c1.json`, whose
nightly measured 0.94/0.93/0.93: same ratios, different machine state.
Final: `versus-20261011-063822-7a10e0d35fa5-dirty.json`. BOLT relaid out 61%
of profiled functions but moved instructions by under 1 point, and dropping
frame pointers by about 1. The fork still retires 4-10% more instructions
than the official build. Wall ranges overlap on every workload.

Not yet tested: the official compiler is a stage 2 build (the compiler is
compiled by a stage 1 compiler of the same source, LLVM 23.1.3), ours is
compiled by stage 0 (1.100 beta, LLVM 23.1.1). That is the largest
remaining difference in how the two are built.

Pitfalls: a wait loop `while pgrep -f "bench/versus.py"` matches its own
command line and never exits. A concurrent `suite.py` run injected its
type-error edit into a `versus.py` run on the same snapshot: every bench
script now takes an exclusive snapshot lock (`bench/README.md`).

## 28. First type error in 0.09 s: a resident rust-analyzer (2026-10-11)

Goal (`docs/toolchain.md`): the first type error in under 0.2 s. New
`src/early.rs`, opt-in with `JUSTRUST_EARLY=1`. A daemon per workspace
(`justrust __early-daemon`, socket in `~/.justrust/early/<hash>/`) runs
the managed toolchain's rust-analyzer (`rust-analyzer-preview` of the
pinned nightly, added on first use by `toolchain::add_component`, with
its own target dir). When an agent build starts, the recorder asks the
daemon in a side thread. The daemon stats the workspace and path-dependency
sources (780 files, a few ms), pushes changed files to rust-analyzer,
pulls diagnostics for them, and keeps only severity-error diagnostics that
carry a rustc code. They are printed as one block marked early, and cargo
keeps running and printing its own errors unchanged. After the build,
`summary.early` scores them against rustc's errors (matched, early only,
rustc only), and the report adds an `early:` line that flags anything
rustc did not confirm. The cargo command, env and output are untouched
(fail open: no daemon or not ready means no early output). `justrust
early start|stop|status`.

**Latency** (`bench/suite.py`, new `first_error` column: seconds until the
first `error` line on the agent's stderr; new scenario
`desktop-type-error-early`, daemon started untimed). Run
`20261011-045152-5cea93061a6c-dirty.json`, n=6, a busy machine (all
samples noisy):

| scenario | wall median | first error median | range |
|---|---:|---:|---:|
| desktop-type-error (rustc) | 3.08 s | 2.96 s | 1.76-4.04 s |
| desktop-type-error-early | 2.46 s | **0.09 s** | 0.08-0.11 s |

Confirmation on a quiet machine with the final defaults (experimental
diagnostics on, inference-limit filter), `20261011-early-final.json`, n=6,
all samples quiet: first error **1.83 s -> 0.08 s** (0.07-0.09), wall
1.96 s vs 2.02 s (within noise: early adds no wall time).

Of the 0.09 s, the daemon's answer arrives 0.06-0.085 s after the run
starts (`summary.early.secs`). The rest is justrust startup and the
toolchain pin. The spike (`bench/ra_spike.py`) measured a didChange plus
pull at 115-130 ms per edit on the same crate. The first pull after a
load takes 0.5-1 s. Costs: about 50 s to load the Desktop workspace cold
(15-28 s with rust-analyzer's own target dir warm), 4.7 GB RSS resident,
idle exit after 3 h (`JUSTRUST_EARLY_IDLE_SECS`).

**Accuracy** (`bench/early_accuracy.py`, untimed): a clean sweep pulls
every one of the 278 workspace files on pristine sources, which rustc
accepts. A corpus of 19 one-error edits of the kinds agents make, each
compared with a real `justrust check -p jcode-desktop-ui`.

| config | clean-sweep FP | first rustc error found | FP in mutations | result |
|---|---:|---:|---:|---|
| stable diagnostics only | 0 | 7/19 | 0 | `early-accuracy-20261011-051136.json` |
| + experimental, unfiltered | 3 | 13/19 | 57 | `...-051952-experimental.json` |
| + experimental, minus E0282 / `{unknown}` (shipped) | **0** | **13/19** | **0** | `early-accuracy-20261011-052951.json` |

- The only false positives were rust-analyzer's own inference limits:
  three `E0282 type annotations needed` in `panel_gmail_read_card.rs`,
  on `assert_eq!` over a type it could not compute (`{unknown}`).
  Dropping E0282 and any message mentioning `{unknown}` removed all of
  them and cost no true positive.
- Found early: E0308 (5 shapes), E0599, E0425, E0432, E0609 (reported as
  E0559 on the same line), E0061, E0063, E0277, and an error in the
  sibling dependency crate `jcode-core`.
- Missed, by design of rust-analyzer: borrow check (E0382, E0499,
  E0384), match exhaustiveness (E0004), `format!` argument checks, syntax
  errors (rust-analyzer reports them without a rustc code, so they are
  filtered), and errors at *callers* of a changed signature in other
  files (E0061 and E0593: only the changed files are pulled).
- So rust-analyzer finds 68% of first errors at 0.09 s, with no wrong
  ones in this corpus. The rest still come from cargo at 1.7-3 s. Still
  opt-in: 4.7 GB per workspace, and the corpus is small.

Next: pull reverse dependencies of a changed signature (callers), map
rust-analyzer's syntax errors onto a code, and start the daemon
automatically for workspaces with recent agent builds. A resident rustc
(phase 4) would cover borrowck and give rustc's exact diagnostics.

## 29. Early diagnostics: callers, syntax errors, auto-start; recall 13 -> 14 of 19 (2026-10-11)

Follow-ups to finding 28 (`src/early.rs`), measured with
`bench/early_accuracy.py` on a private copy of the pinned Desktop
snapshot (`EARLY_ACCURACY_ROOT`, so the shared snapshot lock stays free;
same commits and fixup as `bench/suite.py`).

- **Callers of a changed signature.** The daemon keeps the text of every
  source file. On each query it diffs the changed files: a changed line
  that declares an item (`fn`, `struct`, `enum`, `trait`, `type`,
  `const`, `static`), or a changed parameter line of a multi-line `fn`
  signature, marks that item's name. Body edits mark nothing, so they cost
  no extra pulls. Files that mention a marked name as a whole word are
  queued, module-qualified mentions first (`effort::label` for a change in
  `effort.rs`), then by path proximity, 24 at most. If the edited files
  themselves are clean, the client asks for them in a second `callers`
  query, 4 files per round, stopping at the first round with errors. The
  agent sees a second early block (`in files using a changed signature`).
  - First version: every file mentioning `label` (81 files), nearest first,
    all at once: 4.3-5.1 s, and the right file was not in the first 24
    (`early-accuracy-20261011-065632.json`). Module-qualified ranking plus
    batches: the caller's E0061 at **0.59 s** (`...-070028.json`).
  - rust-analyzer reports `fn label(&str, u8): FnOnce<(&str,)>` not
    satisfied (E0277) on the line before rustc's E0593. It was the only
    false positive this added and is now filtered.
- **Syntax errors.** rust-analyzer's `syntax-error` diagnostics (no rustc
  code) are now reported as plain `error:` lines, the first per line only.
  When a file does not parse, its type errors are dropped (they are
  artifacts of the parser's recovery).
- **Auto-start** (`JUSTRUST_EARLY=auto`): after an agent build, start the
  workspace's daemon in the background when it had 3 or more agent builds
  in the last hour and at least 8 GB of memory is available. Auto mode only
  queries running daemons and never starts one on the critical path.
  Checked end to end on a scratch crate in a scratch `JUSTRUST_HOME`: no
  daemon after builds 1-2, started after build 3, the next failing build
  printed `early errors ... after 0.01s`, confirmed by rustc. Still opt-in.

| corpus (19 mutations) | first error found | false positives | clean sweep FP (278 files) | result |
|---|---:|---:|---:|---|
| finding 28 (shipped) | 13/19 | 0 | 0 | `early-accuracy-20261011-052951.json` |
| + callers + syntax errors | **14/19** | **0** | **0** | `early-accuracy-20261011-070028.json` |

The remaining five misses are not reported by rust-analyzer at all
(checked with `bench/early_probe.py` and `rust-analyzer diagnostics` on a
3-line crate): use after move (E0382), two `&mut` borrows (E0499),
assigning an immutable binding (E0384), a `match` on `&str` missing its
catch-all (E0004; rust-analyzer only checks exhaustiveness for enums and
bools), and `format!("{}")` with no argument. This rust-analyzer build
runs no MIR borrow check diagnostics, even with experimental diagnostics
on. So 17/19 is out of reach with rust-analyzer alone. Borrowck and these
lints need rustc itself: a resident rustc (phase 4, jr-rustc), or a fast
`rustc -Zno-codegen` of the one edited crate, which is about 1.3 s on the
UI crate today (finding 28's `front` column). Auto stays opt-in.

## 30. Prebuilt dependency store: portable keys and a cold Desktop check from an empty cache (2026-10-11)

Code: depcache schemes 6 and 7 (3d48354, 3c5ac34), `src/depcache_prebuilt.rs`
(`JUSTRUST_PREBUILT`, `justrust cache --publish`), suite scenario
`desktop-cold-check-prebuilt`, S3 store infra and the prebuild script in
justrust-cloud (96e266f). Docs: `docs/c-sysroot.md`.

### Keys were not portable across users

`bench/prebuilt-portability.sh` builds the same snapshot under two cargo
home paths (same registry, private depcaches) and compares per-crate base
keys from `JUSTRUST_DEPCACHE_EXPLAIN`:

| scheme | justrust snapshot f97fcc2, 56 dep units | why |
|---|---:|---|
| 5 | 32/56 equal | registry source paths are written into rmeta, so every crate with a dependency got a different extern hash per cargo home |
| 6: `--remap-path-prefix=<cargo home>=/justrust-cargo-home` | 56/56, outputs byte-identical, no-op rebuild 0 compiled | |
| 6, Desktop, publisher with a symlinked registry | **0/833** | rustc's cwd is the resolved registry path, normalized as a literal path in the key |
| 7: resolved registry/git paths normalized like the cargo home | 808/833 hit (below) | |

### desktop-cold-check from an empty local cache

`bench/suite.py run desktop-cold-check-prebuilt desktop-cold-check
desktop-cold-check-nocache -n 3 --toolchain nightly-2026-10-08`, private
snapshot copy (`JUSTRUST_BENCH_WORK`), under the bench lock, laptop busy
(other processes 6 to 10 cores). Result
`bench/results/20261011-074513-86681282a9a0-dirty-nightly-2026-10-08.json`.
The prebuilt store was published once (865 bundles, 0.30 GB) from a
different cargo home path and target dir, and served over local HTTP. Each
measured run starts with a fresh target dir and an **empty** local depcache.

| scenario | median (min-max) | rust units compiled | dep hits | build scripts replayed |
|---|---:|---:|---:|---:|
| nocache (every justrust cache off) | 65.6 s (64.8-66.8) | 887 | 0 | 0 |
| **prebuilt, empty local cache** | **50.8 s (50.5-60.7)** | 79 | 808 (prebuilt) | 0 |
| warm local depcache (the floor) | 14.6 s (14.3-15.3) | 54 | 833 | 45 |

Run 20261011-073713067-2892610404 (prebuilt): the 808 hits took 57 s of
summed restore wall (max 1.1 s, fetch + verify + unpack), saving 882 s of
rustc. What is left:

- **Build scripts: none replayed.** `publish` skips build-script entries
  because their key includes the host package-db stamp. aws-lc-sys 12.8 s,
  libsqlite3-sys 7.6 s, ring 4.8 s, onig_sys 2.3 s ran in this build and
  sit on the critical path.
- **25 Rust units missed**, all downstream of the C chain (aws-lc-rs,
  ring, rustls, aws-sdk-*, reqwest, azure_*): 30 s of rustc, aws_sdk_bedrock
  alone 11 s. Their keys depend on build-script outputs (aws-lc/ring static
  libs) that differed between the publisher's and the consumer's runs.
- The local crates (jcode_base 10.5 s, jcode_desktop_ui 5.5 s) are in
  both the floor and this run.

Readings: the Rust half of a store works across cargo homes (808/833
restored, -23% wall versus nocache). Closing the remaining 36 s to the 14.6
s floor needs the store to carry build-script runs and those runs to be
host-independent, which is the vendored C toolchain (`docs/c-sysroot.md`,
layers 1 and 2). Not measured yet: a real network store (S3 over the
internet), and other seed sets than the Desktop lockfile.

### Follow-up: why the 25 missed, scheme 8 (2026-10-11)

Probe: two cold Desktop builds without a store, consumer cargo home
(`~/.cargo`) vs publisher home (a dir whose `registry`/`git` symlink to
`~/.cargo`), then compare the C archives and every dependency key.

| change | ring archive | aws-lc archive | equal dep keys |
|---|---|---|---:|
| scheme 7 (finding above) | differs | differs | 808/833 |
| + `-ffile-prefix-map` in CFLAGS for registry build scripts | **identical** | differs | 740/833 |
| + same `--remap-path-prefix` set on every machine (scheme 8) + per-binary rustc shim | identical | differs | **807/833** |

Three causes, all fixed in the working tree:

- **C objects embed the source path.** ring and sqlite compiled by cc put
  the registry path (via the publisher's symlinked home) into DWARF line
  tables, so the static archive bytes, and every dependent key, differed.
  `-ffile-prefix-map` for the cargo home, its resolved registry/git dirs and
  the target dir makes ring and sqlite byte-identical.
- **rustc folds matching remappings into the SVH.** With identical source
  bytes, rustix's rmeta differed only in its 16-byte hash because one side
  passed one matching `--remap-path-prefix` and the other two (replayed by
  hand: home only `369b45b0`, home+registry `98962ae1`). 16 crates were
  roots like this (rustix, alsa, png, onig, rusqlite, ...). Scheme 8 passes
  the same set (cargo home, resolved registry, resolved git, target dir) on
  every machine, whether or not anything is symlinked.
- **The bench was measuring a mix of binaries.** `~/.justrust/bin/shim/rustc`
  is a single symlink every justrust build repoints, so concurrent builds by
  other agents' binaries ran their shims inside my builds (the 740 and an
  intermediate 444 were partly that). The shim is now per binary
  (`bin/shim/<hash>/rustc`); a build by another binary on the same target
  dir stays fresh (0 compiled, 48 fresh, no Dirty).

Remaining 26 keys: aws-lc's archive. All 365 objects have identical
content, but their names differ: cc names objects
`<hash of dirname relative to CARGO_MANIFEST_DIR>-<file>.o` with std's
`DefaultHasher`, and the dirname is relative only when it starts with the
manifest dir spelling, which differs between a symlinked and a plain home
for aws-lc's generated sources. The next step is to key the `--extern`/native
lib inputs by archive member content rather than names, or to run registry
build scripts with `CARGO_MANIFEST_DIR` resolved, then make build-script
runs publishable (see `docs/c-sysroot.md`).

## 31. jr-rustc stage 2 vs stage 1 (2026-10-11)

Same release config (thin LTO, cgu 1, no PGO/BOLT, frame pointers off),
compiler compiled by stage 0 (1.100 beta) vs by our own stage 1 (as the
official dist is). Instructions relative to stock, run
`versus-20261011-074900-608b369adb66-dirty.json` (walls are noisy in this
run: other sessions' benchmarks queued on the lock and load was high, so
only the instruction column is usable):

| compiler | reference-loop | body-check | first-error |
|---|---:|---:|---:|
| official nightly | 0.86 | 0.94 | 0.93 |
| release, stage 1 | 0.97 | 1.11 | 1.10 |
| release, stage 2 | 0.93 | 1.05 | 1.05 |

Stage 2 removes 4-6 points, about as much as PGO did on stage 1. The build
takes 24 min instead of 6.5. Next: PGO+BOLT on stage 2 (`JR_STAGE=2` in
jr-rustc's build-jr.sh, pgo-train.sh, bolt.sh).

## 32. Pinned-nightly flags on the reference loop: none ship (2026-10-11)

Phase 2 of `docs/toolchain.md`: each flag measured alone with
`bench/factors.py` (new slow factors, both sides on the pinned nightly,
the flag injected through cargo `--config` so manifests stay untouched).
Results `bench/results/factors-20261011-045132-nightly-flags-a.json` (a)
and `factors-20261011-051155-nightly-flags-b.json` (b, `--quiet-wait 120`,
0-1 noisy samples per side). n=6 per side, medians with min-max.

| factor | workload | on | off | saved | verdict |
|---|---|---:|---:|---:|---|
| `-Zcache-proc-macros` (UI crate), b | desktop-test-edit | 5.54 s (5.1-5.7) | 5.79 s (5.6-6.7) | +4% | within noise |
| same, a (all samples noisy, 9-15 other cores) | desktop-test-edit | 14.49 s | 12.57 s | -15% | noise |
| same | desktop-body-check | 1.60 s (1.5-1.7) | 1.55 s (1.3-1.9) | -3% | within noise |
| `-Zshare-generics=y` on every dep | desktop-test-edit | 5.39 s (4.6-5.7) | 5.49 s (5.1-6.1) | +2% | within noise |
| `-Zhint-mostly-unused` on the 10 biggest deps | desktop-test-edit | 5.59 s (5.0-6.1) | 5.63 s (5.2-6.0) | +1% | within noise |
| same, cold `test --no-run`, depcache off (n=2) | desktop-cold-test-build | 114.7 s | 114.6 s | 0% | within noise |
| `-Zthreads=16` vs 8 (local crates) | desktop-test-edit | 6.15 s (5.7-6.4) | 5.61 s (5.4-5.9) | -10% | within noise, worse |
| `-Zthreads=4` vs 8 | desktop-test-edit | 5.42 s (5.3-6.2) | 5.38 s (5.1-5.7) | -1% | within noise |

Why, from the per-pass split of the UI crate (medians, run b):

- **cache-proc-macros**: `macro_expand_crate` 1.87 -> 1.59 s (-0.28 s),
  the only pass that moved. The flag caches *derive* macros only; the UI
  crate has 190 derives but 724 `#[gpui::test]` attributes and 1309
  `div()` builder calls, which it does not touch. Most of the 2.7 s
  "macro expansion" in `docs/toolchain.md` is not derive output, so this
  flag cannot reach the goal. Also flagged "potentially unsound" by
  rustc. Not shipped. A cache for attribute proc macros (jr-rustc) is the
  real target.
- **share-generics on deps**: `monomorphization_collector_graph_walk`
  0.37 -> 0.30 s, codegen 0.75 -> 0.71 s, about 0.1 s total. The opt-level 2
  deps (gpui, wgpu, text stack) default to not sharing; turning it on
  needs a full dependency rebuild and would change every depcache key for
  a 2% median move. Not shipped.
- **hint-mostly-unused**: works as designed on the hinted crates (cold:
  gpui 42.8 -> 24.6 s, wgpu-core 22.5 -> 6.9 s) but the codegen moves to
  their users (aws-sdk-bedrock 12.0 -> 16.2 s, jcode-desktop-ui 14.6 ->
  17.8 s), and the cold build is critical-path bound, so wall is
  unchanged at 114.7 s. Warm loop unchanged. Not shipped; with the
  depcache cold builds rarely compile these crates anyway.
- **-Zthreads**: 8 (Desktop's wrapper default) stays. 16 is worse
  (macro_expand 1.80 -> 1.88 s, more sync overhead), 4 is the same.

Bottom line: no pinned-nightly flag moves the warm reference loop beyond
noise; no justrust default changes. The loop's 5.5 s here is frontend
(macro expansion, resolve) plus incremental persist, which only a
resident compiler or attribute-macro caching in jr-rustc will cut. The
factors stay in `bench/factors.py` (slow) to rerun after a toolchain bump.
Pitfall: an unlocked `versus.py` run by another agent edited the shared
snapshot during the first attempt (type error injected into a test-loop
sample); all timed runs must hold `/tmp/justrust-bench.lock`.

## 33. Benchmarking the remote route from the agent's seat (2026-10-11)

New: `bench/versus.py --remote` adds a `remote` stack (`JUSTRUST_REMOTE=force`
from the laptop, wall includes sync and ssh; a local fallback is an error),
and `bench/on-remote.sh` runs any benchmark *on* the build machine
(c7i.8xlarge, Xeon 8488C, 32 threads). Laptop: Core Ultra X9 388H, 16
threads. Same pinned Desktop snapshot, warm target dirs on both sides.

The first run (`versus-20261011-081038-69d8c04310ce-dirty.json`) said remote
was 0.39x local overall. Three bugs, all fixed:

| bug | effect | fix |
|---|---|---|
| a workspace outside git was mirrored crate dir by crate dir, so `jcode-build-meta`'s `rerun-if-changed` root `Cargo.toml` did not exist remotely | its build script reran every call and 8 local crates recompiled: no-op `test` 11.4 s remote vs 0.5 s local | mirror the whole workspace when a path dependency is not in a git repo; take over root-created dirs inside a widened root |
| the cached IP was never refreshed after the machine restarted (new public IP every start) | every remote call timed out, then fell back | `describe()` keeps `~/.justrust/remote/ip` equal to AWS's |
| the machine lacked `libxkbcommon-x11`, `libX11`, `dbus`, `EGL` | every Desktop binary or bin-test build failed to link there | provisioning installs them |

Also: the machine linked with mold where the laptop uses wild (link pass
1.26 s vs 0.25 s on the reference loop); justrust now installs wild there
whenever it links locally. Routing skipped any run with a pinned run id,
so a benchmark could never take the remote path; the remote agent now
marks its builds explicitly (`JUSTRUST_ON_REMOTE_AGENT`).

After the fixes (`versus-20261011-084326-26e91bc15a8f-dirty.json`, n=5,
and `versus-20261011-085118-bd4ede273a6d-dirty.json` with wild on both):

| workload | justrust local | remote | remote vs local |
|---|---:|---:|---:|
| reference loop (test, 1 crate) | 7.07 s | 8.35 s | 0.85x |
| deep edit (16 crates, test) | 12.59 s | 16.44 s | 0.77x |
| body check | 2.06 s | 2.37 s | 0.87x |
| first error | 1.60 s | 2.03 s | 0.88x |
| no-op test | 0.50 s | 0.56 s | 0.82x |

Sync and ssh are now small (40 to 100 ms sync, ~0.1 s fixed). The rest is
the machine: per-phase, the UI crate's frontend is 4.1 to 4.3 s remote vs
2.9 to 3.6 s local, codegen 1.2 to 1.5 s vs 0.8 to 1.4 s. One rustc thread
on the Xeon 8488C (spot, shared host) is slower than a laptop P-core, and
warm incremental loops are single-crate, nearly single-threaded work. The
machine's 32 threads only pay where parallelism exists: cold and
multi-crate builds (finding 25: cold `check -p jcode-tui` 40.6 s remote)
and big test suites. Routing already decides per command from measured
walls; these numbers say warm single-crate loops should stay local on this
instance type. For warm loops a high-clock instance (c7i is 3.2 GHz
all-core; z1d/m7i-flex/c8g are worth measuring with
`bench/on-remote.sh`) is the lever, not more cores.

The synthetic workload (`bench/versus.py --synth 1`) is running on the
machine with `bench/on-remote.sh`. One point there (`test -- reload_config`
in jcode-desktop) fails its pre-build with `CARGO_BIN_EXE_jcode-desktop`
undefined for the `version` integration test; the same command builds
locally, so it is a state left on the machine by the builds that failed to
link before the library fix, not yet explained.

## 34. Test binaries in parallel, likely failures first (2026-10-11)

Mined 1359 recorded `test` runs: cargo runs test binaries one after
another, and small binaries queue behind each other (Desktop
`test --workspace`, 11 binaries: ~5 s per run spent behind the longest
one; `jcode -p jcode-sdk`: 10-12 s). Within one binary, libtest already
uses every core, so the remaining headroom inside a binary was small
(10.5 s over 707 s of binary wall); what agents wait on there is when the
failure they caused shows up. 51 of 196 failing runs failed again on a
test that had failed in the previous run of the same workspace.

What changed (`src/test_pool.rs`, commits 586c035, bf56d97, 29f4c71): in
agent mode the cargo runner (`justrust __test-runner --pool=<sock>`) hands
each libtest binary to the recorder and exits 0, so cargo moves on. The
recorder runs up to ncpu/4 (2-8) binaries at once, failed-last-time and
longest first, and prints each binary's output as one block when it ends.
With history (`~/.justrust/tests/<cwd hash>.json`: per-test times over
0.2 s, tests that failed and have not passed since, last start), one
binary runs as several processes by exact name from its own `--list`
with the user's filters applied: failed-last-time plus tests in modules
whose files changed since the binary last ran, each test that alone is
over 25% of the binary's recorded time (max 4), and the rest. Every
selected test runs exactly once; "filtered out" counts are adjusted.
A failure next to other processes is rerun by exact name with one
thread: reproduced, it is reported at once; otherwise the process is run
alone with nothing else running and that result is reported, with a note
naming the tests that failed only in parallel. Doc tests, benchmarks,
`--nocapture`, `--test-threads 1`, `--format`, unknown libtest flags:
run by cargo as before. `JUSTRUST_TEST_PARALLEL=0` turns it off.

Results, `bench/suite.py`, off (`JUSTRUST_TEST_PARALLEL=0`) vs on, n=3,
round 2 on a quiet machine (other cores 0.6-3.7; round 1 had 6-15 other
cores and is noise, files `testpool-*-1.json` kept):

| scenario | off | on | |
|---|---:|---:|---|
| justrust-body-test (full `test`, 4 binaries) | 2.57 (2.25-2.86) | 1.76 (1.69-1.76) | -31%, faster; test phase 1.07 -> 0.32 s |
| desktop-workspace-test (no edit, 11 binaries) | 13.57 (13.30-17.89) | 8.66 (8.61-15.33) | -36%, ranges touch; test phase 13.0 -> 8.2 s |
| desktop-ui-full-test-fail: first failing test on screen | 14.43 (14.23-14.57) | 6.99 (6.37-8.56) | -52%, disjoint |
| desktop-ui-full-test-fail: wall | 14.60 (14.40-14.73) | 22.04 (13.83-26.46) | slower, see below |
| desktop-test-edit (reference loop, 1 binary, filter) | 6.04 (5.99-7.02) | 7.16 (6.83-7.25) | +1.1 s, test phase 0.04 -> 0.004 s; the difference is compile (5.6 -> 6.6 s), not the pool |

Files `bench/results/testpool-off-2.json`, `testpool-on-2.json`. Run ids
for the failing scenario: 20261011-085222007-4038215624,
20261011-085248489-4038215378, 20261011-085302331-4038215804.

The failing-scenario wall is longer because Desktop has two flaky tests
(`the_hint_chip_uses_only_spare_tab_space`, which also failed 7 times in
plain serial runs since 10-07, and `bundled_github_applet_renders_valid_documents`,
which fails only next to a selection test in the same process). When one
of them fails next to other processes, the pool confirms it alone: a
whole extra run of the 1290-test binary (+8-12 s). The agent already
saw the real failure at 6-8 s. Off, those flaky failures are just
reported as failures; on, the report names them as passing alone.

Remaining: the confirmation run could rerun only the flaky test alone
instead of the whole binary when the rest passed; the reference loop
(`-- fps_counter`, 3 tests) gets nothing from the pool and its +1 s here
sits in compile, rerun with more samples to rule out a pool cost.

Stage 2 with PGO+BOLT (`JR_STAGE=2`, needs `build.profiler = true` so the
stage 1 std has `profiler_builtins`), run
`versus-20261011-092245-17daf818e79f-dirty.json`, instructions relative to
stock:

| compiler | reference-loop | body-check | first-error |
|---|---:|---:|---:|
| official nightly | 0.86 | 0.94 | 0.93 |
| jr-rustc stage 1 PGO+BOLT | 0.90 | 1.03 | 1.03 |
| jr-rustc stage 2 PGO+BOLT | 0.89 | 1.01 | 1.01 |

Stage 2 adds only 1-2 points on top of PGO+BOLT (it overlaps with what PGO
already recovers). The best fork build is now +3.5% instructions on the
reference loop and +7-9% on check and first error vs the official build;
wall ranges overlap everywhere. Every config difference against the
official dist that we know of is now matched except the training set
(rustc-perf vs our own workloads) and the toolchain that builds the C/C++
parts. Returns per step are now 1-2 points for an hour of build each, so
this is where we stop chasing parity: fork changes are measured as
`jr-rustc stage 2 PGO+BOLT` with and without the change, never against the
official nightly. `bench/factors.py` factor `jr-rustc` defaults to
`build/jr/pgo-bolt-stage2`.

## 35. Hot patching test binaries (phase 5 prototype): works on Desktop, saves the link, ~0.35 s (2026-10-11)

Prototype in `bench/hotpatch/` (README there), not wired into justrust,
nothing default-on. Subsecond (Dioxus) patches a long-lived process
through a jump table its code must call through (`subsecond::call`). Test
processes are fresh every run, so jr-hot patches before `main` instead and
needs no source changes:

- jr-hot is the linker. The first link with `JR_HOT=1` is a normal link
  plus `--no-gc-sections` and records each codegen unit's inode. rustc
  hardlinks reused units from the incremental cache, so a new inode *is* the
  set of regenerated units, with no diffing.
- Later links do not relink: `ld.lld -shared` links only the changed units
  (5-7 of 257 for the reference edit) at a fixed address 256 MiB below the
  no-ASLR PIE base. Every other symbol is defined in a generated linker
  script as its address in the base binary (locals matched by their CGU's
  `FILE` symbol group). Writable globals bind to the base's copy.
- jr-hot is also the runner: ASLR off, `LD_PRELOAD` a 140-line C loader
  that maps the image, applies its relocations, writes `jmp rel32` over the
  entry of every replaced function (287-403 per edit), and registers its
  `.eh_frame` so panics unwind through patched code.

Correctness: on a toy crate, function body, string literal, test body,
new private fn, new `#[test]` fn (libtest's test list lives in the
patched unit) and a panic caught through patched code all give the same
results as a full link. A new static falls back. On a copy of the Desktop
snapshot, a test edit that makes `assert_eq!` fail is reported failing,
and fixing it passes again, all patched. What forced fallbacks before each
fix: TLS definitions in a changed unit (harmless, only TLS *accesses* fall
back now), unit-local statics (the patch's fresh copy is the only one
reachable), instances `--gc-sections` had dropped from the base (hence
`--no-gc-sections`), and 1-4 byte functions too small for a jump (skipped
when their bytes are unchanged, which they always were). After that, every
Desktop edit tried (about 25) patched.

Timing, `bench/hotpatch/bench.py` (flock, alternating blocks of 3, 6
samples per mode, machine shared with other agents' benches):

| workload | normal wall | hot wall | link pass normal -> hot | jr-hot patch step |
|---|---:|---:|---:|---:|
| Desktop reference loop (test fn edit) | 8.97 s (5.6-13.1) | 8.02 s (5.2-10.0) | 0.54 -> 0.20 s | 154 ms |
| Desktop non-test body edit | 8.54 s (7.3-22.9) | 7.17 s (6.9-8.7) | 0.60 -> 0.18 s | 133 ms |
| toy crate (3 tests) | 0.103 s | 0.099 s | ~0 | 10 ms |

Results: `bench/results/hotpatch-{desktop-20261011-090939,desktop-body-20261011-091218,toy-20261011-093114}.json`,
run ids in the samples (for example normal 20261011-090740134-36074, hot
20261011-090810250-38553). Wall ranges overlap (noisy shared machine), so
read the link column: justrust's own per-pass split shows the link going
from 0.54-0.60 s to 0.18-0.20 s (that is jr-hot's patch step plus process
overhead), and linker CPU from 1.2-2.9 s to 0. The patch step is 115-195
ms, of which ~120 ms is reading the 640k-symbol table of the 245 MB base
binary; caching the base's symbol index across edits would bring it to
~30 ms (lld on 7 objects is 18-35 ms).

Assessment for the reference loop: hot patching removes the link, about
0.35-0.45 s of ~7.5 s today (5%). The loop is frontend-bound (3.4-3.6 s)
plus codegen and incremental persist (~1.6 s), which patching cannot touch.
So it is not worth shipping ahead of phase 4. Once a resident compiler
brings frontend + persist under a second, the link is the largest fixed
cost left, and patching is then worth 30-50% of the loop. Next steps when
that happens, in order: cache the base symbol index (patch step -> ~30 ms),
wire it into justrust's linker shim and test runner behind
`JUSTRUST_HOTPATCH=1`, rebase automatically after N patches or when the
patch exceeds a size, and verify results with a periodic full link.

## 36. Linking: wild for local crates, dependencies as one dylib (2026-10-11)

The link of the Desktop UI test binary (283 MB, 582 rlibs) runs after
every edit on the reference loop. Two ways to make it cheaper, measured on
the pinned snapshot.

**Link alone, replayed.** The exact link command of one reference-loop
build (args, object files, rustc temp dir) captured and replayed 7 times
per linker on the same inputs (`~/.jcode/scratch/linkcap/replay.py`
method: capture via a `clang` on PATH, relink with `-fuse-ld=mold` or
`--ld-path=wild`):

| layout | mold 2.42 | wild 0.10 | output |
|---|---:|---:|---:|
| stock (all deps static) | 0.92 s (0.60-1.24) | 0.35 s (0.34-0.41) | 283 / 242 MB |
| deps as one dylib | 0.54 s (0.39-0.93) | 0.20 s (0.18-0.23) | 101 / 99 MB |

wild links the same inputs 2.6x faster than mold, and its variance is
far smaller. Both binaries pass the UI suite (1303 tests, 2 runs each,
no failures; one earlier run under load had 1-2 flaky failures with
either linker).

**Shipped: wild for local crates** (commit 7a10e0d, `src/linker.rs`).
`justrust toolchain wild` downloads the pinned release (sha256 checked).
The shim adds `-B ~/.justrust/bin/ld` to local linking units only. gcc
and clang take `ld`/`ld.mold`/`ld.lld` from `-B` before their own,
whatever `-fuse-ld` says, so it covers `cc`, Desktop's
`clang -fuse-ld=mold` wrapper, and rustc's self-contained lld (via
`-Zpre-link-arg` on a pinned nightly). Those names are justrust, which
runs wild and on failure runs the linker that was asked for, with the
same args (fallback tested with a failing stub: lld and mold picked up,
logged to `~/.justrust/linker-fallbacks.jsonl`). Link args are untracked:
toggling `JUSTRUST_LINKER=off` on/off/on over a warm Desktop target dir
compiled 0 units (744 fresh). Dependency units are never touched, so
depcache keys do not move.

On the reference loop (`bench/factors.py run wild mold -n 6`, ABBA,
frozen binary, quiet: 0 noisy samples,
`bench/results/factors-20261011-linker.json`):

| factor | on | off | link pass on/off |
|---|---:|---:|---:|
| wild (vs Desktop's mold) | 7.03 s (6.1-7.8) | 7.03 s (6.3-7.9) | 0.23 / 0.44 s, disjoint |
| mold (vs lld via cc, wild off) | 7.21 s (6.2-7.6) | 8.14 s (6.7-8.4) | 0.46 / 1.26 s |

Suite runs, `JUSTRUST_LINKER=off` then on (sequential, not interleaved,
`bench/results/20261011-linker-{off,wild}.json`): 9.86 s -> 6.96 s
median, `compare` says within noise (off had 4 of 6 noisy samples);
link 0.71 -> 0.22 s.

So wild halves the link pass (-0.2 s, -3% of the loop), which is real
but below what wall time resolves at n=6. It ships because it is free
(no rebuild, fail open) and the link share grows as the rest of the loop
gets faster (hot patching, resident compiler). wild's incremental mode
is not implemented in 0.10 (its README: the end goal, not yet
available), so there is no incremental variant to measure.

**Prototype: deps as one dylib** (not shipped). A generated
`jcode-desktop-deps` crate (`crate-type = ["dylib"]`, `pub extern crate`
of every UI dependency), a dev-dependency of the UI crate, referenced by
`#[cfg(test)] extern crate`. rustc then links the test binary against
`libjcode_desktop_deps.so` (680 MB) instead of 582 rlibs: binary 283 ->
101 MB, link 0.92 -> 0.54 s (mold) and 0.35 -> 0.20 s (wild). On the
loop (ABBA n=5, very noisy machine, 10-15 other cores): wall 14.0 s
stock vs 12.6 s dylib, ranges overlap, link pass 1.11 -> 0.48 s,
codegen 2.40 -> 1.64 s. The saving on top of wild is ~0.15 s per edit,
and it costs a 3.3 s dylib link whenever any dependency changes, a
second copy of every dependency, a manifest change in the project
(or cargo fork work to do it transparently), and `LD_LIBRARY_PATH`
plumbing for running the binary outside cargo. Not worth it at this
size. Worth revisiting only if the link pass becomes a big share again
(for example after hot patching removes codegen), and then in jr-cargo,
not in project manifests.

## 37. Cold Desktop check from an empty cache: 53.3 s -> 14.8 s, build scripts in the prebuilt store (2026-10-11)

Follow-up to 30. Code: d6bc187, 2c0eced (`src/buildscript.rs`
`HERMETIC`, `hermetic_stamp`, `v1/s/` store tier; `src/depcache_prebuilt.rs`
publish). The prebuilt store now carries build-script runs for packages that
compile vendored C and probe nothing else: ring, aws-lc-sys,
libsqlite3-sys (bundled), onig_sys. Their key is the C toolchain (resolved
`cc`/`ar` by content, `cc --version`, which of the package's fallback
pkg-config modules exist) instead of the host package database, and the
package + `build.rs` instead of the script binary (it embeds
`CARGO_MANIFEST_DIR`). Other scripts stay host-keyed and are never
published.

Cross-machine causes fixed on the way (probe: two cold Desktop builds,
plain vs symlinked cargo home, `bench/prebuilt-portability.sh`-style):

| cause | effect | fix |
|---|---|---|
| cc names objects by a hash of the source dir relative to `CARGO_MANIFEST_DIR`; aws-lc builds from its canonicalized dir | aws-lc archive member names (and bytes) differed per cargo home spelling | hermetic scripts run with `CARGO_MANIFEST_DIR` resolved |
| `CARGO_HOME` keyed in script env | set by one side only | not keyed (as for rustc units) |
| registry `rerun-if-changed` files signed by size+mtime under the cargo home path but by content under its resolved path | no hermetic entry matched across homes; only onig_sys (no inputs) hit | resolved registry/git dirs count as cargo home |

Probe result: ring, aws-lc, sqlite archives byte-identical; 4/4 hermetic
script keys and 831/833 rustc keys equal (807 in 30's follow-up; the 2
left are ratex_font_loader and ratex_svg, downstream of
ratex_katex_fonts whose rmeta hash differs: rust-embed folder, not yet
diagnosed).

### Measured

`bench/suite.py run desktop-cold-check-prebuilt desktop-cold-check
[desktop-cold-check-nocache] -n 3 --toolchain nightly-2026-10-08`, private
snapshot copy, under the bench lock. Store republished each time from a
different cargo home (833 bundles 0.31 GB + 4 build-script runs), served
over local HTTP; every prebuilt run starts with a fresh target dir and an
empty local depcache.

| scenario | median (min-max) | rust units compiled | dep hits | scripts replayed | result |
|---|---:|---:|---:|---:|---|
| nocache | 53.3 s (52.6-54.7) | 887 | 0 | 0 | `20261011-093818-3c691739da09-dirty-nightly-2026-10-08.json` |
| prebuilt, finding 30 | 50.8 s (50.5-60.7) | 79 | 808 | 0 | `20261011-074513-86681282a9a0-dirty-nightly-2026-10-08.json` |
| prebuilt, scheme 8 + hermetic keys, before 2c0eced | 20.9 s (20.8-21.1) | 54 | 833 | 1 (onig) | `20261011-093818-...json` |
| **prebuilt, 2c0eced** | **14.8 s (14.7-14.9)** | 54 | 833 | 4 | `20261011-094421-82adf0a496f6-dirty-nightly-2026-10-08.json` |
| warm local depcache (floor) | 12.7 s (12.6-12.7) | 54 | 833 | 45 | same file |

A new machine with only the store is now 2.1 s from the warm floor
(-72% vs nocache). The gap: the other 41 build scripts still run (all
small, the largest unicode-general-category 0.39 s), and aws-lc's 20 MB
OUT_DIR unpacks in 0.54 s. The 54 compiled units are the workspace crates,
same as the floor.

Remaining: a real network store (S3 deploy pending the user), the two
ratex keys, and other projects' seed sets.

## 38. Where a no-op cargo run goes, and jr-cargo with mimalloc (2026-10-11)

Workload: `desktop-noop-test` (Desktop pinned snapshot, warm, 0 units
rebuilt), jr-cargo at c400320c3 (resolve stamp). `CARGO_LOG_PROFILE`
traces of 4 runs through justrust, plus samply at 8 kHz on cargo.

| phase (cargo tracing span) | ms | notes |
|---|---:|---|
| total cargo `main` | 660-790 | busy machine; ~0.3-0.5 s when quiet |
| `resolve_with_previous` (targeted resolve) | 250-300 | ~half is registry index summaries: read `.cache` files (~7% of samples) and parse them |
| `download_accessible` | 180-290 | not downloads: `read_manifest` of 721 unpacked registry crates, one thread (TOML parse 45, deserialize 50, normalize 90, to_real_manifest 55) |
| `compile` (fingerprints, job queue, output replay) | 110-150 | `fingerprint::calculate` ~120 ms incl. mtime stat of dep-info files |
| `load_pkg_lockfile`, workspace, init | ~40 | |
| rustc -vV probes | ~0 | already cached by cargo (`.rustc_info.json`) |

Samples by library: **48% of cargo's samples were in libc**, mostly
glibc malloc/free (manifest, summary and unit-graph allocations) plus
`read`/`statx`. The in-process upper bound for a resident cargo was also
checked: running `cli::main` 8 times in one process (`JR_REPEAT`
experiment, no caches kept between iterations) took the same 0.15-0.55 s
per iteration as a fresh process, so process startup itself is negligible
(`cargo -V` 15 ms). A daemon only wins if it keeps parsed manifests,
index summaries and fingerprints across runs; the state that costs time is
the parsing, not the exec.

Change 1 (jr-cargo 7ad6b3cf8): mimalloc as cargo's global allocator.
Raw cargo `test -p jcode-desktop-ui --lib --no-run`, 10 alternating runs
each under the bench lock (`jr/noop-bench.sh` in jr-cargo):

| binary | median | min-max |
|---|---:|---:|
| stock cargo (nightly-2026-10-08) | 0.366 s | 0.261-0.430 |
| jr-cargo c400320c3 (resolve stamp) | 0.298 s | 0.234-0.591 |
| jr-cargo + mimalloc | **0.257 s** | 0.179-0.343 |

-14% vs the previous fork, -30% vs stock, 0 units rebuilt in any run,
identical output (Fresh lines sorted). Installed as the pinned
toolchain's `jr-cargo`; submodule `toolchain/cargo` bumped to 6f30c42ee.

Next (in progress): parse the 721 registry manifests in parallel while
`Downloads` holds the package cache lock (`download_accessible` 250 ms ->
~70 ms in traces), then cache index summaries / parsed manifests across
runs.

Update: the point-3 failure is explained. Jcode Desktop's rustc wrapper
`scripts/rustc-parallel-frontend` is `#!/bin/sh`. On Debian/Ubuntu `/bin/sh`
is dash, which drops environment variables whose names contain `-` when it
`exec`s rustc, so `CARGO_BIN_EXE_jcode-desktop` (cargo does set it, checked
with `-vv`) never reaches rustc and `tests/version.rs` fails to compile.
Arch's `/bin/sh` is bash, so the laptop never sees it. Same on cargo 1.98.1
and 1.99.0, and with a clean target dir on the machine. Fix belongs in
jcode-desktop (`#!/usr/bin/env bash` for the wrapper, or do not exec
through sh); it affects any Ubuntu machine, including its CI runners if
they build that test with the wrapper.

## 40. Where macro expansion goes on the reference loop (2026-10-11)

jr-rustc stage 2 PGO+BOLT, one-line edit in a `#[test]` of
jcode-desktop-ui, `-Ztime-passes` and `-Zself-profile`: rustc total 5.32 s,
`expand_crate` 1.78 s (the largest pass), codegen 0.91, dep-graph and
result-cache persist 0.63, link 0.45, resolve 0.43. Proc-macro time inside
expansion is 1.01 s over 883 calls, and **0.96 s of it is `#[gpui::test]`
(724 calls)**. Derives are 0.05 s in total (Deserialize 0.026, Action
0.010, Serialize 0.009).

So `-Zcache-proc-macros` (factor `cache-proc-macros`, within noise) could
not help: it caches derive macros only. On every edit rustc re-runs the
`gpui::test` attribute macro on all 724 tests although 723 have identical
input tokens. The fork change: cache attribute proc-macro expansions the
same way upstream caches derives (an incremental query keyed by invocation
and input tokens), worth up to ~0.9 s of the 5.3 s compile.

## 39. jr-cargo: parallel manifest prefetch, cargo-fork factor now helps (2026-10-11)

Follows 38. After resolving, `Downloads` turned 721 `PackageId`s into
`Package`s one at a time, each a full `Cargo.toml` parse (250 ms of a
Desktop no-op). jr-cargo 08fc34c92 parses the already-unpacked registry
manifests on all cores first, while the package cache lock is held, and
`read_package` takes a parked result only for the same path and source.
`.cargo-ok` checks, last-use tracking and checksums stay on the
sequential path. Speculative parses print nothing. `Manifest`/`Package`
moved from `Rc` to `Arc` so they can cross threads. Kill switch
`JUSTRUST_CARGO_PREFETCH=0`. In traces `download_accessible` went from 180-290 ms to ~70 ms.

Raw cargo `test -p jcode-desktop-ui --lib --no-run`, 12 alternating runs,
two rounds under the bench lock (machine at load 12-16, so absolute
numbers are higher than in 38):

| binary | round 1 | round 2 |
|---|---:|---:|
| stock cargo | | 0.746 s (0.73-0.79) |
| jr-cargo mimalloc (6f30c42ee) | 0.558 s (0.51-0.61) | 0.555 s (0.52-0.60) |
| jr-cargo + prefetch (08fc34c92) | **0.471 s (0.44-0.48)** | **0.476 s (0.44-0.63)** |

Output is identical up to the (already nondeterministic) order of Fresh
lines, 0 units rebuilt. Cargo testsuite: 4042 passed. The failures were the 2 known environment ones,
`death::ctrl_c_kills_everyone` (hangs without a tty, killed), and 3
(`lto::cdylib_and_rlib`, `lto::dylib`,
`package_features::other_member_from_current`) that pass on rerun (load).

System 2, agent level (`bench/factors.py run cargo-fork`,
`bench/results/factors-20261011-103622-cargo-fork.json`), through justrust
with the pinned toolchain, 11 on/off interleaved: **0.22 s (0.2-0.3) vs
0.39 s (0.4-0.5) for stock cargo, -43%, verdict helps** (finding 24 had
0.49 vs 0.52 s, within noise).

Remaining in a no-op, from the traces: targeted resolve ~200 ms (index
summaries from `~/.cargo/registry/index/*/.cache`, then the resolver),
fingerprints ~120 ms. Next: a per-workspace cache of the targeted resolve
result keyed like the resolve stamp, which would skip the index entirely
on a no-op.

Fork change (jr-rustc `attr_macro_expansion` query: attribute proc-macro
outputs cached in the incremental session like upstream's derive cache,
under `-Zcache-proc-macros`, `JUSTRUST_RUSTC_ATTR_CACHE=0` to disable).
Correctness on the pinned Desktop snapshot (`bench/attr-cache-check.py`):
an edited `#[gpui::test]` body runs (the test panics as edited), a type
error inside it is reported, restore passes, and a one-line edit expands
`#[gpui::test]` once instead of 724 times.

Effect, same jr-rustc build (stage 2 PGO+BOLT) on every side, caches off,
run `versus-20261011-111710-07262f4deb42-dirty.json`, 8 rounds:

| | reference-loop | instr vs stock | body-check | instr |
|---|---:|---:|---:|---:|
| no proc-macro cache | 5.58 s | 0.93 | 1.93 s | 1.06 |
| upstream derive cache only | 5.74 s | 0.93 | 1.94 s | 1.03 |
| + attribute cache (jr-rustc) | 4.75 s | **0.74** | 1.87 s | 1.03 |

20% fewer instructions on the reference loop (0.93 -> 0.74), wall median
-15% (ranges overlap; the factor run `factors-20261011-105626-07262f4deb42.json`
agrees: 5.42 -> 4.39 s, -19%). Check is unchanged because `check -p`
does not compile the tests. First fork change that moves the loop.
Caveat inherited from the derive cache: a proc macro that reads files or
the environment is not re-run when only those change.

## 41. First synthetic agent workload: justrust 0.81x of stock overall, lost on big test suites (2026-10-11)

`bench/versus.py run --synth 1 --points 40 -n 1` on the build machine
(c7i.8xlarge, run with `bench/on-remote.sh`), result
`bench/results/versus-synth1-20261011-181043-21e77126c6ff-dirty-remote-c7i.8xlarge.json`, report `bench/VERSUS-SYNTH.md`.
40 points drawn from 4,711 real agent build calls (96% map onto the pinned
snapshots); 38 measured on all three stacks. Points 2 and 28 failed on
every stack: the dash/`CARGO_BIN_EXE` wrapper bug of finding 33.

| | points | stock | jr-nocache | justrust | justrust vs stock | instr justrust/stock |
|---|---:|---:|---:|---:|---:|---:|
| **expected wait per call** | 38 | 14.55 s | 18.99 s | 18.00 s | 0.81x | 1.35 |
| check | 9 | 2.19 s | 2.51 s | 2.62 s | 0.83x | 1.05 |
| test-filtered | 10 | 7.03 s | 7.72 s | 8.12 s | 0.87x | 1.20 |
| test (no filter) | 7 | 22.19 s | 34.04 s | 34.08 s | 0.65x | 1.55 |
| test --workspace | 1 | 70.5 s | 140.3 s | 93.8 s | 0.75x | 6.15 |
| build | 6 | 7.26 s | 7.64 s | 7.90 s | 0.92x | 0.92 |
| build --release | 4 | 48.1 s | 49.0 s | 49.5 s | 0.97x | 1.00 |

This is the first benchmark that weights by what agents actually run, and
it disagrees with the hand-picked scenarios: on the warm one-crate loops
justrust is within noise of stock, but most of the expected wait is in
unfiltered test runs of big suites, and there justrust loses badly. Of the
131 s total lost over 38 points, 65 s are three `test -p jcode-tui --lib`
points (2,470 tests): stock 22.6 / 29.8 / 38.3 s, justrust 41.3 / 51.0 /
63.3 s. In justrust's own phase split those runs spend ~17 to 20 s in
`test_run` and another 24 to 26 s in `tail` (after the build, outside test
binaries), with ~1.7x the CPU of stock, while the compile part matches.
`test --workspace` (8,911 tests counted by justrust vs 1,862 by libtest
lines for stock, so the stacks may not run the same set) is the other
large loss. Instructions are 1.35x stock overall, nearly all of it test
execution. The same jcode-tui command was not covered by any named
scenario, which is why this never showed up before.

Next: profile justrust's test runner on `test -p jcode-tui --lib` (what
the 25 s `tail` is, whether the parallel runner or the per-test
confirmation of overlapped failures re-runs tests), make the stacks run the
same test set on `--workspace`, and fix finding 33's Desktop wrapper bug
so points 2 and 28 count. Also: one run per point (`-n 1`) on a spot VM,
so single points are noisy; the per-command means over 7 to 10 points are
the reliable rows.

Acceptance through the real agent path (`justrust test`):

- `bench/attr-cache-accept.py` on the pinned snapshot, jr-rustc stage 2
  PGO+BOLT: the full jcode-desktop-ui lib suite (1313 tests) has the same
  test set with the cache on and off. The only outcome difference in the
  suite run (`changelog_panel::...hot_reload_keeps_position...` in one run,
  `workspace::...the_hint_chip_uses_only_spare_tab_space` in another) also
  fails intermittently with the cache off and passes alone on both sides
  (3 full-suite runs per side: one flaky failure on each side). Edge cases,
  all PASS: a new `#[gpui::test]` appears and passes; a renamed test runs
  under its new name and the old one is gone; edit A, edit B, revert to A
  runs A, B, A; changing only the attribute (`#[gpui::test]` ->
  `#[gpui::test(iterations = 3)]`) re-expands (body runs 1 -> 3 times); an
  invalid attribute argument is reported, and fixing it builds and passes.
- Reference loop on the real `~/jcode-desktop` checkout (AGENTS.md target,
  `justrust test -p jcode-desktop-ui --lib -- fps_counter` after a one-line
  edit in fps_counter.rs, file restored from a backup copy), 8 paired runs,
  alternating order: cache off 5.90 s median (5.08-6.52), on 4.56 s
  (3.60-5.24), **-23%, faster in 8 of 8 pairs**. The UI crate's frontend
  went from 2.69 s to 1.30 s (runs 20261011-120719442-2276316 off,
  20261011-120724754-2276848 on). Every run exited 0.

Pitfall: a shell `exec 9>lock; flock 9` leaks fd 9 into children; an
orphaned `wl-copy` inherited it and held the bench lock for 30 minutes.
`suite.bench_lock` now names the holders when it waits, and jr-rustc's
pgo-train.sh uses `flock -o`.

Update (cause and fix): the 25 s "tail" was the test pool's faithfulness
check. jcode-tui's lib tests have 3 real failures plus a few that fail
only next to other test processes; when the quick by-name rerun did not
reproduce every failure, the pool reran the *whole* 2,500-test binary
alone. Now only the names that passed are rerun alone; those that pass
again are reported as parallel-only (named in a note, not counted), the
rest stay failures, and only a crash with no named failure reruns the
binary. Laptop, warm, `test -p jcode-tui --lib`, 2 runs each under the
bench lock: stock 11.5 / 11.7 s; justrust before 22.7 / 22.6 s; after
11.3 / 11.5 s, same 3 real failures as stock. `JUSTRUST_TEST_PARALLEL=0`
and `JUSTRUST_TEST_TIMES=0` were already at stock speed (11.0 to 11.5 s),
which isolated the pool.

Rerun points (`change: rerun`, justrust 1.22 s vs stock 0.74 s mean): the
gap is mostly noise on the shared spot host, not a fixed cost. On the
machine with nothing else running, no-op `check -p jcode-base` is ~0.29 s
justrust vs ~0.21 s stock (4 of 6 pairs). Laptop, run
`20261011-155942999-1`: 0.355 s wall = 23 ms before cargo starts, 300 ms
cargo (stock alone ~265 ms), 26 ms tail, 7 ms exit, so the fixed overhead
is ~0.1 s. Neither `CARGO_TERM_VERBOSE` nor `RUSTC`=shim moved plain
cargo's no-op CPU time measurably (interleaved, 20 rounds: 354 / 310 /
362 ms medians). The synth run's 0.90 s and 1.28 s rerun walls were all
`cargo startup` with io stalls of 0.5 to 0.8 s in the same runs. A
`--no-run` then filtered test sequence on the laptop matches stock (0.6 to
0.8 s both). Caution: ad hoc timing on the build machine during a
benchmark skews it. Ad hoc runs at 23:00 to 23:05 UTC overlapped synth2
points 8 and 9 (stock 70 s / 30 s, flagged noisy), so rerun those points
before quoting them.

Remote failures of points 2 and 28 fixed in 1c50a2b: justrust makes
`/bin/sh` bash on its own AWS machine (at provisioning and on each
justrust install), checked there: `cargo test --no-run` in jcode-desktop
builds `tests/version.rs`. `bench/versus.py run --synth 1 --only-points
2,28` reruns single points for debugging (nothing saved).

Early synth2 on the machine (justrust 9ab215a, both fixes in, partial at
point 13 of 40, one run each): the former point-2 command (`test --
reload_config`, now listed as point 3) measures on every stack (stock
12.73 s, justrust 12.56 s). `test --workspace` (point 12) is stock 74.5 s,
justrust 59.1 s, jr-nocache 57.7 s, where synth1 had justrust 93.8 s
against stock 70.5 s. Justrust's samples are flagged noisy, so wait for the
full result before quoting it.

Full synth2 (`versus-synth1-20261012-002025-9ab215affbd3-dirty-remote-c7i.8xlarge.json`,
justrust 9ab215a: pool rerun fix + fail-fast + /bin/sh fix): **40/40
points measured** (synth1: 38). The report's headline, justrust 1.14x
stock (16.01 vs 14.10 s), is wrong. Stock's points 7 (`build`, 70.2 s)
and 8 (`check -p jcode-desktop-motion`, 30.0 s) ran while I was timing
by hand on the machine. Without those two: stock 14.22 s,
justrust 14.45 s, **0.98x** (synth1 was 0.81x). Per command against
synth1:

| | synth1 stock / justrust | synth2 stock / justrust |
|---|---:|---:|
| `test -p jcode-tui --lib` (3 points) | 22.6/41.3, 29.8/51.0, 38.3/63.3 | 22.0/22.6, 29.2/30.9, 35.5/37.3 |
| `test --workspace` | 70.5 / 93.8 | 74.5 / 59.1, 11.2 / 7.7 |
| `command: test` (no filter, 7) | 22.19 / 34.08 (0.65x) | 20.62 / 21.80 (0.95x) |
| rerun points (excl. 7, 8; synth2 adds `test --workspace` rerun) | 0.74 / 1.22 | 2.10 / 1.95 |

So the test-suite loss of finding 41 is gone, and what is left is a 2 to
10% overhead on most warm points. Remaining outliers: point 22 (a no-op
filtered test, 0.36 vs 1.43 s) was 1.40 s of `cargo startup` with 1.11 s of
machine-wide io stall in that run. On the idle machine afterwards it is
0.38 s justrust vs 0.35 s stock (3 rounds). I tested one hypothesis, that
the detached `split --refresh-hints` after an edited build slows the next
call: 4 paired rounds with `JUSTRUST_SPLIT_HINTS=0` vs on, refresh stamp
removed, gave 373 to 396 ms either way, so it is not the cause. A clean
rerun of synth with nothing else on the machine is needed before a
headline number is quoted.

Clean synth3 (`versus-synth1-20261012-030041-6e924fb57e90-dirty-remote-c7i.8xlarge.json`,
nothing else on the machine, 40/40 points): **justrust 0.98x of stock**
(13.80 vs 13.55 s per call), jr-nocache 13.51 s. By command: test
--workspace 1.30x, build 0.96x, test 0.95x, build --release 0.95x,
test-filtered 0.90x, check 0.85x. The losses are now a handful of
sub-second calls that took 0.6 to 1 s longer than stock, all spent in
`cargo startup` with 0.5 to 0.8 s of io stall (points 16, 17, 22, 32, 35:
stock 0.44 / 0.97 / 0.36 / 0.41 / 1.54 s, justrust 1.11 / 1.75 / 1.04 /
1.34 / 2.17 s). They did not reproduce by hand, not even with the same
repo switch, output to a file, or an edited-then-restored file with its
old mtime (0.38 to 0.40 s every time), and only sometimes through
`--only-points` (point 22: 1.35, 1.06, then 0.40 s; the jr-nocache stack
too). Their unmeasured pre-builds restore hundreds of depcache artifacts
(~1.2 GB dirty after a jcode-tui pre-build), so the measured call
competes with writeback.

Harness fix: `run.py` now calls `sync()` before the measured call, since
an agent's think time between calls covers the writeback. Every stack
gets the same treatment. With it, the five points with their predecessor
(`--only-points 15,16 16,17 21,22 31,32 34,35`, one run each): justrust
0.48 / 0.92 / 0.40 / 0.46 / 1.57 s vs stock 0.42 / 0.72 / 0.39 / 0.51 /
1.51 s. One jr-nocache sample (point 32, 1.06 s) was still slow, so the
flush removes most of the effect but not all of it. `sync()` changes the
generator-independent harness only, so synth1 to synth3 stay comparable
with each other but not exactly with later runs.

synth4 with the `sync()` (`versus-synth1-20261012-053402-7556b5ad52f7-dirty-remote-c7i.8xlarge.json`,
same AWS c7i, 40/40): **justrust 0.97x of stock** (13.28 vs 12.88 s),
jr-nocache 0.97x. The rerun outliers are gone (rerun points 1.31x
*faster* than stock). Remaining losses, in order of size: release builds
(+4 to 6 s of 63 to 69 s), check 0.83x, type-error points 0.71x. Two
were followed up on the machine:

- Release build (`build --release -p jcode-desktop` after a jcode-base
  edit, 3 rounds): stock 62.4 / 62.1 / 62.5 s, justrust 71.1 / 62.2 /
  61.9 s, `JUSTRUST_SCHED=0` 62.0 / 61.7 / 62.3 s. Same instructions on
  both stacks. No steady gap: one slow justrust round, as in the synth.
  Not pursued further without more rounds.
- Type-error check (`check -p jcode-app-core --tests` with a type error,
  6 rounds): stock 3.67 to 3.75 s, justrust 4.11 to 4.15 s,
  `JUSTRUST_PASSES=0` 3.73 to 3.80 s. **`-Ztime-passes` costs ~0.4 s
  (10%)** on this unit. The shim added it to every local unit compiled
  with `RUSTC_BOOTSTRAP=1`, which Jcode's parallel-frontend wrapper
  sets, and its output only fills the per-phase split in the report.
  Now off by default (`JUSTRUST_PASSES=on` restores it). After: justrust
  3.77 to 3.88 s, opt-in 4.06 to 4.19 s, stock 3.69 to 3.75 s. Flipping
  it compiles 0 units on jcode-desktop-ui (no Dirty).

synth5 with time-passes off (`versus-synth1-20261012-072519-1aabb66382ca-dirty-remote-c7i.8xlarge.json`,
same machine, 40/40): **justrust 1.03x of stock** in expected wait per call
(12.89 vs 13.23 s), jr-nocache 0.98x. The time-passes change shows up where
predicted: type-error points 2.41 -> 1.74 s (stock 1.71), check 2.54 ->
2.19 s (stock 2.12). Read the headline with care: it rests on the two
`test --workspace` points (58.5 vs 73.0 s, 7.0 vs 11.4 s). Dropping the two
best and two worst points gives 0.99x. The per-point geomean is 0.94x, and
justrust is faster on only 9 of 40 points. So on most calls justrust still
costs a few percent: the per-call overhead of finding 26 (~0.05-0.1 s
recording, shim per unit) plus `jcode-tui --lib` test runs ~1.3 s (5%)
slower. Point 7 (no-op `build` on jcode-desktop, 0.38 vs 1.16 s) looked
like the largest relative loss, but it is intermittent. Across synth3 to
synth5 justrust took 0.39, 0.99 and 1.16 s, all in `cargo startup` with io
stall. Rerun 3 times with its predecessor (`--only-points 6,7`) it took
0.39 s (jr-nocache 0.39, stock 0.36), and 3 times by hand 0.38 s. So it
is not a steady cost; the cause of the occasional startup stall is still
open.

The `test -p jcode-tui --lib` gap (machine, warm, 4 rounds each): stock
20.0 to 20.8 s, justrust 20.7 to 22.0 s, `JUSTRUST_TEST_PARALLEL=0` 20.4
to 20.6 s, `JUSTRUST_TEST_TIMES=0` 20.4 to 20.6 s. Timeline of one
justrust run: the "failed last time" group (4 tests) is done at 0.14 s,
the main group (2,482 tests) at 20.07 s, about the same as stock's single
process. Then the pool spends 0.9 s rerunning, alone, the tests that
failed only beside other test processes, before it reports them as
parallel-only. That faithfulness check costs ~1 s only on suites with
order-dependent tests (jcode-tui has several; the failure count varies
from 5 to 14 between runs). It is the price of not reporting a flaky
failure as real, so I left it. Making it cheaper would mean reporting
first and confirming after, which changes what the agent sees.

Before and after, `bench/versus.py compare` synth1 (21e7712) vs synth5
(1aabb66), same seed and machine, the 38 points both measured:

| stack | synth1 | synth5 | wall | instructions |
|---|---:|---:|---:|---:|
| stock | 14.55 s | 13.28 s | -9% | -0.1% |
| jr-nocache | 18.99 s | 13.62 s | -28% | -23.3% |
| justrust | 18.00 s | 13.06 s | **-27%** | **-23.6%** |

Stock's instructions did not move, so its -9% is machine variance. Most
of justrust's gain shows in instructions too (less test work through the
pool, fail-fast, no `-Ztime-passes`), not only in wall time.

## 42. Public prebuilt store on S3: one timeout paused it for the whole build (2026-10-11)

Deployed `infra/prebuilt-store.yaml` (justrust-cloud) as stack
`jcode-justrust-prebuilt-production`, us-east-1. Client URL:
`JUSTRUST_PREBUILT=https://jcode-justrust-prebuilt-production-280196252242.s3.us-east-1.amazonaws.com`.
Checked from outside with curl: anonymous GET under `v1/` 200, outside
`v1/` 403, listing 403, anonymous PUT 403, plain http 403. Filled with
`prebuild/run.sh` (43 seed crates, check/build/test --no-run): 576
bundles, 0.47 GB, 6 build-script runs.

First end-to-end cold `check` of the seed project from another directory,
empty depcache: **18 of 313** dependency units from the store. Every one of
the 313 base keys and manifest entries matched the store, so keys were
fine. The breaker file said `manifest: HTTP 0`: one curl with a 3 s connect
timeout failed while dozens of shims connected at once on a loaded laptop,
and the first failure paused the store for 10 minutes, i.e. the rest of
the build. 200 parallel manifest GETs from the same machine afterwards: all
200, connect up to 2.1 s.

Fix (src/depcache_prebuilt.rs): connect timeout 10 s, one retry for
connection failures and 5xx, and the breaker trips after 5 consecutive
failures (a success resets the count) instead of on the first.

Result, interleaved, fresh target dir and empty depcache each run, machine
busy (load 39-49, other agents' builds):

| run | S3 store | no store |
|---|---:|---:|
| 1 | 43.0 s, 313/313 prebuilt | 70.2 s, 0/313 |
| 2 | 38.6 s, 313/313 prebuilt | 60.1 s, 0/313 |

-38% wall on a cold build of 43 popular crates over the public internet
path to S3, with the CPU mostly taken by other builds (the store run's own
CPU was 32.5 s, it waited 33.9 s for CPU). On a quiet machine the floor is
the restore time; not measured yet.

## 2026-10-11: remote sync dominated a cold `justrust check` (run 20261011-154955613-3707337)

jcode-website fresh worktree, `justrust check` auto-picked remote (cold target, no history).
Remote total 282.1s, of which sync 250.1s; compile itself was 31.1s. Two files over 8 MB were
skipped, so the cost is the long tail of small files (public/benchmarks/data trials, thousands of
JSON transcripts). Local cold check would likely have been ~60-90s. Ideas: send only files the
cargo build can read (crate dirs + build-script inputs), or count the file total before choosing
remote for a cold run.

## 2026-10-11: wild `-B` broke wasm32 links (fixed in 290bded)

`trunk build` of jcode-website failed: `rust-lld: error: unknown argument: -B/home/jeremy/.justrust/bin/ld`.
The shim added the wild `-B` dir to every local linking unit regardless of `--target`.
Now only Linux targets (or no `--target`) get it.

Update (fail-fast, 9ab215a): the pool now honors cargo's fail-fast. Real
Jcode workspace, laptop, warm, under the bench lock: `test --workspace`
stock 76.0 s (1,868 tests, exit 101), justrust 44.5 s (1,792 tests, exit
101, the unstarted binaries named in a note). Before, justrust ran every
binary (8,911 tests). Counts differ slightly because the pool orders
binaries by history (likely failures first), so a different set has
finished when the first failure stops the run; both report the failure.

## 2026-10-11: three-way uv benchmark, remote loses on new checkouts to upload time

uv 44b2e58, a new git worktree per case, mean of 2 interleaved rounds. Laptop
load 12 to 39 from other agents. Remote c7i.8xlarge us-west-2, both caches warm.

| case | stock cargo | justrust --local | justrust --remote |
|---|---:|---:|---:|
| `check --workspace` | 89.7 s, 484 CPU-s | 54.0 s, 111 CPU-s | 83.2 s, 0.6 CPU-s |
| `test -p uv-resolver --no-run` | 121.1 s | 39.3 s | 109.4 s |
| one-line edit, `check -p uv-resolver` | ~1 s | ~1.2 s | ~1.0 s |

Remote compile itself was 17.7 to 23.2 s (574/574 and 475/475 depcache hits).
The rest is the first rsync of a new checkout: 40 to 121 s for ~45 MB of source
(sync 87484/120623/39825/49956 ms). Fix belongs in justrust: content-addressed
upload or seeding the remote tree from the same git commit (fetch on the machine,
then sync only the diff) would put remote at ~20 s here, 2.7x local.
Logs: ~/.jcode/scratch/uvbench/final.txt.

## 30. jr-rustc and the proc-macro cache on by default (2026-10-11)

`justrust toolchain jr-install` packages a jr-rustc build (stage 2
PGO+BOLT driver, rustdoc and clippy built against the same driver, cargo
and rustfmt from nightly-2026-10-08, rust-src) as toolchain `jr-<commit>`.
With no `justrust.toml` pin, recorded local builds now use the newest
installed jr toolchain, and the shim adds `-Zcache-proc-macros` to
incremental units whose real rustc is that toolchain's. Opt out:
`JUSTRUST_CACHE_PROC_MACROS=0` or `JUSTRUST_TOOLCHAIN=system`. Remote builds
are unchanged. Installed here as `jr-ac20de5e7921`.

Checked with the default (no env) through `justrust` on real projects:

- justrust itself: `test` 263 passed, `clippy` clean, doctests pass, run
  meta shows `real_rustc` = the jr toolchain.
- The shim adds the flag to an incremental unit and not with
  `JUSTRUST_CACHE_PROC_MACROS=0` (observed through a wrapper rustc), nor for
  a non-jr rustc or a non-incremental dependency (unit test).
- jcode-desktop-ui full lib suite: 1279 passed, 1 failed:
  `panel::selection_tests::pinned_task_selection_copies_without_collapsing_card`.
  It fails with the system rustc too (`JUSTRUST_TOOLCHAIN=system`, 1 of 1
  failed): the checkout has another session's uncommitted `panel.rs` edit,
  not a toolchain effect. Doctests pass.
- jcode `check -p jcode-core` ok.
- Reference loop on the real `~/jcode-desktop` (one-line edit in
  fps_counter.rs, `justrust test -p jcode-desktop-ui --lib -- fps_counter`,
  file restored from a backup copy), 8 alternating pairs: **default 5.70 s
  median (5.2-8.8) vs system rustc 8.65 s (8.2-10.1), faster in 8 of 8.**
  That combines the newer nightly, PGO+BOLT build and the proc-macro cache
  versus the distro's stable rustc 1.98.

Caveat that ships with it: a proc macro whose output depends on files or
environment variables (not its input tokens) is not re-run when only those
change.

End-to-end over the whole default path, installed binary, no settings
(`bench/e2e-default-toolchain.sh`, fixture `bench/e2e`: a workspace with
its own attribute, derive and env-reading proc macros), 15/15 PASS: default
toolchain is jr; body edit, attribute-arguments-only change, derive-input
change and revert each produce the right values; a type error inside an
attributed fn is reported, and the fix passes; doctests through jr's
rustdoc pass, also after an attribute change; clippy through jr's clippy;
`JUSTRUST_TOOLCHAIN=system` uses the system rustc; a `justrust.toml` pin to
an uninstalled jr toolchain fails open to the system rustc with a message;
the documented caveat is real (an env-reading macro keeps its cached
output after an unrelated edit) and `JUSTRUST_CACHE_PROC_MACROS=0` re-runs
it. Also rerun after the change: justrust's own 263 tests and clippy at the
committed HEAD, `jcode check -p jcode-core`. Toggling the cache flag does
not rebuild an up-to-date crate (cargo does not fingerprint the flag the
shim adds), and the next edit compiles with the current setting.

## 43. Public prebuilt store: fleet on AWS works, but downloading loses to compiling on a quiet laptop (2026-10-11)

Deployed: daily prebuild fleet (justrust-cloud `infra/prebuild-fleet.yaml`,
EventBridge Scheduler -> one egress-only c7i.8xlarge with the prebuild role,
latest justrust release, `prebuild/run.sh`, self-terminates, log to
`fleet/logs/`), and CloudFront in front of the bucket
(`https://d1j555vqgf2xkj.cloudfront.net`). Seeds 43 -> 105 crates. Two
fleet runs triggered through the real scheduler role: 1486 manifests,
1.47 GB in the store, ~2 min per run on 32 cores.

Bug found: the static musl release keyed proc macros by `cfg!(target_env)`
("musl") instead of the host's C library, so release-built keys of every
proc macro and its dependents never matched a glibc build's: 86 of 647
misses on fleet artifacts from the laptop. Fixed (host libc detected at
runtime, glibc version via getconf), released as 0.1.0-alpha.5: 615/647
prebuilt.

Then the speed. 105-crate seed project, cold `check`, empty depcache,
fresh target dir, alpha.5 binary, laptop in San Francisco on ~10 MB/s,
interleaved:

| run | CloudFront | S3 direct | no store (compile) | load |
|---|---:|---:|---:|---:|
| 1 | 80.7 s (615 hits) | 126.5 s | 36.3 s | 8-16 |
| 2 | 112.1 s | 138.2 s | 39.2 s | 4-14 |

Earlier on a busy machine (load 24-48) the store won (finding 42: 39-43 s
vs 60-70 s, 43 crates), and a second busy round here was mixed (73-78 s vs
63-86 s). What it costs: 615 bundles, 343 MB compressed (gzip fast) for a
`check` that compiles in ~280 CPU-seconds; ~34 s of pure transfer at
10 MB/s, plus a curl process, TLS handshake and manifest round trip per
unit (median restore 1.2 s, p90 2.9 s, ~11 in flight). On 16 free cores
compiling those crates is cheaper than moving their outputs. CloudFront
helps (edge hit 0.13 s vs 1.3 s for a 1.6 MB bundle) but not enough.

Decision: the store stays **opt-in** (`JUSTRUST_PREBUILT=default`); the
alpha.4 default is reverted. It pays off where CPU is scarce (busy
machines, small VMs, CI) or the network is fast. What would make it a
default win, in order: (1) fetch only units whose recorded compile cost
beats their size at the measured bandwidth (per-unit cost decision, the
same rule as the remote route), (2) one persistent HTTP/2 connection
instead of a curl process per request, (3) zstd and no rlib for `check`
(rmeta is enough), (4) sizes in the index so the client can decide without
a manifest request.

## 2026-10-11: cargo shim breaks `trunk build` for wasm32 (pinned toolchain has no wasm std)

`~/.local/bin/cargo` -> justrust. `trunk build` in ~/jcode-website runs
`cargo build --target=wasm32-unknown-unknown`, and justrust builds with its
pinned toolchain `jr-ac20de5e7921`, which has no `wasm32-unknown-unknown`
std: `error[E0463]: can't find crate for core`. The system toolchain
(/usr/lib/rustlib) has the wasm targets, and `PATH=/usr/bin:$PATH trunk
build` succeeds. Fix in justrust: when `--target` names a target the pinned
toolchain lacks, either add it to the pin or fall through to the system
rustc, and say so in one line instead of failing the build.

### Rerun after the fixes (2026-10-12)

Same seed, generator and machine,
`versus-synth1-20261012-002025-9ab215affbd3-dirty-remote-c7i.8xlarge.json`,
`bench/VERSUS-SYNTH.md`. Fixes since the first run: the test pool settles
parallel-only failures by rerunning those names, not the whole binary
(7138120); it honors cargo's fail-fast (9ab215a); `/bin/sh` is bash on the
machine (1c50a2b), so points 2 and 28 now build (40/40 points, no errors).

| | stock | justrust | justrust vs stock |
|---|---:|---:|---:|
| before, 36 shared points | 15.34 s | 18.98 s | 0.81x |
| after, same 36 points | 14.34 s | 14.69 s | 0.98x |
| after, all 40 points (raw headline) | 16.01 s | 14.10 s | 1.14x |

The raw 1.14x overstates it: on points 7 (`build`, rerun) and 8
(`check -p jcode-desktop-motion` after `check -p jcode-desktop`) stock's
own warm target dir was invalidated by the unmeasured pre-build (70.2 s and
30.0 s, against 0.37 s and 0.18 s in the first run), a harness artifact,
not something justrust did better. Leaving those out (and 2 and 28, which
had no data before), justrust moved from 19% slower than stock to even.
By command (all 40): check 2.41 s vs 5.41 s (stock includes point 8),
test-filtered 0.95x, test 0.95x, build --release 0.93x, test --workspace
1.28x. justrust is now close to stock on warm loops and still a few
percent behind on test and release builds; the remaining gap is the
remote machine's per-thread speed and justrust's ~80 ms per call
(`bench/noop-overhead.py`), not test-runner bugs.

Next for the harness: detect a stock pre-build that recompiles more than
the previous point left warm (a cache invalidation) and flag the point,
and run -n 2 so single outliers do not move the means.

## 44. Prebuilt store: fetch only what beats compiling, then it wins by default (2026-10-11)

Finding 43's per-unit numbers (same laptop, 105-crate seed project): fetch
median 1.18 s, compile median 0.18 s. Fetching was faster for only 34 of
455 units. So the store now publishes `v1/costs` (`<key> <compile ms>
<bundle bytes>`, written by `justrust cache --publish`, sizes equal to the
real S3 bundles for 615/615 keys) and the client fetches a unit only when
`0.7 s + bytes / rate < compile ms`, with `rate` an EWMA of this
machine's measured bundle downloads (5 MB/s until measured; it settled at
5.7-6.0 MB/s here). Unpriced keys are not fetched from the public store.
`JUSTRUST_PREBUILT_ALWAYS=1` restores fetch-all.

Quiet laptop (load 6-14), cold `check`, empty depcache and target dir,
CloudFront store, interleaved:

| round | cost-aware store | fetch all | compile only |
|---|---:|---:|---:|
| 1 | **22.0 s** (76 fetched) | 32.7 s (615) | 35.1 s |
| 2 | **19.0 s** (71) | 55.4 s (615) | 35.3 s |
| 3 | **25.8 s** (63) | | 34.0 s |
| default build, var unset (load 16-20) | **28.9 s** (77) | | 43.5 s |

-30 to -45% wall against compiling, fetching ~12% of the units (the
long-compiling ones: tokio, axum, syn, zerocopy, h2, ...). A project the
store knows nothing about (nalgebra + bevy_ecs, 102 units, 0 hits): 12.4,
12.8, 16.3 s with the store vs 11.1, 12.3, 12.8 s without, within noise;
the index and costs downloads are 77 KB, ~0.25 s once per 10 minutes.

Shipped on by default again (`JUSTRUST_PREBUILT=off` disables), the fleet
publishes `v1/costs` every run. Remaining: one HTTP/2 connection instead
of a curl process per request (the 0.7 s fixed cost is most of a fetch),
rmeta-only bundles for check, and pricing by this machine's own compile
speed instead of the fleet's.

Verification of the released 0.1.0-alpha.7 (installed with the public
installer into a scratch dir, var unset), interleaved on the seed project:
store 23.0 s / 32.1 s (78 / 65 fetched) vs off 46.8 s / 43.1 s (load
14-20). Fail open: unreachable store, a garbage `JUSTRUST_PREBUILT` value,
and a private store without `v1/costs` all built the uncovered project in
11.7-12.9 s (baseline 11.1-12.8 s); the dead store tripped the breaker
after 6 failures. A fleet run triggered through the real scheduler
installed alpha.7 and rewrote `v1/costs` (1177 keys; sizes equal to the
real bundles for 1145/1145 checked; fleet compile times are 0.86x the
laptop's, so its prices err toward compiling).

## 45. macOS: recording, sampling, and the depcache on real Macs (2026-10-12)

Issue #1. GitHub Actions `macos-latest` (arm64) and `macos-15-intel`, CI
run 38196948040. Cross-checking from Linux compiled clean but missed every
runtime difference below; only runs on real Macs found them.

- `ru_maxrss` is bytes on macOS, KiB on Linux: the first report showed an
  81,552 MB rustc. Fixed, now 67-80 MB for the smoke crate.
- ld64 writes a dylib's output path (inside the target dir) into
  `LC_ID_DYLIB`, so the store refused every proc macro as carrying the
  target dir: a second checkout of a serde-derive crate got 16 hit/1 miss
  (serde_derive). With `-Wl,-install_name,@rpath/...` on proc-macro
  misses: 17 hit/0 miss, ~14.4 s of rustc saved, and the restored proc
  macro loads (the test passes). Linux on the same crate: 17 hit/0 miss.
- A lock probe released by close can stay held by a child another thread
  is forking (it shares the open file until exec). Flaky slot tests on
  both platforms; probes now unlock explicitly.
- Pressure stall info does not exist on macOS: reports say "not measured".

## 45. Remote `build` with the binary brought back: release wins, debug ties (2026-10-11)

Question: `build` and `run` always stay local (`src/route.rs`) because the
binary must end up here. Is a remote build plus artifact download faster?

Setup: the same `git archive` snapshot of justrust on the laptop (12
threads, load 22-32, other agents building, the realistic case) and the
hosted c7i.8xlarge (32 cores, us-east-1, 145 ms round trip). Plain cargo on
both sides, `cargo build --bin justrust`, dependencies fetched first. Edit:
one line appended to `src/summary.rs`, 3x per profile. Scripts were ad hoc
(`bench.sh`, `edit.sh`, not committed); numbers in ms.

| case | laptop | remote compile |
|---|---:|---:|
| cold dev | 14.7 s | 13.3 s |
| edit dev | 1.73-1.87 s | 1.66-1.67 s |
| cold release | 22.3 s | 17.0 s |
| edit release | 21.6-28.2 s | 13.3-13.4 s |

Artifacts: dev binary 101 MB, release 10.1 MB. A real code edit (changed
string plus a new branch in `summary.rs`) shifts addresses, so the delta
is far from zero. Compressing the new binary with `zstd --long=31
--patch-from=<previous binary>` on the machine:

| artifact | full | zstd -3 | delta -3 (compress time) | delta -19 |
|---|---:|---:|---:|---:|
| dev (101 MB) | 101 MB | 21.9 MB | 5.4 MB (~0.3 s) | 2.1 MB (35 s, useless) |
| release (10 MB) | 10.1 MB | 3.3 MB | 2.2 MB (~0.04 s) | 0.9 MB (5.7 s) |

Fetch the delta and apply it on the laptop against the previous binary:
release 0.38-0.43 s, dev 0.64-0.80 s. The result is byte-identical
(sha256 equal). Raw download from the machine varied 5-19 MB/s, so a full
debug binary (100 MB) would cost 5-19 s and only deltas are viable.

Reading:
- **Release edit: 21.6-28.2 s local vs ~13.8 s remote (13.4 compile +
  0.4 fetch), 1.6-2x.** Worth shipping: release builds parallelize
  (codegen units, LLVM) and the 10 MB binary deltas to ~2 MB.
- Dev edit: 1.8 s local vs ~2.4 s remote (1.7 + 0.7 fetch). Remote loses.
  The 100 MB debug binary is the problem: most of it is debug info, and
  `objcopy --strip-debug` leaves 29 MB. Shipping it needs split debuginfo
  that stays remote, or lazy fetching.
- Cold: small wins (1.1x dev, 1.3x release) before transfer. A cold
  build has no base binary to delta against, so a dev binary costs a full
  zstd download (22 MB, 1-4 s). Worth it only for bigger projects, where
  the shared dependency cache does most of the work.
- Not 10x anywhere: this crate's local build is mostly one crate's
  frontend, and the machine is 1.3-2x per core-bound step, not 10x.

Plan: route `build --release` (and `run --release`) remote when history
says the local build takes over ~5 s, ship the binary as a zstd
`--patch-from` delta against the last binary fetched (a full zstd copy
when there is none), and keep dev `build`/`run` local until split
debuginfo is in place.

Later the same night (CI run 38199891022, both Macs green), more runtime
bugs found only by exercising the workflows on real runners:

- clonefile keeps the cache entry's mtime, so restored rlibs looked older
  than the unit's `invoked.timestamp`: a seeded slot rebuilt 8-10
  dependency units ("quote changed"). Restores now get a fresh mtime;
  a new slot after a one-line edit builds 1 unit, 11 fresh.
- Pinned toolchains failed to install on every Mac: 1.98.1 ships no
  cranelift for apple-darwin. Optional components are now skipped;
  `1.98.1` installs and builds on both Macs.
- `peer_cache_serves_and_installs_entries` failed twice in Linux CI: each
  CachePut chunk was appended on its own thread, so a last chunk could
  complete the bundle before an earlier one landed. Chunks now append on
  the reader thread. Not reproduced locally (0/60 before and after).

Requirement to evidence (macos-latest / macos-15-intel, run 38199891022):

| Requirement | Check | Observed |
|---|---|---|
| builds on macOS | clippy -D warnings, cargo test | 254 passed / 254 passed |
| xattr arity | depcache key memo tests | pass |
| reflink | depcache restore + slot seed | 17 hit/0 miss; slot seeded 0.0-0.1 s |
| affinity, cgroups | sched off, builds succeed | ok |
| sampler | `show --json` asserts | build cores 1.56 / 1.50, rustc 111 / 63 MB, avail 3.6 / 10.4 GB |
| no silent zeroes | report text, `psi_unavailable` | "stalls not measured" |
| pid guard | `status --json` after a run | `active: []` |
| proc macros cached | 2nd checkout of serde-derive | 17 hit/0 miss, test passes |
| routing | `show` where line | local, reason given |
| toolchain pin | 1.98.1 install + test | "pinned toolchain 1.98.1", 1 passed |
| release packaging | thin-LTO stripped build runs a test | 1 passed |

## 46. Five days of agent builds: where 13.7 h went and what the ceiling saves (2026-10-12)

`bench/runlog.py` over `~/.justrust/runs` (5785 runs, 2026-10-06 to
2026-10-11). Bench snapshot runs excluded (3270, 15.1 h) and one hung
jr-cargo test (6985 s, run 20261011-083315454-3785807): 2514 agent runs,
13.7 h wall, median 5.0 s.

| kind | runs | wall | median | ceiling assumed | saved |
|---|---:|---:|---:|---|---:|
| dependencies rebuilt | 369 | 6.34 h | 37.6 s | deps from cache, 5 s + local (max 2 s) + tests | 5.57 h (88%) |
| local crates only | 1233 | 5.09 h | 7.1 s | 0.5 s + tests | 4.66 h (91%) |
| compile error | 202 | 1.88 h | 10.3 s | first error in 0.2 s | 1.87 h (99%) |
| no-op | 710 | 0.38 h | 0.5 s | 0.05 s + tests | 0.18 h (48%) |
| **total** | 2514 | 13.7 h | | | **12.3 h (90%)** |

Dependency rebuilds are the largest bucket and almost all cold targets:
173 runs (3.1 h) with zero fresh units and no rebuild reason, 11 more
(0.75 h) cold release builds, despite 34k depcache hits in these runs.
That is the cold-build path (prebuilt store, remote), not the incremental
loop. Test execution is only 0.67 h of the 13.7 h.

LLVM in local-edit runs: 22-32% of rustc time in test/build (jcode test
0.39 of 1.39 h), more than the reference loop's ~2% because jcode's edits
regenerate more codegen units. Check runs have none.

## 47. Shadow split: moving one leaf module out of the giant crate makes its test loop 12x faster (2026-10-12)

Manual prototype (`~/.jcode/scratch/shadow-proto`, `ab.py` alternates
variants, each iteration edits the file then runs `justrust`). `orig/` is the
Desktop snapshot, `man/` moves `fps_counter.rs` from `jcode-desktop-ui`
(the big crate) into a leaf crate `jr-shadow-fps`. Direction: the leaf is a
dependency of the old crate, which re-exports it under the old path
(`pub(crate) use jr_shadow_fps::fps_counter;`), so `crate::fps_counter::*`
in `workspace.rs` keeps working and test names stay
`fps_counter::tests::*`.

Edit `fps_counter.rs`, run its tests (n=6, ab-run2):

| variant | command | median | range |
|---|---|---:|---:|
| orig | `test -p jcode-desktop-ui --lib -- fps_counter` | 6.0 s | 5.23-7.3 |
| man | `test -p jr-shadow-fps --lib -- fps_counter` | 0.49 s | 0.47-0.53 |

Earlier pass (ab-run): 7.97 s vs 0.69 s. About 12x. Run ids: orig
20261011-214107008-2994872 (6.0 s), 20261011-214117810-2996443 (5.23 s);
man 20261011-214117316-2996324 (0.49 s), 20261011-214100356-2993688 (0.47 s).

After a leaf edit, `check -p jcode-desktop-ui` in man costs 2.14 s median
(1.83-2.15 warm, run 20261011-214058213-2993551): the UI crate must be
re-checked because it depends on the leaf.

Edits that stay in the big crate (ab-run3, n=6): the split costs a little.

| command | orig | man |
|---|---:|---:|
| `check -p jcode-desktop-ui` | 2.33 s | 2.56 s |
| `check -p jcode-desktop-ui --lib --profile test` | 3.33 s | 3.88 s |

(orig 20261011-215144495-3054752 / man 20261011-215039775-3045639;
orig 20261011-215156485-3056267 / man 20261011-215149468-3055296.)

So the win needs command routing: when the agent runs
`test -p jcode-desktop-ui -- fps_counter` and only leaf files changed, run
`test -p jr-shadow-fps -- fps_counter` in the shadow; keep the big crate in
the command whenever its own files changed, and always add the leaf
package when the target package had modules split out so moved tests do
not vanish from the filter. The productized opt-in is `JUSTRUST_SHADOW=1`
(below, once measured).

## 48. Hot patching through `justrust test`: fires on every edit, link 0.33 -> 0.12 s, wall within noise; a test-process zygote would save under 0.5% (2026-10-12)

Follow-up to 35 (prototype) and the opt-in port in 8ec1f42
(`JUSTRUST_HOTPATCH=1`, `src/hotpatch.rs`). The first factor run of that
port (`factors-20261011-165959-...`, -2%) never patched: it predates the
response-file fix.

**It fires.** On a private copy of the Desktop snapshot through
`justrust test -p jcode-desktop-ui --lib -- fps_counter`, outcomes in
`<run dir>/hotpatch.jsonl`: the reference test edit and the non-test body
edit both patch 5-6 of 257 units (361-526 functions redirected). One edit
fell back: a regenerated unit held a `thread_local!` access
(`R_X86_64_TPOFF32` to `PREVIEW_CACHE`, `panel_diff.rs`). 12 TPOFF32 and 1
GOTTPOFF sites exist across the UI crate's 256 units.

**Fixed: thread-locals bound to the base** (ea4bae1). In an executable a
thread-local's offset from `%fs` is a link-time constant
(`st_value - round_up(PT_TLS memsz, align)`), so the changed object is
copied, each local-exec site gets the base's offset written in, and
initial-exec `mov x@gottpoff(%rip),%r` is relaxed to `mov $off,%r` the
way ld relaxes IE to LE; the relocations become `R_X86_64_NONE`. Dynamic
TLS models (never emitted for an executable's own code) still fall back.
An edit inside the `PREVIEW_CACHE.with` closure now patches (`1 tls`), and
the `panel_diff` tests that exercise that cache pass on the patched binary.

**Fixed: base symbol index cached** (ea4bae1). Reading the base's 962k
symbols was 130-160 ms of every patch step. Now an on-disk index
(`<exe>.jrhot/index`, 43 MB: name hashes sorted, pointing into the
base's own string table, keyed by inode, size and a hash of the section
headers) is built by a detached niced child right after each base link
(0.3 s, nobody waits) and mapped by every patch. Patch step 150-350 ms ->
36-58 ms quiet (`basesyms 0`, lld on 5-6 objects 23-39 ms is now most of
it). Unit test: the index resolves every symbol of a real binary exactly
like the in-memory table.

**Correctness** (`~/.jcode/scratch/hp/loop.py`, private target dir):
full `jcode-desktop-ui --lib` suite, patched (2 runs, 6/257 units
patched): 1303 passed, 0 failed, 10 ignored; normal relink (2 runs): 1303
passed, 0 failed, 10 ignored (one parallel-only flake in a patched run,
`workspace::tests::the_hint_chip_uses_only_spare_tab_space`, passed alone,
as the pool reports it). Failing edit (expected value of an existing
`assert_eq!` changed), patched: 2 passed, 1 failed with the right
`left: "FPS · idle"` message; the fix, patched: 3 passed; repeated twice
(runs 20261011-214133-hp2998720 fail, -214141 fix, -214148 fail, -214156 fix).
An edit that adds a new `assert_eq!` (a new generic instance) or adds a
line before other code regenerates 131-146 of 257 units (CGU
repartitioning, span shifts), which is above the half-of-units limit and
relinks normally: correct, just not patched.

**Measured**, `bench/factors.py run hotpatch hotpatch-body -n 8 --warmup 2
--quiet-wait 60` (ABBA, frozen binary for body after other agents
reinstalled justrust mid-run twice; every `on` sample patched, per-sample
link pass and outcome now recorded by the bench):

| factor | on | off | link pass on / off (median) | patch step |
|---|---:|---:|---:|---:|
| hotpatch (test edit) | 8.23 s (7.8-17.3, 3 noisy) | 8.39 s (7.0-10.3) | 0.12 / 0.33 s, disjoint | 47-105 ms |
| hotpatch-body | 7.78 s (7.5-8.9) | 7.93 s (7.7-8.5) | 0.10 / 0.32 s, disjoint | 40-52 ms |

Results `bench/results/factors-20261011-214238-891dfea50aae.json`,
`factors-20261011-221526-239bef0338bc-dirty.json` (sample run ids, e.g.
hotpatch on 20261011-214813385-3005003103, off 20261011-214921978-3005003856).
The link pass drops by 0.2 s in every sample, but that is 2.5% of a loop
whose frontend (3.9-4.2 s) and codegen + incremental persist (~2.2 s) are
untouched, so wall is within noise. Cost: each patched process pays ~2 ms
more startup in the loader (map parse, 6k relocations, 361 jumps: 9.2 vs
7.2 ms no-match run). **Stays opt-in**: no measurable wall win today; it
becomes worth turning on when a resident compiler makes the frontend
small (35's argument still holds).

**Zygote (fork per run after the loader patched): not built, measured
first.** A zygote saves at most what a test process spends before `main`:
exec, ld.so relocation (587k relative relocations in the UI binary, 93% of
ld.so's 2.3M cycles), the hot patch loader, and static init. Measured by
timing exec up to an `LD_PRELOAD` constructor that `_exit`s (40 runs,
medians) vs a full no-match run, and counting test-binary processes per
`justrust test` with a preload that logs each pid
(`~/.jcode/scratch/hp/zy/`, runs 20261011-zy-tui-3428028-{1,2,3}):

| binary | exec to preload ctor | full no-match run | fork+exit+wait instead | processes per warm full-suite run | suite test_run | startup share |
|---|---:|---:|---:|---:|---:|---:|
| jcode_tui lib (2,518 tests) | 4.2 ms | 5.1 ms | 0.17 ms | 10 | 13.6-13.8 s | ~50 ms, 0.4% |
| jcode_desktop_ui lib (1,313), patched | 8.5 ms | 9.2 ms | 0.29 ms | 2 | 10.8-11.3 s | ~18 ms, 0.2% |

The pool does not start a process per test: libtest runs a whole group
per process, and the pool splits a binary into at most ~6 groups plus a
`--list` and the by-name confirmation reruns of failures seen next to
other processes. The jcode-tui suite's time is test bodies (27 s CPU,
libtest-reported 16.8 s across processes), and its 2.5 s `tail` is the
confirmation rerun of 2-4 overlapped failures alone (test bodies again),
not startup. So a zygote could save at most ~50 ms of a 16.6 s run, and it
would need a fork-safe point before `main` in arbitrary test binaries
(threads started by static init, open fds, `/proc/self/exe` users).
Not worth building. What would pay on big suites is less test work (run
only affected tests, cheaper confirmation), not cheaper processes.

## 49. Why agents rebuild dependencies: cold targets and depcache misses (2026-10-12)

Follow-up to 46 (dependency rebuilds: 6.3 h of 13.7 h agent wall). Mined
`~/.justrust/runs` (`bench/runlog_misses.py`), agent runs only.

**Cold target dirs** (184 runs, 3.8 h, zero fresh units):

| cause | runs | wall |
|---|---:|---:|
| experiments in throwaway target dirs (`~/.jcode/scratch/...`) | 125 | 2.09 h |
| target dir emptied (cargo clean / deleted) | 32 | 0.63 h |
| first release/profile build | 7 | 0.57 h |
| toolchain switched in the same target dir | 22 | 0.46 h |
| first build of a checkout | 10 | 0.20 h |

**Every dependency miss** (repeat the same rustc + package + version +
opt level + emit, 10.6 h of rustc):

| cause | units | rustc |
|---|---:|---:|
| key changed for the same crate and rustc (upstream metadata cascade: a git dep like gpui at a new rev, feature unification, scheme-era churn) | 16.6k | 5.73 h (54%) |
| first time with this rustc (system -> nightly -> jr toolchains this week) | 6.7k | 3.33 h (31%) |
| evicted by the 30 GB GC cap | 1.8k | 0.89 h (8%) |
| justrust key scheme bumped (8 bumps in 2 days) | 1.6k | 0.65 h (6%) |

Most of the churn was the week's own work (toolchain switches, scheme
bumps, cache experiments). Live check today, two fresh target dirs on
Desktop, `check -p jcode-desktop-ui`, local: first 818 hit / 14 miss (gpui
and its 13 workspace crates: Desktop's gpui git rev moved), second 832/0.
**The cache works now, but a fully hit cold check still takes 29.5 s**
(run 20261011-232856920-140417): dependency restore spans 13.3 s (832
restores, up to 57 at once; the machine had 22 other rustc running, 19 s
CPU stall), then 57 local crates are compiled from scratch (58.7 s rustc,
jcode_base 7.2 s alone). So the next wins for cold targets are:

1. local crates from a cache (`localcache.rs`, opt-in, not yet hitting on
   Desktop: build.rs stamps time via `env!`), or cloning a warm slot's
   target dir instead of starting empty;
2. restoring the whole dependency set as one reflink of a prepared
   target tree, not 832 shim round trips through cargo's scheduler.

Shipped: the GC cap now defaults to a quarter of the free disk, 30 to 150
GB (here 150 GB, cache at 27 GB). At 30 GB it evicted 3-5 GB every 1-3 h.

### 45b. Shipped: optimized `build` routes remote, binary comes back as a delta

`src/remote_artifacts.rs`, routing in `src/route.rs` (`routable_build`).
Same laptop (load 20-22, other agents building), justrust repo, one-line
edit to `src/summary.rs` then `justrust build --release`, interleaved:

| round | remote (client wall, incl. fetch) | local |
|---|---:|---:|
| 1 | 18.0 s | 25.0 s |
| 2 | 17.5 s | 30.7 s |
| 3 | 17.0 s | 37.9 s |

Remote median 17.5 s vs local 30.7 s, **1.75x**, disjoint ranges. Fetch:
full zstd copy 3.5 MB in 0.8-1.6 s after a local build; consecutive
remote builds come back as a 0.1 MB delta in 0.4-1.0 s (the comment-only
edit barely changes the binary; a real code edit measured 2.2 MB in
FINDINGS 45). No-op remote build: 1.7 s, artifact check 0.38 s. Run ids
20261011-235746732-770608 (local 1), 20261011-235817389-780593,
20261011-235834417-782823, remote runs 20261011-235919370-786512 onward.

## 50. Local crate cache: workspace crates restored by content, revert 10.6 -> 0.7 s, fresh target dir 66 -> 6.3 s (2026-10-12)

Code: `src/localcache.rs` (239bef0 and follow-ups to 8f1aff6+), shim wiring
in `src/shim.rs`, `depcache::plan_with`. Factors `localcache-revert`,
`-revert-sde`, `-revert-small`, `-loop`, `-fresh` in `bench/factors.py`
(new samplers `revert` and `fresh`). Measured with a pinned copy of the
justrust binary (`JUSTRUST=<copy>`): other agents reinstalled justrust
mid-run three times.

**What it is.** depcache only took registry and git units. Workspace and
path crates now go through the same two-level key (args with the profile
dir normalized, `--extern`s by crate metadata, then every dep-info source
and tracked env var by content), plus the link args the shim adds (wild).
Differences from depcache: no extra rustc args (local units compile
exactly as cargo asked, so recording never changes outputs, incremental
state, or fingerprints, and the key includes the working dir: a worktree
at another path misses, measured), `-C incremental` accepted (a hit leaves
the incremental dir alone), and the store is off the critical path. The
shim hard-links the outputs into `s/` and a niced, idle-io
`justrust __localcache-finish` copies, hashes, runs the target-path guard
and commits. Off with `JUSTRUST_HOTPATCH=1`. Own cache dir, 10 GB LRU cap.

**Results** (Desktop snapshot unless noted, ABBA, medians, min-max):

| factor | workload | on | off | |
|---|---|---:|---:|---|
| localcache-revert | edit fps_counter.rs, build, undo, build (measured) | 12.00 s (10.6-14.0) | 12.76 s (12.1-15.1) | 0/5 hits, see below |
| localcache-revert-sde | same with `SOURCE_DATE_EPOCH` fixed | **0.70 s (0.68-0.73)** | 10.61 s (10.4-11.3) | 5/5 hits, -93% |
| localcache-revert-small | justrust snapshot, full `test` | **0.88 s (0.80-2.14)** | 2.41 s (2.15-3.70) | 4 of 5 units hit |
| localcache-fresh | fresh target dir, unchanged sources, deps from depcache both sides | **6.27 s (5.4-6.7)** | 66.14 s (59.7-88.8) | 54/54 local units restored |
| localcache-loop | plain reference loop (every build misses) | 8.93 s (8.5-9.2) | 8.70 s (8.4-11.7) | within noise |

Results: `factors-20261011-230158-adeeaebe403f-dirty.json` (revert-sde,
revert-small), `factors-20261011-233833-2b487ca729a9-dirty.json` (fresh),
`factors-20261012-001556-fc0cd2778c87-dirty.json` (loop),
`factors-20261011-221705-a2c6ad1308d2-dirty.json` (plain revert). Example
runs: revert-sde on 20261011-230734816-3809690194 / off
20261011-230713186-3809690629, fresh on 20261011-234637064-336728534 / off
20261011-234507388-336728483.

**Why the plain Desktop revert never hits.** `jcode-desktop-ui`'s build.rs
stamps `JCODE_DESKTOP_BUILD_ID` (build time in ms) into the crate with
`cargo:rustc-env`, read by `env!`. Every build is a new input, so every
result is unique (20 entries, same sources, 20 build ids). It is correct
to miss: the restored binary would report another build's id. Options
measured or considered:

- Set `SOURCE_DATE_EPOCH` by default: makes Desktop hit (revert-sde), but
  changes what the project's binaries say about themselves, and it is a
  build input of every registry build script that reads it (all
  dependency keys would move once). Not done: the user's choice, not ours.
  Mentioned to the project instead.
- Detect volatile inputs and skip storing (shipped): a tracked env value
  that parses as a Unix time from the last ten minutes, or a unit whose
  sources match two stored entries with other env values, is not staged.
  Desktop's UI test unit now stages nothing (`staged:false`), so it costs
  only the lookup.

**Miss-path cost on the loop** (`<run>/localcache.jsonl`, 10 samples):
plan (key, lookup) 14-18 ms after memoizing lib-dir hashes (85 ms before),
stage 17-27 ms. A reflink of the just-written 285 MB test binary took
~170 ms because btrfs flushes dirty pages before cloning, so staging uses a
hard link and the finisher copies it and checks the inode/size/mtime
stamp, dropping the entry if a later build rewrote the file. The
remaining loop delta (+0.23 s median, rustc wall 8.19 vs 8.10 s) is
within the factor's noise. The finisher measured 0.72-0.93 s wall,
0.59-0.79 s CPU per 240 MB UI binary, niced. Disk: 240 MB per stored UI
test binary, ~15 GB after two hours of benchmark edits before the
volatile guard (every edit stores a new result), capped at 10 GB by GC.
Units whose sources are back to a stored state are what the cache is for.
The loop's normal edit is a new state, so the stored copy only pays off on
revert, branch switch back, or a new target dir.

**Correctness.**
- Restored outputs are byte-identical to a compile (justrust test binary,
  `cmp`), and the tests pass on them: 157/157 justrust, 3/3 Desktop
  fps_counter in every hit sample, the same counts as the off side.
- No `Dirty` units: after a hit, a `-v` no-op is all Fresh. Toggling
  `JUSTRUST_LOCALCACHE` on and off in one target dir leaves 48/48 Fresh.
- Key poisoning (`include_str!` data file, a build-script
  `cargo:rustc-env` read by `env!`, `option_env!`): each change misses and
  the test prints the new value, going back to the original inputs hits
  and prints the old one.
- Refused, as designed: integration tests that embed
  `env!("CARGO_BIN_EXE_*")` (the target dir as data), so justrust's
  `remote_agent` test always recompiles.

**Decision: on by default** (`JUSTRUST_LOCALCACHE=0` turns it off). The
win scenarios are large (fresh dir -90%, revert -64% to -93%) and the
miss path costs ~35 ms in the shim, with correctness checks clean. Not
yet measured on the synthetic workload. A fresh worktree at another path
and second checkouts do not hit (absolute paths in local outputs). Making
those hit needs `--remap-path-prefix` for the workspace, which changes
panic paths and debuginfo the user sees: a follow-up to measure.

## 51. Resident build daemon: prebuild on edit cuts the reference loop 8.9 -> 5.1 s; coalescing ties slots (2026-10-12)

`justrust __buildd` (`src/buildd.rs`), one per user over
`~/.justrust/buildd/sock`, spawned like `__daemon` (own process group,
inherited fds closed). Agent builds (`check|test|build|clippy`, no tty)
hand their request to it, and it runs the same `justrust <args>` with the
client's exact env and cwd, so routing, slots, toolchain, recording and the
report are unchanged. Three parts:

- **Coalescing**: identical request (cwd, argv, env minus agent identity
  such as `JCODE_SESSION_ID`/`KITTY_*`, and a source stamp: path, size,
  mtime, inode of every file under the workspace and its path
  dependencies, ~8 ms on Desktop) attaches to the running build and gets
  the full output and exit code.
- **Prebuild on edit**: the daemon watches the sources (inotify) of each
  agent session's last request and starts it 0.4 s after an edit burst,
  niced 10. `test` is prebuilt as `test ARGS --no-run`. Tests never run on
  a guess, and the agent's own request waits for the compile and then runs
  its tests on Fresh units. A new edit, or the agent asking for another
  command, stops an unclaimed prebuild. 3 prebuilds per request between
  asks (bounds builds that write into their own sources).
- **Warm state**: the measured win isn't there. justrust's own pre-cargo
  work is 5 ms plus a 16 ms median systemd scope (p90 42 ms, 467 no-op
  runs). The ~80 ms "overhead" in older notes was jr-cargo vs stock cargo
  noise, not justrust. A jr-cargo daemon mode would have to keep parsed
  manifests and index summaries resident (FINDINGS 38: a fresh process
  costs nothing, the parsing does). Not attempted.

Factors (Desktop snapshot, ABBA), `factors-20261011-234427-dcbed51dadfc.json`,
`factors-20261012-002718-fa7980014c4e-dirty.json`,
`factors-20261012-003954-c998b6a9ce04-dirty.json`:

| factor | on | off | verdict |
|---|---:|---:|---|
| buildd-prebuild, compile-only test prebuild (final), reference loop with 4.6 s think time | **5.14 s** (4.9-5.4), 0 units | 8.87 s (8.8-9.0) | helps, -42% |
| buildd-prebuild, first version (ran tests speculatively) | 4.37 / 4.74 s | 9.26 / 8.79 s | helps |
| buildd-coalesce: A and, 1 s later, B run the identical command | B 8.52 s, CPU A+B 10.4 s | B 8.64 s, CPU 10.5 s | within noise |
| buildd-noop: lone no-op `test` | 0.68 s | 0.65 s | +0.03 s, within noise |
| buildd-loop: reference loop, no think time | 8.92 s | 8.96 s | within noise |
| buildd-prebuild-interference: other agent's wrong-guess prebuild running while B does the loop | B 9.19 s | B 8.84 s | -4%, within noise |

Run ids (first on/off sample): prebuild 20261012-004117749-1180832501 /
20261012-004055505-1180832632, interference 20261012-003028138-1056780716 /
20261012-003018557-1056780260, coalesce 20261011-235557538-455398264 /
20261011-235547557-455398328, noop 20261011-235152778-455398861.

Coalescing ties because slots already coalesce (`join_identical`, FINDINGS
7): B waits for A's slot, then pays a 0.4 s no-op cargo. In real history
identical requests overlap rarely: 12 cross-session and 30 same-session
overlaps in 5,679 agent runs. The daemon keeps it for exact output sharing,
but it is not the win.

The think time is mined from Jcode sessions: the gap between an agent's
last edit and its build call has median 4.6 s, p75 13 s, p25 0 s (2,241
calls). With 0 s (`buildd-loop`) nothing is gained or lost.

Predictor (mined, 3,436 build calls that follow an edit and a previous
build in the same session):

| prebuild | right | prebuilds per edit | wasted per edit |
|---|---:|---:|---:|
| session's last request (shipped) | 53% | 1.0 | 0.47 |
| last 2 | 62% | 1.72 | 1.10 |
| last 3 | 68% | 2.27 | 1.59 |

Wasted CPU: 0.47 builds per edit. A wrong guess usually compiles the same
edited crate for a sibling command, and it is stopped as soon as the agent
asks for something else, so the cost is at most one median agent build
(9.6 CPU-s over 4,896 compiling agent builds, mean 91 CPU-s with cold
outliers) per wrong guess, niced. Measured interference on another agent
is within noise (-4%).

Correctness (`bench/buildd-correctness.py`, prebuild vs a direct build with
the daemon off in its own warm target, exit code and normalized test and
error lines):

    pass: prebuilt=True exit daemon=0 direct=0 same_output=True no_dirty_after=True
    fail: prebuilt=True exit daemon=101 direct=101 same_output=True no_dirty_after=True
    OK

The normalization drops pass counts of a failing run, because those differ
between two direct runs too: likely-failures-first with fail-fast prints
"2 passed" once and "0 passed" the next time. Exit 101 matches on the tiny
crate as well. A daemon that hangs up makes the client build locally (fail
open). Stdout is byte-identical apart from thread ids.

Decision: **on by default** for non-terminal agent builds on Linux, off
under `CI`. Kill switches: `JUSTRUST_BUILDD=0` (no daemon) and
`JUSTRUST_BUILDD_PREBUILD=0` (daemon, no prebuild). Benchmarks pin
`JUSTRUST_BUILDD=0` unless a scenario measures it, so prebuilds of restored
files never overlap samples. Remaining risks: build scripts with side
effects run on a guess (as with any `cargo check` from rust-analyzer), and
a prebuild holds the session's slot until stopped. Next: prebuild the next
command from history instead of the last one (the 68% ceiling of last 3
without the waste), and serve `check` from early rust-analyzer state.

## 52. Windows: recording, sampling, and the depcache on Windows (2026-10-12)

Windows went from plain-cargo passthrough to the same recorder, agent
output, reports, depcache, test pool, pinned toolchains and build daemon
as Linux and macOS. One platform layer (`src/sys.rs`) carries every
operation whose Unix and Windows forms differ. Unix keeps its exact calls,
so on Linux nothing changes. Map from each changed path to the check that
covers it (`windows-latest` CI job, "Windows integration paths", unless
noted):

| Path | Windows form | Check |
|---|---|---|
| process exec (shim, cargo, test runner, linker) | spawn, inherit stdio, exit with the child's code, ignore Ctrl+C meanwhile | smoke: check/test/compile error |
| unit and build CPU, max RSS | job object around the child (whole tree), `GetProcessTimes` fallback, peak working set | sampler: `avg_build_cores > 0`, `peak_unit_rss_mb` sane |
| sampler (`procfs`) | Toolhelp snapshot (one per sample), `GetSystemTimes`, `GlobalMemoryStatusEx`, command line via `NtQueryInformationProcess`; no PSI | `psi_unavailable`, `procfs::tests::reads_child_process` |
| locks | `std::fs::File::lock`/`try_lock` everywhere (flock on Unix, as before) | unit tests, status |
| rustc shim, `cargo` proxy | hard link or copy of `justrust.exe` plus a per-file `.justrust` sidecar (symlinks need developer mode) | install/uninstall in CI |
| toolchain paths | `.exe` on every `bin/` tool; host triple `x86_64-pc-windows-msvc`; sha256 via `certutil` | pinned 1.98.1 install and test |
| depcache restore | copy with a fresh mtime (no reflink on NTFS); no xattr hash memo (an NTFS stream would bump mtime) | depcache across checkouts |
| depcache proc-macro key | PE `.rustc` section of the DLL. The whole-file hash missed in every second checkout: the DLL embeds its target dir (PDB path), so serde_derive and its dependents (9 hit/8 miss) | `0 miss` in the second checkout |
| depcache libc family | `msvc`/`mingw` (wine showed the host's glibc loader leaking into the key) | `libc_family_is_the_hosts` |
| sockets (build daemon, test pool, sync daemon, peer cache) | AF_UNIX via `uds_windows` (Windows 10 1803+) | test pool in every `test` |
| ssh to hosted machines | `NUL` instead of `/dev/null`, no ControlMaster (Windows OpenSSH lacks it) | `ssh_target_is_strict` |
| split: Cargo.toml relative paths | always `/` (a `\` in a TOML basic string is an escape) | `relative_paths` |

Not on Windows, failing open: build slots (need copy-on-write clones, so
builds share the target dir), cgroup priority, wild and hot patching
(ELF), the sync daemon's watcher (pushes ride the per-run rescan), and
automatic remote routing (the machines are Linux). The remote agent and
hot patching compile out.

Validation before CI: cross clippy for msvc, gnu and darwin; the lib
suite under wine (x86_64-pc-windows-gnu, mingw linker) 250/251, the one
left needing a real rustc on PATH; end-to-end under wine with the Windows
Rust 1.98.1 toolchain: `check`, `test` with a failing test shown in full,
`show`, `log`, `runs`, `install`, and the depcache across two checkouts
(17/17 hits, saved 40 s of rustc). Wine quirks that are not justrust's:
rustc ICEs with incremental on (plain `cargo.exe` too), no process CPU
times (`GetProcessTimes` returns 0), no command line query.

Linux unchanged. Recorded and unrecorded builds still share artifacts (a
serde crate after `JUSTRUST_DISABLE=1 cargo build`: 1 local unit
compiled, 2 fresh, 0 Dirty, same as main). Suite A/B, main at merge-base
`dcbed51` vs the port, run twice interleaved on a busy machine (other
agents' benches, 2-8 foreign cores):
`bench/results/20261012-windows-port-ab-{main,win}-{1,2}.json`. Round 1
reads +15..54% slower, round 2 -17..+3%: the order flipped with the load,
every sample of both flagged noisy. A quieter direct measure, no-op
`justrust check` of this repo, 15 interleaved pairs: main 0.158 s median
(0.103 min), port 0.155 s (0.106 min). No regression measurable.
