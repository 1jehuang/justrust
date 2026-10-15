# Agent Instructions for justrust

## Clean-room rewrite worktree policy (2026-10-15)

This checkout is the explicitly requested `clean-room-rewrite` branch/worktree.
For this rewrite, this section supersedes conflicting legacy instructions below.
`ARCHITECTURE.md` and `PROTOTYPE.md` define the current design and first milestone.
Implement in Rust using standard Cargo/rustc/std as bootstrap tools. Selective
third-party utility crates are permitted; keep dependencies minimal. Do not
embed the old compiler, Cargo, rustc, LLVM, or their vendored implementations
as the new compiler's internals. Create a fresh package rather than restoring
the deleted implementation or its dependency graph.

Use the installed stable justrust client for checks/tests/lints and `cargo fmt`
for formatting. Do not install this experimental package over the stable client.
Keep experimental hosted workspaces, binaries and caches separate. Work and
commit/push on `clean-room-rewrite`, not main. Preserve the stable checkout.
Legacy toolchain paths, reinstall requirements, benchmark commands and component
ownership descriptions below describe the old implementation, not requirements
to recreate it. Retained `bench/`, `docs/`, `data/`, CHANGELOG.md and FINDINGS.md
are historical/reference material unless explicitly adopted for the new engine.
Preserve licensing and attribution. No compiler compatibility or speed claims
follow from synthetic graph fixtures.

justrust is Rust compile as a service for coding agents. Every change should make
the agent edit, check, and test loop faster or more informative, and be
backed by measurements.

## Mission: optimal compile times, whatever it takes

We are all in. justrust is becoming one all-in-one solution that owns the
whole path from source to test result, and we will do whatever it takes to
reach optimal compile times. Maintenance burden, forks, and our own
infrastructure are acceptable costs. Do not reject an idea because it means
forking a tool or running servers. Reject it only if measurements show it
is not faster.

What we own, or are working toward owning (plan: `docs/toolchain.md`):

- **The vendored toolchain.** Owned rustc and Cargo source trees live here in
  `toolchain/rustc` and `toolchain/cargo`, including Rust's library sources.
  LLVM, std/sysroot, codegen backends (cranelift), the linker (wild, mold),
  the C toolchain and -sys library sysroot, rust-analyzer, and the test
  runner are part of the end-to-end ownership plan. Source vendoring of our
  compiler and Cargo does not mean every external dependency is already
  vendored or every component has an independently redesigned implementation.
  Pinned and shipped artifacts give us control of flags and cache keys.
- **Prebuilt artifact servers.** A cargo-like registry service that serves
  prebuilt dependency artifacts (rlibs, build-script outputs, proc macros)
  keyed by exact toolchain and flags, so cold builds download instead of
  compiling.
- **A remote compile service.** Server-grade CPUs with a shared cache that
  justrust uses whenever remote is faster than local (cold builds, full
  regenerations, big test suites), and local otherwise. The decision is made
  per build from measured cost.

When choosing work, prefer what reduces expected agent wait across the
session-weighted synthetic workload (`bench/synth/README.md`) the most.
The Desktop one-line test edit is a diagnostic, not the overall benchmark.
Small, safe wins still ship, but
never stop at "good enough" when a deeper change (a fork, a server, a new
component) would be measurably faster.

## Vertically integrate end to end. External correctness is the contract.

justrust is one vertically integrated source-to-test-result system. We own
the entire toolchain end to end, not a collection of upstream-compatible
replacements. Our forks are independent development lines, not patch sets
whose purpose is to stay close to upstream. Completely different internals,
large rewrites, replacement components, and merging formerly separate tools
into one system are all valid. Optimize for measured agent wait, not familiar
component boundaries.

Internal compatibility with upstream is not a goal. Neither are upstreamability,
small diffs, easy rebases, or retaining upstream's architecture. Do not use any
of those as a reason to reject a faster correct design. Redesign compiler/Cargo
protocols, query and dependency graphs, IR and artifact formats, scheduling,
cache interfaces, codegen, linking, and test execution together as needed.
Version the integrated toolchain and its cache keys so incompatible artifacts
cannot be reused. Divergence is not itself a problem to solve.

Preserve the external contract: supported Rust language semantics, existing
project manifests, build-script and proc-macro behavior, native-library and
platform ABIs, build correctness, and useful diagnostics. Stock fallback is
a separate compatibility path, not a restriction on our native architecture;
never feed private formats to stock tools. External correctness is the
compatibility requirement, not internal resemblance to stock tools. Upstream
is an optional source of useful fixes and ideas, not our architectural authority
or release schedule. Evaluate relevant correctness and security fixes and
adapt them to our design. Cherry-pick, reimplement, or rebase only when useful,
never to minimize divergence for its own sake.

## Native toolchain integration, not permanent shims

We own the toolchain: embed functionality in the component that owns the
operation instead of building permanent interception layers around it.
Prefer compiler phase telemetry inside jr-rustc, build-script caching and
artifact-cache orchestration inside jr-cargo, and native linker selection
and fallback inside jr-rustc over shell wrappers, stderr parsing, or linker
dispatcher shims. Shared libraries are fine; embedding does not require
copying cache implementations into multiple repositories.

Shims are transitional compatibility and fail-open mechanisms, not the
end-state architecture. Keep them where stock-toolchain support or a measured
advantage requires them. Do not remove a working shim until its native
replacement preserves diagnostics, exit status, effective build pipelining, cache
correctness, and fallback behavior. Measure representative builds before
and after, including cold and incremental debug/release builds. Removing a
process boundary is not itself evidence of a speedup. Prioritize build
performance; checks and tests remain correctness and regression gates.

