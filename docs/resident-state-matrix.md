# Bounded resident state-transition matrix

`bench/resident_state_matrix.py` is a new, opt-in diagnostic, not a change to the
successful existing resident harnesses. It compares the source-built native
`ra-jit` reference entry (`blaze cargo test`) with native `jr-engine test` on a
pristine `f97fcc2` copy whose directory is named `source63`. The original working
source, including `/home/jeremy/.jcode/scratch/ba30/source`, must not be used.
General faster-than-Blaze remains unproven, including after any scoped v28 win.

## Safety and shared contracts

The harness imports environment creation, full pinned source validation, source
hashing, process timeout/raw capture, command construction, and frozen toolchain
identity checks from `resident_engine_compare`. It imports the credential-excluding
receipt allowlist, ordinary `parse_pass` mutation, filtered test discovery, and
exact result validation from `resident_workload_compare`. The summary testbody
mutation is the existing `runtime.edit_bytes` API. Existing harnesses and resident
engine sources are not modified.

Inputs must be existing executable native ELF tools, a preinstalled complete
`nightly-2026-10-27-x86_64-unknown-linux-gnu` toolchain matching the shared frozen
manifest, and pristine pinned source63. The harness itself does not download or
seed anything, build engines, or change an installed toolchain. Native cold test
commands may populate/download their own dependencies within the measured command.
Both engines use
the same explicit proc-macro server if supplied. Otherwise both server variables
are explicitly `0`, the existing rustc-built-proc-macro compatibility control.
The native entry can move its requested roots below `JR_ENGINE_HOME/resident-v1`,
just as in the existing harness. HOME stays ambient. Requested private environment
roots are launcher inputs, not proof that an engine never reads ambient state.

Before mutations, all three writable files are backed up with exclusive creation,
file fsync, directory fsync, SHA256 receipts, and durable `finally` restoration:
`src/summary.rs`, `src/shim.rs`, and `Cargo.toml`. Restoration attempts every backup,
even if another fails. Full source, engine/server, and complete toolchain identities
must match before and after. Toolchain links must still point to the same frozen
root. Mutation outside those three files is detected by the full source manifest,
not silently removed. A hard kill or power loss cannot run Python's `finally`, so
retain the durable backups for manual restoration. Do not run another tool against
this source copy concurrently. The source63 directory is a disposable pinned
fixture, never a developer's active checkout.

## Session layout and explicit counts

Default `--samples 5` means five independent sessions per stack. Each replicate
starts with fresh private engine, tool, dependency, target, and XDG roots. Stores
are reused through the ordered phases within that replicate/stack. Nothing is
seeded after a failed cold attempt. Stack order rotates by replicate, including
cold prepare. Both stacks finish a phase before moving to the next. The actual
reference establishes contracts even when rotation runs the candidate first.

There are 14 phases, each with exactly five planned observations per stack by
default: 140 workload invocations and 10 separate version invocations. Samples
are bounded to 5 through 30 and each subprocess timeout is finite, positive, and
at most 3600 seconds (default 360). An invalid version, timeout, or launch error
retains its raw receipt and causes explicit unattempted records for the remaining
phases of that stack/session. It does not silently remove a sample or retry using
warm stores. Other oracle failures retain their evidence and allow subsequent
recovery phases to run. Overall exit status is nonzero if any phase is incomplete
or rejected.

