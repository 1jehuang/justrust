# Immediate noncapturing closure validation

Scope: `9b3c336a0`, immediately invoked closures with explicitly typed parameters,
scalar/unit results, and an empty environment. This is not stored/captured closure
or `Fn` trait completeness. No speed or full Rust compatibility claim is made.

The JSONL contains thirteen selected complete `justrust export` records, with
normal path/private-workspace redaction. Hosted checks used the installed stable
client and separate experimental workspace. Local runs were required because
stock-oracle fixtures invoke the local `/usr/bin/rustc`, absent on the hosted
machine. No rewrite package was installed.

| Run | Command and observed result |
| --- | --- |
| 20261018-195439712-1405412 | hosted `justrust check`, pass |
| 20261018-195442495-1405542 | local `test --test closures --test panic`, 9 pass |
| 20261018-195503242-1412550 | hosted `test --test graph_integrity`, 5 pass |
| 20261018-195517912-1405231 | hosted `clippy --all-targets -- -D warnings`, pass |
| 20261018-195531279-1418401 | frozen `11c92c837` local full `test --no-fail-fast`, 3338 pass / 2 fail, 184 binaries |
| 20261018-200147180-1760095 | rebased on `8434f8f05`, hosted check, pass |
| 20261018-200153600-1764035 | same base, local exact regression suites below, 48 pass |
| 20261018-200251599-1804384 | same base, hosted graph integrity, 5 pass |
| 20261018-200308088-1758013 | same base, hosted clippy, pass |
| 20261018-200335556-1827590 | final `9b3c336a0` on `72dc91bae`, hosted check, pass |
| 20261018-200337281-1828181 | final base, local exact regression suites, 48 pass |
| 20261018-200400838-1840477 | final base, hosted graph integrity including published-fact reachability, 7 pass |
| 20261018-200420038-1825945 | final base, hosted clippy, pass |

Exact regression command:

```text
justrust test --local --test closures --test generic_inference --test wide_integers_graph --test panic --test literal_output_authority
```

The frozen full run's two failures were precisely:

* `generic_inference::rejects_opaque_effect_argument_no_context`: source accepted
  where the old test expected an Unsupported boundary.
* `wide_integers_graph::unused_invalid_wide_values_are_validated_before_codegen`:
  earlier `InvalidMetadata` with `conflicting local scalar inference constraints`
  instead of the expected later diagnostic.

Current origin's coordinated inference repairs were rebased before both 48-test
passes. The full suite was not rerun after those repairs, so these focused passes
are not represented as a clean final full-suite result. All stock/native closure
cases, panic boundary fixtures, and other full-run suites passed in the frozen
run. The two panic catch fixtures retain exact frozen source text and remain
Unsupported, with only their closure-boundary diagnostic fragments updated.

The closure oracle test requires stock rustc to be present and checks fourteen
whole programs: nine check/build/run positives and five real type/move/loan
negatives. Other closure tests inspect separate per-declaration graph identities,
unused body verification, explicit Unsupported boundaries and live parser depth.
All graph ratchets were retained without loosening or adding whole-program kinds.
