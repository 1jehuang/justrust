# Whole-generation reuse: v55 barrier and v56 native prerequisite

Date: 2026-10-31. This investigation concerns the licensed Blaze-derived
resident engine, not the clean-room compiler.

## Release decision

The v55 investigation below is preserved as historical evidence. The v56
source-only candidate described at the end implements native invalidation
prerequisites in `src/main.rs`. Neither candidate enables generation reuse.

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

## v56: native reload correctness prerequisite, still no activation

The coordinator expanded ownership to `src/main.rs`. This distinct candidate
does not edit either `cargo/whole.rs` or `cargo/daemon.rs`, add a switch,
serialize ambient environment, emit an activation marker or set `kept`.
The v55 frozen artifacts remain unchanged. Whole changed generations still
take the exact original cold restart route, including ABI/toolchain changes.
This is a staged prerequisite, not a claim that warm reuse is ready.

### Retained-state changes

Native programs are identified by an executable output, no rustc symbol files
and an empty symbol table. Rustc-backed/nonwhole programs retain the original
same-members/debug fast path and its known/stale handling.

On native reload, even with equal members/debug, `Program::reloaded` now
uses `start_over` to refresh checkout/base, members, debug, linker arguments
and lint capping. The native-only invalidation in `start_over` then:

* Drops cached `own_lines` and the initial native `args.link_cmd`. Clearing
  only the map would leave the initial executable's line cached in `Args`.
* Cancels pending native splices, waits for active splices to settle and
  removes `LINKED` baselines for native executable/overlay destinations.
  Old executable files are not deleted by this invalidation.
* Resets full-object and last-written state, tests/test-list caches,
  interruption/moved state and widened dependencies. Native `Rejections`
  are reset entirely, preserving patching mode, rather than carrying frozen
  values from a potentially different graph. Builds are already settled by
  the request loop before reload; no running build is invalidated underneath.
* Marks every current owned watched source file for original diagnostics,
  regardless of Rust text or mtime changes. The existing `sync` ownership
  and `listed.sees` filters still exclude files outside the program's build.
* Evicts every `CLEAN` entry for those files. File/crate IDs and unchanged
  body hashes alone do not prove equal semantics after cfg, environment or
  dependency reload.

The request loop also removes native outputs from `harnessed`, ensuring a
manifest-only test-list change regenerates the harness, and resets outline
optimization state. Existing changed-crate computation, `listed.touch`,
`as_built` invalidation, settling/joins, failed reload errors and diagnostics
remain in place. Native state is conservatively retired even if a reload
reports an error after updating part of the database. The next request is
not allowed to treat that partially refreshed native state as an old object.

The retained prebuilt standard-library object and process-global `SYSROOT`
are intentionally not retargeted in place. Native artifacts still depend on
the existing cache/query keys. No claim of complete ABI-change support or
native-worker reuse follows from this prerequisite.

### Fixture source, not acceptance results

Two new unit fixtures live in `src/main.rs`:

* `native_reload_cfg_only_rechecks_unreachable_errors_and_recovers` uses
  the real `RootDatabase`, actual cfg setters and actual `errors_of`. Rust
  bytes remain identical through repeated error/recovery transitions. A
  newly enabled unused function has E0308, and an unchanged body changes
  type through a cfg-selected alias. Diagnostics are compared with an
  independently loaded fresh database, not a mock success result. The same
  fixture exercises real `Program::reloaded` and asserts native retained
  state is retired even with unchanged membership/debug.
* `native_reload_leaves_original_nonwhole_fast_path_unchanged` covers the
  original nonwhole predicate and state retention with a real database.
  Its state-level unit checks are not a rustc-linked executable workload.

No fixture has been executed by the source owner. Root owns compilation,
native Cargo manifest/default-feature transitions, precise diagnostics,
recovery, worker PID/lifecycle controls and any timed comparisons. Merely
running this candidate with `RA_JIT_WHOLE_RELOAD=1` still does not enable reuse.

### Proposed ABI restart guard before a future activation

The daemon must decide whether a process can survive the new load before
retaining it, rather than discovering an ABI mismatch after changing its
database. A future opt-in needs a versioned old/new native-load certificate
covering resolved compiler/toolchain and sysroot identity, host/target spec
and ABI, effective target/rust/link flags, standard-library/prebuilt identity,
and Cargo/toolchain configuration used to resolve them. A path alone, a
manifest timestamp, or `WorkerConfig::id` alone is insufficient. The same
path can be repointed or its contents replaced.

Any changed or unresolvable certificate must go through the existing cold
restart branch, without rejecting an otherwise supported build. Never replace
this with an unsupported-build error. The switch itself must remain exact-1,
client-startup/process-cached, default OFF and not an ambient-env payload.

Concrete further ownership proposed: `crates/ra-jit/src/inputs.rs` to expose
the exact watched configuration/toolchain invalidation reason and
`vendor/rust-analyzer/crates/load-cargo/src/lib.rs` to expose the resolved
native load/sysroot identity alongside the workspace load. Coordinate this
with the already owned checkout/daemon protocol and main request loop.
Certificate reuse must also account for config changes between generations
and worker startup races, not only a changed `Cargo.toml` timestamp. This
proposal is not implemented in v56. The conservative guard implemented for
this stage is still the unchanged `whole::checkout { kept: false }`.

