# Safe sharing rollout, 2026-10-14

This work maximizes sharing of trusted public inputs and artifacts, not sharing of tenant filesystems. Private source, private registries, credentials, mutable Cargo configuration and proc-macro expansion outputs remain account-private. Public publishers are trusted maintenance jobs, never tenant build processes.

## Requirement and observation map

| Requirement | Actual observation | Status |
|---|---|---|
| Share complete immutable toolchains | JR and official nightly prefixes published as content-addressed archives. Fresh HTTPS installs verified compiler, Cargo upstream/fork, std and declared components. Tiny check, three tests, strict clippy and expected compiler failure exercised both prefixes. | Client/publisher integrated. Fleet and future-host opt-in rollout tracked below. |
| Never activate a downloaded experimental compiler accidentally | JR installed with `installed_at=0`, unpinned status stayed system, and repeated install preserved marker inode. | Passed. |
| Reject corrupt or unsafe toolchain packages | Unit and process tests cover schema/identity/digest/size limits, archive traversal and links, incomplete installs, atomic installation, independent-process locks and official fallback. A real nightly package exposed a missing `libexec` compatibility root, which was fixed and rerun without fallback. | Passed. |
| Share large static public artifacts without enlarging tenant upload limits | Synthetic 96 MiB incompressible public bundle published and restored into an empty cache, private upload rejected. Real bedrock artifact restored hash-identically: 129,090,240 raw bytes, 25,839,023 gzip bytes. | Passed. The real sample already fit the old compressed cap, so no old exclusion or speedup claimed. |
| Share public Cargo archive inputs without trusting mutable index state | Pinned HTTPS metadata, manifest and concatenated archive pack restored only verified archive bytes to fresh account Cargo homes. Fresh authoritative sparse index remained necessary. Private registry, git, symlinks, redirects, tampering and overwrite tests reject. | Passed, opt-in. |
| Count the cost of archive sharing | Three alternating full-network seeded/cold pairs: direct cold median 1.057 s, seed download plus verification/restore/fetch median 1.697 s. Seeded fetched 13,340,586 bytes over three GETs versus 9,085,570 archive bytes for direct Cargo. | No default promotion. Sharing works but was slower for this workload. |
| Do not treat native compiler identity as a complete native closure | Native build-script public pack/import rejects host-dependent and historical metadata. Real header mutation and namespace tests show why compiler-only or filesystem-only keys are insufficient. | Guard shipped. Portable native script sharing remains unauthorized. |
| Hash executed proc-macro code, not only Rust metadata | A real proc macro linked native helper versions 1 and 2. Rust metadata was identical but expansions returned different results. Whole executable hash now distinguishes them and invalidates old metadata-only xattr memoization. | Passed. |
| Audit transitive native artifacts as well as script outputs | Ordinary compiled registry rlibs may still be publicly shared. Their native search-path contents and transitive extern hashes require separate analysis, not the script-only guard's proof. | Audit in progress before new fleet publication. |
| Prebuild compatible compiler/profile keys | Fleet runner explicitly pins client digest, validates compiler matrix, separates compiler/profile target directories, disables public reads, and preserves object-before-manifest-before-index ordering. | Executable runner tests and new pinned fleet rollout pending. |
| Keep normal billing, isolation and host lifetime limits | No quota, billing, publishing credentials or tenant sudo relaxation. Current hosted machine reserved by independent profiles; no install or machine changes during that lease. | Source invariants preserved, final installed hosted acceptance pending explicit lease release. |

## Exact code and evidence

Public commits: native script guard `c00e638`, large bundle policy `4915710`, proc-macro hash correction `0944f21`, shared toolchain client `20f5d42`.
Cloud commits: publisher `21f5dd2`, future-host bootstrap opt-in `d5678af`, native non-authorizing rootfs candidate `dea0a38`, pinned HTTPS Cargo archive sharing `6f29c51`.

Detailed component records live in `docs/public-bundle-limits.md`, the shared-toolchain protocol documentation, and the cloud `docs/public-cargo-input-sharing-20261014.md` and `docs/native-closure-acceptance-20261014.md`.

Exact additional local validation records are committed in `data/runs/sharing-20261014/2026-10.jsonl`:

- Proc-macro fix check: `20261013-194428777-731662`.
- Metadata-only memo and executable mutation regressions: `20261013-194435872-732292`.
- Real native helper proc-macro execution proof: `20261013-194627381-739511`.
- Strict all-target lint: `20261013-194630628-740057`.
- Shared toolchain check/scoped/CLI/lint: `20261013-193737970-686755`, `20261013-194118892-716665`, `20261013-194124908-718563`, `20261013-194125234-719016`.
- Full consolidated local tests: `20261013-194746505-749789`, 380 passed, three ignored.
- Final strict local lint: `20261013-194811157-758446`.
- Initial fixture failure retained: `20261013-193744386-687523`, exit 101. Invalid synthetic commit fixtures were corrected; this is not hidden or relabeled as a passing run.
- Isolated exact committed release packaging: `20261013-195503383-785887`, source `20f5d42`, binary SHA-256 `914b157247b2619ba8c77457201439a6b6b33d1ecf9210f462214d8c8f7dad78`. Local compilation was required by the independent hosted lease, not selected as a faster hosted alternative.

Eight actual fresh CDN-installed JR/nightly tiny-workload records are separately committed in `data/runs/xps14-toolchain-download`. JR download/install took 41.3 s and nightly 58.6 s. These are functional install observations, not controlled speed comparisons against source builds or official component downloads.

## Completed earlier fleet run

The independent east prebuild machine `i-0ee2642c767873f92`, launched at 02:32:24 UTC, uploaded its final log at 02:52:22 UTC and terminated. It used the older installed release and one nightly compiler, so it is not proof of the new exact client or JR compiler matrix.

Its actual final log reports 211 new bundles (0.36 GB), 211 manifest lines, zero script runs and 304 skipped. The index grew from 10,513 to 10,724 lines, with new version `uZL8OaIK3KwbCIsu.SX2Sg9S74P1Sjkd` at 02:52:23 UTC. Some builds read old trusted prebuilt entries because the old runner unset the default-enabled store variable. The new runner explicitly disables public reads. Do not call this old run an independent empty-cache experiment.

## Rollout and remaining boundaries

New-host toolchain sharing is a fixed-origin bootstrap opt-in, not arbitrary tenant-selected root extraction. Existing hosts are not silently rewritten. JR remains inactive until explicitly pinned. Registry archive seeding remains off by default because the observed full transfer cost lost to direct Cargo fetching.

Pinned native rootfs publication, verified installer/mount receipts, complete environment and kernel/CPU determinants, audited sandbox execution and native ABI/runtime acceptance are not completed. The prototype says `sharing_authorized=false`. Do not describe the script guard as making all compiled native dependencies unshareable or all remaining native cache keys sound.

No performance speedup is claimed by this rollout document. No matched `bench/suite.py` before/after result was produced for these changes, and noisy concurrent functional test timings are not substitutes.
