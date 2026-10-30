# Local daemon lookup attribution

The daemon's previous `waiting for a compile-only prebuild` duration started
before `source_roots` and `source_stamp`. It therefore included recursive
source validation, even when no prebuild was running. Historical logs showing
870–1,640 ms under that label cannot establish a prebuild-wait bottleneck.

Commit `bbf63226e` separates lookup time into source-validation and prebuild
coordination intervals. The latter includes stale-prebuild cancellation and
coordination, not just sleeping. Lookup also includes request identity and
build selection/start. No source filtering, stamp semantics, cache identity,
prebuild policy, or result reuse behavior changed.

Hosted correctness checks passed:

- Check: `20261029-220523313-3250217`.
- Eight daemon tests: `20261029-220530275-3251742`.
- Strict all-target Clippy: `20261029-220535109-3249540`.

The controlled local `JUSTRUST_BUILDD=0` experiment is a diagnostic only.
It still invokes native Cargo input validation and does not authorize cached
command-result replay. One attempted root comparison rebuilt `src/record.rs`
under severe concurrent compiler contention and is excluded from performance
claims. Installation and new per-phase observations follow after the client
recording implementation is frozen.
