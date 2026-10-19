# Hosted scratch artifact audit and narrow cleanup

Date: 2026-10-19, 04:27-04:30 UTC. Stable client noninteractive
`justrust remote ssh` and `justrust remote status` were used. No compiler tests,
full oracle suite, benchmark or main build was started during this work.

## Read-only inventory and authorization boundary

The hosted machine reported a full disk. The scratch inventory contained these
Cargo-tagged real target directories (human-readable du sizes, not additive
physical-storage guarantees):

| Hosted worktree target | Approximate allocation | Ownership disposition |
| --- | ---: | --- |
| `jr-graph/target` | 12 GiB | This lane owns it; completed builds, paused before cleanup |
| `cr-strfmt/target` | 12 GiB | Retired owner absent from current swarm; not cleared for removal |
| `cr-forloops/target` | 12 GiB | Retired owner not independently confirmed; not cleared |
| `cr-dyn/target` | 11 GiB | Retired owner not independently confirmed; not cleared |
| `cr-prelude/target` | 17 GiB | Owner activity not independently confirmed; not cleared |

All paths in the table are beneath the hosted `/home/jeremy/.jcode/scratch`.
Cargo cache tags alone are not proof of inactivity or permission to delete.
The audit observed an unrelated hosted test process, which was left untouched.
Other worker targets, main targets and all private caches were excluded.

The coordinator explicitly authorized only the following absolute hosted path:

`/home/jeremy/.jcode/scratch/jr-graph/target/debug/incremental`

The broader target, its generated `debug/build` artifacts (about 9 GiB), and
all other lanes were outside that authorization. No local target was touched.

Before cleanup this lane paused all its builds/sync and notified dragon, who
confirmed no target-dir sharing or running build. Stable remote status reported
an idle sync daemon, zero roots mirrored and zero files pushed. Fresh hosted
process and fd/cwd/exe scans found no references to the candidate target. The
Cargo target tag, realpath equality, directory and non-symlink checks passed.
Local git tracked no target files, and durable receipts and recovery backups
were outside target. These are point-in-time checks, not a claim that future
builds could never reference the path.

## Exact cleanup result

At 04:29:39 UTC, a guarded command removed only the authorized incremental
directory. It rejected a different full path, a symlink, a missing Cargo tag or
any observed open reference before removal. The command checked the directory
was absent afterward and that the hosted source and `debug/build` remained.

| Observation | Before | After |
| --- | ---: | ---: |
| Filesystem block size | 4,096 bytes | 4,096 bytes |
| Total blocks | 126,692,793 | 126,692,793 |
| Free blocks | 4,096 | 630,230 |
| Available blocks | 0 | 626,134 |
| Available bytes | 0 | 2,564,644,864 |

The candidate's pre-removal `du -sx -B1` allocation was 2,585,686,016 bytes.
The observed available-space increase was about 2.39 GiB. These distinct
measurements are retained rather than equated. `df` reported about 2.4 GiB
available afterward, while its integer percentage still rounded to 100%.
The before/after command also captured df and statfs in its saved output.

No sources, stashes, checkpoints, receipts, private caches, other target trees
or local build outputs were removed. All nine original stashes remain intact.
No source change, install, push or main edit/build occurred. This disk recovery
is not validation of the hosted build path or a compilation speed claim.
Further cleanup requires separate authorization and fresh activity checks.