## Source ownership and policy

The compiler and Cargo are ordinary source trees in this repository, not
submodules or independently released pinned forks. Edit them here and commit
coordinated changes together with the client, tests, and cache protocols.
`jr-rustc` and `jr-cargo` remain component/binary names, not separate source
repositories that agents should edit or rebase.

This root AGENTS.md is authoritative throughout `toolchain/`. Imported
technical documentation describes existing implementation and test fixtures,
not an upstream architectural mandate or contribution policy. Retain licenses,
attribution, required documentation inputs, and test fixtures. Do not restore
inherited agent policies, separate-repo workflow instructions, or mandatory
upstream rebasing. Build and source-import details are in
`docs/toolchain-vendoring.md`.

## Source locations and related services

| Piece | Location |
|---|---|
| This client, local depcache, cache key scheme, remote cache wire format (MIT) | here (`src/depcache.rs`, `src/depcache_remote.rs`) |
| Self-hostable single-tenant server (FSL) | `server/` here |
| **Multitenant cloud**: hosted build hosts, global shared dependency cache (R2), prebuild publish, tenant isolation, poisoning defenses | `~/justrust-cloud` (private, github.com/1jehuang/justrust-cloud, `build.jcode.sh`) |
| Accounts, billing, API keys, `/internal/builds/*` | `~/solosystems-backend` (`api.jcode.sh`) |
| Cargo (`jr-cargo`): owned source and build helper | `toolchain/cargo/`, `toolchain/cargo/build-jr.sh` |
| Compiler (`jr-rustc`), Rust libraries, and included tools: owned source and build helper | `toolchain/rustc/`, `toolchain/rustc/build-jr.sh` |

Anything multitenant goes in `~/justrust-cloud`, never here (see
`docs/remote-strategy.md`, licensing). Remote cache protocol changes touch
both `src/depcache_remote.rs` here and `src/cache.js` there: change both,
test both (`justrust test`, `npm test`), push both.

## Production changes are allowed

You may change the live system without asking: deploy the worker
(`wrangler deploy` in `~/justrust-cloud`), upload fleet files to S3, trigger
prebuild fleet runs, update hosted machines, run D1 migrations, cut releases.
Do it, verify it worked (logs, a real build, the index), and note what you
changed in the commit or FINDINGS. Still never destroy user data (accounts,
billing, private caches) and never weaken the security invariants in
`~/justrust-cloud/AGENTS.md`.

## Git workflow

- Work directly on `main`. No feature branches.
- **Commit and push as you go.** Each working improvement gets its own commit,
  pushed right away. Do not batch up a session of changes into one push.
- Commit only your own changes.
- Use the user's configured Git identity (Jeremy Huang). Never invent an agent
  identity or override `user.name`/`user.email`.

## Build, test, install

- Use justrust on itself: `justrust check`, `justrust test`, `justrust clippy`.
- `cargo fmt` before committing. Keep `justrust clippy` warning-free.
- After a change that affects behavior, reinstall so every agent on the machine
  gets it: `cargo install --path . --locked`.
- Verify on a real workload, not only unit tests. The main targets are
  `~/jcode-desktop` (`justrust test -p jcode-desktop-ui --lib -- fps_counter`
  after a one-line edit) and this repo (`bench/edit-loop.sh`). Restore any file
  you edit in another repo from a backup copy, not with git.

## Measuring

- Do not claim a speedup without before and after numbers from
  `bench/suite.py` (pinned snapshots, see `bench/README.md`): run it before
  and after, commit both result files, and quote `bench/suite.py compare`.
  `justrust show` explains a single run.
- For general speed claims and default promotion, use the comprehensive
  session-weighted workload: `bench/suite.py run --synth 1 -n 1` before and
  after, then `bench/suite.py compare`. Keep seed, point count, generator,
  weights, snapshots, and execution mode identical. Inspect command/change
  breakdowns for regressions, not just the aggregate mean.
- Use `bench/versus.py run --synth 1 -n 1` for stock-versus-justrust claims.
  Named suite scenarios and `bench/factors.py` are focused diagnostics and
  component attribution, not substitutes for the comprehensive gate. A
  targeted win can ship with an explicitly scoped claim, but a default
  promotion needs the broader comparison and relevant regression checks.
- Supplement the synthetic workload with cold-build, contention, hosted,
  and correctness checks relevant to the change. Its coverage is not universal
  (see `bench/synth/README.md`, Known gaps). If the broad gate is blocked,
  report it as blocked rather than substituting `desktop-test-edit`.
- Record notable observations in `FINDINGS.md` with the run id.
- Run `justrust export` from the repo root and commit `data/runs/` so every
  cited run id exists in the repository (slim, path-redacted summaries, see
  `data/runs/README.md`).
- `justrust history` mines past Jcode sessions; `justrust runs` lists recorded
  builds.

## Design rules

- Recording must never change build results or invalidate cargo or rustc
  caches. Check that a recorded and an unrecorded build share artifacts (no
  `Dirty` units) after touching the shim or environment.
- Fail open: if justrust cannot record, run the real cargo unchanged.
- Agent output: errors and failing tests in full, then the detailed report.
  Never hide an error. The full output is always available through `justrust log`.
- Linux first. Keep `/proc` usage in `procfs.rs`.
