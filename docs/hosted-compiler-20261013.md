# Current hosted compiler comparison, 2026-10-13

## Controlled inputs

Pinned suite snapshots: Desktop `7d38fb7`, Jcode `abff94eeb`.
Frozen client: `7096694`, SHA-256
`df08132d448939a857127f2fe8b039fa3996cee65b27f75d109415378a2caa41`.
Own authenticated host: M8azn, AMD EPYC 9R05, 24 cores, same retained volume.
Each compiler uses its own persistent target namespace, two discarded warmups,
and seven measured samples per scenario. This is warm incremental execution,
not a cold-host versus cold-local comparison. Buildd, test-filter compilation,
and Cargo daemon were explicitly off. Existing JR proc-macro/interface features
remain at their normal candidate defaults. No fleet or pricing change was made.

## Authoritative suite comparison

`python3 bench/suite.py compare bench/results/hosted-compiler-m8azn-nightly-before-20261013.json bench/results/hosted-compiler-m8azn-jr-after-20261013.json`

| Scenario | Default nightly | Optimized JR | Suite verdict |
|---|---:|---:|---|
| desktop-noop-test | 1.309 s | 1.487 s | +14%, within noise |
| desktop-body-check | 2.437 s | 2.465 s | +1%, within noise |
| desktop-test-edit | 5.803 s | 5.012 s | -14%, faster |

The reference compile phase falls 4.402 to 3.654 seconds. Body-check compile
instead rises 1.052 to 1.134 seconds (about 8%); do not claim every workload wins.
All 42 measured samples explicitly report hosted execution and zero failing
filtered tests. Every reference/no-op sample passes the same three tests.
Their exact exported IDs are committed, not inferred from the machine hostname.

## Network and correctness qualifications

The accepted backend version remains `b0d2a727-9cf0-4bef-ae66-dddec389686b`
with US-west placement. Actual health probes during/after this comparison have
`cf-ray` KIX/SYD ingress and `cf-placement: remote-SJC`, unlike the earlier
SJC-ingress epoch. Raw credential-free headers and timing data are preserved in
`bench/results/hosted-compiler-network-20261013.json`. Higher client-visible
transport here must not be attributed to the compiler or called a backend
placement rollback. No-op overhead remains a major limitation.

Full pinned UI tests on both compilers report 1302 pass, 1 fail, 10 ignored.
The same viewport assertion also fails on isolated default nightly. The
caller-umask bug was fixed separately without weakening security checks. See
`docs/hosted-umask-20261013.md` for all correctness IDs, original failures,
legacy/concurrent-child compatibility and artifact reuse regressions.

## Decision

Keep `nightly-2026-10-08` as the default. `jr-b7907f1fe8f7` remains staged
and opt-in: the reference loop wins, but body/no-op do not, and the full pinned
UI correctness gate is not clean. This comparison establishes a 14% reference
improvement, not 10x against stock. The prior round-robin stock-local result is
separate evidence and has different ingress geography, so its numbers cannot be
combined with this trial to manufacture an overall speedup.
