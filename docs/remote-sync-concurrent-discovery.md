# Concurrent fresh source discovery

The source scan issues two independent read-only Git queries on each scan: the tracked/nonignored listing and discovery of ignored lockfiles. They now overlap using one scoped helper thread. There is still a fresh primary listing, fresh ignored-directory traversal, and fresh metadata observation on every scan. No watcher signal, prior manifest, Git index timestamp, or prior error is trusted as a freshness proof.

## Safety and ordering

The primary query flags, stderr suppression, lossy Git path decoding, and fallback filesystem walk are unchanged. Ignored lockfiles merge only after the primary listing is complete, in the same original vector order. Filtering, oversized order, symlink exclusion, missing-file handling, stamp extraction, and duplicate observations are unchanged. Neither Git query writes index/configuration state.

The prior sequential pair was not an atomic worktree snapshot: files and ignore rules could change between either query and metadata observation. Overlapping independent read-only queries does not claim snapshot isolation. A stable tree must yield identical primary bytes and identical ignored-lockfile vectors. Metadata remains freshly observed after both queries complete.

Thread creation is fallible. Resource exhaustion falls back to synchronous ignored-lockfile discovery, not a missing-lockfile result. A panicked worker is joined and retried synchronously as well. A dedicated regression injects both failure modes, then removes the ignored lockfile and verifies a fresh fallback no longer includes it. Existing Git, non-Git, ignored-lockfile, deletion, size, mode, symlink, and map oracle tests gate the production integration.

## Focused experiment

`justrust test --local --lib remote_sync::tests::concurrent_discovery_diagnostic -- --ignored --nocapture` performs seven paired serial/concurrent discoveries. Each pair asserts exact raw NUL-delimited primary Git output and exact ignored-lockfile vector equality. Set `JUSTRUST_SCAN_BENCH_ROOT` for another source tree. Local execution inspects actual local inputs instead of the hosted mirror.

Before production integration, actual vendored tree discovery medians were 624.26 ms serial and 431.57 ms concurrent (`20261029-212602272-2872756`). Jcode medians were 17.10 ms serial and 11.59 ms concurrent (`20261029-212624106-2876465`). Both passed all seven equality comparisons. These are focused unoptimized-test diagnostics, not an overall Rust build speed claim. No additional dependency or persistent cache was introduced.

Final production-helper measurements also passed seven exact pairs each: vendored tree 720.37 ms serial versus 568.18 ms parallel (`20261029-212754243-2896909`), Jcode 27.63 ms versus 19.97 ms (`20261029-212825733-2904953`). CPU contention was substantial, so do not compare these absolute medians to the earlier batch as if conditions were fixed. Whole fresh scans of 68,227 files and four oversized files took 832.64, 639.17, and 627.09 ms (`20261029-212827676-2906128`). These whole-scan observations are not a controlled overall before/after ratio.

Hosted maintenance was explicitly announced during final integration validation. Final local checks are a scoped scheduled-maintenance exception, not a change to hosted-default policy. Check (`20261029-212747947-2895882`), 17 passing tests with five diagnostics ignored (`20261029-212754478-2896996`), and all-target Clippy with `-D warnings` (`20261029-212902917-2922505`) passed. Final real hosted workflow verification belongs to the coordinator after maintenance ends.
