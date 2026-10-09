# Agent Instructions for justrust

justrust is an attempt at a faster all-in-one Rust compiling solution for
coding agents. Every change should make the agent edit, check, and test loop
faster or more informative, and be backed by measurements.

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

When choosing work, prefer what moves the reference loop
(`docs/toolchain.md`, Target) the most. Small, safe wins still ship, but
never stop at "good enough" when a deeper change (a fork, a server, a new
component) would be measurably faster.

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

- Do not claim a speedup without before and after numbers from `justrust show`
  or `bench/edit-loop.sh`.
- Record notable observations in `FINDINGS.md` with the run id.
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
