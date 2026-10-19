# Central mutable-capture acceptance

Tested source snapshot: `d90c5ca576b21a6529e1ee05302fd6c3bad5cccc`, merging the independently checked mutable scalar capture implementation and its scoped evidence onto the published owner-local cleanup graph cut.

| Check | Run | Result |
| --- | --- | --- |
| Serial scoped closure, authority and graph suites | `20261018-215449968-965456` | 18 passed |
| `JUSTRUST_TEST_JOBS=4 justrust test --local --no-fail-fast` | `20261018-215820745-1029429` | 3,430 passed, zero failed/ignored/filtered, 191 binaries |
| `justrust clippy --remote --all-targets -- -D warnings` | `20261018-220545862-1023145` | Passed, explicitly hosted, zero warnings |

`cargo fmt --check` passed before the frozen full run. No isolated-retry warning appeared in the full output. Local execution is required for the existing absolute-path stock compiler oracles. The full suite ran under substantial unrelated compiler contention, so its 444.8-second duration is correctness evidence, not a speed comparison.

The scoped implementation uses ordinary checked shared or mutable hidden reference parameters for immediate closures capturing explicitly typed scalar parent parameters. It preserves strongest-mode selection, original binding identity, argument evaluation order, canonical source replay and independent generated-body ownership checking. Nested shared-to-mutable mode upgrades and parent environment offsets are covered. No aggregate-reference proof was relaxed.

Stored closures, move captures, general local or transitive captures, callable traits and full Rust support remain incomplete. The unchanged full-program corpus remains 3/20. This milestone does not activate the private const, object-layout, owner-identity or semantic-cache branches.

The three selected redacted records are in `central-acceptance.jsonl`. Earlier scoped and infrastructure failure history remains in this directory's existing worker receipt files.
