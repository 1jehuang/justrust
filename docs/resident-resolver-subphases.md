# Native resolver subphase telemetry (v80)

Status: native correctness gates and positive client observability verified by root on October 31, 2026. Production sources remain frozen at the v80 source release; this final documentation update records existing root receipts only. This is licensed Blaze-derived resident code, attributed to the Blaze contributors under the resident workspace's MIT OR Apache-2.0 license. It is not clean-room compiler work. No speedup, general semantic equivalence, or quiet-machine result is claimed.

## Scope and motivation

Coordinator-reported accepted v79 observations put the client-owned resolver at 25.154–31.148 ms within 28.930–35.106 ms planning across eight actual rows. Those observations motivate attribution, not a performance claim for this change. The rejected v8 typed JSON projection experiment in `resident-typed-planning.md` is not repeated.

Owned outputs are `crates/blaze-metadata/src/lib.rs`, its manifest and `crates/ra-jit/src/cargo/mod.rs` under `toolchain/resident`, plus this document. The accepted baseline anchor is `34dc0a188`. Root explicitly expanded ownership to the client module for the necessary opt-in subscriber lifecycle. The manifest adds `tracing.workspace = true`, using the already locked tracing 0.1.44 and existing workspace declaration (0.1.41 compatible requirement, default features disabled, `std` enabled). Root owns the lockfile and must add exactly the existing tracing dependency edge for blaze-metadata, with no version or unrelated dependency changes.

## Stable span contract

All seven callsites are fieldless INFO spans with explicit target `ra_jit::planning`. They use owning `EnteredSpan` guards from `info_span!(...).entered()`, not timers or wrappers.

| Name | Exact covered work |
| --- | --- |
| `resolver workspace/config` | `resolving_with`: config load/start and offline configuration, context/workspace creation, root/replace checks and lock text read |
| `resolver package loading` | Path package traversal, path dependency map, lock parse/resolve, sources construction and fetching pinned packages |
| `resolver summary preparation` | Existing package export and summary conversion, memory registry/patch/member preparation, resolver setup, CLI features and unification selection |
| `resolver version resolve` | The existing `resolve_ws` call, including evaluation of its registry constructor arguments |
| `resolver finalization` | Lock merge, unread package fetching and summary conversion, lock write decision/write, unused patches, manifest map and `Resolved` construction |
| `resolver feature graph/export` | Entire `described`, including feature roots/resolution, graph construction, reached ordering, metadata JSON export and `Made` construction |
| `resolver program projection` | `programs` after `made` returns: unchanged JSON parse, feature/package/target traversal, target selection and program construction |

These are broad ownership labels, not claims that every operation under a label is one primitive. Fetching newly selected unpinned packages remains after version resolution in finalization. Summary preparation includes CLI feature validation and member selection. No calls move across phase boundaries.

Each sequential `resolving_with` guard is explicitly dropped before the next stage. Finalization drops before invoking the existing `then` callback, so it does not cover `described`, downstream users of `resolved`, or caller work. Function-scoped graph/export and program guards end on their function exits. Normal early errors, `?`, explicit returns, `bail!`, and unwinding drop the currently live owning guard. Process abort does not promise a drop. Existing callback APIs are unchanged and no new callbacks are introduced. Original local lifetimes and order are otherwise unmodified.

## Actual dispatch and inactive behavior

Source inspected: locally locked tracing 0.1.44 `src/macros.rs::span!` checks static/dynamic level enablement, cached callsite interest and `__is_enabled` before constructing `Span::new`; the rejected branch constructs a disabled span. `src/span.rs::entered` calls `do_enter`, whose subscriber dispatch requires `self.inner`. Fieldless static labels add no formatting or payload allocation. Inactive callsites do not enter Recorder and therefore add no Recorder clock reads or span-record allocations. This is not a zero-total-overhead claim: level/callsite checks, first-use registration, disabled span/guard handling and drop still exist. No custom environment lookup or clock is added to blaze-metadata.

The existing `ra-jit/src/trace.rs::Recorder::wanted` accepts span metadata at INFO or above when the target starts with `ra_jit`, so these exact targets qualify without recorder modifications. `register_callsite` returns always/never interest; the recorder advertises an INFO max level. When installed and enabled, `new_span` reads elapsed time and locks the callsite registry, and close reads clocks and may allocate/lock retained detail records. Turning off trace-file output alone does not disable an already installed Recorder.

