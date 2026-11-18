# Official upstream Blaze comparison, 2026-11-18

This follows `ultrafast-hosted-trial-20261118.md`. The earlier successful
Cargo comparison did not establish how our derived engine compares with official
upstream Blaze. This experiment adds actual published upstream binaries rather
than treating a source-built reference or an old marketing number as upstream.

## Release selection and identity

At approximately 04:18 UTC on November 18, the official HTTPS release channel
pointers named:

| Channel | Published version | Native version output |
| --- | --- | --- |
| stable | `0.20261027.0` | `blaze 0.20261027.0 (49ca152)` |
| canary | `0.20261105.4` | `blaze 0.20261105.4 (e930296 2026-11-05)` |

Stable and canary are distinct controls. Current main's README describes v2 and
explicitly labels its advertised performance numbers as v1 measurements. Neither
main nor the canary pointer is a substitute for the stable channel pointer.

The archive hashes below were checked against each release's HTTPS `SHA256SUMS`:

| Artifact | SHA256 |
| --- | --- |
| stable Linux x86-64 archive | `df61ce6d8216087a40e986d4f84cf2a230638bde014e38519e08b50282e90e5d` |
| stable native `ra-jit` | `04e84de57aad8a19102edd454e2cdd8ac2e4b58461a4070479919207f3a5590e` |
| canary Linux x86-64 archive | `a8b8f3b36b7397671d3a5bcf739e5e1407e5941ee2345aaf5dc2b0868d44d8e6` |
| canary native `blaze-engine` | `65a3a904d5b3d9ebe5487b8573d3b39c84b5405649a4dd94c9c3626d1dff41da` |
| our native `jr-native` | `f0f968042f3f9f0b8ff433361ea0812b6189f617975041a6ac8695bb00f356fa` |

Our engine reports the Blaze-derived origin
`5d4f78f7577cf7170993b74f735099dd03f5c6ca` and owned build identity
`resident-v1-0604af55167213a1e1ee33940c5cc66164e9901ed96228eace0d017427943c72`.
Matching package version numbers therefore do not establish identical binaries
or source. The upstream artifacts retain their original licenses and companions.

## Dedicated real-host protocol

The existing isolated operator host was restarted, not replaced: AWS instance
`i-0e0843922bdb5f48d`, `m7i.4xlarge`, 16 vCPU and 64 GiB nominal memory.
The root disk was retained. Its new public address was authenticated against its
SSM-returned SSH host key. No instance IAM profile, account credentials, private
project source, cache publishing keys, tenant routing or billing changes were
introduced. A finite four-hour stop timer was armed again.

The shared pinned fixture is the public justrust tree
`f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d`. Each arm and repetition gets its
own source copy, Cargo cache, target directory, compiler state and runtime
namespace. The original source and toolchain are hash-checked before and after.
Cold means empty private build stores, not a flushed OS page cache. Registry
fetches remain part of cold latency. Commands execute serially in rotating arm
order. Source restoration uses private byte backups. Only each experiment's
private daemons are stopped.

The ordinary edit test is a reachable body change in four selected summary
tests. Fresh runtime nonces, exact names/status/counts, a deliberate type error,
a failing assertion and both recovery paths are mandatory. The first repetition
also requires full-library parity: 147 passed, one ignored, 148 total, no filtered
tests. Failed correctness or identity gates withhold all speed medians.

The pinned Cargo baseline uses the exact October 27 nightly from the earlier
trial. Official v1 is invoked as `ra-jit cargo test`, our engine as
`jr-native test`. Stable uses the same explicit profile and macro-server policy
as our engine. Its executable received the same narrowly scoped AppArmor user
namespace permission as ours. The global unprivileged-userns restriction remains
on. No compiler errors were suppressed and no fallback was added.

## Official stable: compatibility gate fails

The actual five-repetition three-arm run used frozen harness commit
`718cecfd7`. It completed in 426 seconds and preserved 194 command/probe records.

| Arm | Accepted workload observations | Full-library gate |
| --- | --- | --- |
| pinned Cargo | 36 of 36 | passed |
| our derived engine | 36 of 36 | passed |
| official stable | 0 of 36 | failed before tests ran |

The 36 observations are seven phases in each of five repetitions plus one
full-library gate. Version and cleanup records are not workload observations.
All source, executable and pinned-toolchain identities remained unchanged;
private cleanup and source restoration passed.

Official stable repeatedly refuses the ordinary successful builds because
reachable standard-library code contains writable statics without a rustc
symbol. The diagnostics name `__RUST_STD_INTERNAL_VAL` and `MIN`, including paths
through `test::test_main_inner` and thread spawning. Its normal, error and recovery
observations therefore do not satisfy the same runtime/diagnostic oracle.
This is not the earlier AppArmor namespace failure: that prerequisite was already
provided equally to both engines. Full worker logs are preserved as well as the
command output, including the diagnostic summary's omitted functions.

**There is no valid stable-versus-derived speed ratio on this fixture.** A failed
compile is not a fast build and cannot be ranked against successful test
execution. Conversely, this single compatibility result does not prove our fork
is universally better than upstream.

Untouched originals were stored under account-private records:

- `bench/ultrafast-upstream-stable-20261118-0423-metadata`
- `bench/ultrafast-upstream-stable-20261118-0423-summary`
- `bench/ultrafast-upstream-stable-20261118-0423-records-lossless`
- `bench/ultrafast-upstream-stable-20261118-0423-raw-lossless`
- `bench/ultrafast-upstream-stable-20261118-0423-workers-lossless`

