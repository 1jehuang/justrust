# Stronger caching across the source-to-result pipeline

This is an implementation inventory, not a claim that every possible cache has
been enumerated or that all elapsed time can disappear. Cache hit count is not
the objective. Expected agent-visible wait, correctness, memory and transfer
cost are the objective. Existing upstream boundaries do not constrain the
native design.

## Why a cached command can still be slow

A result can be unchanged while the system repeats expensive work to discover
that fact. A green dependency node does not necessarily have its value retained.
A cached compiler artifact does not cache the Cargo graph, lint traversal,
filesystem validation, remote admission or transport around it. Those are
separate opportunities with different invalidation requirements.

In the concrete Clippy investigation, a warm command used 0.04 s inside Cargo
but 1.57 s end-to-end (`20261029-201238735-1925361`). Another actual-edit lint
used 2.25 s compiler time, including 0.595 s expansion, 0.181 s resolution,
0.171 s type checking, 0.056 s borrow checking and 0.366 s crate lint traversal
(`20261029-200741491-1880584`). These are different states, not a controlled
compiler before/after comparison. See `clippy-performance-20261029.md`.

## Inventory and implementation boundaries

“Present” describes a real implementation, not necessarily the selected hosted
binary. Source availability, experimental enablement, installed toolchain and
hosted activation must be reported separately.