The v56 source-only freeze is in
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/whole-generation-reuse-v56`.
It contains full four-file originals/candidates, a repository-relative patch,
SHA-256 inventory and explicit no-activation release. Only source formatting,
diff inspection and artifact checks are permitted here. No builds, tests,
native workloads, installation, commits or speedup claims were made.

## v57: repair the independent cfg fixture, not production behavior

Root executed the actual v56 gate after the source-only handoff. Check,
native build and codegen gates exited zero, but the resident unit suite had
361 passing tests and one failing test. This is a **failed candidate gate**,
not acceptance. Root's preserved log is
`ROOT/candidate-unit-v56/full-native.log`, lines 23-37. The failed assertion
compared two actual E0308 diagnostics (ranges `142..145` and `205..209`)
against an empty fresh-oracle result. The original v56 source freeze and
failure evidence are not modified by v57.

Source investigation identifies a fixture setup mismatch:

* The live database setter interns feature key `feature` with value `bad`.
* The v56 fresh fixture used metadata `cfg:feature="bad"`.
* `test-utils/src/fixture.rs` parses cfg metadata using comma splitting and
  `split_once('=')`, preserving the value verbatim. `FileMeta::from_fixture`
  in `test-fixture/src/lib.rs` interns that same value without unquoting it.
  Thus the fresh database enabled a feature whose value included the quote
  characters. Rust `#[cfg(feature = "bad")]` did not match it.

The production reloaded database returning both expected errors is evidence
that the tested diagnostic path did not skip these errors in that run.
It is not sufficient evidence of overall production correctness or native
generation reuse. The observed failure is in the independently constructed
fixture's cfg inputs, rather than a demonstrated production invalidation
failure. Root must rerun the repaired full gate before any acceptance claim.

v57 changes only fixture code under `#[cfg(test)]` and this document:

* Use the parser's documented unquoted fixture syntax `cfg:feature=bad`.
* Before invoking oracle diagnostics, assert there is exactly one relevant
  fresh crate, its complete cfg options equal the live database's options,
  and its parsed Rust bytes equal the original unchanged source.
* Preserve exact actual/fresh diagnostic equality, repeated cfg-only
  error/recovery transitions and the requirement for both E0308 diagnostics.
  No expected error is removed and no comparison is relaxed.

Production native invalidation code, cold restart behavior, ABI boundary,
`cargo/whole.rs`, and `cargo/daemon.rs` are unchanged from v56. There is still
no generation-reuse switch or marker. The source owner ran no native tests,
builds or workloads. v57 is a fixture-only source repair, not a speedup or
acceptance claim.

