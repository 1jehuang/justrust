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

## Published-base revalidation

The original `441d0426d` commit is preserved at
`refs/jcode/checkpoints/macro-many-441d`. Its one scoped change rebased cleanly
onto exact published `8550f407a66b4f3d05fb65c568bc74cb8935c960` as
`74cf5973ccbbe83862e81d3d6979c7760fc5fc22`. The documentation/receipt-only
`3537c8fef` followthrough was merged as `2a3237f902c941cb230cff0e1e32a7889083af77`.
That merge changes no source, tests, build configuration or graph gates.

`rebase-8550.jsonl` retains five own exported runs:

- Hosted all-target check: `20261018-210459332-3442860`.
- Hosted full library plus macro/authority/graph gate:
  `20261018-210509233-3443609`, 1,434 passed with zero failed, ignored or
  filtered: 1,380 library, 47 macro/authority and seven graph-integrity tests.
- Hosted strict all-target clippy: `20261018-210534053-3445885`.
- Optional unchanged `04_primes_sieve` native CHECK:
  `20261018-210653505-3451753`, rejects at 2:27 with unknown function-local
  macro (`vec!`).
- Optional unchanged `13_trait_objects` native CHECK:
  `20261018-210707143-3453223`, rejects at 5:15 with unknown function-local
  macro (`format!`).

The optional stock executables both match their complete existing `.stdout`
files byte-for-byte. Native checks do not pass, so no native runtime success,
manifest promotion, fixture edit or full-program improvement is claimed. The
three acceptance gates above were verified `remote on hosted`; the two optional
CLI probes used the stable client's local run path. Formatting and diff checks
also pass. No push or primary-checkout change accompanies this validation.
