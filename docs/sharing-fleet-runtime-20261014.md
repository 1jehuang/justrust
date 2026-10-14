# Sharing fleet: actual scheduled runtime acceptance, 2026-10-14

The next naturally scheduled trusted producer has now passed runtime acceptance. This closes the scheduled-fleet observation gate left pending in the deployment receipt. It does not turn earlier configuration inspection into historical runtime proof.

## Actual observations

Bounded read-only AWS/S3/CloudTrail observation joined the scheduled launch, immutable terminal receipt, producer log and version-pinned index. No producer was launched by this observer, and no account-host build, SSH, installation or production mutation was performed.

- The scheduled producer launched at **09:00:13 UTC on October 14, 2026**. Its terminal receipt reports **complete / succeeded at 10:14:49 UTC**, with **zero optional failures**.
- The producer verified accepted client source `059674bf76fa218ba7e475279a86b4b6caf32aac`, binary SHA `0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`.
- Actual command sections exercised `nightly-2026-10-08` and corrected immutable `jr-b7907f1fe8f7b788dbace9ca649ce4729408f943`, each with default and debug0 profiles. Both public-workspace compiler passes also ran.
- Publication reported **81 new bundle files**, 81 manifest lines, about 0.14 GB rounded, **zero public build-script runs** and 710 skips. The final index contained **19,994 sorted unique artifact keys**. Bundle publication count is not a claim of 81 net new unique index keys.
- The three-hour shutdown was armed. Console output showed normal poweroff, and AWS observed the instance terminated at 10:46 UTC, before its deadline. Its exact termination timestamp was not observed.

## Requirement-to-observation map

| Requirement | Actual acceptance observation |
|---|---|
| Natural scheduled producer, not a manual test launch | Existing scheduler and exact CloudTrail scheduler-role/instance join |
| Safe client and immutable compiler pins | Selected binary digest plus actual toolchain install/command log |
| Default/debug0 and workspace compatibility | All four compiler/profile sections plus two workspace compiler passes |
| Terminal outcome matches execution | Exact traced receipt ID, succeeded/complete, zero optional failures |
| Trusted public publication | 81 reported bundle files, zero public script runs, version-pinned index hash/count |
| Bounded lifetime | Armed three-hour deadline, actual poweroff and terminated state |
| Private/native boundaries preserved | Read-only observer; no IAM, billing, cache, default or native-authorization changes |

Independent read-only review checked these joins and limitations. Final archival audit verified all 12 private artifact sizes/hashes, lossless log/index decompression, all matrix/workspace sections, terminal pins and credential exclusion. Private cloud commit **`ca2e957231bf2430646396e8015af8c500a5eaee`** contains `docs/prebuild-sharing-runtime-20261014.{md,json}` and the exact archive manifest. Infrastructure identifiers and raw provider outputs are not copied into this public note. Raw CloudTrail session credentials were excluded even from that private archive.

## Remaining limitations

- No genuine first-new-tenant account-host bootstrap was created or observed.
- Public native build-script replay remains unauthorized until its complete trusted mounted native closure, kernel/CPU/environment determinants and ABI/runtime acceptance are established.
- Registry eager seeding remains off because the measured full-network path was slower for the evaluated workload.
- This observation did not independently download every one of the 81 newly published bundles. Prior fresh-cache sharing acceptance remains separately recorded in `sharing-reload-followthrough-20261014`.
- Protected runner object history and digest were verified, but the runner did not emit its own runtime execution digest.
- No compile-time speedup, complete parallelism comparison or hardware core ceiling is claimed. Those belong to separate experiments.

No further fleet-deadline follow-up is needed for this run because it is already terminal.
