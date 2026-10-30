# Watch-directory topology reuse

The October 30 follow-up found that cached commands reconstruct the directory
watch set from all 68k manifest entries on every flush, even when no source
paths changed. Daemon logs attributed 0.14-0.39 s to the whole watch phase on
this local checkout. This is distinct from source scanning and remote admission.

`Root` now retains the derived directory set. Added or deleted manifest paths
invalidate it. Stamp-only edits and unchanged scans reuse it. Every flush still
calls `watch_all` over those paths, so failed registrations and forgotten inode
watches can be retried. The source scan, diff, transfer and authoritative remote
barrier remain unchanged. Watcher silence is never used to infer freshness.

Regression tests compare the cached set with a full reconstruction across
unchanged files, stamp changes, nested additions, removals, an empty tree and
reintroduction. A fresh watcher successfully registers the cached paths again.
The latter test is limited to platforms with actual directory watchers.

Focused paired diagnostic `20261029-210919207-2652800` used the actual hosted
manifest (67,139 files, 6,433 directories). Raw five-sample reconstruction/reuse
timings are in `bench/results/watch-directory-reuse-20261030.json`. These isolate
derived-set construction, exclude scanning, watch registration, admission and
network cost, and are not an end-to-end speedup claim. The comprehensive
snapshot gate is unavailable while another benchmark holds its lock.

Hosted full suite: 834 passed, eight ignored, run
`20261029-210931833-2656734`. All-target Clippy with warnings denied passed:
`20261029-210951534-2652243`. These runs include the concurrent manifest-diff
regression test. No compiler/native Cargo experimental default was changed.
