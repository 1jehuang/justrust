# Offline Query elapsed inventory

`bench/provider_cost.py` is a finite diagnostic, not a provider cost model.
It reads a bounded measureme binary profile with the existing strict
`query_profile` decoder. No native changes, builds, network, or source replay
are needed by this tool.

```sh
python3 bench/provider_cost.py --profile CAPTURE.mm_profdata
python3 bench/provider_cost.py --profile CAPTURE.mm_profdata --large-capture \
  --allow-unmapped-event-labels
python3 -m unittest discover -s bench -p test_provider_cost.py -v
```

Default decoding is strict. `--large-capture` explicitly selects existing
larger decoder bounds, not unbounded input. The independent
`--allow-unmapped-event-labels` opt-in retains missing virtual event labels
as unresolved, without weakening headers, strings, grammar, timestamps, or
record decoding. Both malformed and unsupported inputs fail without echoing
capture strings or paths in decoder errors. Full function arguments are never
emitted. Only the decoder's static label matching ASCII
`[A-Za-z_][A-Za-z_0-9]*` is a named group. Unsafe, missing, or unresolved
Query names are grouped as JSON null, never guessed from arguments or IDs.
This is a syntactic static-label criterion, not a registry of supported queries.

## Output contract

- Raw capture SHA-256 identifies the exact bytes, not a normalized profile.
- Each `(query_kind, profiler thread)` has an interval record count and
  `interval_union_ns`, using `reuse_ceiling.interval_union_ns`.
- All-Query unions are independently computed per profiler thread, including
  unknown-name intervals. They are not sums of kind unions.
- Missing label counts cover all events. Unknown Query name counts separately
  cover Query intervals. `label_coverage_complete` describes resolution across
  all decoded records, not completeness of instrumentation.
- `capture_flags` stays `unknown`, `capture_complete` stays null, and
  `cost_for_same_results_ns` stays null. An external capture receipt can record
  configured flags, but the profile alone does not establish them.
- No totals are formed across kinds or threads. Nested and overlapping spans
  are elapsed observations, not CPU work, exclusive provider cost, saved time,
  avoidability, or a global ceiling. Native hash-result markers are not joined
  to provider intervals. Absence is not zero execution or zero cost.

## Frozen real capture replay, 2026-10-30 UTC

The coordinator supplied two successful checks on a real pinned private source
using the frozen compiler driver SHA-256
`8cfb723e9ffb482bdd9518251f281c5169aae7211804e9d12ff5bbb29f321569`.
The source was restored by the coordinator after the body edit. Receipts report
fixed flags `-Zjr-empty-lint-level-fast-path=no`,
`-Zjr-cache-small-query-values=no`, `-Zjr-cache-associated-items=no`,
`-Zself-profile-events=default,function-args`, and
`-Zincremental-verify-ich`. These are receipt provenance, not inferred flags.
The unchanged source SHA-256 is
`8272de3756d2323b05777c851b19c2cc6aec6bdf073af79056da29370daf6bde`.

Actual CLI replay used `--large-capture` both strictly and with explicit
`--allow-unmapped-event-labels` for each capture. Strict runs exited 2 for
unmapped virtual labels, preserving the failure. Opt-in runs exited 0.
All observed Query names resolved to safe static identifiers, but overall
label coverage is **incomplete** in both captures.

| Observation | Unchanged | Body edit |
| --- | ---: | ---: |
| Query interval records | 33,322 | 58,014 |
| Named static kinds | 59 | 181 |
| Unresolved labels, all event records | 73,091 | 11,633 |
| Unknown Query name intervals | 0 | 0 |
| All-Query independent union, thread 3 (ns) | 666,646,752 | 981,657,444 |
| `lower_to_hir`, thread 3: records | 8,936 | 8,936 |
| `lower_to_hir`, thread 3: union (ns) | 171,660,512 | 190,255,239 |
| `resolver_for_lowering_raw`, thread 3: records | 1 | 1 |
| `resolver_for_lowering_raw`, thread 3: union (ns) | 362,731,230 | 361,959,701 |
| `analysis`, thread 3: records | 1 | 1 |
| `analysis`, thread 3: union (ns) | 293,219,388 | 599,954,636 |
| `expn_that_defined`, thread 3: records | absent | 1,515 |
| `expn_that_defined`, thread 3: union (ns) | unknown, absent | 1,337,773 |

These rows overlap and nest. **Do not add them.** In particular the high-count
`expn_that_defined` edit observation occupies only 1,337,773 ns of observed
elapsed union on thread 3. That is a prioritization diagnostic, not evidence
of equivalent savings or a classification of same-result executions.
This is one pair of captures, not a benchmark speedup or default-promotion gate.

Capture SHA-256:

- Unchanged: `502a8398f972976cd4363fbf9fb530baf50d9d79204621d3c8911fe09ecb4b22`
- Body edit: `9f693e25b61d115869b5ee2f1b4ce4f9c8ffb952b578e8910bb7fbeb2a27bf71`

Private receipts, raw captures, strict failures, and successful output are
retained under the coordinator's scratch directory
`provider-cost-real-20261030`. Reports are named
`{unchanged,body-edit}-provider-cost-{strict,unknown}.{json,stderr}`.
Raw captures and source arguments are not committed. Eight offline tests cover
shape rejection, overlaps, nesting, thread separation, unsafe names, empty
profiles, bounded strict decoding, and subprocess CLI replay on real binary
format fixtures, including explicit unknown-label handling.
