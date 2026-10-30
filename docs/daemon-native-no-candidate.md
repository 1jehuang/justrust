# Native execution with no coordination candidate

Date: 2026-10-30. Experimental opt-in:
`JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE=1`. Only the exact value `1` enables it.
Default is off. Original `JUSTRUST_BUILDD_NATIVE_EMPTY=1` is unchanged and
continues to require globally empty active and finished maps.

## Contract and gate

This is not a disjoint-root exemption. It makes no claim about dependency,
filesystem, target-directory or build-script isolation. Cargo always executes
and decides freshness normally. Different request keys may share source roots
or dependencies, just as ordinary concurrent Cargo invocations may. Same-root
or overlapping-root requests are not categorically refused by this flag.

While holding the daemon state lock, a real request may reserve an unstamped
native owner only if all of these are true:

1. The flag has value `1`, and the request is not a prebuild.
2. No active entry has this request key.
3. No finished entry has this request key, even if stale or compile-only.
4. No other-key active entry is a wrong-guess cancellation candidate for this
   session: it has no exit status and is either an unclaimed prebuild or has
   already served its early answer.

The fourth predicate is shared with `cancel_wrong_guess`, not duplicated.
An absent session uses the existing empty-string identity, so two absent
sessions are not presumed independent. Completed other-key builds, other-key
finished results, claimed non-early prebuilds, ordinary active client builds,
and active builds belonging to another session do not themselves block this
operational gate. Their paths are irrelevant to the decision.

Eligibility and native insertion occur in the same locked region as the old
empty-map path, normal insertion, and watcher insertion. Requests scanning
outside that lock must still refuse an unstamped same-key owner on recheck.
The child has `stamp=None`, is never an attach/replay candidate and cannot be
published to the finished replay map. Client disconnect, early-answer behavior,
watch registration, recent-session pruning and child exit handling are unchanged.

## Skipped-operation audit

| Normal operation skipped | Reason it has no candidate at reservation |
| --- | --- |
| `source_stamp` | No source verdict is used or invented. Cargo performs freshness. |
| `cancel_stale_prebuild` | Same-key active entry is absent. |
| `cancel_wrong_guess` | Shared candidate predicate rejects eligibility if any candidate exists. |
| `settle_compile_only` | Same-key active and finished entries are absent. |
| active matching/attach | Same-key active entry is absent. |
| finished removal/replay | Same-key finished entry is absent. |

The gate observes current candidates, not future ones. Like ordinary
coordination, it does not prohibit later other-key requests, watcher activity
or early-answer transitions. The daemon state lock prevents map reservation
races, not concurrent Cargo filesystem effects. No stronger isolation is claimed.
If any candidate is present, the existing scanned path handles cancellation,
settlement, attachment or replay according to its existing rules.

## Validation scope

New tests cover exact-value opt-in and prebuild refusal, all same-key active
and finished combinations of stamped/unstamped, exited/running and compile-only
state, and the full other-key session/prebuild/claimed/early-served/exited matrix.
Missing sessions are included. The existing real socket-pair reservation refusal
test now runs in noflag, original flag and new flag modes for real and prebuild
requests. Existing unstamped reaping/no-replay and root-authority counterexample
tests remain in place.

Hosted gates passed on the frozen source:

- `justrust check`: `20261029-231418822-3852605`.
- `justrust test -- buildd::tests`: `20261029-231422945-3854168`,
  17 passed, 0 failed, 2 ignored.
- `justrust clippy`: `20261029-231432266-3851394`, warning-free.

`cargo fmt` ran before these gates. These are correctness results, not timing
comparisons. Coordinator owns
real-Cargo socket acceptance, installation, export and any paired diagnostic.
No performance/default promotion is claimed by the implementation.