| Ordered phase | Controlled source state and runtime oracle |
| --- | --- |
| `cold-prepare` | Fixed instrumented baseline. Actual elapsed engine test command, four exact summary test names/results and one fresh execution marker. |
| `repeated-noop-1` | Identical source bytes and mtimes, same stores, new invocation nonce. Exact summary results and one fresh marker. |
| `repeated-noop-2` | Second identical-input noop, reported separately, same runtime requirements. |
| `summary-testbody-edit` | Imported summary testbody edit recipe and exact per-replicate recipe marker, plus one fresh invocation marker and exact summary results. |
| `summary-edit-recovery` | Remove the body edit before the ordinary-function case, with fresh runtime and exact summary results. |
| `ordinary-shim-edit` | Only `src/shim.rs` changes from the preceding stack phase, using the imported `parse_pass` recipe and `shim::tests::` filter. Actual reference establishes exact names/counts and a positive exact invocation count for the ordinary-function marker. Candidate must match both, plus one fresh invocation marker. |
| `signature-base` | Restore the baseline, including the shim. Execute an independent used signature fixture in the summary test. |
| `signature-invalidation` | Change that used function argument from `u64` to `u32`, adapt its call to `7u32`, and convert the result back to `u64`. Same semantic assertion, then one fresh marker and exact summary results. |
| `manifest-feature-add` | Preserve changed signature and add only `[features]` with unused `jr_matrix_unused = []`. No feature enabled, same project semantics, fresh runtime and exact summary results. |
| `type-error` | Restore baseline manifest/signature while inserting imported summary type-error recipe with a unique invocation diagnostic string. Require status 101, `E0308`, current diagnostic string, no parsed libtest run and no runtime marker. |
| `type-recovery` | Restore valid baseline, require exact summary results and fresh execution marker. |
| `assert-failure` | Imported assertion failure recipe with current unique panic string. Invocation probe is moved before the failing assertion. Require status 101, exactly the intended failed test, three passed, no ignored, matching filtered count, fresh panic string and one fresh runtime marker. |
| `assert-recovery` | Restore baseline, require exact passing summary results and fresh marker. |
| `all-lib` | Separate unfiltered `--lib -- --test-threads=1 --nocapture` case. Only admitted after actual reference establishes all exact names/statuses/counts and two fresh probe invocations. Candidate must exactly match. |

The pinned full-lib fixture contains 148 names, with 147 passed, zero failed,
exactly one ignored, and zero filtered. The sole allowed ignored test is
`split_index::tests::attach_cost`, independently declared `#[ignore]` at
`f97fcc2:src/split_index.rs:657`. No blanket ignored-test exclusion is used for
this case. Conversely, that specific allowance does not permit new ignored tests,
changed names, different totals, or status drift. The actual reference's complete
148-name result is required on each replicate, not a hardcoded synthetic name
list or a candidate-derived baseline. No claim is made here that the worker ran
that native reference: this harness's implementation validation is mock-only.
The root's earlier `rmp01` probe rejection by a blanket `ignored == 0` guard
must remain recorded as its original failure. This new per-name allowance does
not rewrite or retroactively relabel that probe. Any corrected offline admission
is separate evidence, not a fresh timed native run or a cold-performance claim.

## Freshness, receipts and exclusion

Baseline instrumentation puts an execution probe in one summary test and one
shim test. It reads `JR_MATRIX_NONCE` at runtime and prints
`JR_MATRIX_RUNTIME_<nonce>`. A new UUID challenge is supplied to every invocation
without rewriting the source. Thus noops preserve source bytes and mtimes while
still rejecting stale output, missing output, extra markers, or a daemon that
cannot propagate the current environment. That last case is an explicit
unsupported state, not permission to accept a stale marker. Full-lib runs execute
both probes, requiring exactly two copies of the current marker. Error strings
are also invocation-specific. This is instrumented source, not untouched baseline
performance. Runtime environment changes are intentional challenge inputs and may
themselves affect an engine cache or daemon protocol, even on source-noop phases.
The independent signature fixture avoids changing a production ABI and all its
callers without native validation. It is a bounded invalidation probe,
not a claim about the whole production call graph or all ABI invalidations.

