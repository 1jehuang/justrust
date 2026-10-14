# Agent Instructions for justrust

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

- **The vendored toolchain.** rustc (pinned nightly, then our fork), LLVM,
  cargo, std/sysroot, codegen backends (cranelift), the linker (wild, mold),
  the C toolchain and -sys library sysroot, rust-analyzer, and the test
  runner. Pinned and shipped by justrust, so we control every flag and every
  cache key.
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

## Related repositories (where things live)

| Piece | Repo |
|---|---|
| This client, local depcache, cache key scheme, remote cache wire format (MIT) | here (`src/depcache.rs`, `src/depcache_remote.rs`) |
| Self-hostable single-tenant server (FSL) | `server/` here |
| **Multitenant cloud**: hosted build hosts, global shared dependency cache (R2), prebuild publish, tenant isolation, poisoning defenses | `~/justrust-cloud` (private, github.com/1jehuang/justrust-cloud, `build.jcode.sh`) |
| Accounts, billing, API keys, `/internal/builds/*` | `~/solosystems-backend` (`api.jcode.sh`) |
| cargo fork (`jr-cargo`): build, install, rebase steps in its `JUSTRUST.md`. Submodule pointer `toolchain/cargo` here pins the commit justrust expects | `~/jr-cargo` (private, github.com/1jehuang/jr-cargo, branch `justrust`) |
| rustc fork (`jr-rustc`): `./build-jr.sh`, rebase steps in its `JUSTRUST.md`. Submodule pointer `toolchain/rustc` here pins the commit justrust expects | `~/jr-rustc` (private, github.com/1jehuang/jr-rustc, branch `justrust`) |

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