| Level | Present today | Missing or incomplete stronger form | Correctness boundary |
| --- | --- | --- | --- |
| Source identities | Per-root source manifests, Git listing, stat scan, early watcher pushes | Durable content/Merkle snapshots, cheap validated generations, narrower per-action input closure | Watcher overflow/loss, open/mmap writes, renames, symlinks, ignored inputs, directory and failed-path probes must invalidate; watcher silence alone is not proof |
| Request coalescing | Local `buildd` shares matching in-flight requests; one-shot prebuild reuse; build slots can join matching runs | Native action-level sharing across slots/hosts without running duplicate compiler work | Exact command/env/toolchain/input epoch, cancellation, output replay and Cargo pipelining; do not accidentally share mutable test effects |
| Whole-command results | One-shot local prebuild results, not a general hosted result cache | Reuse successful and deterministic failed check/lint answers locally before any remote execution | Need an authoritative complete input receipt, including external and negative dependencies; no hosted service use on a local hit, no fake admission or billing bypass |
| Cargo planning | Native resolution stamps/prefetch and an opt-in fork-after-`create_bcx` daemon | Broader retained resolution/unit graph state, cheaper validation, Clippy-compatible native planning reuse | Manifest/config/lockfile changes, target auto-discovery, feature graph, compiler changes, mutable registry state and offline/locked behavior |
| Cargo freshness validation | Per-run mtime caches, native fingerprints, optional checksum freshness, interface cutoff | Share overlapping directory traversal and dependency checks within a build; artifactless validation receipts for safe result replay | Cache only within the valid validation epoch; include directory maxima and exclusion semantics, do not trust a source-only stamp |
| Dependency artifacts | Versioned content-addressed artifacts, machine/prebuilt/account/peer tiers | Broader useful prebuild coverage, metadata-only delivery, concurrent/lazy materialization, better cost-based placement | Toolchain/flags/target/native ABI identity, exact artifact hashes, tenant isolation; a download slower than compiling is not a win |
| Workspace artifacts | Content-addressed local/path units, source reverts, same-path cross-slot reuse | Exact-target fallback is opt-in; general cross-checkout reuse and cross-mode metadata reuse remain incomplete | Runtime embedded target paths cannot be advertised as portable; source/debug paths and proc-macro outputs must have correct current-checkout semantics |
| Build-script executions | Registry/git script-output cache with declared rerun inputs, selected native toolchain/environment stamps | Native Cargo-owned cache, workspace script runs, complete native-system dependencies | Scripts can read arbitrary files, directory listings, environment, clock, randomness and network; cache only under a declared/tracked contract, not presumed purity |
| Proc-macro expansion | Native derive cache flag where the selected JR compiler supports it | Attribute/function-like expansion reuse and durable expansion graph | Input tokens, config, macro binary and dynamic dependencies, spans/hygiene, diagnostics, external reads and ambient state; binary identity alone is insufficient |
| Parse/expand/resolve | Some query reuse, but substantial whole-crate work remains | Stable syntax/expanded-item representations with incrementally reusable resolution and lowering | Stable node/source identities, cfg, macro hygiene, name lookup/negative dependencies and diagnostic generations |
| Semantic query values | In-memory memoization inside one compiler session; selected disk values across sessions | Retain more proven-green values only when cheaper than recomputation; generation-safe resident semantic graph | Stable encoding or valid arena ownership, compiler/schema identity, side effects/diagnostics, bounded memory and eviction; never reuse references to freed arenas |
| Invalidation precision | Red-green backdating, interface fingerprints/cutoff | Finer owner/body/field dependencies and durable high-stability epochs | Unchanged output discovered after execution is not proof execution could have been skipped; invalidation edges must be sound |
| Lint results | Rust module lint queries; Clippy has a combined crate-wide late pass | Query-cached module/owner-local Clippy passes and replayable diagnostics | Stateful passes, cross-module aggregates, lint configuration and levels, shared early-pass collectors, spans and dependency reads; moving the entire pass is unsound |
| Monomorphized IR/codegen | Rust incremental CGU work products, compiled dependency artifacts | Smaller stable per-instance IR/object caches and reuse across compatible sessions/projects | Generic substitutions, ABI/layout, target CPU/features, compiler/backend identity and optimization context; preserve native symbols and relocations |
| Optimization/ThinLTO | LLVM/rustc incremental mechanisms exist, selection is toolchain/config dependent | Native cost-tested backend/ThinLTO cache retention and imported-summary reuse | Imported call graph, optimization flags/profile inputs, target identity and output equivalence; do not turn off optimization to claim a cache win |
| Linking/materialization | Fast linker selection, reflink restoration, experimental hotpatching | Native incremental link state, smaller relocatable changes, reusable test process images | Symbol/COMDAT/section layout, constructors/TLS/unwind state, native libraries, dlopen and test process isolation |
| Test results | Parallel pool, failure-first scheduling, split/confirmation, experimental selection | Per-test dependency/result cache for hermetic tests, affected-test execution | Same binary is insufficient: fixtures, filesystem, environment, network, time, randomness and shared mutable state matter; arbitrary test results cannot be safely memoized |
| Multiple historical states | Artifact cache retains multiple source outcomes, native incremental state has bounded history | Content-addressed query/IR history across branch switches/reverts with memory/disk budgets | No incompatible-session value reuse; retain useful states by saved wait per byte rather than indiscriminate storage |

## Implementation priorities for this investigation

1. **Native freshness validation reuse.** Eliminate repeated work proving that
   overlapping file/directory dependencies did not change. This is on the
   no-op path and does not require trusting an incomplete command-result key.
   Owner: native Cargo. Keep a request-bounded validation epoch and regression
   tests for overlapping roots, directory entry changes and exclusions.
2. **Retain measured semantic query results.** Extend the existing native
   green-query experiments to concrete expensive, disk-serializable families.
   Owner: compiler query definitions/codecs. Verify ordered cold/unchanged/body
   edit/revert/type-error sequences and exact diagnostics with verify-ich.
3. **Make module-local Clippy work incremental.** Introduce a bounded audited
   subset, not an unsafe conversion of the entire combined pass. Owner: native
   Clippy registration and pass factories. Gate on diagnostic equivalence and
   repeated cross-module edits before default promotion.
4. **Evaluate exact-target workspace artifacts.** Existing opt-in tier can
   reuse binaries that the portable tier correctly rejects. Keep namespaces
   and exact target/profile identity separate. Do not promote by default
   without the comprehensive gate. Do not call existing code a newly invented
   cache level.