`records.json` persists exact full before-phase input SHA256 manifests before
launch, previous per-stack input manifests, the changed-file list, nonce,
stack/replicate/phase, explicit cache scope, command, safe environment receipt,
status, elapsed time, noise proxy, parsed results, oracle reasons and acceptance.
Every invocation has separate full `.stdout` and `.stderr` files, created
exclusively and never reused. Version invocations have the same raw receipts.
Raw file SHA256s are captured immediately and verified again at session completion
so late writes invalidate all medians. Parsing is bounded to 64 MiB per stream,
including a bounded read if a late writer grows the file. Oversized output fails
closed with full raw files retained, never a truncated successful receipt. This
is a parser/admission bound, not a live disk-byte quota on subprocess output.
No output from a preceding invocation,
shared global log, console truncation, or warm rerun is substituted. Receipts
contain private paths and unredacted native diagnostics. Keep the output private.
Only the imported explicit environment allowlist is serialized. Ambient tokens,
cloud credentials and arbitrary environment variables are not logged. The
non-secret challenge is recorded separately in the row's `nonce` field.

`metadata.json` retains backups, source/tool/toolchain identities, private requested
environments, version outcomes, phase counts, per-replicate actual reference
contracts, store policy and cold-scope caveats. `summary.json` reports each phase
and stack separately with planned/observed/accepted counts, failure reasons,
exclusion reasons and a median only when all planned samples passed the oracle
and final identities. Unsupported states and partial runs remain visible with
null medians. There is no arbitrary aggregate weighting or silently reduced
sample size.

Cold prepare includes the actual elapsed first test command in fresh private
stores. It does **not** flush OS page caches, toolchain disk pages, ambient HOME,
or machine-wide dependency state. Version queries happen before preparation.
Tool/dependency disk-store coldness and OS coldness are different claims. Mutation,
backup and input-hashing work is outside subprocess timings. Sharing one source
path between stacks can still change mtimes between different mutations, so
compare phase input/delta receipts rather than assuming process isolation implies
an untouched filesystem. Timings include launcher/IPC/runtime work and can be
contaminated by other agents, background work and resident daemon CPU. The shared
noise proxy does not prove a quiet machine or attribute daemon CPU reliably.

## Mock validation and READY usage

Only synthetic mocks are required for implementation validation:

```sh
python3 -m unittest discover -s bench -p test_resident_state_matrix.py -v
```

These tests do not execute ELF fixtures, engines, compilers, builds or native
workloads. They exercise rotated sessions, noop hashes/mtimes, imported recipes,
signature and manifest deltas, current nonce oracles, exact all-lib ignored/name
handling, reference admission failure, timeout exclusions, raw receipt mutation,
changed tool identity, credential exclusion, bounded raw admission, pinned-source
`git show` recipe/manifest checks, and interruption restoration.

READY exact invocation for the root coordinator, not executed by the worker.
These reference/candidate/toolchain paths and the explicit server-0 control are
recorded in the committed rwc02 metadata under
`bench/results/resident-broader-rwc01-20261030/runs/rwc02/metadata.sanitized.json`.
Reference SHA256 is
`fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263` and
v28 candidate SHA256 is
`5cd25191a65a6f24452cd0c46bd779a49d6482b6685a7068c60545867708d0da`.
The operator must verify those existing frozen artifacts still match before use:

```sh
python3 bench/resident_state_matrix.py \
  --reference-engine /home/jeremy/.jcode/scratch/blaze-native-engine-20261030/reference-bin/ra-jit \
  --engine /home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-v28-bin/jr-engine \
  --source /home/jeremy/.jcode/scratch/ba30/source63 \
  --toolchain /home/jeremy/.blaze/toolchains/nightly-2026-10-27-x86_64-unknown-linux-gnu \
  --output /home/jeremy/.jcode/scratch/ba30/rsm01 \
  --samples 5 --timeout 360
```

All inputs must be absolute existing paths. The fresh output must not exist,
be disjoint from source, and be at most 60 bytes long. If `rsm01` already exists,
choose another fresh short path and retain the old receipts unchanged.
An optional `--proc-macro-server /absolute/frozen/native/server` replaces server-0
with the same explicit server on both stacks and adds its hash to identity gates.
The root coordinator owns the real gating run and its interpretation. This is
not `bench/suite.py run --synth 1 -n 1`, not a comprehensive session-weighted
synthetic workload, not statistical significance, not a stock-versus-owned
architectural proof, and not a default-promotion gate.
