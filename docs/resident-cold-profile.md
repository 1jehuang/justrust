# Resident cold-path diagnostic, October 30, 2026

This is one actual cold profile of the **experimental licensed Blaze-derived
resident**, not the clean-room compiler. Frozen candidate source corresponds to
commit `91220ff43`. It is not the later v31 standard-library demand experiment,
and none of that experiment's correctness or performance is established here.

## What the raw profile reports

Root ran `profile-cold-v30.py` once with statistics enabled. It made two actual
cold native calls in distinct private stores, first the source-built reference,
then the candidate. Each stdout contains the same four named summary tests
passing, zero failures/ignored, and 144 filtered tests. Before/after manifests
report all 63 input source files, toolchain files and tool binary hashes
unchanged. The candidate's frozen main matches the committed source bytes.

| Candidate reported interval | Time |
| --- | ---: |
| Client total | 26.3385 s |
| Daemon | 22.9830 s |
| Client planning | 3.3375 s |
| Eager standard-library preparation | 9.1 s |
| Project cold preparation | 5.1 s |
| Source checking | 1.4 s |

The worker reports 17 standard-library crates and 25,924 of 25,960 roots compiled,
plus 16,200 instances. Project cold preparation reports 3,990 roots and 40,606
instances. Source checking reports 31 files. Complete worker logs retain
unsupported operations and fallback details, not just successful headline rows.

**CPU phase totals overlap and are not additive wall time.** For example, the
standard-library phase receipt includes 80.4 seconds of translation. Neither
summing phases nor subtracting an entire eager phase yields justified savings.
This is a source of hypotheses for a separately gated experiment, not evidence
that skipping any preparation preserves semantics or improves latency.

## Limits and reproducible audit

This is **not a performance comparison**, comprehensive workload, savings
estimate, general speed claim, promotion gate, or quiet-machine proof. Raw
process noise explicitly remains unknown because waited child CPU does not
account for the resident daemon. Sequential reference/candidate observations
are retained without turning them into a speedup ratio. Four passing filtered
tests do not establish all supported Rust semantics.

The portable bundle at
`bench/results/resident-cold-profile-20261030/` retains exact state JSON,
stdout/stderr, both worker logs, the actual runner, committed helpers/licenses,
source63 manifest, frozen main and the 3,757-entry compiler source hash inventory.
It contains no binary, private cache or authentication material. Historical
absolute paths are inert receipt strings and are never dereferenced by replay.

Actual offline validation passed for all 29 inventoried inputs. All 14 controls
passed, including coherently re-signed changes to test names/counts, native
status/filter, binary/source/toolchain identities, std-root counts, CPU/client
timing and noise claims, plus detached replay with an empty PATH. Results are
retained in `audit.json` and `offline-tests.log`. These are offline evidence
checks, not newly executed native builds or workloads.
