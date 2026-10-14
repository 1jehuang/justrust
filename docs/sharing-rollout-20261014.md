# Safe sharing rollout, 2026-10-14

This work maximizes sharing of trusted public inputs and artifacts, not sharing of tenant filesystems. Private source, private registries, credentials, mutable Cargo configuration and proc-macro expansion outputs remain account-private. Public publishers are trusted maintenance jobs, never tenant build processes.

## Requirement and observation map

| Requirement | Actual observation | Status |
|---|---|---|
| Share complete immutable toolchains | Initial JR and nightly HTTPS installs passed tiny check/test/lint/failure workflows. A later actual Ubuntu load found a missing rustfmt driver in the short JR ID. A corrected full-commit ID now passes compiler, Cargo, rustdoc, rustfmt and clippy loader checks on Ubuntu; the old immutable object is preserved and rejected on explicit reuse. | Corrected package accepted without default activation. New-tenant bootstrap and fleet reuse remain separately bounded below. |
| Never activate a downloaded experimental compiler accidentally | JR installed with `installed_at=0`, unpinned status stayed system, and repeated install preserved marker inode. | Passed. |
| Reject corrupt or unsafe toolchain packages | Unit and process tests cover schema/identity/digest/size limits, archive traversal and links, incomplete installs, atomic installation, independent-process locks and official fallback. A real nightly package exposed a missing `libexec` compatibility root, which was fixed and rerun without fallback. | Passed. |
| Share large static public artifacts without enlarging tenant upload limits | Synthetic 96 MiB incompressible public bundle published and restored into an empty cache, private upload rejected. Real bedrock artifact restored hash-identically: 129,090,240 raw bytes, 25,839,023 gzip bytes. | Passed. The real sample already fit the old compressed cap, so no old exclusion or speedup claimed. |
| Share public Cargo archive inputs without trusting mutable index state | Pinned HTTPS metadata, manifest and concatenated archive pack restored only verified archive bytes to fresh account Cargo homes. Fresh authoritative sparse index remained necessary. Private registry, git, symlinks, redirects, tampering and overwrite tests reject. | Passed, opt-in. |
| Count the cost of archive sharing | Three alternating full-network seeded/cold pairs: direct cold median 1.057 s, seed download plus verification/restore/fetch median 1.697 s. Seeded fetched 13,340,586 bytes over three GETs versus 9,085,570 archive bytes for direct Cargo. | No default promotion. Sharing works but was slower for this workload. |
| Do not treat native compiler identity as a complete native closure | Native build-script public pack/import rejects host-dependent and historical metadata. Real header mutation and namespace tests show why compiler-only or filesystem-only keys are insufficient. | Guard shipped. Portable native script sharing remains unauthorized. |
| Hash executed proc-macro code, not only Rust metadata | A real proc macro linked native helper versions 1 and 2. Rust metadata was identical but expansions returned different results. Whole executable hash now distinguishes them and invalidates old metadata-only xattr memoization. | Passed. |
| Audit transitive native artifacts as well as script outputs | A real bundled C helper changed from 1 to 2 with identical Rust metadata. The old CLI restored stale output 1; complete `.rlib` hashing plus memo namespace `m3` produced miss/output 1, miss/output 2, then unchanged hit/output 2 through the actual shim interface. | Observed collision repaired and regression shipped. This is not universal native-closure authorization. |
| Prebuild compatible compiler/profile keys | Fleet runner explicitly pins client digest, validates compiler matrix, separates compiler/profile target directories, disables public reads, and preserves object-before-manifest-before-index ordering. Eight executable runner tests and the actual AWS template validation passed. | Pinned fleet launched at 03:21:05 UTC, runtime publication and fresh reuse not yet accepted. |
| Keep normal billing, isolation and host lifetime limits | Exact committed client `92bc376` installed, then truly plain check/full 382 tests/strict all-target lint/repeated check all ran hosted. A new Git root with a tracked `src/target` module compiled and passed its test while an ignored private sentinel stayed absent from the mirror. Post-deploy checks exercised normal account authorization and billing. | Passed on the existing west 24-core account host. New-host root bootstrap is separately bounded below. |

## Exact code and evidence

Public commits: native script guard `c00e638`, large bundle policy `4915710`, proc-macro hash correction `0944f21`, shared toolchain client `20f5d42`, complete native archive hash correction `fe472c6`, actual shim integration `7a7515f`, combined source-sync integration `92bc376`, helper loader validation `617733d`, non-mutating installed-prefix reuse validation `ec42e0f`.
Cloud commits: publisher `21f5dd2`, future-host bootstrap opt-in `d5678af`, native non-authorizing rootfs candidate `dea0a38`, pinned HTTPS Cargo archive sharing `6f29c51`, pinned fleet runner `5e9fa37`, eight executable runner tests `6a6e547`, stack parameter contract `9bcb368`, future-host distribution enablement `42e3fcc`, publisher helper loader gate `9c8b702`.

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

## Accepted installed client and current deployment

The exact combined committed source `92bc376` was installed at 03:28:30 UTC, excluding unrelated dirty daemon source. That initial accepted CLI was version `0.1.0-alpha.16`, SHA-256 `f3dcac67099a29418eb3c7c230fbe90a765804606111750bf9666bcf86242c83`. It is a historical accepted epoch, not an assertion about the current mutable global CLI. Plain existing-host acceptance completed at 03:28:52 UTC:

- check `20261013-202837489-988095`;
- full 382 passed, three ignored `20261013-202840364-988347`;
- strict all-target lint `20261013-202850469-989124`;
- repeated check `20261013-202851219-989168`.

