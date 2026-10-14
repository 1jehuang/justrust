# Hosted shadow leaf routing, October 14, 2026 (UTC)

## Delivered behavior and limits

`ab55ea8` forwards the existing explicit `JUSTRUST_SHADOW` control to the hosted
agent. Previously the client silently discarded it. The default remains off,
and the internal `JUSTRUST_SHADOW_CHILD` recursion control is not forwarded.
Normal billing admission and tenant isolation are unchanged. `6232dd1` records
the public opt-in in benchmark provenance without internal controls or secrets.
Both changes are committed and pushed, and the behavior change is installed.

This is **qualified leaf testing, not full-crate semantic equivalence**. The
parent crate's uses of the leaf are checked by the next normal build. Existing
eligibility checks reject parent references, child modules and unsupported
command flags. Do not enable it globally or count its timing as a general Rust
compiler speedup. No compiler fork was changed or promoted in this experiment.

## Matched measurements

All measurements used the actual pinned Desktop/Jcode snapshots, the same
own-account west host while it was `m8azn.metal-24xl` (96 cores), normally billed
hosted requests, nightly-2026-10-08 and the host's static jr-cargo. Each warm
scenario has two warmups and seven measured samples. The newly installed
frozen client SHA-256 is
`93adb8f1a1e5fdd81b16d85cb6601d1fbc4424feac941fab3030434c1dff1dc9`.
The older-client baseline is preserved separately and is not the paired control.

| Same new client | Shadow off | Shadow on | Off recheck | Interpretation |
|---|---:|---:|---:|---|
| Reference test edit | 5.670 s | 1.155 s | 5.556 s | All seven fast samples noisy, uncontrolled |
| No-op filtered tests | 0.693 s | 0.658 s | not repeated | Within noise |
| Body-edit check | 1.804 s | 1.920 s | not repeated | Within noise |

The observed reference reduction is about 80%, but **is not an accepted causal
speedup**. Fast samples report 6.72 to 8.29 other cores during their short host
execution. The host owner confirmed no overlapping work from their session in
that interval. The raw sample flags are preserved, not relaxed. `1b188a7` fixes
`bench/suite.py compare` incorrectly advertising a noisy range-separated result
as faster. Three red failures were observed first, and all 40 Python benchmark
tests pass after the fix. The actual public comparison now says
`uncontrolled (noisy or unknown samples)` for this reference workload.

First use is materially worse: the first normal snapshot was 5.60 s; the first
leaf build was **20.38 s, 98 compiled units**, marked noisy. It warmed a separate
generated workspace with different dependency feature unification. This cost
is not discarded or treated as a warm speedup.

An additional explicit `--config build.jobs=8` off/on/off trial measured
6.000/6.268/6.319 s. Every run actually compiled the normal UI package: extra
Cargo config arguments deliberately make this request ineligible for shadow.
Those three files verify normal fallback, **not an eight-job shadow comparison**.
The snapshot sources were restored from saved bytes after every trial.

## Requirement-to-runtime mapping

The real public CLI and pinned source were exercised, not only a synthetic
fixture. The nine rows and actual requested/routed arguments are in
`bench/results/hosted-shadow-acceptance-20261014.json`.

