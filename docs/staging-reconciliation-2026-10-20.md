# Staging reconciliation and preserved local work, 2026-10-20

## Cause and scope

Validated feature commits had advanced `main` through independently scoped
private indices while the shared checkout's original index and working files
remained older. As a result, `+566` in Jcode represented staged file differences,
not unpushed commits. At the start of reconciliation, local and remote `main`
were both `56c275ab56838236289b493fc74d1cab0e988118`, with zero commits ahead or
behind. Committing the original shared index wholesale would have reverted
published features: its diff contained 3,807 insertions and 69,179 deletions.

This is a checkout/index consistency incident, not evidence that the published
feature commits were absent from the remote.

## Preservation and reconciliation

Before any index change, the original index, staged binary patch, status,
HEAD and SHA-256 fingerprints for 3,467 paths were saved. Clearing staging
changed no working-file bytes. The resulting tracked differences were 251
missing files and 190 modified files. The 178 original untracked entries fell
to 18 when paths already published in HEAD were recognized as tracked.

A further complete working-tree binary patch and a verified archive of all
changed and untracked file contents were saved before restoring anything.
Reconciliation then restored only:

- 251 missing paths already present in published HEAD;
- 100 modified paths whose local contents exactly matched historical committed
  versions of those same paths.

All 90 remaining modified files and all 18 remaining untracked files were
preserved byte-for-byte. Their hashes were verified after restoration. The old
index and all restored states remain recoverable from the backups. These
counts describe this incident, not an invariant for future repository status.

Recovery copies exist both under
`~/.jcode/scratch/staging-reconcile-20261020-0717` and persistently outside
scratch under `~/.local/state/justrust-recovery/20261020-0717`. The persistent
copy includes the index, staged and working-tree binary patches, verified file
archive, original hashes, classifications and recovery ledger. It is local
recovery material, not an instruction to publish unvalidated source snapshots.

## Remaining work is mixed, not automatically ready

The 90 modified files combine potentially useful work with stale reversions.
Preserved candidates include source NonNull APIs, iterator/range modules,
closure reference-place/result-origin work and tests. For example, the local
Hash file adds text hashing but also reverts the shipped generic Hash/Hasher
protocol. It must not be committed wholesale.

Historical per-file comparisons and scratch-only trial merges were prepared.
Their results are review aids, not authority to delete later compiler checks or
proof that an apparently clean merge is correct. No trial merge was applied
automatically to the remaining working files.

The first isolated recovery keeps only the original 65-line NonNull addition
on published `56c275ab`, retains the original test programs, and replaces the
ad hoc harness with stock/native differential checks. Candidate tree
`1a1ff8e908774852dff03407a040e71875af62b8` contains no compiler changes.
Matching pinned-toolchain `check --tests` passed, run
`20261020-003116328-2972341`. The original pointer-behavior program then failed
native CHECK at workers 1 with unresolved nominal source constraints, run
`20261020-003213469-2977471`; native BUILD/run and workers 8 were not reached.
This is recovered draft work, not an accepted NonNull support claim. Its patch
and both actual logs are preserved in the persistent recovery directory.

## Follow-through rules

1. Preserve original content before reconciling old source with newer commits.
2. Recover one coherent feature against current HEAD, not an old whole compiler.
3. Never drop source, graph, lifetime or call authority to make a draft pass.
4. Record failed acceptance honestly and retain its original fixture.
5. Commit and push only reviewed, gated slices. Keep unrelated working changes
   out of each commit.
6. When publishing with a private index, explicitly account for the shared
   index/worktree afterward. A successful push alone does not synchronize them.

No full-Rust compatibility, passing corpus, performance or completed recovery
of every remaining feature is claimed by this reconciliation.
