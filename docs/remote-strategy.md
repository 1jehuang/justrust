# Remote strategy

Why justrust should compile remotely, what remote builds can and cannot speed
up, and how the product and licensing should be structured. The machine itself
is covered in [remote.md](remote.md). The phased plan is in
[toolchain.md](toolchain.md).

## Where compile time goes (measured)

Measured on justrust itself (47 dependencies, 16-core laptop) with
`JUSTRUST_PASSES=always JUSTRUST_DEPCACHE=0`. Codegen means rustc generating
LLVM IR plus LLVM's own work.

| Scenario | Codegen share | LLVM alone | Link |
|---|---|---|---|
| Clean debug, local crate | 60% (3.7 of 6.2s) | ~15% (IR generation was 2.8s) | 8% |
| Clean debug, all crates | 29% (10.8 of 36.9s) | ~9% | 3% |
| Clean release, local crate | 80% (9.6 of 12.0s) | ~70% (passes 3.4s, ThinLTO 5.0s) | 2% |
| Clean release, all crates | 50% (47 of 94s) | ~45% | 1% |
| Debug rebuild after `touch` | 17% | small | **51%** |

Conclusions:

- The LLVM optimizer only dominates release builds.
- The debug edit loop is mostly the frontend (single-threaded, re-checks more
  than the edit touched), rustc's IR generation, and linking.
- Clean builds are dominated by dependencies: 17.6s of 36.9s of rustc time.
- Today justrust's codegen bucket merges IR generation with LLVM. Splitting
  them is a follow-up.

## What remote can and cannot win

Round trip: about 60 ms in our own measurement, and 29 to 42 ms to us-west-2
per [remote.md](remote.md). Planning uses 60 ms as the worst case. With a persistent connection an
edit costs 2 to 3 round trips (120 to 180 ms). Latency is not the limit.

| Workload | Remote gain | Why |
|---|---|---|
| Single-crate edit loop | Small, 1.2 to 2x | Mostly a single-threaded frontend. More cores barely help, but a faster, unthrottled CPU does |
| `check`, `clippy`, remote `test` | Good when local runs take more than ~0.2s | Only diagnostics and results come back, a few KB |
| `build` that runs locally (Desktop hot reload) | Limited by bandwidth | The artifact must come back. Use binary diffs and split debug info |
| Clean builds, new checkouts, agent slots, CI | Large, near instant | Dependencies come from the shared cache |
| Many agents building at once | Large | Builds scale on the server instead of fighting over 16 laptop cores |
| Release builds and tests | Medium to large | LLVM, ThinLTO, and tests parallelize well |

The pitch is not "compile my edit faster." It is: never compile dependencies,
scale out parallel agents, and offload heavy builds and storage.

## Economies of scale: the shared cache

Most of the value comes from caching across users and projects.

| Artifact | Shared across | Value |
|---|---|---|
| Popular crates.io dependencies | All users | Large |
| Proc macros and build script outputs | All users | High (`syn`-heavy crates are slow) |
| `.rmeta` only, for `check`/`clippy` | All users | Very high, small to transfer |
| Whole dependency set keyed by lockfile hash | Users of one project | One download replaces the dependency build |
| Own crates at a commit | One team, its agents, CI | Big for swarms |
| Own crate after an edit | Nobody | Stays in the edit loop |

Why it improves with scale:

- Usage follows a power law. A few thousand crates cover most builds, so hit
  rates rise quickly with users.
- The marginal cost of a hit is storage and egress only.
- justrust sets the flags, so it can standardize toolchains, profiles, and
  remapped paths. Fragmentation is the main enemy of hit rate.
- A self-hosted copy starts with an empty cache. The full global cache is the
  moat.

Pre-building:

- On demand: the first miss builds once, everyone after gets it free.
- Proactive: on each stable rustc release, build the top N crates using the
  feature combinations users actually request, learned from misses.

Hard parts:

- **Hermeticity.** Some build scripts probe system libraries, read the
  environment, or embed absolute paths. The cache key must capture every input
  that can change the output, and units that cannot be made hermetic stay out
  of the shared tier.
- **Poisoning.** The shared tier holds only artifacts our servers built from
  crates.io sources. Never artifacts uploaded by users. Private code is
  isolated per organization.

## Artifact size: users stop managing target dirs

Rust artifacts are huge. Measured on the dev laptop with `du`:

| Directory | Size |
|---|---|
| `jcode-desktop/target` | 940 GB |
| `jcode/target` | 920 GB |
| `~/.justrust` (shared dependency cache) | 22 GB |
| `justrust/target` | 4.5 GB |

The partition (btrfs) reports only 842 GB used in total, so these directories
share blocks and `du` counts the shared blocks once per directory. Even so,
build output is likely the largest single use of the disk.

Causes: debug info, incremental caches that keep growing, monomorphized
generics copied into each crate, and one copy per target dir, profile, and
agent slot. Swarms multiply all of it.

Remote moves storage to the server:

- Dependencies, `.rmeta` files, incremental caches, release builds, and test
  binaries live on the server.
- `check`, `clippy`, and remote `test` leave almost nothing on the local disk.
- Only artifacts the user runs locally come back, as binary diffs where
  possible.
- Users never need `cargo clean` or `cargo sweep`, and swarms stop filling
  disks.

What we take on: server storage costs, LRU eviction, expiry of old
incremental state, deduplication, and a local fallback for offline work.

Follow-up: break down `jcode-desktop/target` into incremental caches, debug
info, and duplicate slots to measure what a zero-target-dir mode saves.

## Open source and licensing

Decision: open core.

**Open source:**

- The justrust client and local tool. It drives adoption and earns trust.
- The wire protocol and cache key format.
- A self-hostable, single-tenant server: remote executor plus artifact cache.
  Users send source code to it, so it must be auditable. Teams that self-host
  are future customers.

**Closed, at least for now:**

- The multi-tenant control plane: the global shared cache, the prebuild
  fleet, poisoning defenses, tenant isolation, scheduling, and billing.

Reasoning:

- The value is in operations and in the network effect of the cache, not in
  the code. A self-hosted copy starts empty.
- Opening the poisoning and isolation layer before it is proven publishes the
  attack surface.
- Comparable products follow this split: Turborepo with Vercel Remote Cache,
  Nx with Nx Cloud, Bazel with BuildBuddy or EngFlow, and sccache's open
  storage backends.
- If more is opened later, use FSL or AGPL so cloud providers cannot resell
  it as a competing service.

Share versus total: open source captures a smaller share of the value created
but likely a larger total. Local build speedups were never chargeable. Hosted
cache, compute, and team features are.

## Next steps

1. Split justrust's codegen bucket into IR generation and LLVM.
2. Time a real one-line edit in `jcode-desktop-ui` with full pass timings.
3. Measure the composition of `jcode-desktop/target`.
4. Use recorded runs to measure how much dependency build time a global cache
   would have served.
5. Design cache keys and hermeticity rules for a server-built shared tier.
