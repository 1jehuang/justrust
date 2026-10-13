# Hosted admission phases and rejected HTTPS forwarding, 2026-10-13

## Decision

No HTTPS tunnel is shipped or enabled. Every pinned whole-loop comparison is
**within noise**, and the direct recheck is as fast or faster than the tunnel.
The retained change is opt-in server phase observability, not a speedup.
The 10x stock-local goal remains unmet.

## Exact workload and executable

All seven-sample comparisons use the same frozen client revision `7d4474b`,
SHA256 `a89c983da61666c32a3d4ea2f1dad5a790ef56277b0333709de0451cc8966891`,
nightly-2026-10-08, own-account M8azn host and pinned Desktop snapshots
jcode abff94eeb / jcode-desktop 7d38fb7. Two additional runs warm each scenario.
Full per-sample files and all 138 exact owned exported records accompany this note.
This is warm remote execution, not a cold-host or stock-toolchain comparison.

## First diagnosis

Nine existing normal admission requests were instrumented, with no extra POST,
no bearer capture and no response-body changes. Each was HTTP200 and every
build finished hosted with three FPS tests passing, zero newly compiled units.
All nine reached CDG and used no proxy. Median cumulative HTTP total was 1.077s.
Per-request stage deltas have medians: DNS18ms, TCP162ms, TLS171ms,
response wait733ms. The last value includes network transit and server work.
It is **not** a measurement of billing SQL.

The first discarded run took60.314s: the actual idle host had stopped,
reconnect/wake took48.64s, source sync51.42s, host execution7.232s.
It still passed all three tests. It is retained, not silently represented as
an ordinary warm admission or removed from the evidence allowlist.

## Credential-preserving tunnel experiment

The scratch prototype adds a private0600 Unix local SSH forward from the
already-authenticated tenant master to build.jcode.sh:443. Curl keeps the original
HTTPS URL, SNI and certificate verification. The bearer is supplied only to
local curl stdin and remains inside end-to-end TLS on the tenant host.
No charge is cached, skipped, moved after execution or separately duplicated.
The precise local forward is canceled after the trial. No host restart,
backend change, account mapping or production client tunnel feature is involved.

Two harness mistakes are retained. The first selector omitted the /v1 prefix
and therefore never selected the tunnel. That complete file is renamed
`hosted-https-forward-selector-miss-20261013.json` and is **not** tunnel evidence.
The corrected trial ran all27 actual forwarded charges successfully, then a
validation assertion incorrectly expected curl's Unix remote_ip to be empty.
Curl instead reports a truncated Unix socket path. Corrected validation checks
that path, ATL ingress, HTTP200 and the actual hosted exit0 for every record.
No build was rerun to hide either harness failure.

`bench/suite.py compare` reports:

| Scenario | Direct before | Actual tunnel | Direct recheck | Verdict both comparisons |
|---|---:|---:|---:|---|
| Noop test | 1.20s | 1.27s | 1.14s | within noise |
| Body check | 2.54s | 2.17s | 2.19s | within noise |
| Edited test | 5.94s | 5.58s | 5.24s | within noise |

All27 forwarded admissions reached ATL with HTTP200, median HTTP0.733s.
Direct ingress varied among CDG, PDX, SJC, ATL, MRS and LHR. The corrected
trial establishes transport selection, **not** a reliable full-loop win.
Raw comparisons, per-request timings and credential-blind wrapper sources
are retained in `hosted-admission-control-20261013.json`.

## Authoritative worker phase accounting

Private commit `e75e03e` adds fixed numeric Server-Timing fields only when
`X-Justrust-Timing: 1` is present. Ordinary requests have no new timing header.
Deployed Worker version: `b5757617-3683-4a8a-8b7c-e108830d7876`.
Prior exact version: `a81f280e-d738-442b-bbfb-9f4fd47fc977`.
Remote binding metadata is byte-equal. Placement, identity, authorization,
quotas, charges, cached-whoami TTL and queued-before-response ordering are unchanged.
71 Node and11 Python tests pass, including secrecy, no extra charge, refused
admission, missing-binding failure, parallel identity and gated queued visibility.

Nine normally admitted real builds measured:

| Awaited path | Median |
|---|---:|
| Authoritative backend charge, including service transit |271ms|
| Parallel cached whoami-derived identity |67ms|
| Additional identity wait after charge |0ms|
| Queued batch, including D1 transit |97ms|
| Entire worker path |365ms|

Do not add parallel identity duration to charge duration. Workers clocks advance
on I/O, so these are awaited-path times, not CPU or pure SQL time. Later ingress
was DFW/ATL/IAD, not the earlier CDG epoch. The seven measured noop median1.03s
(range0.99-3.07) is not attributed to the observation change. An outlier spent
2.681s in HTTP while the worker reported349ms, establishing external transport
variance that should not be mislabeled as database work.

Installed plain hosted acceptance: check143805935-3266167, full tests
143811417-3266722 (355pass,0fail,2ignored), lint143814785-3267213.
All use exact20261013-prefixed exported records, actual remote on hosted,
zero newly compiled units and56fresh. The first check reconnect/sync was4.307s,
so it is not advertised as a latency improvement.

## Next useful work

Keep admission authoritative. Backend charge and queued batch are now separate
measured targets. Identity is already fully overlapped on these calls, so moving
it to the background or trusting a client account id buys no demonstrated benefit
and would weaken the contract. Expose these bounded timings in normal client
trace output so every agent can diagnose the next real regression without a
special curl wrapper. Compiler frontend/codegen remain larger reference-loop
costs than these bookkeeping stages.
