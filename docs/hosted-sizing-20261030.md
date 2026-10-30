# Hosted account sizing rollout, October 30, 2026

## Live configuration

The owner's existing AWS account host was resized in place from
`m8a.24xlarge` (96 physical cores, 384 GiB provisioned RAM) to `m8a.8xlarge`
(32 physical cores, 128 GiB provisioned RAM). EC2 and the running guest both
confirmed the smaller shape. The guest reports about 123.5 GiB usable RAM.
The instance identity, EBS root volume, attached-volume mappings and ownership
tags were preserved. The existing `/bin/sh` to Bash maintenance fix survived.

This is an account-host operational change, not a global fleet instance-family
promotion. The fleet provisioning default was already `c7i.8xlarge` (32 vCPUs)
and remains unchanged. Existing stopped/started account hosts retain their
selected EC2 type. Replacement hosts follow fleet configuration.

The private cloud repository now contains an explicit operator selector:

```sh
cd ~/justrust-cloud
python3 tools/host_size.py default  # m8a.8xlarge
python3 tools/host_size.py high     # m8a.24xlarge
```

A real resize requires a separately stopped, idle host and maintenance
coordination. The tool never stops, starts, terminates or replaces a machine.
A running host already at the requested type is a verified read-only no-op.
This is not per-build scheduling and there is no automatic reversion. See
`docs/host-sizing.md` in the private repository for the maintenance sequence.

## Deployment and safety

- Coordinated a pause on new hosted work with concurrent agents. Waited for
  active builds and a native cache benchmark to finish and for the benchmark
  owner to retrieve its artifacts.
- A remaining lifecycle record belonged to a daemon-reload-interrupted build.
  Its owner explicitly confirmed abandonment and no remaining work. Reconciled
  that record through the authenticated run-event API, not a direct DB edit.
- Stopped through `justrust remote down`, waited for EC2 `stopped`, resized with
  the selector, and restarted through normal `justrust remote up` admission.
- The initial immediate post-mutation EC2 read briefly returned the old type.
  The tool failed closed rather than starting or rolling back. A subsequent
  independent EC2 read confirmed the target type and identical disks/tags;
  a read-only selector rerun passed before the normal start.
- Cost reporting includes the verified small-host rate, USD 1.94752/hour in
  us-east-1 and us-west-2. AWS Price List API verified both regions on this date.
  This is instance-compute reporting, not a claim about total bill savings.
- Fixed stopped-host status reconciliation so current type/cost refreshes
  without accruing stopped runtime or repricing historical intervals.
- Private cloud commits: `7716a1f` cost reporting, `a450f8c` selector,
  `cb2953f` stopped-host status reconciliation. The latter was deployed as
  Worker version `ebb48307-d4b7-4139-a4c1-710de884845d`.

## Measurement scope

No compile speedup or equal-performance claim is made. The comprehensive
hosted baseline was attempted with:

```sh
python3 bench/versus.py run --synth 1 -n 1 --only remote --remote \
  --no-instructions --no-report
```

It could not acquire the shared snapshot lock, held by unrelated ongoing
benchmarks. Only this waiting attempt was cancelled. The other benchmarks
were not interrupted. There is no completed matched comprehensive comparison,
and targeted correctness checks do not substitute for that gate. The smaller
account machine is the user's explicitly requested operational setting, not
a measured global-default promotion.

## Acceptance

The pre-resize forced-hosted client check passed, run
`20261029-212935144-2928945`. This is a correctness observation, not a paired
performance baseline. Live post-resize API status reports `running`,
`m8a.8xlarge`, and USD 1.94752/hour. EC2 confirms the same root volume and
instance. The guest reports 32 CPUs, and the running-target selector no-op
passes with EBS mappings unchanged.

Post-resize forced-hosted acceptance:

| Command | Run | Outcome |
| --- | --- | --- |
| `justrust check --remote` | `20261029-213409377-3008036` | Passed, 62 fresh units, no compilation. First connection remirrored sources in 50.167 s, with 1.30 s host work. |
| `justrust test --remote` | `20261029-213502310-3012481` | 837 passed, 0 failed, 10 ignored. |
| `justrust clippy --remote --all-targets -- -D warnings` | `20261029-213518592-3013767` | Passed. |
| Desktop FPS smoke | `20261029-213523554-3008035` | Blocked by four current source-fixture E0063 errors: missing `effective_budget` in `ApiEvent::TokenUsage` initializers. The host executed the build and reported the errors in full. No Desktop source edits were made. |

The cloud suite passed 277 Node tests plus 70 existing Python tests and 15
initial selector tests. Later selector consistency tests are recorded in the
private repository. The hosted maintenance pause was explicitly released after
the client check, full tests and strict Clippy passed. Selected redacted run
summaries, including the failed Desktop build, are committed in
`data/runs/hosted-sizing-20261030/2026-10.jsonl`.