Integration repair: `main.rs` routes `Some("cargo")` into `cargo::command` before the normal `trace::install` calls, and `trace::finish` consumes noted spans rather than pending detail. The candidate therefore installs the existing subscriber only inside `telemetry_enabled().then(...)` at `cargo::command` entry and captures an `Instant` there. After unchanged `command_code(args)` returns, including an error result, it drains `take_detail(started)` into `note_all`, before the unchanged `finish` and `std::process::exit(result?)`. All command phase guards have dropped before the drain. This preserves the original Result propagation and child exit status. The disabled path executes neither the closure nor the drain, adding no clock read, subscriber install or detail allocation. No `main.rs` or `trace.rs` change is made. Subscriber installation uses the existing fail-ignored `set_global_default`, so root must still verify positive runtime labels if another subscriber was already installed. The existing cached client opt-in uses `RA_JIT_STATS` presence or nonempty `RA_JIT_TRACE`; `finish` uses those output settings too. This is native observability wiring, not a parsing shim. The lifecycle does not add panic catching: an unwind drops the span guards, but the command wrapper does not promise to export after a panic.

## Existing filtering and interpretation

Recorder retains only detail spans lasting at least 200 microseconds (`SHORTEST_US = 200`). It retains at most 4000 details between collections (`MOST`), and its process-wide callsite table admits at most 255 callsites (eight site bits, zero reserved). Callsite exhaustion can reject these spans even with the right target. `take_detail` drains details, resets the dropped counter and filters records by end time relative to the supplied start. Detail spans have no level, so the existing level-based `breakdown` table does not attribute time to them. Inspect actual Chrome trace detail events rather than expecting these labels as top-level stats rows.

A missing tiny span is not proof that the phase did not execute. Root's positive profile must report retained labels, cap/threshold effects and actual paths. If the per-label requirement cannot be satisfied because spans are below the threshold, report threshold-censored labels and aggregate repeated phase observations where possible, or propose a separately reviewed native aggregate mechanism. Do not weaken the existing threshold, fabricate durations, or silently treat absence as zero. Summing retained spans alone undercounts censored work. No per-package spans are added.

## Source checks and root release gates

The private frozen handoff root is `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/resolver-subphases-v80`. It contains full originals, full candidates, an exact patch, SHA256 inventory, source-only check script/output and RELEASE. Static checks remove only the exact seven two-line instrumentation declarations, five explicit drops and one manifest dependency addition, then require byte-for-byte equality with the saved original sources. The client module similarly must equal its original after removing the exact opt-in setup and detail-drain blocks. They also check unique labels, explicit target, phase/drop boundary ordering and unchanged embedded tests. These are static fixtures, not executed Rust tests or runtime proof.

No build, test, native invocation, probe, installation, lockfile edit or commit was performed by this lane. Root's subsequently completed checks and their exact observed counts are recorded below. Root also retains responsibility for integration and unchanged external behavior. The candidate is instrumentation only, with no typed projection, caching, skipped errors, or semantic reordering. No next optimization is implemented here.

## Root-observed native validation, October 31, 2026

The coordinator ran the native gate task `107030tti1`, exit 0, 245.10 s total, using the pinned `nightly-2026-10-27-x86_64-unknown-linux-gnu` toolchain. Its saved `build-test-candidate-v80.sh` performs release locked check of ra-jit tests and hir-ty-codegen, builds the actual `jr-engine` binary, then runs existing blaze-metadata, codegen and engine package test targets. No such command was executed by this lane. All five command exits and the orchestration exit are 0. Existing warnings remain in the logs, so this is not a warning-free claim.

Exact saved test counts, read directly from `candidate-unit-v80` reports and the metadata raw output journal:

| Existing native target | Observed result | justrust run id |
| --- | --- | --- |
| `blaze-metadata` | Package test command exits 0; its library harness explicitly reports **0 passed, 0 failed, 0 ignored, 0 measured, 0 filtered out** | `20261031-091359767-3186454` |
| `hir-ty-codegen` | **53 passed, 0 failed, 0 ignored, 0 filtered out** | `20261031-091410105-3187720` |
| `ra-jit --bin jr-engine` | **373 passed, 0 failed, 0 ignored, 0 filtered out** | `20261031-091410535-3187827` |

The metadata raw journal is `candidate-unit-v80/justrust-home/runs/20261031-091359767-3186454/output.jsonl`. It records `running 0 tests` and the exact zero-test result, not a nonzero metadata assertion count. The command compiles the package and its existing targets, but must not be described as passing an invented metadata test suite. Root's source and lock verification logs passed after the gate. The resulting binary reports build identity `resident-v1-2ccc6152c30dc36f4927cf0b5873bd7fa7f3ffc82ba2fceddd1efba6ffcce756` and Blaze-derived origin `5d4f78f7577cf7170993b74f735099dd03f5c6ca`.

