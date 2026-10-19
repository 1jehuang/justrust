# Private original root-arm OR M1 scoped receipts

Source checkpoint: `bd912d003`, composed non-destructively from preserved
`f2cf166a5` and approved published `aa1d3c8c1`. This is private, not main
acceptance. No local compiler build, full suite, push, installation, resource
ceiling change or new kind was performed. Existing source seal authority is
unchanged. Exact contract: `ROOT-OR-PATTERNS.md`.

All run IDs below use prefix `20261019-`; the nine exact owned records are in
`scoped.jsonl`. Each compiler invocation used `JUSTRUST_REMOTE=force`,
`JUSTRUST_BUILDD_PREBUILD=0` and `-j 2`. Test binaries used `--test-threads=1`.
All final compiler receipts report hosted execution and nightly-2026-10-08.

| ID suffix | Scope and result |
| --- | --- |
| 000255013-3325738 | Initial `check`: missing exhaustive Alternatives arm in pattern_locals, exit 101. Repaired explicitly fail-closed. |
| 000437970-3330886 | `check`: exit 0. |
| 000444804-3331316 | Initial root_or_patterns: accepted-source runtime test passed, policy test failed on stock-valid p04 binder duplicate diagnostic. Exit 101, not all-green. Repaired per-row name resolution and explicit binder Unsupported. |
| 000549746-3334288 | Root workflow: runtime passed, policy test failed on preexisting p09 parameter Syntax boundary. Exit 101. Preserved grammar, repaired taxonomy only, not a language-support claim. |
| 001202202-3539609 | Eight selected binaries: 76 passed, none failed/ignored/filtered. Includes all 10 unchanged graph_integrity tests, root_or_patterns(2), root_or_authority(3), match_graph, pattern_graph, matches_macro_authority, pattern_usefulness_authority and pattern_decision_authority. |
| 001224008-3540674 | `test --lib root_or`: 5 passed, 1434 filtered. Source Arc seal mutation/foreign-owner test, two pending lease tests and two exact cumulative work/header tests. |
| 001233908-3541089 | Actual `clippy --all-targets -- -D warnings`: exit 0, 40 compiled units. |
| 001344298-3544474 | Final root_or_patterns after test-only exact offending-root object controls: 2 passed. Includes unused invalid bodies, complete report, no failed BUILD105, no CHECK103 and no offending-root OBJECT103. Unrelated valid objects are allowed. Production, authority tests and graph gates were unchanged from the 76+5 receipts. |
| 001347662-3544603 | Final strict all-target lint: actual exit 0, fresh cached artifacts, no compiled units. |

`cargo fmt --all --check` and `git diff --check` passed. Hosted reports noted the
existing oversized engine crate; no restructuring, speed claim or baseline
promotion was made.

## Stock evidence

`experiments/results/root-or-stock33-20261019.json` retains exact 33 source files,
toolchain text, the original 64-cell result table, separate two-profile I9
results, raw nonempty debug streams and all 228 compiler/runtime stream hashes
from both profiles. There were 24 stock accepts/runs and 8 rejects per original
profile. I9 separately rejects the exact trailing separator `0 | =>`.
I8 remains the distinct `|=` lexical negative. No replacement or inferred codes.

Toolchain: `/usr/bin/rustc` 1.98.1,
`48a229ceaefd4985c50990b14116b6d856af0985`, edition2024,
x86_64-unknown-linux-gnu, panic=abort, opt-level 0 and 3. Stock probes were serial
in isolated scratch before compiler edits (the original 32 before implementation
approval, I9 separately approved afterward). No native acceptance is
inferred from stock results. The native matrix embeds these same exact bytes.

Current support is only original binder-free root-arm scalar/tuple OR, not
nested OR, binders/guards/references/nominals, generated owners, parameter/let/for
patterns, unreachable warning support, full Rust or corpus09 promotion.
