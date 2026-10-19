# Iterable inference validation receipts

This directory contains only four task-owned records selected verbatim from
`justrust export --out "$JCODE_SCRATCH_DIR/inference-receipts"` on source machine
`xps14`, for the scoped iterable-literal inference increment. Unrelated runs from
the shared recording database were not committed. The exporter redacts this
scratch checkout as `repo: private`, preserving numeric verdict/test evidence.

- `20261018-194248635-1125267`: hosted `justrust check`, passed.
- `20261018-194252485-1125621`: local `justrust test --local --test local_literals
  --test iterable_literals --test format_scalars`, five tests passed with real
  stock rustc. Hosted workers lack the stock oracle executable.
- `20261018-194252959-1125695`: hosted `justrust test --lib --test graph_integrity
  --test loops --test compiler --test local_literals --test iterable_literals
  --test full_programs --test prelude_methods`, 1,412 tests passed, including
  unchanged graph gates and all three exact-output passing corpus programs.
- `20261018-194303654-1125094`: hosted `justrust clippy --all-targets -- -D warnings`,
  passed without warnings.

Composition: `520819783` plus the current scoped inference patch. Formatting and
`git diff --check` also passed. These are correctness receipts, not controlled
performance measurements and not evidence of full Rust support.