All four actually ran hosted with the pinned nightly and 62 fresh units, not the legacy run-ID override path that forces local execution. A fresh Git-listed `src/target/value.rs` root then passed check/test/strict lint/recheck (`20261013-203001689-994801`, `20261013-203003356-995315`, `20261013-203003944-995374`, `20261013-203004439-995423`). The test returned 42 and the ignored sentinel was absent on the actual host. No manual source copy was used. An unrelated desktop release build was observed on the machine, so these functional results are not quiet timing comparisons.

Worker version `6b86ccf2-312e-434d-9bf9-9469ad51c39a` was deployed at 03:29:13 UTC with fixed trusted distribution enabled only for future bootstraps. The actual version metadata and dry run retain the existing D1, API and R2 bindings, west region, instance types, eight-host cap, 15-minute idle policy and private-cache limits. Rollback target is `5056707f-3f2a-4daf-bdb2-e758a0cf3c77`. The first post-flag cloud suite correctly failed its old exact-variable invariant; the approved flag was added without relaxing any other expected value, then all 95 Node and 11 Python tests passed before deployment. A separate initial command invoked npm from the Rust repository and exited before any mutation, then was rerun from the cloud repository.

The independent pinned fleet `i-0e641b79dc37fb3a3` launched at 03:21:05 UTC. The actual stack update completed with launch-template version 2 and client SHA `04662c0de99f0da0d7b18c49efcbcbf9ff74a37ee169a8e4d887bcdaf4cf56fe`, source `fe472c6`, compiler matrix `nightly-2026-10-08 jr-b7907f1fe8f7`. Protected client and runner were downloaded again and compared before launch. IAM, egress-only network, schedule and hard three-hour lifetime were preserved. Its final runtime publication and content-verified fresh client reuse remain pending, not inferred from launch success.

Additional native-key and exact-client packaging records are in the same evidence namespace: `20261013-200848037-858313`, `20261013-200849521-858455`, `20261013-200854149-858992`, `20261013-201333279-898502`, `20261013-201334316-898680`, `20261013-201528836-909630`, `20261013-201553166-911946`, `20261013-201624279-918678`. The exact pinned fleet snapshot passed 381 tests with three ignored before the separate sync test raised the combined suite to 382.

## Actual helper-runtime closure and immutable repair

Fresh Ubuntu CDN installation at 03:31 UTC loaded both core compilers and the static Cargo fork, but the short JR bundle's rustfmt exited 127 because it required the official nightly `librustc_driver-f344ad00d19f4329.so`. Successful compiler version probes had not established helper closure. The old immutable ID `jr-b7907f1fe8f7` and bytes were not replaced.

A separate trusted prefix added the matching driver and retained the accepted static Cargo fork `125bdc8c1`, not the experimental scheduler Cargo. It was published as `jr-b7907f1fe8f7b788dbace9ca649ce4729408f943`, archive SHA-256 `3d71cb715a2148747c46877cba3aa0ef9cc879777d4fa64e444823d0b9193659`, 529,371,954 compressed bytes and 1,603,730,793 regular payload bytes. Publisher and client now run bounded helper probes without ambient library-path overrides. Real missing ELF loader/library fixtures exit 127 and are rejected. Explicit installation reuse checks known complete component markers without modifying or deleting an existing prefix.

The exact source `ec42e0f2e92eb429ac5a020419f03e9a1288a15c` was built from a separate Git archive and installed at 03:41:56 UTC. Its stable accepted binary is version `0.1.0-alpha.16`, SHA-256 `2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53`. The actual host installer printed that digest at 03:42:24 UTC on glibc 2.39. Fresh corrected JR install took 13.9 s and nightly 7.2 s, with no official fallback. Rustc, rustdoc, Cargo, static JR Cargo, rustfmt and clippy loaded; the JR marker stayed inactive, unpinned status stayed system, and repeat install preserved marker inode. Explicit reuse of the old short ID failed with the expected helper error and left its marker digest unchanged. These are functional observations, not speed comparisons or a fresh-tenant bootstrap proof.

Corrected hosted unit/lint records are `20261013-204019831-1054567`, `20261013-204022802-1054767` (22 passed), and `20261013-204026632-1054565`. The earlier missing `cargo-fmt` synthetic fixture failure `20261013-203904628-1048007` remains exported (21 passed, one failed, exit 101), alongside initial check `20261013-203902705-1047681`.

The final truly plain hosted check/full tests/strict all-target lint/recheck passed: `20261013-204221552-1065216`, `20261013-204245695-1067158` (390 passed, three ignored), `20261013-204256351-1067708`, `20261013-204301091-1067934`. Concurrent agents subsequently replaced the mutable global CLI with other digests; these run records contain no exact client digest field. Retain their functional hosted results, but do not attribute the entire sequence to one frozen executable or claim the current global CLI is our accepted digest. The direct installer digest and separate immutable binary remain verified; subsequent benchmarks use that explicit frozen artifact.

All 37 selected records in `data/runs/sharing-20261014/2026-10.jsonl` retain failures and are path-redacted. No unrelated peer rows are included.

## Rollout and remaining boundaries

New-host toolchain sharing is a fixed-origin bootstrap opt-in, not arbitrary tenant-selected root extraction. Existing hosts are not silently rewritten. JR remains inactive until explicitly pinned. Registry archive seeding remains off by default because the observed full transfer cost lost to direct Cargo fetching.

Pinned native rootfs publication, verified installer/mount receipts, complete environment and kernel/CPU determinants, audited sandbox execution and native ABI/runtime acceptance are not completed. The prototype says `sharing_authorized=false`. Do not describe the script guard as making all compiled native dependencies unshareable or all remaining native cache keys sound.

No performance speedup is claimed by this rollout document. No matched `bench/suite.py` before/after result was produced for these changes, and noisy concurrent functional test timings are not substitutes.
