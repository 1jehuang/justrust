# Whole-generation reuse v55: safety prerequisite, not activated

Date: 2026-10-31. This investigation concerns the licensed Blaze-derived
resident engine, not the clean-room compiler.

## Release decision

**Do not enable whole-generation worker retention in this candidate.**
`cargo/whole.rs` and `cargo/daemon.rs` remain byte-for-byte unchanged.
`RA_JIT_WHOLE_RELOAD=1` is not implemented or advertised as functional.
There is no positive retention marker, no changed serialization, and no
change to existing nonwhole `kept` semantics. In particular, do not infer
that an ON request exercises reuse. Both OFF and ON still follow the
original whole-generation restart behavior.

The assigned scope permits documenting a concrete missing prerequisite
instead of setting `kept` without a correctness guarantee. That is the
outcome here. No builds, tests, native workloads, installation, commits,
or performance measurements were run for this candidate.

## Existing reload guarantees and their boundary

* `whole::checkout` computes generation from the newest workspace member
  manifest or root `Cargo.lock` timestamp and unconditionally sets
  `kept: false`. Its `WorkerConfig` includes selection, `cfg_test`, and
  `worker_env()`. Its `command_env` is empty.
* `daemon::warm` selects a worker by `WorkerConfig::id`, retains a matching
  generation, restarts a changed generation when `!checkout.kept`, and
  otherwise sends the existing worker through `Worker::warm` and
  `Worker::ask`. The changed-generation `Warmth::Failed` restart precedes
  the `kept` decision. Existing source diagnostic handling is distinct from
  transport/load failure handling and must remain intact.
* `inputs::Inputs::watch` records manifests, lockfile, Cargo configuration,
  toolchain files, local build-script entry points, flags and script input
  observations. `stale` and `Checkouts::reload` reload workspaces, including
  build-script state, before a request builds. Script variables are supplied
  through the request environment and `Inputs::variables`.
* The main request loop settles builds, snapshots `CratesAsLoaded`, reloads
  the database, computes changed crates, touches rejection/list state, and
  clears `as_built`. These are real guarantees, but not a proof that all
  retained native linker state now describes that database.

## Concrete missing prerequisite: native linker-plan invalidation

The retained `Program` has `own_lines: FxHashMap<PathBuf, String>`.
`Program::outputs` returns the cached command for an executable when one
exists. Its cache key is only the executable path. A fresh command is
constructed by `own_link_line` only on a miss.

`Program::start_over` updates members, debug information, lint capping and
`link_args`, resets compiled state, and distrusts changed crates. It does
**not** clear `own_lines`. Searching all `own_lines` references finds
initialization in new programs, insertion/lookups in `outputs`, and iteration
in `sync`, but no reload invalidation. Merely causing `start_over` therefore
does not establish a fresh native linker command.

This command is not independent of a manifest generation. `link::native`
collects crate `#[link]` attributes and build-script libraries, search paths
and arguments. A manifest feature addition can change the dependency graph,
cfg-selected native declarations, or a build script's output while preserving
the CLI selection and therefore the `WorkerConfig` ID. Even a script-free
current graph does not establish that the next graph has the same native
requirements. A new dependency or newly cfg-enabled declaration can introduce
them. The existing reload route can consequently refresh the database but
still reuse an executable-path-keyed linker command from before reload.
This is a source-level safety gap, not a claimed reproduced failure.

There is an additional fast path in the reload loop: equality of members
and debug information with no rustc-linked target skips `start_over`.
That predicate does not explicitly compare the refreshed `Parts::link_args`
or `lints_capped`. Full native-plan equality is not established by that
predicate either. The process-global `SYSROOT` is also initialized once:
watching a changed toolchain file does not by itself prove safe replacement
of the loaded native ABI and sysroot within that process.

None of these observations justify weakening staleness, changed-crate
invalidation or error reporting. They require stronger invalidation or an
explicit old/new equivalence contract in the native engine. Those changes
belong outside the three owned files of this assignment.

## Why no bounded opt-in or eligibility fixture was added

The client has the new metadata and the daemon has generation/configuration
identity, but neither owned path has the old and new complete native linker
plan or a reload result proving equivalent native ABI inputs. A check of
only current metadata for no scripts, no `links`, or host targeting would
not prove absence of cfg-selected `#[link]` declarations or equivalence to
the previous generation. Treating that check as eligibility would create a
false safety boundary. The only proven eligibility set under the present
scope is empty, and a parser/always-false unit fixture would not test native
reuse. No mock or synthetic eligibility tests are presented as acceptance.

## Required follow-up before enabling

1. Give native reload an explicit invalidation/equivalence contract for
   executable linker lines, link arguments, native libraries/search paths,
   generated artifacts, toolchain/sysroot and ABI. Invalidate retained
   `own_lines` on relevant reloads and cover the same-members fast path.
2. Establish that the client request's environment reaches both initial
   load and reload correctly, including build-script `rerun-if-env-changed`
   variables. Preserve `WorkerConfig::id`, selection and `cfg_test` identity,
   without serializing ambient environment merely to carry the switch.
3. Only after that prerequisite, parse the client startup switch once per
   process with exact `OsStr == "1"` semantics and default OFF. Do not make
   daemon startup environment authoritative for the client's switch.
4. Place a fixed, value-free positive marker only at the actual same-worker,
   changed-generation whole-retention branch. OFF must emit zero markers.
   Keep nonwhole `kept` semantics and failed-load restart behavior unchanged.
5. Add real native fixtures for feature/dependency changes, lockfile changes,
   script/native-link changes, toolchain/environment changes, changed CLI
   selection, diagnostics and failed-load recovery. Root owns actual
   same-PID/reload controls and strict counterordered OFF/ON measurements.

## Measurement context

The coordinator supplied an accepted OFF observation in
`ROOT/cold-check-overlap-v54-off/02-manifest-feature-add-worker-0.log`:
roots approximately 2.5 seconds, compile 3.2 seconds, mandatory check
1.3 seconds. This investigation did not independently collect that evidence.
It motivates the work but is not an ON comparison or a speedup claim.

The source-only freeze directory is
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/whole-generation-reuse-v55`.
It contains full originals and candidates, a patch, SHA-256 inventory and
an explicit release decision. The patch is documentation-only.
