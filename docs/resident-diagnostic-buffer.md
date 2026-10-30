# Rejected resident diagnostic text buffer experiment (v20)

**Decision: REJECT. Production `main.rs` was restored exactly to HEAD.** This was a small allocation-reuse hypothesis in the licensed Blaze-derived experimental resident engine, not the clean-room engine. All correctness gates passed, but no repeatable real wait benefit was demonstrated. No allocation counts were measured. These noisy observations are descriptive, not a general speed claim, statistical significance claim, comprehensive session-weighted gate, or default promotion.

## Bounded hypothesis and identical hash semantics

The archived candidate made `errors_of` own one initially empty `RefCell<String>` per call. Its `asked` closure temporarily borrowed the storage for each function source, cleared its length, and appended every `SyntaxText::for_each_chunk` chunk without normalization or filtering. The helper hashed the resulting complete UTF-8 string with `FxBuildHasher.hash_one(scratch.as_str())`. The original owned `String` hashes through the same Rust string hash semantics, including the string terminator. Comments, whitespace, Unicode, and all trivia remained in the input. Hash collision properties were unchanged. Hashing chunks separately would not be equivalent and was deliberately avoided.

Only storage was reused. There was no persisted or shared scratch, query wrapper, new cache, cache key change, diagnostic selection/filter change, root graph change, or other function-cache policy change. Storage lived only until `errors_of` returned, retaining at most its largest examined function capacity. This is not evidence of an allocation-count reduction in a measured workload, much less proof of lower agent wait.

## Correctness admission

The two focused `resident_diagnostic_buffer` tests compared complete strings and Fx hashes against the original `text().to_string()` path across empty syntax, Unicode, comments/trivia, nested syntax, chunk boundaries, multiple functions, and large-to-small-to-empty storage reuse. Root ran release check/build and positive native admissions: helper **2**, graph reuse **1**, listed-crate roots **1**, telemetry **3**. Full native run `20261030-113913476-2300871` passed **348** tests, exactly baseline 346 plus two helper tests, not 349. The evidence binds all 348 raw test names and footer counts.

Fixed-five runtime sessions RSB11 (v4 engine baseline) and RSA17 (v20 candidate) passed exact four-test execution markers, type-error and assertion-failure admission, and both recoveries, with source/toolchain identities unchanged. The separate interleaved observation also passed every gate across **141 rows and 282 raw streams**, including 42 identical-byte edits through all six old/new/Blaze orders seven times each, with no sample exclusion.

## Measured result and rejection

| Observation | Baseline engine | Candidate engine | Blaze control | Candidate versus baseline |
| --- | ---: | ---: | ---: | ---: |
| Fixed five, warm edit median | 0.114094150 s | 0.113326389 s | 0.112126644 → 0.112618931 s | -0.673%, reference-normalized -1.107% |
| Separate 42-edit interleaved, warm edit median | 0.10399137149943272 s | 0.10771834100160049 s | 0.10450627249520039 s | **+3.584% slower** |

The small favorable fixed-five movement did not survive the separate interleaved diagnostic. Noise contamination is recorded as **unknown**. The interleaved result supports rejecting this candidate for lack of demonstrated wait benefit, not asserting a universal regression. No engine source, helper tests, or default behavior changes are shipped by the evidence-only commit.

## Reproducible offline evidence

`bench/results/resident-diagnostic-buffer-20261030/` retains full candidate and baseline source bytes, exact HEAD-to-candidate patch and source-ready receipt, frozen native source and executable identities, check/build/unit scripts and logs, the exact seven v20 native runs with allowlisted output/meta/summary, private-home bundle-local slim exports, raw RSB11/RSA17 suites, and the interleaved runner/state/original/streams. Root restoration was verified independently by comparing the actual checkout to HEAD and the archived baseline hash.

Run these without native tools, scratch inputs, network access, or original absolute paths:

```sh
python3 -B bench/results/resident-diagnostic-buffer-20261030/validate.py
python3 -B bench/results/resident-diagnostic-buffer-20261030/test_validate.py
```

The validator replays the candidate patch from baseline in memory, binds exact source/toolchain/tools/profiles/source63 identities, recomputes edits and medians, parses raw test names/statuses and failure/recovery markers, and validates paired ordering. Nineteen offline tests include positive and moved-directory replay plus fail-closed marker/status/testcount/hash/median/source/tool/profile/toolchain/edit/order/recovery mutations. Archived scripts and absolute paths are historical receipts only, never executed by validation. `audit.json` and `offline-tests.log` retain the replay outcome. This followthrough used no new native workloads and did not edit global run records or production source.