5. **Authoritative command-result receipts.** Native Cargo/compiler must emit
   the evidence needed for safe local success/error replay. This is the path
   that can remove the hosted round trip entirely on unchanged lint results.
   It is not implemented merely by retaining terminal output or trusting
   `buildd::source_stamp`.
6. **Resident frontend and full expansion graph.** Retain valid semantic state
   across edits, including expansion/resolution and diagnostic generations.
   This is a substantial native compiler architecture change, not a wrapper
   or a warm process that only avoids loading a dep graph.
7. **Hermetic action boundary.** Make build-script/proc-macro/test inputs
   explicit or sandbox-recorded, including failed probes and directory reads.
   Only then broaden these result caches safely. Clock/network effects need
   an explicit policy, not a claim that file dep-info captures everything.

These priorities are based on measured paths and implementation feasibility.
A successful local experiment is not automatically a deployment decision.
Production selection remains per-command and cost-based, with cold builds,
contention, memory and eviction included in the measurements.

## Why not just cache every mtime or every successful stdout?

* Metadata-only persistent source hashes can miss same-size/same-mtime changes,
  file replacement and changes through already-mapped writable files. A durable
  source authority is required before replacing content validation.
* Do not carry a source-hash memo across a remote download without validating
  again. Sources can change while the fetch is in flight. Existing remote
  artifact paths deliberately revalidate inputs before restoration.
* A local source-tree stamp does not capture arbitrary external `include!`
  inputs, build-script/proc-macro ambient reads, native dependencies or missing
  file probes. It is insufficient to authorize arbitrary completed-result
  replay.
* A green query whose value was not retained may rerun. More serialization is
  not automatically faster: compare encode/decode/validation and memory cost
  against the avoided provider work.
* Cached test output is not a test execution. Preserve explicit rerun
  semantics and external effects unless the test has a hermetic cache contract.
* Stable and stock fallback formats remain separate from private native ones.
  Version cache schemas/toolchain identities before incompatible changes.

## Acceptance and measurements

For every candidate: state the exact reuse scope, input/negative-dependency
closure, cache identity and eviction behavior. Compare ordinary execution
against the candidate under matched toolchains and source snapshots. Check
successful artifact bytes/runtime and error/warning text/status. Include
same-content touches, actual edits, reverts, new/deleted modules, cfg/features,
macros, external files, native libraries, cancellation and cache corruption
where relevant.

Use the pinned comprehensive `bench/suite.py run --synth 1 -n 1` comparison
for general speed claims/default promotion. Hosted synthetic measurement uses
`bench/versus.py --remote`. Named scenarios and direct compiler fixtures are
focused diagnostics, not replacements for the broad gate. Existing benchmarks
share a snapshot lock; wait/coordinate rather than mutating their inputs.
Record an unavailable gate as unavailable and keep unproven experiments off.

## Implemented experiments and activation status

The investigation now has concrete native implementations, not only proposals:

* **Associated-item query retention:** `84c0f3e16` adds tracked, default-off
  `-Zjr-cache-associated-items=yes`. Ordered real-compiler acceptance covered
  34 cases and five existing incremental fixtures in each mode. Proven-green
  provider executions fell from 10 to 0, at a measured 1,522-byte query-cache
  increase. This is mechanism evidence, not an end-to-end speed claim. See
  `associated-item-retention-20261030.md` for the matching private sysroot.
* **Overlapping Cargo input validation:** `a93a0de7c` implements invocation-local
  subtree maxima with symlink/error fallback. `b0878ebcc` gates native use on
  explicit `JUSTRUST_CARGO_SUBTREE_CACHE=1`; client forwarding is committed in
  `90b66b89d`. The same final binary, 32 samples per mode/scenario, measured
  overlapping roots at 44.99 to 31.11 ms but a single root at 35.37 to 39.16 ms
  (a regression). Default-unset/off/on reused artifacts, and edit/add/delete
  checks rebuilt correctly. See `cargo-subtree-cache.md`. The regression and
  missing comprehensive gate prohibit default promotion. Forwarding the flag
  does not install a new Cargo binary on hosted machines.
