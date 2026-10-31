# Resident client planning subphases, v79

Source-only telemetry instrumentation on coordinator-supplied source base
`0aa1d0727` plus evidence `5acba45f`. This is not an optimization or a speed claim.

## Motivation and bounded scope

Root's accepted warm profile v78 (`ROOT/accepted-warm-profile-v78/state`, eight
rows) reported client planning 29.943 to 35.005 ms within noop wall time 51 to
59 ms, versus worker time below 1.1 ms. Those are coordinator-reported profile
observations, not measurements run by this worker. This patch makes the next
planning attribution visible before selecting an optimization.

Only production `cargo::metadata::programs` is instrumented. It reuses the
existing parent module's private `Phase` guard, with static literal labels:

| Phase label | Inclusive operations |
| --- | --- |
| `client planning rustc host` | `rustc_path`, existing answer closure setup, explicit-target cloning or the existing `-vV` host answer and host extraction |
| `client planning target cfg` | Existing `--print cfg --target` answer and complete cfg parsing/collection |
| `client planning asked setup` | Existing environment collection and Asked/Build/TargetPlatform construction, including existing toolchain release lookup |
| `client planning owned resolver` | Existing `blaze_metadata::programs(&asked, targets)` call, error propagation and tuple projection |

The outer `client planning` span already lives in `cargo::whole::planned`. It
continues to include these operations and its own remaining work. Subspans are
successive, not nested into each other, and should not be added to the inclusive
parent as though they were disjoint additional wall time. Guard setup/drop and
uncovered surrounding planning work mean no exact sum equality is promised.
With an explicit target, the host span does not invent a `-vV` call.

## Disabled behavior and error preservation

`Phase::new` already checks the once-latched existing client-only stats/trace
selector. When disabled it stores no Instant, reads no clock, and allocates
nothing. The new labels are static strings. No new environment selector, cache,
resolver interface, output parser, interception layer or telemetry API is added.
No stats settings enter checkout/worker configuration or cache keys.

Explicit drops end the first three guards immediately after their original
operations. If an original `?` returns early, RAII drops the active guard and
records the partial failing phase through the existing API. The resolver guard
similarly drops on success or failure. Original `rustc_path`, `rustc_says`,
argument lists, conditional host answer, env collection, release lookup and owned
resolver calls retain the same order and number. Original error contexts, result
projection and return type are unchanged. Non-programs metadata paths and all
existing tests are untouched.

## Source validation and root gates

Worker validation is rustfmt and Python structural source comparison only. After
removing exactly the seven added guard/drop lines, the candidate must equal the
full original metadata.rs byte-for-byte. This checks all existing call sites,
arguments, ordering, error text and tests, not a duplicated resolver simulation.
The existing Phase API has no capture/assertion seam for this module. No global
environment-racing fixture or new test-only telemetry behavior is introduced just
to assert label strings. Root owns compilation, full resident/native correctness
and positive profile observations showing all four labels under the existing
telemetry selector, plus disabled/no-clock behavior review of the existing guard.
No builds, tests, native commands/probes, locks, installations or commits are run
by this worker. Earlier v77 fixture and frozen releases remain untouched.

Full originals/candidates, new-doc absence marker, patch, sourcechecks, SHA256SUMS
and STOPEDITS RELEASE are frozen under `ROOT/planning-subphases-v79`. This retains
the licensed Blaze-derived resident implementation and its existing attribution
and dual MIT/Apache-2.0 license lineage. It is not a clean-room rewrite or a newly
licensed independent resolver. No license or attribution files are changed.

## Root-observed acceptance and attribution, October 31, 2026

The root native gate `610641drlg` completed successfully (coordinator-reported
outer duration 217.94 seconds). Read-only review of the actual journals confirms:

- `ROOT/candidate-build-v79/check.exit`, `compile.exit`, and
  `orchestration-exit` are all 0.
- `ROOT/candidate-unit-v79/codegen-native.log` records **53 passed**, 0 failed,
  0 ignored, 0 filtered. Its exit receipt is 0. Run id:
  `20261031-084851470-3065538`.
- `ROOT/candidate-unit-v79/full-native.log` records **373 passed**, 0 failed,
  0 ignored, 0 filtered. Its exit receipt and the unit gate exit are 0. Run id:
  `20261031-084852010-3065692`.

Root positive profile `317616f4d6` also passed (coordinator-reported outer duration
34.86 seconds). Its actual `ROOT/planning-subphases-profile-v79/state.json`
contains eight successful rows, four edits alternating with four noops. Every row
has status 0, no timeout, and all four new labels. The accepted profile reports
source/toolchain unchanged, identical engine hashes before/after, and stop status
0 without timeout or launch error. The preparation run is separate from these
eight rows.

Ranges below are computed directly from the eight rows' trace event durations,
converted from microseconds to milliseconds:

| Inclusive phase | Observed range, ms |
| --- | ---: |
| `client planning` | 28.930 to 35.106 |
| `client planning rustc host` | 2.669 to 2.985 |
| `client planning target cfg` | 0.801 to 0.998 |
| `client planning asked setup` | 0.019 to 0.026 |
| `client planning owned resolver` | 25.154 to 31.148 |

The owned-resolver call is the dominant measured portion of client planning in
this bounded profile, making its internal work the next attribution target.
This coarse span does not distinguish manifest reads, configuration, resolution,
or program selection inside the resolver. It does not justify a particular cache
or resolver rewrite without further measurement and correctness proof.

These are inclusive attribution observations, not before/after comparisons,
an instrumentation-overhead estimate, or a speedup claim. The profile explicitly
classifies contamination as unknown and does not establish a quiet machine.
The workload is not the comprehensive session-weighted suite, and no general
performance, default promotion, or statistically significant difference from v78
is claimed. The author only read journals and updated this document after root
execution. The original source freeze remains unchanged, and the implementation
retains its licensed Blaze-derived lineage rather than a clean-room claim.
