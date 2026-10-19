# Source edit recovery and hosted capacity, 2026-10-19

## Shared source recovery

At 16:54 UTC the coordinator used the unified-diff fallback of Jcode's patch
tool to materialize the already committed mutable-slice change. The fallback
spliced at recorded coordinates without validating the old context. With
concurrent working changes, this damaged portions of compiler source. The
committed graph, private indices and workers' isolated acceptance trees were
not damaged.

Shared edits were held while the coordinator reconstructed overwritten spans
from exact left/right anchors and preserved worker snapshots. An initial
whole-file transfer also exposed command-output truncation. Complete source
was subsequently transferred in bounded JSON-decoded chunks and checked
against full-file SHA-256 hashes. The complete pre-write backups and recovery
proofs remain in the coordinator's scratch recovery directory.

The final reconstruction retained concurrent macro source-binding hooks,
loan-renewal hooks, operation-addressed runtime location tables, capture
metadata exports, C-ABI visibility/export fields and byte-literal changes.
Mutable-slice integration was then applied only where the full old context
matched uniquely. The recovered frontend, borrow, backend, CFG, IR and
ownership modules parse independently with rustfmt. This is syntax and
preservation evidence, not a claim that all shared in-progress feature work
passes a complete build.

Jcode fix `9d22cbb8e` on its own primary branch adds exact-context validation,
all-file preflight, before-write rechecks and rejection of stale-coordinate
patches. Its actual package tests passed: 39 patch/apply-patch tests and two
edit-stat compatibility tests. The already running daemon was not restarted.
Until that fix is rolled into the runtime, use exact replacement or the
context-validated Codex patch format, not the unsafe unified fallback.
Never transfer a large file from an unchecked command preview.

A second use of the still-unpatched unified fallback damaged 21 bounded spans
in five shared files at about 17:29 UTC. The owner preserved damaged bytes
and the complete input, then reversed only those exact splices. Nineteen
removed spans agreed between committed and indexed beforeimages; the other
two parser spans were independently confirmed from worker snapshots and
committed lifetime-parser sources. No whole file was replaced. An inverse
splice audit reconstructed all five files exactly. At 17:36:07 UTC the
coordinator independently matched their full SHA-256 hashes and parsed each
with rustfmt without writing it. Shared edits resumed after verification.
This is preservation and syntax evidence, not full compiler acceptance.

## Recurrent hosted ENOSPC

At 17:13 UTC the hosted root filesystem had no available bytes. Read-only
accounting found many independently mirrored test targets. No active target,
source, private cache, run record or toolchain was removed for this recovery.
The coordinator used the installed agent's native shared artifact LRU:

```sh
env JUSTRUST_DEPCACHE_MAX=8G /home/ubuntu/.justrust/bin/agent/7d88a669bafab7bda02c6f001602ddee/justrust cache --prune
```

The cap applied only to that command. Native default 600-second grace,
recent-hit rechecking and atomic-rename protections were unchanged. The tool
reported eviction of 2,798 recomputable dependency-artifact entries totaling
7.14 GB, leaving 7.19 GB. An independent statvfs measurement at 17:15:47 UTC
reported 7,694,229,504 available bytes out of 518,933,680,128. Concurrent host
activity means the available-space difference is not an exact eviction total.

Only bounded tests using existing artifacts were readmitted. Broad all-target
link campaigns and additional large mirrors remain inappropriate at this
capacity. Completed acceptance must remain separate from ENOSPC failures.

At 17:20:55 UTC the owner explicitly authorized removal of the inactive hosted
`duckling-placeholder-stage/verify/target`, with no future jobs until handback.
The coordinator first performed a dry-run, then acquired the same exclusive
per-root FNV lock used by native eviction, checked all relevant process
cwd/exe/fd/maps references, and verified canonical same-filesystem generated
Cargo target markers. Both scans found no live references. Only that hosted
target was removed at 17:21:41 UTC. The source manifest was byte-checked and the
source tree preserved. Private indices, patches and the separate local target
were untouched. Available bytes changed from 7,692,972,032 to 26,286,813,184.
The owner was notified before bounded hosted work was readmitted.
An independent read-only host measurement at 17:43:54 UTC still found
26,277,695,488 available bytes. Build-error reports must be checked against
their actual event time and destination, not assumed to prove current ENOSPC.

## Compatibility status

Noncapturing stored closures, mutable slice parameters, root slice returns,
nominal lifetime rows, rich dyn identities, raw/byte static literals and raw
slice constructors have separately committed feature-specific evidence.
Capturing closures, universal Iterator projections, opaque returns, registry
feature integration and native C ABI remain active work. An unchanged cached
`utf8-width 0.1.8` dependency has native check/build/run evidence, not a general
registry compatibility claim. The last complete unchanged committed corpus
measurement remains 5/20. A subsequent fixture-14 hang was independently
traced to nominal interning re-entering the same PlanState mutex. Its focused
repair is separately validated, not evidence of complete language support.
No full-Rust, global-test-pass or speed claim follows from these milestones
or from source recovery.