* **Module-local Clippy queries:** `a29b1e69a` adds the opt-in subset
  `NegMultiply` and `NeedlessQuestionMark`. All 48 actual-driver invocations
  passed, including ordered edits, errors/recovery, diagnostic replay and
  four same-directory cfg switches. Official UI passed 1,877 main cases
  (five ignored), 194 secondary cases, and the other groups without changing
  snapshots. Self-profile recorded `lint_mod` executions at three cold, zero
  unchanged, and two after an edit confined to one module, preserving replayed
  warnings from the other module. Switching cfg alone had exposed stale green
  queries, so the driver uses a versioned incremental-cache namespace. The
  initial four-pass subset reordered two UI diagnostics, so `NeedlessBool`
  and `BoolComparison` remain crate-wide. Stateful and cross-module passes
  remain crate-wide too. See `clippy-module-cache-experiment.md`. Default off,
  with no end-to-end speed claim.

* **Native input-receipt prerequisite:** `c6004a097` adds an explicit local,
  opt-in producer at Cargo's actual freshness decision. A bounded fresh leaf
  check can record dep-info inputs, hashed environment and output context with
  a caller nonce and private atomic output. Four actual native CLI tests pass.
  Every record remains `trusted: false` and `command_replay_eligible: false`:
  planning, config, discovery/negative inputs, toolchain and remote path mapping
  are not complete. No client consumer or hosted forwarding is installed.
  See `cargo-freshness-receipt.md`. This adds evidence production, not a
  finished local result cache or a latency improvement.

The comprehensive baseline was unavailable because another measurement held
its shared snapshot lock, including a ten-minute timed-out wait. These
experiments remain default-off. Focused acceptance cannot substitute for the
session-weighted benchmark needed to promote defaults.

None of these changes implements authoritative whole-command result replay.
The reinstalled shipping client passed 831 tests with six ignored
(`20261029-205934738-2526594`), 20 transport tests
(`20261029-205606640-2479618`) and all-target Clippy with warnings denied
(`20261029-205609282-2475935`). That no-op lint still took 2.58 s on the
client despite 0.05 s on hosted Cargo and 62 fresh units, with 1.94 s reported
in synchronization. This is a different run under concurrent toolchain work,
not a matched regression comparison, but it confirms that artifact freshness
alone does not make the entire command subsecond.
A truly local unchanged-result hit is the way to avoid both remote admission
and transport. That still requires complete positive/negative input receipts,
not a source-only timestamp or a cached terminal transcript.

## Shipped warm-sync follow-through

Two small shipping-client changes now reuse already-proven structure without
skipping validation: `f3a7dc752` retains derived watch-directory paths until
manifest membership changes, and `1c3656078` merges sorted manifests instead
of performing two sets of tree lookups. Watch registration is still retried,
every command still rescans inputs, and the remote barrier remains mandatory.
See `watch-directory-reuse.md` and `remote-sync-linear-diff.md` for scoped
component diagnostics and exact-oracle coverage.

After installation, real hosted all-target Clippy passed with 62 fresh units:
`20261029-211321346-2704621` took 1.4 s (869 ms sync), followed by
`20261029-211322726-2703167` at 1.2 s (673 ms sync), with 0.03 s Cargo in each.
These are verification observations under concurrent work, not matched suite
speedup evidence. Source validation and admission still prevent a demonstrated
subsecond result. Full shipping tests passed 834 cases with eight ignored.

