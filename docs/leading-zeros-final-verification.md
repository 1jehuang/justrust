# Final whole-result verification: integer leading zeros

Re-executed on commit `bf49b8639` with the preserved pending character-cast
changes present, not just on the isolated implementation snapshot.

## Executed gates

- `cargo fmt --check`: passed.
- `justrust test --local`: **3,227 passed, zero failed**, 167 binaries,
  run `20261018-025934836-652814`.
- `justrust test --local --release --test leading_zeros`: passed,
  run `20261018-030202925-833612`.
- `justrust test --local --release --lib leading_zero`: **24 passed**,
  run `20261018-030316905-847538`.
- `justrust check --remote --all-targets`: passed,
  run `20261018-030352918-849070` (fresh artifacts).
- `justrust clippy --remote --all-targets -- -D warnings`: passed,
  run `20261018-030353455-847528` (fresh artifacts).

A server restart interrupted the first release-library attempt, exit 143,
run `20261018-030232168-838410`. The explicit resumed invocation above passed.
Local execution is required by the `/usr/bin/rustc` stock oracle. Receipts,
including the interruption, are in
`data/runs/xps14/2026-10-18-leading-zeros-final.jsonl`.

## Requirement-to-observation mapping

| Requirement / changed public output | Check executed in final gates | Observed result |
|---|---|---|
| All 12 exact integer receiver types, u32 result, width-specific zero/sign behavior | `exact_integer_receivers_return_u32`, 12 frozen width fixtures, native word/wide tests | Accepted and exact stock outputs matched |
| Every u8/i8 bit pattern and every u128 one-bit position | `bytes_exhaustive`, `u128_positions` fixtures | Complete outputs matched independent count oracle and stock |
| Once-only receiver effects and divergent completion | `receiver_is_lowered_once`, two divergent frontend tests, `once`/`divergent` fixtures, emitter once-only test | Side effects once; divergence preserved; output matched |
| Nested method calls and distinct slice-length authority | `estimator_binary_receiver_and_nested_methods`, mixed-slice test, `nested` fixture | Accepted in proper order, output matched |
| No accidental nominal/generic method interception | Two ordinary-method frontend tests and `nominal` fixture | Original method signatures/results preserved |
| Wrong receiver/arity/typeargs/result, including dead source | Eight frozen invalid fixtures and `wrong_receivers_and_arguments_do_not_acquire_builtin_authority` | Rejected semantically, not as resource/infrastructure failures |
| Missing/duplicate/reordered/forged source rows, receiver/result/completion/orphans | Two replay-rejection frontend tests | Malformed cases rejected |
| Immutable public source boundary | `sealed_publication_rejects_in_place_receiver_mutation_and_sibling_swap` | Same-ID payload mutations rejected by publication seal |
| Bounded replay, cycles and resource accounting | `replay_budget_authority_and_exhaustion`, CFG cycle/budget test | Exhaustion rejected and cycle bounded |
| IR/CFG validation includes unused and dead rows | IR unused operand/result/completion test and CFG actual-operand/dead-row test | Malformed rows rejected |
| Substitution, provenance, once-only lowering | Two CFG replay/substitution tests and once/origins test | Valid lowering preserved, forged replay rejected |
| Ownership initialization, moves and borrow liveness | Two ownership leading-zero tests | Uninitialized/moved reads rejected; borrow kept live through read |
| Complete unchanged original estimator module | `estimator` exact-source fixture, provider SHA256 check, nine fixed plus 14,693 sweep cases per execution | Hash retained; stock and independent integer oracle agreed |
| Public CLI and observed graph CHECK/BUILD at workers 1/8 | `frozen_stock_before_native`, in debug full suite and release acceptance | 224 native check/build cells and 80 executable outputs per profile passed |
| Unrelated pending character-cast compatibility | Entire final 3,227-test suite includes character cast and authority tests | Passed with pending work present; pending files remain uncommitted |
| Broader compiler regressions | Entire final test suite, all-target check, strict Clippy and format | All passed |

Reference autoderef and UFCS are explicitly legal-unsupported, not claimed as
native successes. Helper replay is not independent reconstruction of every
expression from the AST. Full Rust, whole core/alloc/std, general Cargo projects,
and optimization/performance claims remain outside this completed slice.
