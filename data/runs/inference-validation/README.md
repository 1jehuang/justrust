# Iterable inference validation receipts

This directory contains only task-owned records selected verbatim from
`justrust export --out "$JCODE_SCRATCH_DIR/inference-receipts"` on source machine
`xps14`, for the scoped iterable-literal inference increment. Unrelated runs from
the shared recording database were not committed. The exporter redacts this
scratch checkout as `repo: private`, preserving numeric verdict/test evidence.

- `20261018-194248635-1125267`: hosted `justrust check`, passed.
- `20261018-194252485-1125621`: local `justrust test --local --test local_literals
  --test iterable_literals --test format_scalars`, five tests passed with real
  stock rustc. Local execution guarantees the installed oracle is exercised.
- `20261018-194252959-1125695`: hosted `justrust test --lib --test graph_integrity
  --test loops --test compiler --test local_literals --test iterable_literals
  --test full_programs --test prelude_methods`, 1,412 tests passed, including
  unchanged graph gates and all three exact-output passing corpus programs.
- `20261018-194303654-1125094`: hosted `justrust clippy --all-targets -- -D warnings`,
  passed without warnings.

Initial composition: `520819783` plus the scoped inference patch. Formatting and
`git diff --check` also passed. After concurrent nonforce push rejection, heap,
dyn/format authority and engine-guard changes were merged without changing the
inference implementation. Followup receipts:

- `20261018-194444318-1153830`: hosted check after heap/dyn composition, passed.
- `20261018-194447137-1154168`: the same three local stock suites, five tests pass.
- `20261018-194449233-1154665`: hosted selected suite extended with `--test heap
  --test native_heap_authority --test literal_output_authority`, 1,437 tests pass.
  `justrust show` confirms `where remote on hosted`. This includes the mandatory
  `Command::new("rustc")` scalar-output stock oracle, so PATH lookup succeeded in
  that hosted run. Its resolved path and version were not recorded. This does
  not prove that historical hardcoded `/usr/bin/rustc` paths exist there.
- `20261018-194505686-1153720`: hosted strict all-target clippy, passed.
- `20261018-194515928-1165017`: local literal-output authority, eight tests pass.
- `20261018-194612747-1212524`: after engine guard integration, hosted engine,
  graph-integrity, local-literal, iterable-literal and full-corpus suites, all
  28 tests pass.
- `20261018-194618873-1216421`: final composed hosted strict all-target clippy,
  passed.

Semantic commit: `b7fdb3cd9`, pushed through merge `6f4f99ab4`. The local oracle
lookup was separately verified as `/usr/bin/rustc`, Rust 1.98.1
(`48a229ceaefd4985c50990b14116b6d856af0985`, 2026-09-01),
`x86_64-unknown-linux-gnu`, LLVM 22.1.8. Both real corpus sources remain unchanged.
These are correctness receipts, not controlled performance measurements and not
evidence of full Rust support.

## Typed source-conflict follow-up

The exact former opaque-block rejection fixture is promoted only after stock
and native runtime stdout/stderr/exit parity. Local stock `/usr/bin/rustc`
1.98.1 was observed directly, and the promoted test requires PATH `rustc`
without skipping when unavailable. Five unused-body stock-negative cases retain
typed `Unproved` diagnostics with both conflicting numeric domains. The original
wide-integer semantic checks and all graph ratchets remain unchanged.

- `20261018-195313085-1396282`: hosted check passed.
- `20261018-195315469-1396110`: hosted generic inference, wide-integer graph,
  local literals, iterable literals and graph integrity, all 40 tests passed.
- `20261018-195348051-1400861`: the same suites plus library, compiler, loops
  and full programs, all 1,440 tests passed.
- `20261018-195357783-1400755`: hosted strict all-target clippy passed.

Formatting and diff checks passed. These four scoped records are retained
alongside the earlier eleven, not unrelated runs from the shared database.

Repair commit `3ea914ac0` was composed with concurrent stack/engine/canonical
fixes through `f9f902099` and pushed as merge `caec45370`. Final composed hosted
engine/canonical-copy/graph/inference/wide/corpus validation passed all 63 tests
(`20261018-195552250-1435084`), then strict all-target clippy passed
(`20261018-195602800-1437939`). Both final receipts are retained here as well.
