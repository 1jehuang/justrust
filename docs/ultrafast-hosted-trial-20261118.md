# Dedicated hosted ultrafast engine trial, 2026-11-18

## Outcome

The existing owned Blaze-derived `toolchain/resident` engine works on a separate
host, including the published client's explicit `justrust native test` entry.
This is not the clean-room compiler and not the shipping rustc resident-query
experiment. Normal justrust routing, installed client, billing, and tenant
infrastructure were not changed.

The five-replicate matched trial passed all correctness gates. Median host-side
command times:

| Phase | Pinned stock Cargo | Owned engine | Cargo / engine |
|---|---:|---:|---:|
| Cold private stores | 10.204164 s | 21.216608 s | 0.481x |
| Warm unchanged tests | 937.242 ms | 33.051 ms | 28.36x |
| Reachable test-body edit | 984.101 ms | 170.721 ms | 5.76x |
| Type error | 594.188 ms | 76.642 ms | 7.75x |
| Type recovery | 979.287 ms | 152.258 ms | 6.43x |
| Assertion failure | 1031.288 ms | 87.031 ms | 11.85x |
| Assertion recovery | 1032.905 ms | 86.198 ms | 11.98x |

All five engine edit samples were 167.129–177.059 ms. Thus this trial establishes
a substantial warm-loop benefit, but **does not reproduce a universal 10x body
edit improvement**. It also makes cold startup about 2.08x slower. The earlier
91.533 ms / 989.706 ms result in `resident-tiers-measured-20261115.md` used a
different engine build, host, and workload protocol. It must not replace this
trial's measurements or be presented as a matched regression comparison.

The observed summed daemon/worker PSS had median 3.207 GiB, range
3.195–3.734 GiB. These are retained post-command snapshots, not peak memory or a
matched stock-memory comparison. Memory scales with retained projects; eviction
and multi-tenant fairness are not validated by this single-project trial.

## Correctness and methodology

`bench/ultrafast_hosted_compare.py`, committed as `8036addf9`, compares the exact
historical source tree `f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d` using five
independent private stores per engine. The first stack alternates by replicate,
and phases use matched source bytes. Both arms have the same pinned nightly,
debug/optimization settings, disabled remote artifact publishing, and proc-macro
compatibility settings. This compares combined compiler stacks, not residency
alone.

Every successful filtered run requires the exact four summary test names and
counts. Changed runtime phases require fresh nonces; the unchanged phase repeats
the same source and nonce deliberately. Type-error and assertion-failure phases require status
101 and the appropriate diagnostics/runtime behavior. Recovery must execute the
new source. Both full-library gates passed **147 tests, 1 ignored**, with exact
name/status parity. All pristine input and restored private-copy hashes matched,
and engine/toolchain identities were unchanged. The benchmark withholds medians
if any correctness, full-library, identity, or cleanup gate fails.

Cold means new private target/runtime/artifact stores, not flushed OS page
caches. These measurements exclude SSH/network transport, provisioning, initial
engine compilation, and a production service scheduler. The source is one small
real historical project, not the comprehensive session-weighted workload.
No broad default promotion, upstream Blaze equivalence, arbitrary Rust support,
paid-tier readiness, or general 10x claim follows.

## Exact hosted candidate

- Dedicated operator instance: `i-0e0843922bdb5f48d`, `m7i.4xlarge`, 16 vCPU,
  64 GiB nominal RAM, no swap, kernel `7.0.0-1017-aws`.
- Fixed image: `ami-02974d59566764d0b`.
- Public source archive from stable `1816db3247628b9f4ddceb91d618b612dc696ce4`.
- Exact compiler commit: `16d1ffac17ad8b739e32483ac2e65fe7e1804977`,
  `nightly-2026-10-27`.
- Engine ELF SHA256:
  `f0f968042f3f9f0b8ff433361ea0812b6189f617975041a6ac8695bb00f356fa`.
- Engine source identity:
  `resident-v1-0604af55167213a1e1ee33940c5cc66164e9901ed96228eace0d017427943c72`.
- Published client: `0.1.0-alpha.37`, SHA256
  `bd1be5b7b3c731ecc47760b611533c27d94e6b6ce3a3b6104c7be3858375fe66`.

The engine was compiled directly on this host through `build-jr.sh` using
justrust, not built locally and transferred. The release build took 198.8 s,
including dependency acquisition. No instance IAM profile, account credentials,
cache publishing tokens, or private source were copied. SSH was restricted to
the operator address, with the host key authenticated through AWS SSM. An
independent four-hour shutdown timer stops the instance; its encrypted disk is
retained for reproducibility.

The stock and engine trial used an isolated copy of the rustup-installed
matching toolchain. Its metadata uses the original distribution manifest, whose
SHA256 is `34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.
Rustup's rewritten installed manifest was preserved separately. Compiler files
were not substituted. Full toolchain file hashes are in the generated metadata.

## Host prerequisite discovered by actual execution

The initial five-replicate trial failed, while Cargo passed. Native workers
reported `/proc/self/setgroups: Permission denied`, followed by a misleading
`Cli::parse()` resolution error. Kernel audit identified AppArmor's restricted
unprivileged namespace profile. Do not suppress E0599 or disable diagnostics to
make this appear successful.

On this dedicated operator host, an executable-specific AppArmor profile enabled
`userns` for `/home/ubuntu/ultrafast/engine/jr-native` using Ubuntu's unconfined
profile mode. The global `kernel.apparmor_restrict_unprivileged_userns=1` remained
unchanged. The identical engine, source and published client then passed. Fresh
stores were used for the successful timed trial. This is an operator-host
prerequisite, not a tenant sandbox security proof or permission to weaken
production host isolation.

The build helper also now supplies its concrete exact-toolchain local-build
reason, preserving an explicit operator override. A fake-tool regression checks
both cases. An actual hosted incremental invocation with the reason unset
succeeded and produced byte-identical engine executables.

## Original evidence

All benchmark JSON and full raw stdout/stderr bundles are preserved. Account-private
record IDs:

- `bench/ultrafast-hosted-20261118-0329-metadata`
- `bench/ultrafast-hosted-20261118-0329-summary`
- `bench/ultrafast-hosted-20261118-0329-records-lossless`
- `bench/ultrafast-hosted-20261118-0329-raw-lossless`

The failed predecessor is preserved under the analogous `0320` IDs, not rewritten
as success. Lossless uploads verify exact authenticated reconstruction of original
bytes. The successful records JSON SHA256 is
`e7ba904960009596da4abafe1ac87b5cbfecf8ded61c2e4ab5e6cc5595b6c471`.

A premium selectable mode still needs authenticated service routing, tenant-safe
worker lifetime and resource admission, broader workload/compatibility gates,
network-inclusive measurements, and pricing decisions. This experiment creates
none of those implicitly.
