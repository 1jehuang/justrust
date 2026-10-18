# Literal panic macro: bounded native abort support

Status: delivered for the bounded literal/empty-message domain below. Full Rust remains open.

Admit builtin `panic!()` and `panic!("literal"[,])` as genuine divergent expressions, including Rust macro delimiters. Existing lexical macro definitions named panic take precedence; known unsupported module macros must not be silently replaced by a builtin. Builtin selection uses checked macro namespace identity and an explicit not-found result, never matching an error message. Formatting arguments/nonliteral messages remain explicit earlier boundaries, and panic=unwind/cleanup is not implemented by this abort-only milestone.

The source payload retains the checked invocation SpanId and literal text. Use the checked original invocation location (expansion call-site policy) for diagnostics; reject foreign source handles before publication. Box the cold payload rather than enlarging all expression stack frames. IR records a real panic with exact Never type and nonreturning completion; mutable public IR cannot claim a returning panic or fabricate a normal value. Every source/IR clone, storage/depth/name/replay visitor must include it.

CFG uses a distinct terminal Panic operation carrying literal diagnostic data, with no successor and no assignment to a result destination. Lowering must terminate then create only an unreachable continuation for subsequent source validation. Treat it as a terminal in ownership/borrow dataflow and actual CFG let-else divergence proofs. Dead source is still checked, unused generic bodies remain universally checked, and ordinary user functions/macros named panic must preserve their own identity.

On native Linux x86_64, this milestone prints a useful diagnostic containing the checked file, line, column and message to stderr, with partial-write/EINTR handling, then aborts the process with SIGABRT. Reset/unblock SIGABRT before raising it; do not claim exit101 equals abort. Request a nondumpable process before raising the signal so external core handlers do not delay ordinary termination; core-file compatibility is outside this milestone. A terminal syscall-failure fallback may exit101, and a failed/nonpositive getpid must never be passed to kill. Do not reuse the silent arithmetic trap as panic. No stock stderr byte-for-byte or unwinding claim. Existing arithmetic/bounds trap behavior remains unchanged.

Stock-first tests preserve no-argument/literal/escaped-message semantics, branch/coercion/return/initializer/let-else divergence, once-only evaluation and short circuiting, rejected dead-source errors, lexical shadowing, and actual signal/stderr observations. Include public IR wrong-type/completion/source metadata and CFG mutation controls, original projected-method panic witnesses, full regression/resource/lint/format gates and debug/release native checks. Report this bounded abort-only primitive, not general formatting, unwinding, or full Rust.

## Measured acceptance (2026-10-18)

The 47 immutable stock-first sources partition into 12 live native programs,
17 aborting programs, five ordinary invalid programs and 13 explicit earlier
unsupported boundaries. Each profile exercises 188 CLI and 188 observed-graph
CHECK/BUILD cells at workers 1/8, including 116 native executions across both
interfaces. Aborts assert signal 6, message and checked file/line/column, not
stock stderr byte identity. All 24 historical projected-method source payloads
remain unchanged; six original panic-bearing programs now execute successfully.

Hosted all-target check and warning-free clippy passed. The complete local
all-target suite passed 3,024 tests in 147 binaries
(`20261017-180229874-4183246`). Local optimized panic, public authority and
projected-method suites passed 16 tests (`20261017-180258455-14719`). Hosted
private panic controls passed all 17 in both debug and release, including
ignored/blocked SIGABRT, closed stderr, denied/nonpositive getpid without a
kill attempt, denied signal/nondumpable setup, broken-pipe stderr, and repeated
payload storage accounting. Public authority adds seven source/IR/CFG controls.
The 30 available failed and passing receipts are
retained in `data/runs/xps14/2026-10-18-panic.jsonl`.

The complete suite and stock-abort corpus were deliberately run locally after
hosted stock subprocesses stalled. This is not a hosted full-suite pass.
Native post-exec nondumpable handling fixes the observed native abort delay;
stock subprocess handling remains an environment limitation. The final local
suite's silent period was ordinary compilation and test execution, not a
deadlock. No benchmark speedup, unwind cleanup, formatting, std integration,
core-file compatibility or full-Rust support is claimed.