The lossless archive helper authenticated reconstruction of every original byte.
The records array is 605,818 bytes, SHA256
`bc0705211bb8200d45e923e63b8aa0edd926735c59e0d5d76010c39751e5b198`.

## Newer canary host prerequisites

Canary is not silently treated as stable. Its renamed native entry uses direct
`test`, and its first version line is pinned independently. Harness commits
`960fa9566` and `e02f064ee` add explicit version and CLI-mode options while
preserving the two-arm and stable defaults. Thirty Python mock tests pass.

The initial canary probe correctly refused the legacy `BLAZE_BACKEND` override
because that setting was not represented in its unit cache key. The corrected
probe removes legacy backend/optimization overrides and uses canary's native
defaults, with the same Cargo profile requests. This configuration difference
must remain visible, not be presented as a pure measurement of fork overhead.
`BLAZE_TOOLCHAINS` points to the existing pinned toolchain directory.

The next probe found that canary requires block-sharing file placement and
explicitly refuses to copy on this host's ext4 filesystem. To satisfy that host
requirement without changing existing data, a fresh 32 GiB sparse image was
formatted as reflink-enabled XFS and loop-mounted at `/home/ubuntu/ur`. No existing
filesystem was formatted. All arms of the subsequent matched comparison use
this same XFS mount, rather than giving one arm a different filesystem.

On XFS, canary successfully compiled the fixture from fresh private stores and
ran the exact four selected tests: four passed, 144 filtered. This is a
compatibility preflight, not performance certification or full-suite acceptance.
The failed legacy-environment and ext4 probes were preserved, not overwritten.
Their raw receipts, the successful preflight, and official channel/checksum
files are losslessly archived together as
`bench/ultrafast-upstream-20261118-prerequisites-lossless`, original SHA256
`ca7c5649d4b42c7ce82cf96906e8bfd070d5e754c42be94e964bb8f36666d10c`.

## Completed newer-canary five-replicate trial

The XFS trial completed at approximately 04:54 UTC on November 18, using the
frozen `e02f064ee` harness and the separately pinned canary executable above.
All five repetitions ran. The coordinator connection was interrupted by a
server reload, but the existing remote harness survived and was observed to
completion without rerunning or replacing its inputs.

| Stack | Accepted workload phases | Rejected workload phases |
|---|---:|---:|
| Stock Cargo | 36 | 0 |
| Owned Blaze-derived engine | 36 | 0 |
| Official canary | 30 | 6 |

Canary passed cold compilation, same-byte rewrite, ordinary edit, type-error
and recovery, and intentional assertion-failure checks. However, the subsequent
assertion recovery failed in **all five independent repetitions**. The full
library check in repetition zero also failed. Every rejection had the same
native linker diagnostic: `span 131 of ecedf895ca59, whose table has 111`.
The filtered test executable was never built in these rejected phases. We did
not reset the cache to hide recovery failure or count failed compilation as a
fast successful build. The owned engine and Cargo passed both the corresponding
recovery phases and the full library oracle.

Source, executable, and toolchain identities remained unchanged. All fifteen
private source copies were restored, and `cleanup_errors` is empty. The generated
summary correctly says `failed/incomplete`, with **empty timing medians** and
failed full-library parity. There is no correctness-qualified canary-versus-owned
speed ratio from this trial. This fixture supports keeping our existing derived
engine for further experimentation, not a claim that our fork is universally
faster or more correct than upstream.

The phase historically named `warm-unchanged` rewrites identical source bytes
before invocation. It measures a same-content file touch, **not an untouched
no-op build**. In the first repetition canary took about 41.7 seconds there and
recompiled dependencies, while the owned engine took about 30 milliseconds.
Those are individual diagnostic observations, not certified aggregate speed
claims. Commit `326a1c4e3` makes this distinction explicit for future metadata
and corrects the earlier owned-versus-Cargo report without changing old receipts
or the frozen running protocol.

Untouched canary originals were archived as:

- `bench/ultrafast-upstream-canary-20261118-0435-metadata`
- `bench/ultrafast-upstream-canary-20261118-0435-summary`
- `bench/ultrafast-upstream-canary-20261118-0435-records-lossless`
- `bench/ultrafast-upstream-canary-20261118-0435-raw-lossless`
- `bench/ultrafast-upstream-canary-20261118-0435-workers-lossless`

The archive helper verified authenticated byte-for-byte reconstruction of the
records and raw bundles. The records array is 624,764 bytes with SHA256
`41998aa76c0a8460e3290b194ff0bdc8f0fa790aa544f2116aff4d8d060cd8fb`.

## Host shutdown

After authenticated archival, the dedicated instance `i-0e0843922bdb5f48d`
was stopped and its `stopped` state verified at 04:56 UTC on November 18.
The encrypted evidence disk `vol-0bc1075a06eec2b20` remains attached and retained.
No production routing, billing, installed default client, or tenant data changed.

## Limits

These are operator-host compiler-stack experiments on one pinned project, not a
network-inclusive hosted product benchmark, a session-weighted synthetic suite,
a multitenant security acceptance, or authorization to promote a paid default.
Our source-built fork and the official release can differ in build settings and
compiler architecture. Any successful canary timing comparison is descriptive
of those actual configured stacks, not proof that a single patch caused a win.
