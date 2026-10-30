# Native parallelism audit, October 30, 2026

## Decision and scope

Do not increase native compiler defaults from this audit. Existing measurements
support selective frontend threads, not using every core on every crate. A
focused native Cargo experiment is implemented, default off, to let unwrapped
workspace crates exercise frontend parallelism without imposing it on registry
dependencies or stable compilers. No compiler rebuild is needed for this policy.
This is configuration plumbing, not an established speedup or default promotion.

This audit covers owned compiler source and the coordinator-requested Cargo
policy. It does not assert that local saved compiler artifacts equal current
source, or that a project wrapper's flags are universal hosted configuration.
No changes to `FINDINGS.md` are part of this work.

## Exact native source defaults

Source anchors are relative to `toolchain/rustc/`.

| Control | Effective default and precedence | Source |
| --- | --- | --- |
| Frontend pool | **1 thread**, even with `--jobs=N`. Only `-Zthreads=N` or `--jobs-frontend=N` changes this. Supplying both is an error. | `compiler/rustc_session/src/config.rs`, `parse_jobs_all` |
| Backend workers | Available parallelism when neither `--jobs-backend` nor `--jobs` is set. Explicit backend wins, otherwise `--jobs` wins. Backend must support parallel execution. Work is jobserver-limited, not a promise to occupy all CPUs. | `parse_jobs_all`; `compiler/rustc_codegen_ssa/src/back/write.rs`, `start_executing_work` |
| Linker workers | Linker's own default when neither `--jobs-linker` nor `--jobs` is set. Native explicit thread arguments currently apply only to LLD, with available jobserver tokens limiting the request. This is not a mold/wild thread-control implementation. | `compiler/rustc_codegen_ssa/src/back/link.rs`, `linker_and_flavor` and final command assembly |
| Numeric jobs | `0` means available parallelism, capped at 255. Numeric 1 selects the non-synchronizing single-thread representation. `sync` selects synchronization with one worker and requires `-Zunstable-options`. Explicit phase limits cannot exceed `--jobs`. | `parse_jobs_one`, `parse_jobs_all` |
| User crate codegen units | Explicit `-Ccodegen-units` wins, then target override, otherwise **256 incremental / 16 nonincremental**. Number of CGUs is not number of live workers. | `compiler/rustc_session/src/session.rs`, `Session::codegen_units` |
| Persist/link overlap | Default enabled in source, opt out with `JUSTRUST_RUSTC_OVERLAP_PERSIST=0`. Requires dynamic thread-safe frontend mode, incremental state, a suitable output, no `-Zno-link`, and no host-metadata offload. Thus ordinary one-thread frontend builds do not take this overlap path. | `compiler/rustc_interface/src/queries.rs`, `overlap_persist_enabled` |

`compiler/rustc_interface/src/interface.rs` initializes a native jobserver using
the maximum requested frontend/backend/linker bound when needed. An inherited
Cargo jobserver remains authoritative. The frontend pool in
`compiler/rustc_interface/src/util.rs` installs acquire/release handlers or the
explicit native admission attachment. More configured workers do not create
independent ready queries or remove admission limits.

### Building the compiler is a separate setting

`justrust.bootstrap.toml` has no `parallel-frontend-threads` setting. Bootstrap
only adds `-Zthreads` to its own compilations when this setting is present
(`src/bootstrap/src/core/builder/cargo.rs`). This does not disable the resulting
compiler's runtime ability to use a parallel frontend.

`build-jr.sh release` and PGO modes compile rustc itself with incremental off,
thin LTO and **one CGU**. `dev` uses incremental on and thin-local LTO. The one-CGU
release setting is not propagated to user crates. No bootstrap settings were
changed and no expensive rustc rebuild was started.

The inspected stage1 artifact advertises the native `--jobs`, `--jobs-frontend`,
`--jobs-backend`, and `--jobs-linker` flags. Saved release-stage2 identifies as
`rustc 1.101.0-nightly (910e39a6c 2026-10-14)`. `/usr/bin/rustc` separately
identifies as stable `1.99.1 (58cf8f9c3 2026-10-13)`. These identities do not
establish current hosted compiler selection or source-cohort identity.

## Prior measurements, including contrary evidence

All numbers below are historical evidence, not fresh before/after results.

- `bench/results/factors-20261011-051155-nightly-flags-b.json` and companion
  `factors-20261011-045132-nightly-flags-a.json`: Desktop thread16 versus thread8
  6.15 versus 5.61 s, thread4 versus thread8 5.42 versus 5.38 s. Neither supports
  increasing the eight-thread project wrapper. These are targeted scenarios.
- `bench/results/threads-20261012-051915.json`: apparent Desktop thread1/8/16
  6.30/4.66/5.02 s had only one clean thread8 sample. The repeat
  `threads-20261012-055648.json` had seven samples per side, all noisy: thread1
  7.54 s versus thread8 8.11 s and CPU 6.9 versus 9.0 s. Do not present the first
  sweep as an established frontend speedup.
- `docs/hosted-speed-20261013.md`, explicit thread1 versus thread8: reference
  7.201 versus 6.819 s, both within-noise comparison. This is not a broad gate.
- `docs/hosted-workspace-threads-20261013.md`: actual workspace flags verified,
  seven samples each. Thread8 versus thread6 body check 2.419 versus 2.581 s,
  reference edit 5.008 versus 4.968 s, both within noise. Six-thread default
  rejected. Real type errors retained their diagnostics at both settings.
- `bench/results/threads-20261014-004215-remote-m8azn.metal-24xl.json`: five
  samples, pinned 32 cores on the many-core host, stock nightly. Desktop unit
  thread1 versus thread16 4.73 versus 4.45 s. Macro expansion 1.27 versus 1.30 s,
  resolution 0.36 versus 0.35 s, persist 0.43 versus 0.45 s. Borrow check scales,
  but remains a small fraction of this workload.
