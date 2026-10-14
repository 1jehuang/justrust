# Native platform rollout (2026-10-14 UTC)

## State and acceptance gates

This is an implementation rollout, not a claim that hosted Windows/macOS builds
are already available. The existing hosted transport provisions account-isolated
Linux VMs. Its source sync, remote agent installation, execution, and release
artifact path contain Linux assumptions. Returning a Windows or Mac address from
that API without replacing those assumptions is not platform support.

The client explicitly requests `platform: linux` and rejects a ready response
with any other declared platform. Missing platform retains the legacy Linux
contract. Native platforms must remain disabled until their real build, test,
and artifact round trip succeeds. Strict remote-required behavior must never
silently build the wrong target or fall back to local execution.

## Architecture

- Retain the Linux path and its account isolation. Cross-compilation is opt-in
  per supported target and validated native dependency graph, not a global flag.
- Add owner-exclusive native release workers separately from public tenant
  execution. Trusted Jcode release jobs can validate the worker lifecycle before
  exposing any customer API. Native tests execute on the target OS.
- Keep a shared control plane, source distribution, and artifact storage. Cache
  keys include the compiler host, target, exact compiler, flags, ABI/sysroot,
  and relevant inputs. Build scripts and proc macros run on the compiler host,
  so matching target alone cannot establish artifact compatibility.
- Tenant data remains account-prefixed. Only trusted prebuild infrastructure may
  publish shared dependencies. Native customer workers receive neither shared
  publication tokens nor release/signing credentials.
- Package/sign/publish are distinct trusted release stages. A successful compile
  is not permission to publish or to expose credentials to repository code.

## macOS gate

Read the applicable Apple license and AWS terms before activation. macOS Tahoe
Section 3 leasing conditions include a minimum 24-hour lease, exclusive use of
both software and Apple hardware during the lease, advance Apple notice, and
lessee agreement requirements. Do not implement concurrent unrelated tenants on
one Mac by treating separate users or containers as a legal/isolation boundary.
A Mac reserved for our own trusted release work is not a validated third-party
multitenant service.

AWS Mac Dedicated Hosts have a minimum 24-hour allocation. Stopping the instance
does not release the host or end its allocation. AWS pauses Dedicated Host
metering during pending scrubbing, so allocation lifecycle is not identical to
instance uptime. Never reuse Linux idle reclamation
logic for a Mac allocation. Lifecycle cleanup must distinguish the instance,
disks, and Dedicated Host and must preserve user data.

Primary references:
- https://www.apple.com/legal/sla/docs/macOSTahoe.pdf
- https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-mac-instances.html
- https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/mac-instance-stop.html

## Required real acceptance

1. Build a pinned Jcode revision and target with pinned toolchain/SDK inputs.
2. Return the named artifact with checksum verification and verify PE/Mach-O
   architecture rather than accepting filename or executable permission alone.
3. Run native version/smoke tests and selected project tests.
4. Repeat warm and after a source edit. Record durations and cache reuse without
   claiming a speedup absent the pinned benchmark suite comparison.
5. Confirm tenant isolation, denied credential forwarding, bounded admission,
   concurrency, retry idempotency, and cleanup behavior.
6. Install/deploy only verified support. Unsupported routes remain explicit
   errors until end-to-end acceptance passes.

## Initial client validation

Hosted check `20261013-204638704-1088545` passed, followed by 25 hosted client
unit tests in `20261013-204643969-1089027`. Initial clippy run
`20261013-204647055-1089220` identified a collapsible conditional, fixed before
final validation. Later local validation avoids the separately reserved hosted
benchmark window; it does not establish native platform acceptance. Final local
warning-free clippy passed in `20261013-204744502-1094554` and all 25 hosted
client tests passed locally in `20261013-204753721-1095315`.