## Positive actual client profile

Coordinator task `361033u3uq` completed in 35.62 s with exit 0. The existing receipt is `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/resolver-subphases-profile-v80/state.json`, with sibling per-row stdout/stderr and actual Chrome trace files. This lane only read these files. Preparation, all eight measured commands, and final stop exited 0. The receipt has status `passed`, `unchanged: true`, and equal before/after source inventory, native executable hash and toolchain identity.

The actual command was the v80 native `jr-engine test --lib -- summary::tests:: --test-threads=1 --nocapture` on the prepared justrust workload. There are four edit/no-op pairs. Every row's stdout reports **4 passed, 0 failed, 0 ignored, 0 measured, 144 filtered out**. Current-code execution is evidenced by the expected nonce in each pair, `JR_RUNTIME_EXECUTED_8001` through `JR_RUNTIME_EXECUTED_8004`, rather than inferred solely from command success.

All seven new resolver labels appear as actual detail events in **every one of the eight rows**, alongside all four existing outer planning labels (rustc host, target cfg, asked setup, owned resolver). Their client process identity and retained detail export positively verify the subscriber installation and post-command drain repair on this path. The 200 microsecond threshold did not censor any of these seven labels in these rows. This does not prove that tiny spans or other workloads will always be retained, or that no other detail was capped.

| Native resolver detail | Observed inclusive duration range across eight rows, ms |
| --- | ---: |
| workspace/config | 0.386–0.508 |
| package loading | 16.723–19.843 |
| summary preparation | 3.853–5.293 |
| version resolve | 2.876–3.974 |
| finalization | 0.559–0.733 |
| feature graph/export | 1.140–1.575 |
| program projection | 1.318–1.739 |

These are inclusive diagnostic wall durations, not matched before/after measurements or evidence of savings. Do not sum ranges from different rows, add detail to its containing outer resolver span, infer exclusive CPU cost, or compare these ranges to v79 as a speed result. The receipt explicitly labels contamination unknown and notes that waited-child CPU excludes resident daemon CPU, so it does not establish a quiet machine. No instrumentation-overhead or speed comparison was performed.

Package loading is the largest observed named resolver stage in this profile. It includes path traversal, lock parsing, source construction and pinned package fetching, so this evidence identifies a next attribution target, not a justified cache or work-skipping implementation. Source changes still require measurements and external correctness gates.

## Traced failure and recovery gate

Root's additional task `615674bbf9` passed in 49.7 s, exit 0. Its saved `resolver-traced-cfg-v80/state.json` has status `passed`, all **9 rows accepted**, and `unchanged: true`. The matrix uses the v79 reference and the identical v80 candidate binary under startup-control OFF and ON, each with valid, manifest-feature error and recovery phases. OFF/ON here names the existing startup control, not disabled/enabled tracing. Every mode has command statuses **0, 101, 0** in that order. Rust source hashes remain identical before/after every row, while the manifest feature changes exercise the otherwise-unused E0308 case and recovery. The existing exact diagnostic/nonces/toolchain checks were not weakened.

Every row retains a positive `client command` trace, and every v80 row, including both error rows, retains positive resolver stages (`resolver workspace/config` and `resolver package loading`). This verifies client detail collection on these command error returns as well as successes. Only two resolver labels are retained on this smaller fixture under the unchanged threshold/cap policy, unlike all seven on the eight-row main profile. Missing tiny labels are not interpreted as skipped work or zero duration. Reference traces lack the candidate's new labels as expected. All three final stop commands exit 0, and native binary/toolchain identities remain unchanged. This is an external failure/recovery and observability gate, not a performance comparison. Root additionally reported task `541721cmna` passing **761 Python tests**; that is coordinator-reported supplementary evidence, not a test invocation by this lane.

## Final documentation-only freeze

Root reopened only this document after native validation. The original v80 source archive, its RELEASE, exact patch and SHA inventory remain unchanged, including its historical pre-validation document snapshot. The final document is frozen separately at `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/resolver-subphases-v80-finaldoc`, with original/final document copies, documentation-only patch, SHA inventory and STOPEDITS release. The original sourcechecks script's live-document equality check describes the initial release and is not a claim that this authorized later documentation revision equals its historical snapshot. Production-source candidate bytes remain unchanged. No native jobs, tests, installation, lock edits or commits were run for this final update.