The follow-up `edce4013a` bulk-builds freshly observed manifests with tested
last-observation duplicate semantics, and `2966ee5a2` overlaps two fresh,
read-only Git discovery queries with synchronous spawn/panic fallback. Neither
adds a persistent source-stamp cache. Both are installed in the shipping client
(SHA-256 `15a4e8525174df5d63b277d47e052632a565802d5cf4ceb3d86aaed7aa48e2bc`).
Bulk construction passed hosted gates. Final concurrent-discovery gates ran
locally during an explicitly coordinated hosted maintenance pause. After its
release, final installed all-target Clippy passed three unchanged hosted runs:
`20261029-214030501-3048523`, `20261029-214031568-3048590`, and
`20261029-214033316-3048801`. Client wall times were 1.1/1.7/1.1 s, sync
306/304/357 ms, with 62 fresh units and 0.03–0.05 s Cargo startup. This is
not yet subsecond end to end. The fleet changed from 96 to 32 cores, so these
are acceptance observations, not a matched before/after speedup claim.

A reversible real body edit rebuilt the expected units and passed 837 tests
(`20261029-214058307-3051393`). Restoring the backup passed 837 tests again
(`20261029-214107578-3052277`), with one parallel DirectoryNotEmpty cleanup
failure that passed the runner's isolated retry. Restored strict all-target
Clippy passed (`20261029-214114111-3052815`). No temporary source edit remains.
The comprehensive matched synthetic gate is still incomplete. The earlier
snapshot lock holder has exited, but the separate retained baseline stopped
at 22/40 points with seven context/pinned-source failures, not a valid baseline
(`query-retention-benchmark-20261030.md`). No partial result permits default
promotion of the native experiments. See
`remote-sync-bulk-scan.md` and `remote-sync-concurrent-discovery.md` for bounded
component evidence.

## Local native-fresh execution follow-through

The default-off `JUSTRUST_BUILDD_NATIVE_EMPTY=1` experiment is installed and
verified. It skips the daemon's recursive source stamp only when its active
and finished candidate maps are globally empty. It still launches Cargo for
every request, so this is removal of redundant daemon validation, not a
whole-command result-cache hit. Unstamped executions cannot authorize output
sharing or enter the finished-result map.

The same-binary controlled local diagnostic measured warm Clippy medians of
751.823 ms off and 173.682 ms on. Separate installed isolated observations
were 164.025 and 136.757 ms, with zero compiled units and 62 fresh units.
These are scoped local observations under contention, not a hosted latency
guarantee or a comprehensive benchmark comparison. The shared daemon correctly
declined the shortcut when another project's candidates existed. See
`daemon-native-empty.md` and `daemon-native-empty-installed.md` for exact run
records, real edit/failure/restore, cancellation, and overlap acceptance.

The native Cargo receipt now checks recorded content checksums in the bounded
content-freshness case, with actual module, include and environment edits
covered. It still explicitly denies whole-command replay authority. A content
digest checked after compilation is not by itself proof that all compiler
reads, planning inputs, failed probes, configuration and toolchain state were
bound to the output. See `cargo-freshness-receipt.md` for that separate producer
contract. No client result-replay consumer or default promotion is implied.

## Primary design references

The implementation inventory above comes from this checkout. These independent
primary references explain the design principles, not proof of our shipped
coverage or speed:

```text
Rust compiler incremental design:
https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation.html
https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
Cargo build cache and fingerprint authority:
https://doc.rust-lang.org/cargo/reference/build-cache.html
https://doc.rust-lang.org/stable/nightly-rustc/cargo/compiler/fingerprint/index.html
Salsa memo values, dependencies, revisions and durability:
https://salsa-rs.github.io/salsa/plumbing/terminology/memo.html
https://salsa-rs.github.io/salsa/plumbing/fetch.html
Bazel action results plus content-addressable output storage:
https://bazel.build/versions/7.1.0/remote/caching
LLVM ThinLTO incremental backend cache:
https://clang.llvm.org/docs/ThinLTO.html
```
