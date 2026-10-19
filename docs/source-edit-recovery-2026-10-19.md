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

## Compatibility status

Noncapturing stored closures, mutable slice parameters, root slice returns
and declaration-owned nominal lifetime rows have separately committed,
feature-specific acceptance evidence. Capturing closures, universal Iterator
projection authority, raw/static literals, registry consumers and native C
ABI remain active work. The last complete unchanged committed corpus
measurement is still 5/20. No full-Rust, global-test-pass or speed claim follows
from these milestones or from the source recovery.
