# Normally billed HTTP/2 versus HTTP/1.1 admission trial, October 13, 2026

## Decision

Keep the existing curl transport and its normal protocol negotiation. Do not
restore the rejected native pool on the strength of this experiment. Both
`bench/suite.py compare` results classify this trial as **within noise**. The
original native-pool regression remains unexplained, and the 10x stock-local
target remains unmet.

The native ureq 3.4.2 transport already defaults `TCP_NODELAY` to true
(`src/config.rs`, default configuration, and `src/unversioned/transport/tcp.rs`).
A missing no-delay setting is therefore not a valid explanation. This experiment
isolates the negotiated HTTP version using curl, not a replacement HTTP library.
It does not prove that a new native implementation would perform equivalently.

## Controls and acceptance

- Same frozen 7d4474b client in all epochs, SHA-256
  `a89c983da61666c32a3d4ea2f1dad5a790ef56277b0333709de0451cc8966891`.
- Same `nightly-2026-10-08`, authenticated own-account m8azn host, persistent
  remote target, pinned Desktop 7d38fb7 and Jcode abff94eeb snapshots.
- Each epoch has two warmups and seven measured no-edit FPS test runs. All 27
  existing admission requests returned HTTP 200. All 27 resulting builds have
  `remote.machine == hosted`, exit 0, three passing FPS tests, zero compiled
  units, 744 fresh units, and 1310 filtered tests. No extra admission requests
  or billing bypasses were introduced.
- The scratch PATH wrapper added only an explicit curl `--http2` or
  `--http1.1`, the opt-in timing header, and secret-free response timing capture.
  Actual negotiated protocols were verified as 2, 1.1, and 2, respectively.
  Proxy use was zero throughout. Authentication remained on original stdin,
  normal TLS validation and timeouts remained intact, and output retained the
  original JSON body plus HTTP status suffix.
- Six loopback fixture cases covered protocols 1 and 2 against HTTP 200, 402,
  and 503. Each preserved the original request body, bearer stdin, response
  body and status, and made exactly one request. Fixture protocol 2 was
  intentionally allowed to fall back against the HTTP/1-only fixture server.
- Wrapper SHA-256:
  `73b35a2e9e1a27a7a67d2be9593cb812ab82c096054c37f0f29c33e7302dd881`.
  Sanitized controls, negotiated versions, ingress labels, all request IDs and
  timing fields are preserved in
  `bench/results/hosted-http-protocol-controls-20261013.json`.

No project source, build flags, daemon defaults, billing logic, instance type,
or live worker configuration was changed by this experiment.

## Measurements

| Epoch | No-op wall median | HTTP median | Charge median | Queue median | Measured ingress |
|---|---:|---:|---:|---:|---|
| HTTP/2 before | 0.959 s | 574.375 ms | 247 ms | 74 ms | PDX 7 |
| HTTP/1.1 after | 0.832 s | 439.125 ms | 229 ms | 69 ms | PDX 4, SJC 3 |
| HTTP/2 recheck | 0.841 s | 456.557 ms | 241 ms | 67 ms | PDX 4, SJC 3 |

Exact suite comparisons:

```text
desktop-noop-test  0.96 -> 0.83  -0.13 s  -13%  within noise
desktop-noop-test  0.83 -> 0.84  +0.01 s   +1%  within noise
```

The apparent first-pair reduction does not survive the HTTP/2 recheck. It is
also confounded by changing ingress geography and network outliers. Do not
attribute it to HTTP/1.1, combine it with a different epoch, or claim a speedup.
Worker charge and queue timings measure awaited operations including transit.
They are not pure SQL CPU timings, and overlapping identity time is not additive.

## Source fidelity and restored normal client

After observing different test inventory counts in prior notes, compared every
tracked pinned source file against `git archive` plus declared benchmark fixups:
754 Desktop files and 2110 Jcode files matched exactly, with zero mismatches.
The 734-fresh/1299-filtered figures belonged to the separate live Desktop
acceptance, not this pinned benchmark. No cache contamination was established.
These two workloads must not be combined in a speed comparison.

Normal installed e6a1b5e client check
`20261013-150525491-3437785` subsequently used default hosted routing, returned
exit 0, and emitted native authoritative phase timings. It reused all 56 units,
compiled none, and reported no waste. The first reconnect after the frozen
client took 10.2 s, including 9.530 s sync, not an isolated admission regression.
The experiment's scratch PATH and protocol environment did not alter defaults.
All 27 experiment IDs and this normal-client acceptance ID are exported exactly.
