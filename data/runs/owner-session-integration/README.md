# Engine-issued TokenSession owner acceptance

## Snapshots and scope

The reviewed private stack `aacdcee39..d7496f87e` was merged as
`00934a83820d4f7b1f3f359d00d9321da1e9c7e2`. The test-only follow-up
`c97a4aa59` was integrated as `0ef94af949a4b06f004be4077d2d0492d2187979`.
All final acceptance below pins that repaired source snapshot.

Only the TokenSession static is removed. The other seven compiler statics are
not activated by this increment. The reachable mutable-fact debt stays at 62
entries and six carriers. The 1208/1209 graph requirement and all existing
resource/growth ceilings remain unchanged. The lexical TokenSession baseline
shrinks from six to four Atomic occurrences and two to one static occurrence.
This is not complete graph unification, full Rust support, or a speed claim.

Engine owner issuance is consuming and payer-bound. The mutable owner path
stays in the builder, while published NameStore owns only an immutable path
and retained receipt. Published-store reachability, standalone-entry allow-list,
foreign payer/capability, replay and compile-fail controls are retained.
Path storage pays actual Vec capacity before initialization/publication.
Allocator-rounded transient surplus is admitted after allocation, not prepaid
physical-peak accounting. The Arc allocation header is prepaid, but the stable
infallible Arc allocation retains its documented allocator-abort caveat.

## Original failure and exact repair

`20261019-000700408-3337417` is preserved verbatim in the filtered export:
3589 passed, five failed, exit 101. Four tests expected zero retained bytes
after publishing a NameStore; the fifth exhausted Work during newly paid owner
setup rather than at the grouping phase it intended to test. Subsequent lint
in that original shell command did not run after the failed suite.

The repair changes test code only. Four final assertions now require the exact
published receipt computed from actual step capacity. Arena/format/scope refund
and last-owner checks remain. Failed unpublished paths still require zero.
The live empty-program baseline is greater than the receipt, not equal to it.
The grouping test measures real paid setup, repeats it with that exact Work
limit, then requires the first grouping charge to fail with requested Work 1,
used equal to limit, and the exact latched typed failure. No constructor bypass,
production budget change, hardcoded receipt byte value or gate relaxation.

## Acceptance

The JSONL contains exactly 25 original slim path-redacted `justrust export`
records, selected by exact ID without rewriting their contents:

- 13 private d749 records, from `20261019-000154937-3323162` through
  `20261019-000339008-3327844`: forced hosted check, strict lint, 162 integration
  tests, owner/budget/session/name/engine library groups, three compile-fail
  doctests, and debug-profile owner/budget tests.
- Seven central 00934 scoped records from `20261019-000514905-3332707` through
  `20261019-000616289-3335292`: check, owner path 20, budget 36, route 1,
  graph/authority/accounting 74, doctests 3 and strict all-target lint.
- The original failing full-suite record above.
- Repaired complete hosted library acceptance, 1476/1476:
  `20261019-001619033-3553249`.
- Repaired hosted strict all-target lint:
  `20261019-001648046-3553118`.
- Frozen repaired full acceptance, **3594/3594**, exit 0:
  `20261019-001700983-3555423`, 276.719 seconds including compilation.
- Post-full forced hosted strict all-target lint:
  `20261019-002137806-3761440`, exit 0.

`cargo fmt --check` and `git diff --check` passed. Full suites used local
`CARGO_BUILD_JOBS=4 justrust test --local --no-fail-fast` because the real
stock-oracle tests require local absolute toolchain paths. Hosted checks/lints
were forced hosted. Contended wall times are acceptance evidence, not benchmarks.
Historical expansion-receipts records elsewhere in this stack remain historical,
not evidence recorded on this final snapshot.

## Fresh real CLI acceptance

After the repaired full suite, the freshly built `target/debug/jr-rust` checked,
built and ran the same three-file crate with workers 1 and 8. Both printed
`multi ok\n` with empty runtime stderr. An invalid `let x:i32=true` program
returned exit 1 with a nonempty typed diagnostic at both worker counts.
No claim of improved diagnostic spans is made.

Binary SHA256: `4c44ebc3c818b13bb10a56c0fb4c202878eb7ed21c187ef67342590a0d38f7c1`.
The exact sources are:

```rust
// main.rs
mod outer;
fn main() { assert_eq!(outer::answer(),42); println!("multi ok"); }
// outer.rs
mod inner;
pub fn answer()->i32{inner::value()+2}
// outer/inner.rs
pub fn value()->i32{40}
// invalid.rs
fn main(){let x:i32=true;}
```

Source SHA256 values, including original file whitespace:

- main: `886fcac887a55ba7ee3714fa2e750fab1989b4f9429134e29123f819095ac6c8`
- outer: `e8d38e9ecac73bc28d168f549873b4947423c74b0d961c2442f50732df016163`
- inner: `058fb95a0ca16c154bb99ad311bd9c5b1020b3b3ebb1c292721dc42b0d7746ab`
- invalid: `c92a6b40f99fe56e2e7584c4e2316ccfccfee22c29c9a8e792b6419eddc15157`

Corpus remains 3/20. The private OR-pattern and optimizer-accounting work is
not included in this acceptance.
