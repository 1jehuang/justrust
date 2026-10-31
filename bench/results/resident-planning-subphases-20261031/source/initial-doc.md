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
