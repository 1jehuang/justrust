# Build-directory contention

Cargo serializes commands that write the same profile's build directory. A
warm check can therefore wait behind an unrelated test build even when its own
compiler work would take only a few seconds. `justrust show <run-id>` separates
that wait from compilation, source synchronization, and hosted admission. The
`lock_wait` finding also distinguishes build-directory locks from the shared
Cargo registry/package-cache locks.

## Isolate mutable build state, share cached results

Agent-mode checks, tests, and clippy commands can use a private target directory
under `target/justrust-slots/`. Each slot has an exclusive process-lifetime lock.
Session affinity preserves incremental state, and compatible idle slots can be
reused by subsequent sessions. `JUSTRUST_SLOTS` bounds the number of concurrent
private directories. When all slots are occupied, commands retain the existing
shared-target fallback. Identical-request joining is a separate optimization,
not a build-directory lock wait.

Isolation must not depend on the hosted filesystem supporting reflinks. On a
copy-on-write filesystem, seeding can cheaply clone the shared artifacts. On
ext4, copying an entire target directory, especially its incremental caches,
can cost more time and space than it saves. The non-reflink path therefore
keeps an uncontended command on the shared fast path. When the shared directory
is busy, it creates a private slot without copying artifacts or incremental
state. Subsequent commands reuse that warm private profile. The availability
probe is nonblocking and advisory, so two simultaneous starts can still race
onto the shared directory, where Cargo's own lock preserves correctness.
A busy source directory is never copied while Cargo is mutating it. Starting a
private slot without a seed is valid: Cargo checks and rebuilds its contents
normally, using the existing dependency/artifact caches where possible.

Seeding and refresh require nonblocking exclusion of shared-directory writers.
A shared build-directory lock is insufficient when native fine-grained locking
is enabled, because those Cargo processes intentionally hold a shared directory
lock while mutating individual units. Legacy Cargo's directory-lock protocol
must also remain protected. Inability to obtain a safe snapshot must not become
another unbounded lock wait.

## External contract

- User-selected target/build directories remain authoritative. Isolation must
  not silently override them.
- Native fine-grained locking bypasses slots. A separately configured Cargo
  build directory also bypasses slots because a target-profile lock cannot
  protect mutable artifacts stored elsewhere.
- Local `build` and `run` retain their normal output locations by default.
- Never hardlink mutable fingerprints, build-script outputs, or incremental
  files between directories. An edit or failed build in one slot must not
  corrupt another slot or the shared directory.
- Hosted build isolation additionally requires a run-bound artifact handoff.
  Resolving a slot from a recorded run is not enough if another command can
  overwrite the slot before the executable is fetched. Return the requested
  run's verified output or fail visibly, never another command's binary.
- Cache and recording failures must not change Cargo's diagnostics or result.

## Validation

`justrust test --lib slots::` includes subprocess acquisition tests with an
explicitly failed reflink probe. They exercise the actual Cargo metadata and
OS locking paths independently of the test host's filesystem: idle shared
reuse, native and legacy shared-directory contention, empty private allocation,
sticky and cross-session warm reuse, and bounded-capacity shared fallback.
They also verify that private allocation leaves shared artifact bytes intact.
These tests do not measure compiler work or prove a contention speedup.

Use the controlled contention fixture in `bench/contention.py` for a focused
hosted comparison. It runs actual Cargo commands against a small, byte-pinned
workspace, starts the contender only after the holder confirms the real target
lock is held, and checks both test results and shared-artifact integrity. Results
are recorded and can be compared with `bench/suite.py compare`.

This is a contention-specific acceptance test, not a general compile-speed
claim. Default promotion and broad speed claims still require the comprehensive
session-weighted synthetic gate described in `bench/README.md`, plus relevant
real-workload checks. Report a blocked broad gate as blocked.
