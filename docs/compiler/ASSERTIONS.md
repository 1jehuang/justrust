# Bounded builtin assertion macro

Status: bounded implementation delivered. Full Rust remains open.

Follow-up: `EQUALITY-ASSERTIONS.md` and `DEBUG-ASSERTIONS.md` record later bounded extensions. The unchanged `deferred_debug_assert` witness is now an enabled native abort case. Counts and scope below describe the original assertion milestone, not these later extensions.

Admit `assert!(condition)`, an optional trailing comma, and one optional literal message with an optional trailing comma, using parentheses, brackets or braces. Evaluate the condition once with the ordinary Bool requirement. Success yields Unit. Failure uses the existing checked literal panic path and native abort contract in `PANIC.md`. Preserve short-circuiting, nested condition effects and source errors in dead/unused bodies. This does not introduce a new trait-based truth conversion or an executable assertion type.

Select the builtin using checked macro namespace identity only after real lexical/module macro lookup misses. A user macro named assert retains its meaning. Unknown or unsupported module macros must not be replaced by a builtin. Reuse the checked invocation location and source replay authority. Generated source structures must be fully budgeted, and the original delimiter must still govern statement semicolon rules even if the source implementation lowers to an ordinary If expression.

The default native diagnostic is `assertion failed` with the checked invocation file/line/column. A custom literal uses existing escaped-brace handling. Exact stock diagnostic wording and condition pretty-printing are not claimed. Formatting/nonliteral messages, assert_eq/assert_ne/debug_assert and unwind/Drop cleanup remain outside this milestone and must preserve their explicit current boundary, not be treated as accepted assertions.

Acceptance uses immutable independently measured stock sources, actual workers1/8 CHECK/BUILD and graph/native execution, exact once-only and Unit behavior, SIGABRT rather than ordinary success, and public mutation/replay controls. The full regression suite, strict lint and format gates remain mandatory. Local stock-containing acceptance follows the documented hosted abort-subprocess limitation, not a general hosted bypass.

Delivered validation: the 33-source stock/native corpus and nine public authority
tests pass in debug and release. Full local regression passed 3,049 tests in 150
binaries (20261017-181248393-198137), and hosted all-target check and strict lint
passed. Exact failed and passing receipts are retained in
`data/runs/xps14/2026-10-18-assertions.jsonl`.

The accompanying IR validation rejects non-Bool conditions and forged branch
result types, including unused arena rows. Explicit-tail joins remain checked
even when unreachable. Existing source while-let lowering erases some
semicolon/completion information, so those no-tail branches retain a bounded
Unit/Never possibility rather than inventing exact source provenance. Complete
canonical source-flow reconstruction remains open. No new truth conversion,
formatting support, full Rust compatibility or speedup is claimed.
