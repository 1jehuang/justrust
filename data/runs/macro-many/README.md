# Zero/many item macro function output receipts

Isolated detached base: `942b537dc`, validated on 2026-10-19 UTC.
The frozen primary checkout and stable installed client were not changed.
Integration and the composed full gate remain coordinator-owned.

- `stock-2026-10-19.json` retains thirteen new exact source strings (including
  final newlines), SHA-256 identities, actual `rustc -vV`, full diagnostics,
  compile exits and executable stdout. The first twelve sources and the added
  trailing `+` source were measured before changing the item-expansion parser.
  The retained transcript repeats those unchanged sources after implementation.
- `2026-10-19.jsonl` contains twelve slim, path-redacted records selected from
  `justrust export`, limited to this agent's runs. It retains two intermediate
  test-API compile failures as well as the successful scoped gates.

Final corpus: 42 stock sources (27 successful executables, 15 rejections),
168 native workers-1/8 CHECK/BUILD cells, 32 executed native images. The three
unused bad sibling controls require `expected Int(I32), got Bool` in all four
native modes and leave no artifact. Syntax controls pin the expanded original
location and definition/invocation provenance. Eleven Unsupported boundaries are
stock-valid excluded output. Trailing `@` is separately documented as stock
invalid but native transport-Unsupported, not invalid-Rust syntax parity.

Scoped validation: all-target check, seven item-scope unit tests, twenty-four
item-source/authority tests, twenty-three unchanged matcher/local/nested/literal
tests, seven unchanged graph-integrity tests, strict all-target clippy, formatting
and diff checks. See `ITEM-SOURCE-MACROS.md` for exact run IDs and phase details.

The first check ran `remote on hosted`. A later connection timeout triggered the
stable client's automatic local fallback for the scoped acceptance runs. The
native acceptance harness itself sets nonexistent RUSTC/PATH values, so these
native results do not use a stock compiler fallback. Test artifacts/cache use a
separate isolated target directory. No performance, general hygiene, remaining
fragment, export, proc-macro or full-Rust claim follows from these checks.