| Requirement | Actual check and observed result |
|---|---|
| Forward only the public opt-in | Hosted regression `20261013-175137018-204165` first failed for the missing whitelist entry. After the fix, `20261013-175152064-205518` passed. Internal child control remains excluded. |
| Real subprocess protocol, not whitelist inspection | `20261013-175154135-205259` passes the eight-run agent test covering normal snapshot, leaf edit, failed leaf, outside edit, restoration, opt-out and warm reuse. |
| Installed source passes check/full tests/lint | `20261013-175150527-205442`, `20261013-175207660-206395` (361 passed, zero failed, two ignored), and `20261013-175210697-206549` all hosted and successful. |
| Establish a real normal snapshot | `20261013-180351905-256367159` and `20261013-180359850-256367571`: normal UI package, three FPS tests pass. The second includes a real changed-parent snapshot. |
| Route an eligible one-line leaf edit | `20261013-180405935-256367441`: actual `jr-shadow-fps-counter`, all three original FPS test names pass. |
| Preserve compilation errors and exit status | `20261013-180407289-256367453`: actual leaf E0308, exit 101, no false test success. Complete output is committed in the compile-error JSONL artifact. |
| Preserve failing tests and exit status | `20261013-180409142-256367543`: two pass, one deliberately fails, exit 101. Complete output is committed in the test-failure JSONL artifact. |
| Parent changes force a normal build | `20261013-180411023-256367656`: changed parent plus leaf edit executes the original UI package, three FPS tests pass. |
| Opt-out and exact source restoration work | Saved leaf and parent bytes restored and compared. `20261013-180418974-256367614` with shadow off executes the original UI package, three pass. |
| Caller filesystem semantics survive normal fallback | `20261013-180426280-256367927`: all 16 unchanged Linux package/security fixtures pass, original UI package. |
| Unfiltered full tests never become a leaf-only success | `20261013-180427353-256367941`: original full UI, 1,302 passed, one existing workspace hint-chip failure, ten ignored, exit 101. This is the previously observed baseline failure, not a clean full-UI promotion gate. Full output remains available through `justrust log`. |
| Unsupported command flags fail open to normal work | All 21 measured `--config build.jobs=8` samples use the UI package, not a shadow leaf. No unsupported-control speed claim. |
| Truly plain installed dogfooding | `20261013-180649536-269280` check, `20261013-180650413-269533` full 361 tests, `20261013-180653455-269696` strict all-target lint, and `20261013-180653959-269757` repeated check all hosted, exit zero, zero compiled and 56 fresh. No forced routing or run-ID override. |
| Recording shares artifacts | The final four installed plain commands each reuse 56 fresh units with zero compiled units, including a repeated check. |
| No false 10x or uncontended stock comparison | Current stock quiet gate is still blocked by laptop load. No fresh matched standard-local denominator was accepted. The 10x goal remains unmet. |

An earlier final-check helper accidentally set `JUSTRUST_RUN_ID_OVERRIDE`, which
intentionally disables default routing for legacy remote children unless force
is explicit. Its local run `20261013-180514589-262197` is retained and **excluded
from hosted acceptance**. The helper was corrected to discover the emitted run
ID instead. The actual plain default rerun above passed without backend,
credential or routing-source changes. This was a harness error, not a hosted
outage or a default-routing regression.

## Evidence and next decision

All 11 timing/acceptance result files and both complete injected-failure output
files are checked individually against the original run records. The evidence
audit carries their hashes and per-file observations. Every cited run ID is
exported and committed. The exporter conservatively labels generated shadow
workspaces private and removes names/remote metadata. That redaction is retained.
The audit adds explicit local attestations only for this controlled public FPS
fixture, including actual arguments and original-summary hashes. It does not
reclassify arbitrary shadow workspaces or weaken the privacy boundary.

The exclusive host window was released at 01:07 UTC for the owner's approved
heartbeat installation and downgrade to `m8azn.6xlarge`. These measurements
belong to the 96-core epoch, not to the post-resize host. Shadow stays opt-in.
Next work requires a fresh uncontended stock-local control and an exclusive
host window, investigation of short-run CPU attribution/background activity,
and matched workload/feature semantics. Do not combine the earlier 24-core
compiler measurements, this qualified leaf observation and the hardware sweep
into a fabricated aggregate speedup.

## Post-resize current-host acceptance, 01:15 UTC

The owner reports the approved downgrade to `m8azn.6xlarge` in us-west-2a with
the same volume and matching instance-type metadata. Independent truly plain
installed `justrust check` run `20261013-181508205-309064` confirms actual hosted
execution on 24 cores, exit zero, zero compiled units and 56 fresh. This is
current functional dogfooding, not a comparison against the 96-core timings.
The owner could not install the console heartbeat into this existing host's
`/opt/jcode-build/idle.sh`: the build user deliberately lacks general sudo.
Existing idle-stop behavior therefore remains unchanged until the host is
relaunched from the new bootstrap. No permissions or idle/lifetime safeguards
were weakened. Longer remote-only work still needs the owner's existing
keepalive or the future bootstrap rollout.
