# Resource-sharing experiment

This checkout is an explicitly user-authorized isolation exception to the normal
stable-only workflow. Work on `experiment/resource-sharing` here. Do not switch
`~/justrust` off stable, create worktrees, or merge stable with the clean-room
main branch. This independent clone intentionally excludes uncommitted work
from the shipping checkout and has no copied target directory.

## Entry points

- Application launcher: **Justrust Resource Lab**.
- Client experiment: `~/justrust-resource-lab`.
- Private infrastructure experiment: `~/justrust-cloud-resource-lab`, same branch.
- Original shipping checkouts remain `~/justrust` and `~/justrust-cloud`.

Read `~/justrust-cloud-resource-lab/docs/resource-canary.md` for the dedicated
canary lifecycle and acceptance boundary. The canary starts as a disposable,
time-bounded infrastructure probe, not a production-sized benchmark host or a
finished hosted-build route. Do not put retained account data on its scratch disk.
The desktop launcher opens Jcode on the experimental branch.
It does not change justrust credentials, service routing, the installed client,
or automatically send a build to the canary. Plain justrust still uses its
existing configured service until an explicit supported canary route exists.

## Implementation map

The private experimental checkout owns multitenant resource control and storage.
Start with `~/justrust-cloud-resource-lab/docs/resource-sharing-integration.md`
for the component contracts and outstanding acceptance gates. Its pure allocator
is `pool/resource_policy.py`, the confirmed RAM actuator is
`pool/resource_controller.py`, and trusted-public immutable disk publication is
`pool/shared_artifacts.py`. Guest, compiler-pmem and isolated client-dogfood
runners live under `infra/resource-canary-*` with corresponding tests and docs.

The public client does not need a new default installation just to exercise an
owner-authorized experimental SSH route. The dogfood runner uses a separate
client home and ephemeral transport identity. A successful self-managed route
is not proof of production hosted admission or billing integration. Do not run
these routes against retained account data or describe public file-page sharing
as shared private compiler heaps.

## Delivery sequence

1. Verify a dedicated canary host, KVM, isolated disks, and bounded lifecycle.
   Never register it into production placement merely to get build admission.
2. Capture a baseline using the existing benchmark/recording tools. Use
   representative cold, incremental, debug/release and concurrent workloads.
3. Implement coordinated CPU/RAM admission and safe memory reclamation. Keep
   tenant VM isolation, trusted host telemetry and a physical host reserve.
4. Separate public immutable cache artifacts from private account data and
   disposable scratch. Only trusted publishers write the shared cache. Never
   classify an artifact as public merely because its package name is public.
5. Prototype shared compiler state only after measuring duplicated memory.

Each stage ships independently behind experimental capability/version gates.
Do not describe setup, a design document, or a passing unit test as a working
shared-memory compiler or completed hosted dogfooding.

## Safety and promotion

- Scratch canary accounts and disks only. No production account-image mounts,
  copied credentials, cache publication tokens or shared writable ext4 images.
- Use an explicit candidate executable path if client changes become necessary.
  Keep the default installed CLI on the published release.
- Preserve normal authenticated admission and billing for service dogfooding.
  An operator infrastructure probe is not a charged hosted build and must not
  be represented as one.
- Verify isolation, host memory exhaustion behavior, pressure-driven admission,
  interrupted operations, restart persistence and rollback before promotion.
- Performance claims require the pinned comprehensive suite before/after,
  identical workload parameters and recorded run IDs. No speedup is claimed by
  the lab setup itself.
- Changes here are not promoted automatically. Integrate individually reviewed,
  tested improvements into the shipping branches, never merge clean-room main.

## Launcher check

`JCODE_BIN=/usr/bin/pwd bash scripts/resource-lab-launch.sh` checks the actual
entrypoint and working directory without opening an interactive session. The
launcher refuses a missing checkout, detached HEAD or an unexpected branch,
rather than silently checking out a branch over someone's work.