The distinct source-only freeze at
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/whole-generation-reuse-v57`
contains full original/candidate snapshots of the two reacquired owned
paths, an incremental patch relative to v56, SHA inventory and release.

## v58: conservative script-input reload and isolated diagnostic fixtures

### Actual v56/v57 failures, preserved

Root's real native link probe changed only `JR_NATIVE_CHOICE` through A/B/A,
expecting the C archive's native function to return 11/22/11. Rust and
manifest bytes were unchanged and each execution had a fresh runtime nonce.
The v56 candidate produced the correct B result but retained 22 on return
to A. Its cumulative worker log had no second reload. The original engine
did not correctly relink B, so its final 11 was not proof of recovery.
Evidence remains under `ROOT/native-link-reload-v56-{candidate,original}`.
These are correctness observations, not timings establishing a speedup.

Root's complete v57 native gate passed the new cfg-only fixture, but failed
the existing diagnostic skeleton fixture's unchanged callback assertion
(two callbacks versus zero). This is another failed complete gate, not
acceptance. Evidence remains in `ROOT/candidate-unit-v57/full-native.log`.

### Native-only script output was projected away

`Scripts::changed` previously ran scripts and compared `WorkspaceBuildScripts`
with the loaded snapshot. `BuildScriptOutput` equality in project-model
contains cfgs, environment, OUT_DIR and proc-macro paths, but not native link
directives or generated native archive bytes. `link::asked_by_script` reads
native directives separately from stdout on disk. A native-only change can
therefore leave that projection equal while its native link plan changes.
The old equal branch then updated environment witnesses/file stamps and
returned not-stale, without reloading the DB or invalidating native program
state. This consumes a B-to-A change without making the loaded program A.

v58 removes that lossy equality fallback from `inputs.rs`:

* Every observed script environment change immediately requires real reload.
  The environment witness is not mutated by the staleness check. Successful
  `Checkouts::place`/`Inputs::watch` installs the replacement witness.
  Failed/uncommitted reloads cannot consume the change.
* Changed files retain the existing optimization only when
  `Script::reruns_the_same` proves equality using raw stdout and generated
  file digests. That successful proof may refresh file stamps as before.
* Missing or failed raw-output proof requires real workspace reload, rather
  than running scripts and accepting the incomplete cfg/env/OUT_DIR projection.
  File witnesses are not advanced on this branch. The existing real load
  runs scripts and preserves their errors and all native invalidation.

`Scripts` no longer retains unused workspace/config clones for the deleted
fallback. This is intentionally conservative even when changed environment
variables happen to produce identical output. No speed claim is made.

### Shared CLEAN fixture isolation, no weakened oracle

Both diagnostic fixtures allocate fixture-local FileId/crate IDs but access
the process-global `CLEAN` map. Their save/restore guards alone did not prevent
the new fixture's broad FileId eviction from overlapping the existing fixture's
unchanged-body callback assertion. Both now hold the same test-only mutex for
their entire sequences, including save/restore cleanup. Production cache keys
and locking are unchanged. Exact diagnostic equality, cfg/text preconditions,
two-E0308 requirements, recovery and callback-count assertions are unchanged.

### New unit fixture sources and their limits

* `inputs::tests::native_script_env_a_b_a_requires_reload_without_consuming_witness`
  exercises actual `Inputs::stale`, `Inputs::variables` and `Scripts` state for
  A/B/A, repeat requests before commit, unset/set transitions and witness
  immutability. Replacement snapshots explicitly represent successful watch
  installation. This unit does not itself perform a Cargo workspace reload.
* On Linux,
  `inputs::tests::native_script_file_only_archive_change_requires_reload_until_new_snapshot`
  compiles real C objects with `cc` and archives them with `ar` at unchanged
  paths. It asserts native bytes change while Rust bytes and raw link
  directives stay identical, and verifies the real missing-proof/staleness
  path requires reload repeatedly without advancing the file witness.
  It requires the native C toolchain, does not mock successful compilation,
  and does not execute a linked resident program.

These are unit regression sources, not substitutes for root's independent
real Cargo script, native relink, runtime nonce, same-PID/lifecycle and
diagnostic controls. The source owner executed neither fixture and no native
build, test or workload. Root must rerun the full gate and real A/B/A probe.

### Release boundary

v58 owns only `src/main.rs`, `src/inputs.rs` and this document. Whole/daemon
remain unchanged, with `kept: false` for changed whole generations. No
activation switch, positive retention marker or unsupported-build fallback
is introduced. The previously proposed resolved ABI certificate is still
required before generation reuse can be enabled.

Full three-path originals/candidates, an incremental patch, SHA inventory,
preserved failure evidence copies and explicit source-only release are in
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/whole-generation-reuse-v58`.
The v56/v57 freezes and original failure artifacts remain untouched. Worker
checks are source rustfmt, diff inspection and hashes only. No commit,
installation, native result, product acceptance or speedup is claimed.

### Root-observed final correctness release, 2026-10-31

After the immutable source-only v58 freeze, root executed and released the
following actual gates. These results supersede the pending-gate status above
for the reload correctness prerequisite only, not generation retention:

| Root task | Actual result | Evidence under `ROOT` |
| --- | --- | --- |
| `0975668sp6` | Complete native gate passed in 168.75 s: check/build exit 0, 53 codegen tests and 364 resident tests passed; source/root-lock after-checks 0 | `candidate-unit-v58/full-native.log`, `candidate-build-v58/` |
| `277567rzkc` | Six bidirectional native checks passed in 15.0 s: exact environment A/B/A returned 11/22/11 and C-content A/B/A returned 11/33/11; source unchanged | `native-link-reload-v58-candidate/state.json`, `native-file-reload-v58-candidate/state.json` and their raw outputs |
| `303003qoqj` | Manifest-only cfg diagnostic/error/recovery controls passed 9/9 in 39.76 s, across original/OFF/ON labels | `manifest-cfg-reload-v58/state.json` and raw outputs |
| `352678zgk4` | Strict default seven-state correctness regression passed 7/7 in 53.22 s | `native-reload-v58-seven-state/state.json` and raw outputs |

Durations identify gate executions, not a speed comparison. The strict
seven-state receipt's historical scope label says whole-generation-reuse,
but its `RA_JIT_WHOLE_RELOAD` control is 0 and generation reuse is unimplemented.
Likewise the manifest probe's OFF/ON labels exercise the existing default
restart path with different ambient control values, not an activated feature.
These are default correctness regressions, not same-worker changed-generation
retention evidence. No process-retention or performance claim follows.

The rejected v56 fresh-oracle/native B-to-A failures and v57 shared-CLEAN
complete-gate failure remain retained, as do the original and intermediate
native-file failures. Passing v58 results do not rewrite those observations.
The immutable `whole-generation-reuse-v58` snapshot remains the tested Rust
source freeze. This later documentation-only release is frozen separately in
`whole-generation-reuse-v58-final-doc` with originals/candidate, patch, hashes
and release. Root owns the atomic scoped main/inputs/document source commit.
This worker performed no Rust edits, native runs or commits during that release.

Changed whole generations still use the original `kept: false` cold restart.
The toolchain/sysroot/native-ABI restart certificate and actual retained-worker
controls remain prerequisites to any future exact opt-in activation. This is
licensed Blaze-derived resident work, not a clean-room implementation.
