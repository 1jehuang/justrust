# Native flags safety and final sharing acceptance, 2026-10-14

## Shipped identity and scope

Guard commit `832e362` and actual shim integration `059674b` are included in the exact installed source snapshot `059674bf76fa218ba7e475279a86b4b6caf32aac`. Accepted alpha16 executable SHA256:

`0a19acdd5be8fd2963cda9bb551db508fe9a7875db69575c09a8fe5bb294b56d`.

This document and adjacent JSON map the final acceptance, not a speedup. All 26 cited own runs, including two failures observed before fixing eligibility, are committed in `data/runs/sharing-native-flags-20261014/2026-10.jsonl`. Historical cache measurements remain unchanged.

## Safety requirement to observed result

| Requirement | Actual check | Result and boundary |
|---|---|---|
| Host-dependent native CPU flags must not have portable cache eligibility | Native plan regression, all eight supported split/joined/equals forms of -C/--codegen and target-cpu/target_cpu | Before: 23 cacheable dep/local cases, exit101 in 20261013-215342851-1568478. After: shared shape parser bypasses these forms conservatively, even with later explicit overrides. |
| Response-file contents cannot be silently omitted from cache determinants | Real rustc accepted native @file while old plans remained cacheable, red20261013-215708557-1600890 | Any top-level @ argument bypasses both caches without opening, rewriting or expanding the file. Includes opaque missing files and @shell forms. |
| Actual compiler remains unchanged and ordinary cache reuse works | Hosted test20261013-220413535-1642867, two native-cache integration tests | 40 actual shim invocations:36 native/@ bypasses and4 generic controls. 38 actual compiler runs,2 cache restorations. Dependency and local generic controls each miss then hit; all40 resulting executable output assertions are42. No assertion of40 physical compiler calls. |
| Whole installed client passes public workflows | Exact frozen receipt with69 committed source blobs verified; output SHA checks and actual hosted/compiler/argument metadata | Check220816567-1669580; fulltest220822907-1670409:410passed,3ignored; strict all-target lint220843215-1671710 clean; recheck220848764-1671939:0compiled62fresh. All IDs have20261013- prefix. |
| Real end-user edit, repeat and restored source still work | Desktop FPS filter:220940404-1675420,220951644-1676602,220955119-1676852 | Three tests pass each; repeat0compiled746fresh; backup-copy restoration verified with original SHA24de30f7556c2e126bb52f685377f656c6352dfdf83e1bc9f9d03f037d877e8d. Project warnings retained, not a full Desktop UI clean-lint claim. |
| Safe client still consumes public dependency artifacts | Four independently empty Cargo/target/depcache fixtures, two compilers × default/debug0 × check/test | All eight runs below report1 public prebuilt hit,0 registry misses; each test run passes2 runtime tests. Local public CDN workflows, not account-host cache hits. |
| Installed helper runtime closure works on actual host ABI | Authenticated Ubuntu observation05:13:05UTC | Accepted host executable matches0a19 SHA; glibc2.39. ELF requirements libgcc_s.so.1/libc.so.6/loader, maxGLIBC2.39. Trusted Ubuntu24.04 fleet artifact, not universally portable Linux. |

Worker isolated final runs: check20261013-220212601-1632702,23 depcache tests220213700-1632916,2 localcache tests220218197-1633818, strictlint220218365-1633875, build220220162-1634105. Joint source whole-test410/3ignored220417764-1643114 and strictlint220431970-1644192 passed before exact-snapshot installation.

First red isolation caveat: automatic telemetry/update and the task-private build daemon were not initially disabled; task-private update/latest and telemetry markers appeared. Do not claim zero network for that first run. Subsequent worker checks disabled those automatic features and stopped only the task-private daemon. Public/cache/account mutation was not intentionally performed by the worker.

## Fresh public reuse after installing the safety guard

| Compiler/profile | Check ID | Test ID | Result |
|---|---|---|---|
| nightly-2026-10-08/default |20261013-221102336-1688570|20261013-221103246-1688719|1public hit each,0misses,2tests|
| nightly-2026-10-08/debug0 |20261013-221103828-1688906|20261013-221104518-1689051|1public hit each,0misses,2tests|
| full JR/default |20261013-221105094-1689198|20261013-221105897-1689289|1public hit each,0misses,2tests|
| full JR/debug0 |20261013-221106484-1689478|20261013-221107219-1689580|1public hit each,0misses,2tests|

Full JR means `jr-b7907f1fe8f7b788dbace9ca649ce4729408f943`, inactive staged prefix. Same matched public memchr2.8.3 fixtures described in `sharing-public-matrix-20261014.md`. Account/peer/shadow reuse disabled; public always-fetch diagnostic forced for acceptance, not a default latency policy or benchmark. No fabricated key-explanation hashes are provided.

## Remaining boundaries

Native build-script public replay remains disabled until a trusted mounted native closure and kernel/CPU/runtime determinants are established. The rootfs prototype still says sharing_authorized=false. No cross-CPU stale executable was demonstrated here: the red tests demonstrate eligibility errors, while the native archive collision has separate actual miss/miss/hit evidence. No old cache objects were deleted or rewritten, no cache schema bump in this flag guard, and no experimental Cargo/default JR promotion.

Registry seeding remains opt-in: measured complete-network median1.697s versus direct1.057s did not justify an eager default. Actual first-new-tenant bootstrap is still unobserved; we did not delete an existing tenant host or fabricate accounts to force it. Existing-host fresh-prefix package/runtime acceptance is not misrepresented as that missing observation.

Future trusted fleet pin and sanitized status configuration are now deployed as launch-template version3, verified UPDATE_COMPLETE05:19:45UTC and scheduler $Latest05:24:22UTC. No new producer was manually launched and no future runtime receipt is claimed. Cloud deployment receipt documents unchanged security and lifetime invariants.
