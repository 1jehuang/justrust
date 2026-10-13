# Cloud control-plane placement trial, October 13, 2026

## Decision and live state

Reject US-west cloud-worker placement because all three matched comparisons
are within noise. Exact prior version `a81f280e-d738-442b-bbfb-9f4fd47fc977`
is restored and healthy. Accepted backend US-west placement remains unchanged.
Own M8azn build host and default nightly remain unchanged. No 10x claim.

Candidate cloud source `e857502`, deployed version
`fa372f4c-4d58-41a0-afcc-ae350b3353bb`, changed only Wrangler placement.
All deployed bindings and script runtime metadata compare byte-equal with the
previous version. No application source, identity lookup, auth, billing, quota,
private-cache permission, EC2 region or fleet default changed. A conditional
15-minute rollback was armed before deployment, then canceled only after exact
prior-version recovery and a real authenticated hosted check.

## Motivation and controlled inputs

Cloud D1 read-only `SELECT 1` returns primary `LAX`, region `WNAM`, zero writes.
The API binding targets the existing US-west billing service. Official placement
documentation supports explicit cloud-region hints and checking `cf-placement`:
`https://developers.cloudflare.com/workers/configuration/placement/`. The
service-binding behavior is documented at
`https://developers.cloudflare.com/workers/runtime-apis/bindings/service-bindings/`.
These motivate an experiment, not proof of latency improvement.

Current public health probes before and after both enter SJC. The earlier
KIX/SYD network epoch has ended without a client network configuration change.
This experiment does not measure a distant-ingress workload or justify a global
placement win. The candidate reports `cf-placement: local-SJC`; after rollback,
the cloud placement header is absent again, API placement remains `local-SJC`.
All 18 raw public health headers and timings are retained in the control JSON.

Same installed client in both phases, binary source `7096694`, SHA256
`df08132d448939a857127f2fe8b039fa3996cee65b27f75d109415378a2caa41`,
verified equal to the saved frozen executable. Both raw before/after headers
retain repository revision `22b1ea10cdaf`. The initially supplied `JUSTRUST_BIN`
is not recognized by the benchmark harness, which selected the installed CLI.
There was no executable drift. Recovery uses the supported `JUSTRUST` selector
and explicit revision overrides. Raw artifacts are not retroactively rewritten.

Pinned Desktop snapshots `abff94eeb` and `7d38fb7`, default
`nightly-2026-10-08`, persistent target, two warmups and seven measured samples
per scenario, buildd/cargo daemon/filter compilation off, actual authenticated
billing admission for every build.

## Matched measurements

`python3 bench/suite.py compare bench/results/hosted-cloud-placement-before-20261013.json bench/results/hosted-cloud-placement-after-20261013.json`

| Scenario | Original | Candidate | Compare verdict |
| --- | ---: | ---: | --- |
| No-op test | 0.673 s | 0.731 s | +9%, within noise |
| Body check | 1.859 s | 1.910 s | +3%, within noise |
| Reference edit/test | 5.202 s | 5.260 s | +1%, within noise |

Compile medians effectively unchanged: body 1.069 versus 1.071 s and
reference 4.388 versus 4.394 s. All 42 matched samples are truly hosted.
Reference and no-op run three passing tests, body checks run zero tests.
Recovery adds seven truly hosted no-op samples, median 0.981 s, range
0.712-1.195 s. This is recovery correctness evidence, not a restored-latency
or isolated placement speed claim.

## Acceptance and exact records

- Cloud candidate and restored-config tests: 68 Node and six Python pass.
- Plain installed default hosted check `20261013-133117022-2854785`.
- Full own hosted tests `20261013-133120739-2855088`: 351 passed,
  zero failed, two ignored, 56 fresh, zero compiled.
- Warning-free hosted lint `20261013-133123662-2855543`: 56 fresh, zero compiled.
- Restored default hosted check `20261013-133228254-2861734`:
  0.6 s client wall, zero compiled, 56 fresh.

All 49 measured samples plus four acceptance/recovery summaries are exported
and committed. Three raw result files and the exact decision/primary-location/
health-probe artifact are retained. Source configuration is restored to its
original no-placement state, with regression guards for unchanged bindings,
host region, cache policy and quotas. No host restart or user-data deletion.
