# Native-empty unrelated-root exclusion investigation

Date: 2026-10-30. Scope: default-off `JUSTRUST_BUILDD_NATIVE_EMPTY=1`.

## Decision

Do not replace the global active/finished-map emptiness gate with disjoint
`source_roots` checks. No behavior or default change, no replay authorization,
and no performance claim. Native Cargo still executes its full freshness path.
Unstamped owners remain reserved and cannot enter the finished replay map.

## Concrete counterexample

`disjoint_watch_roots_are_not_dependency_isolation` in `src/buildd.rs` creates:

```text
fixture/
  a/Cargo.toml             [workspace], members=["member"]
  a/member/Cargo.toml      shared={path="../../shared"}
  b/Cargo.toml             [workspace], members=["member"]
  b/member/Cargo.toml      shared={path="../../shared"}
  shared/Cargo.toml
  shared/src/lib.rs
```

The member manifests refer to the same canonical dependency directory, but
`source_roots(a)` is only `[a]` and `source_roots(b)` is only `[b]`.
The scanner follows path expressions in the root manifest and discovered path
manifests, not workspace member manifests. Thus a root-prefix exclusion test
would assert isolation where none was established. Lexical normalization also
is not symlink identity. Shared target directories, Cargo configuration and
arbitrary build-script inputs/outputs add other reasons not to treat these
roots as a complete conflict set. This fixture does not claim that ordinary
concurrent Cargo itself is incorrect, only that the proposed isolation proof
is false.

There is a separate concrete lifecycle counterexample even for actually
unrelated source trees: a session has an unclaimed speculative prebuild in A,
then requests B. The normal path calls `cancel_wrong_guess` and signals A's
process group, independent of root paths. The native branch bypasses that call.
A disjoint-root exemption would leave A running and competing for the session's
slot, changing the existing wrong-guess cancellation behavior. Early-served
builds are subject to the same rule.

## Invariants reviewed

- `run` reserves a native owner while holding the daemon state lock. Normal
  requests recheck unstamped same-key occupancy after scanning.
- Stale same-key prebuild cancellation can wait up to ten seconds. Compile-only
  same-key prebuild settlement waits for completion before native tests run.
- Wrong guesses are selected by session, not workspace. Missing sessions use
  the same empty-string identity and cannot be assumed independent.
- `remember_request` prunes to the latest request per session. Watchers select
  dirty remembered roots, scan outside the state lock, then recheck same-key
  ownership under it. Root heuristics are not dependency receipts.
- Client disconnect cancels an orphaned non-prebuild unless early output has
  already been served. Completed unstamped executions never become replayable.
- Finished entries do not retain an authoritative dependency/conflict closure.
  Recent entries can be absent, pruned or changed, so joining them by request
  key cannot reconstruct missing authority for active/finished owners.

## Narrow next design

Separate lifecycle coordination from the optional source-stamp scan. Before
any native reservation, retain same-key stale/compile-only handling and
same-session wrong-guess cancellation with a final locked reservation recheck.
A genuinely stamp-free same-key policy must refuse those owners rather than
invent a source verdict. Keep native owners unstamped and never attach/replay.

For any subsequent cross-project gate, explicitly define its contract first:
concurrent native Cargo may be acceptable without claiming dependency isolation,
but speculative ownership, cancellation and finished-output behavior still need
an explicit state machine. If isolation is required, obtain authoritative Cargo
resolution/input ownership information, account for unknown build-script effects
by refusing the optimization, and invalidate that authority on configuration and
graph changes. Merely expanding the root regex or canonicalizing paths is not
sufficient. Test same-root/overlapping-root refusal, cross-root same-session
cancellation, missing sessions, shared external member dependencies, shared target
directories and watcher scans racing reservations before performance sampling.

## Validation

Hosted gates on pinned nightly-2026-10-08 all passed:

- `justrust check`: run `20261029-230836835-3796659`.
- `justrust test -- buildd::tests`: 14 passed, 0 failed, 2 ignored,
  run `20261029-230840142-3796988`. Includes the root counterexample,
  native owner reservation race and unstamped no-replay coverage.
- `justrust clippy`: warning-free, run `20261029-230848060-3796566`.
- `cargo fmt`: applied before gates.

These are correctness gates, not paired performance measurements. The coordinator
owns export of the cited run summaries.

No install, export or performance sampling was performed in this lane. Those
remain coordinator-owned and require a coordinated source freeze.
