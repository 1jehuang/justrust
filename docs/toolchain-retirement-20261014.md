# Legacy source checkout retirement, October 14, 2026

## Safety audit

The owned development trees are `toolchain/rustc` and `toolchain/cargo`.
Legacy `~/jr-rustc` and `~/jr-cargo` checkouts occupied approximately 44 GB
and 45 GB respectively. Neither had tracked working-tree changes. Rustc had
an untracked `.fetch.log` and local branch commits not present in its remote
tracking refs. These are not disposable simply because the current source
was imported.

Installed toolchain prefixes had no symlinks into either old checkout. Active
builds use the owned trees. Historical benchmark records containing old paths
are retained as provenance, not rewritten into claims about new executions.

## Preservation

`~/.justrust/retired-sources/2026-10-14/jr-cargo.bundle` was created with
`git bundle create --all` and verified successfully, preserving all 224 refs
and complete history. The unique rustc fetch log was copied alongside it.

The corresponding rustc bundle attempt failed because historical object
`0c5c5fe46d3945243e98bf0ecfb6010bbdf3cb9b` was unavailable locally.
A separate `git cat-file -t` confirmed that absence. The combined shell
command exited zero because subsequent Cargo preservation succeeded, so
that exit code is not evidence that rustc was archived successfully.

The rustc repository and its local branches are retained intact. On October
14 both old directories were moved, with all Git metadata and ignored artifacts,
to `~/.justrust/retired-sources/2026-10-14/jr-rustc` and the adjacent `jr-cargo`.
Post-move Git status matched the audit (only rustc's `.fetch.log` untracked).
`justrust toolchain status` still selected `jr-ac20de5e7921` with its existing
`jr-cargo`; installed fallback prefixes were unchanged. The move is reversible
by moving each directory back to its original home path, provided that path is
unoccupied. No files or build artifacts were deleted, and no disk-space
reclamation is claimed. The retirement does not depend on candidate activation,
since installed toolchains contain independent artifacts, not legacy symlinks.

## Boundaries

Unrelated dirty client and Cargo execution-graph work is not part of this
cleanup and must not be staged, discarded or promoted incidentally.
Toolchain build/activation acceptance is recorded separately in
`docs/toolchain-activation-20261014.md` when that work completes.