- `bench/results/factors-20261012-223235-b687ec7aa473-dirty.json`: parallel
  upstream-monomorphization prototype cut collector walk 0.64 to 0.25 s, but
  total loop on/off 7.42/7.33 s and CPU 8.87/7.73 s, every sample noisy.
  The historical prototype is not present under its named environment switch
  in current owned compiler source. It is a rerun candidate, not a shipped win.

None of these targeted historical runs replaces the current comprehensive
session-weighted synthetic before/after promotion gate.

## Focused native Cargo experiment

`toolchain/cargo/src/compiler/jr_frontend.rs` selects `-Zthreads=N` for
`JUSTRUST_FRONTEND_THREADS=N` with **1..255**. Unset, 0, malformed, or out-of-range
values add nothing. Unlike rustc's own numeric parser, this environment switch
intentionally uses 0 to disable the experiment, not to request all CPUs.

The policy is applied only to actual workspace member compilations with actual
nightly/dev rustc. It excludes standard-library units, build scripts, rustdoc,
doctests and doc scraping. Nonmember path dependencies and registry crates are
unchanged. Stable/beta remain unchanged even if Cargo itself is nightly or
`RUSTC_BOOTSTRAP` is set. This version intentionally does not opt stable in.
Workspace proc-macro crates are eligible, their nonmember dependencies are not.

Explicit frontend/global job controls in resolved rustflags, profile rustflags,
or trailing `cargo rustc` arguments suppress automatic selection. Both split
and joined spellings are recognized. Argument files conservatively suppress
selection instead of being parsed by a second parser. A wrapper can still add
flags after Cargo constructs its command, so wrapper-specific runtime controls
must remain coordinated with this opt-in, not silently overridden.

The same selection function is used by `build_base_args` and fingerprint
calculation. Switching the selected count rebuilds affected workspace units,
while dependencies and build-script compiler units remain fresh. Leaving the
opt-in unchanged should be a no-op. Neither global `RUSTFLAGS` nor the native
compiler default is changed. Cargo's existing `[env]` configuration is supported:

```toml
# .cargo/config.toml, only after measuring this workspace
[env]
JUSTRUST_FRONTEND_THREADS = "8"
```

A process environment value overrides this nonforced config entry. Using
`{ value = "8", force = true }` follows Cargo's existing force semantics.
The resolved configured value is used by both command construction and
fingerprints. No new Cargo TOML schema is introduced.

## Viable missing features, not blanket configuration switches

1. **Per-workspace native scheduling policy:** the new opt-in fills the missing
   native Cargo command-selection path for unwrapped workspace crates. Promote
   a count or adaptive policy only after representative warm/cold debug/release
   and check/test comparison, including contention and first-error latency.
2. **End-to-end parallel CGU generation:**
   `compiler/rustc_codegen_ssa/src/base.rs` already precompiles the first batch
   of nonreused CGUs with `par_map` when frontend threading is enabled, then
   feeds remaining CGUs through the existing scheduler. A wholesale claim that
   LLVM IR generation is serial is no longer accurate. A redesigned end-to-end
   queue is a possible measured improvement, not a missing enable flag.
3. **Macro expansion and resolution:** existing query parallelism does not make
   all expansion/name-resolution work parallel. Proc-macro parallel execution
   needs correctness-preserving integration, not an increased worker count.
4. **Upstream monomorphization prefetch:** historical isolated pass win warrants
   a quiet representative rerun before implementation/promotion.
5. **Non-LLD linker worker coordination:** explicit native thread control is
   currently LLD-specific. Supporting an owned linker must preserve linker
   semantics and prove a wall-time advantage, not just remove a shim.

## Validation record

- Hosted final `justrust check --lib --locked` passed, run
  `20261029-181924450-1151557`.
- Three native policy unit tests passed, run `20261029-181953778-1151556`.
- Four real Cargo integration tests passed, zero ignored, run
  `20261029-181936340-1152626`: workspace/nonmember/build-script scope, 4-to-2
  fingerprint invalidation with unaffected units fresh, repeated fresh builds,
  explicit controls, and Cargo config/process/force precedence.
- Full hosted Cargo library suite passed 236 tests before the final
  env-config-only addition, run `20261029-181747992-1144347`.
- Strict library Clippy failed on ten existing unrelated violations in
  `fingerprint/jr_cutoff.rs`, the existing cutoff section of
  `fingerprint/mod.rs`, and `jr_daemon.rs`, run `20261029-181802247-1144346`.
  No warning originated in the new policy. This is not a clean strict-lint gate.
- `cargo fmt --all --check` and `git diff --check` passed.
- Local installable debug candidate: `toolchain/cargo/target/debug/cargo`,
  SHA256 `3b82792313c4ad335cd663f378a2ba23ab8738bd3d7c85ac17ec737b2ffc9a2c`.
  It is not installed over shipping Cargo, is not optimized, and its Cargo
  release identity was not rebuilt with `build-jr.sh` distribution overrides.
  Use a properly versioned release build for any distribution/default promotion.

The first plain debug build unexpectedly routed locally by the existing
policy (debug and cross builds stay local), run
`20261029-181622560-1136970`, 41.22 s, October 30 01:16:22-01:17:04 UTC. The
coordinator was notified to exclude any overlapping local timing samples.
The final incremental local candidate rebuild was explicitly coordinated.
No expensive rustc build, shipping toolchain replacement, global default
promotion, or general speedup claim is part of this change. Run summaries
were exported using `justrust export` for the coordinator to commit alongside
the session-wide measurements.
