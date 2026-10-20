# Normal-path Drop stock corpus

This corpus was frozen and executed with **stock `/usr/bin/rustc` before any native compiler invocation**. It is not a claim that the experimental compiler supports these programs.

## Authoritative receipt

`receipts-v9/` is the receipt used by integration: 56 sources (33 stock-positive, 7 stock-invalid, 16 legal unsupported). Manifest SHA-256 `afc7b27b95496aa84316d9e21485fb7c4f2c18d57701251bcffc152593aee7fe`, stock receipt `a35fe212113376b84ed3b011f149b72a15001d2c78c76e3d38acd5e6427c45c1`, index `213e1c312094ffb078cd49c04e508721188e4b0308b1c990b939ce3af5b585e7`.

Compiler: rustc 1.98.1, commit `48a229ceaefd4985c50990b14116b6d856af0985`, x86_64-unknown-linux-gnu. Flags: `--edition=2024 -C panic=abort -A warnings`. Byte streams are base64, not normalized text. Every receipt was verified by a second stock execution.

History, all retained and still replayable: v1 (26 sources), v2 (+abrupt call arguments), v3 (+edge cases, legal unsupported boundaries), v4 (+manual Copy+Drop N22), v5 (+six no-lint-attribute `*_plain` variants), v6 (+qualified lookalike, ignored dynamic index scope, generic qualified explicit call), v7 (+`if` condition and lazy `&&`/`||` temporaries, added after a reviewer found conditions held temporaries past the branch), v8 (+`parameter_rebind`, `let mut` variant of the `mut`-parameter fixture), v9 (+concrete qualified explicit destructor call). Verification checks only each receipt's own manifest-listed paths.

## Reproduce

From the repository root:

```text
python3 experiments/drop_normal_oracle.py verify
```

The runner uses only `/usr/bin/rustc`, checks compiler/source hashes, imposes a 30-second timeout per subprocess, compares runtime bytes, and refuses to overwrite existing manifests/observations. A new deliberate corpus revision requires a new `--receipts` directory, followed by `freeze`, `observe`, and `verify`. The runner does not invoke, inspect, or derive expected outcomes from the native compiler.

## Positive candidates and observed stock stdout

Below, ` / ` means a newline, including a final newline in every row. Exact bytes remain authoritative in the receipt.

| Fixture | Observed stdout |
| --- | --- |
| scope | inner body / drop 4 / drop 3 / outer body / drop 2 / drop 1 / params body / drop 3 / drop 2 / drop 1 / done |
| two_types | done / drop B / drop A |
| moves_returns | returning / drop 2 / caller owns / identity returned / done / drop 3 / drop 1 |
| conditional | true / consume / drop 1 / after conditional move / after conditional init / after joined init / drop 3 / drop 2 / false / after conditional move / after conditional init / after joined init / drop 2 / drop 1 |
| replacement | rhs / drop 1 / after replace / after self move / drop 2 |
| reinitialize | consume / drop 1 / moved / reinitialized / drop 2 |
| early_return | early / drop / caller / fallthrough / drop / caller |
| loops | continue / drop 1 / break / drop 1 / labeled break / drop 2 / drop 1 / done / drop 3 |
| labeled_continue | labeled continue / drop 2 / drop 1 / labeled break / drop 2 / drop 1 / done |
| break_value | break / drop 1 / receiver owns / drop 2 |
| temporaries | drop 1 / after statement / after binding / drop 3 / after wildcard / drop 2 |
| ignored_place | after ignored place / after move / drop |
| lookalike | explicit custom / no automatic custom |
| call_argument_return | return from later argument / drop 1 / drop 2 / caller |
| call_argument_break | break from later argument / drop 1 / drop 2 / after loop |
| canonical_copy_x32 | canonical copy oracle ok |
| declaration_order | initialized second / initialized first / drop second declared / drop first declared |
| destructor_helpers | body / destructor / first helper / second helper observed field |
| destructor_mutation | body / before mutation / observed mutated field |
| direct_temporaries | drop statement / after direct statement / after direct binding / drop wildcard / after direct wildcard / after ignored existing place / drop binding |
| loop_flags | consume iteration / drop 1 / retained iteration / drop 2 / retained iteration / drop 3 / natural exit / after zero iterations / drop root |
| parameter_paths | true / nested body / drop nested / consume first / drop first original / early after reinit / drop local / drop second parameter / drop first replacement / caller after true / false / nested body / drop nested / drop first original / fallthrough after replacement / drop local / drop second parameter / drop first replacement / caller after false |

The X32 file is byte-for-byte equal to `Fixture X32.full_source` in `tests/canonical_copy.rs`, including final newline. Its measured full-source SHA-256 is `77d24a043ea0f6d3e99e19ac05d82b229b4c56985791c70a57ed246d0ba5c453`. The historical fixture declares `28fc4d7000d2464e5f7f11cfb3467592099b3de0b6e8da40432963667b7e5de6`, which is not the hash of those full-source bytes. X32 retains the historical scalar projection and destructor mutation unchanged. X32's output alone cannot prove that mutation ran, so the other marker-bearing destructor fixtures are required.

## Explicit native-domain mapping and source adaptations

The immutable manifest classifications describe stock and candidate intent; the harness independently records known native-domain exclusions. There are **25 native-positive cases, four native semantic-invalid controls, and 27 native-domain rejection controls**. Domain rejection is not Rust language conformance.

The original `positive/{scope,conditional,moves_returns,loops,labeled_continue,temporaries}.rs` contain `#![allow(unused_variables)]`. This active attribute is outside the current native grammar. They are retained unchanged and require an explicit unsupported-attribute diagnostic. Each corresponding `*_plain.rs` is a separately authored and separately stock-frozen native-positive fixture. Its bytes differ only by removal of that one leading attribute line, verified before v5 freeze. All six pairs produced identical stock stdout/stderr/status. The table above gives the outputs for both members of each pair. No source is rewritten at test execution time.

`negative/copy_drop.rs` uses unsupported derive scaffolding and is likewise a domain rejection, not the native Copy+Drop semantic control. `negative/copy_drop_manual.rs` instead preserves exact `N22.full_source` from `tests/canonical_copy.rs` with manually written Clone and Copy impls. Its measured full-source SHA-256 is `40c8296a2090b353985caf943faee4af5bf717ce531501f8075ebbdb6fbb15e1`; stock rejects it with E0184. This is the native semantic Copy+Drop control.

## Invalid Rust, not unsupported Rust

| Fixture | Stock diagnostic |
| --- | --- |
| negative/copy_drop.rs | E0184, Copy cannot coexist with a destructor (native blocked by derive attribute) |
| negative/copy_drop_manual.rs | E0184, manual Copy cannot coexist with a destructor |
| negative/move_field.rs | E0509, non-Copy field cannot be moved out of a Drop owner |
| negative/explicit_method.rs | E0040, explicit destructor call prohibited |
| negative/explicit_ufcs.rs | E0040, explicit destructor call prohibited |

The field-move negative deliberately uses a non-Copy String field, outside the scalar owner admission slice. It must not be confused with legal scalar Copy field reads. The native harness expects an explicit unsupported owner/type diagnostic for this one case, **not native E0509 conformance**. D17 remains partial: non-Copy field-move conformance is blocked by the excluded owner shape. Native invalid-Rust controls are manual Copy+Drop and the two explicit destructor calls. The derived Copy+Drop fixture is retained as an unsupported-attribute boundary.

## Integration harness

`tests/drop_normal.rs` consumes `receipts-v9/fixture-index.tsv`, exported directly from the observed stock receipt by `python3 experiments/drop_normal_oracle.py export`. It pins the index, manifest, receipt, compiler, and each source hash. Before any native invocation it rechecks the entire stock corpus in metadata and executable modes, then runs a bounded child for native CLI and observed CHECK/BUILD with workers 1 and 8. Successful BUILD artifacts are executed and compared byte-for-byte. Successful and rejected observed results must have no structured resource failure or poisoned budget. CHECK must publish no executable or machine-code node. Each process is bounded, including the child containing in-process observed calls.

Run the target in both bootstrap debug and release profiles only after coordinator authorization. The native child executes all native-positive candidates first, then every semantic/domain rejection control; none are silently skipped. Adversarial cleanup authority tests remain separately owned.

## Legal Rust outside the candidate native slice

All files under `unsupported/` compile and run on stock. Native rejection must be reported as a limitation, never counted as Rust-invalid conformance.

| Fixture | Observed stock stdout |
| --- | --- |
| generic | generic drop |
| generic_function | consume generic / generic function drop |
| nested | outer drop / inner drop |
| array | drop 1 / drop 2 |
| enum | enum drop |
| tuple | tuple owner drop |
| borrow | inspect / after borrow / borrowed owner drop |
| reference_field | reference field drop |
| borrowed_replacement | drop old / after borrowed replacement / drop replacement |
| indexed_replacement | drop old / after indexed replacement / drop replacement / drop second |
| dead_aggregate | dead aggregate not executed |
| dead_generic | dead generic not executed |
| owned_pattern | copied pattern field / drop patterned owner |
| owned_match | matched owned value / drop matched owner / after match |
| owned_if_let | iflet owned value / drop iflet owner / after iflet |
| owned_let_else | letelse owned value / drop letelse owner |

The destructuring probe copies a scalar field legally but is outside the native owned-pattern slice. Indexed and borrowed replacements may encounter their excluded container/reference domain first; these tests do not require a particular error precedence. Dead-code probes require an explicit unsupported boundary even though stock never executes their excluded expressions.

These fixtures do not prove all unsupported boundaries, borrow-based temporary lifetime extension, or unwind behavior. D21 later-use loan conflicts, D23 malicious cleanup/proof mutation, and D24 induced resource exhaustion with no partial publication require separately owned internal controls. The harness's no-failure/no-poison assertions are not a D24 exhaustion test. See `../../../docs/compiler/DROP-NORMAL.md` for the larger implementation contract. Native results are recorded in `../../../docs/compiler/DROP-NORMAL.md`.

## Later additions (v6-v9)

| Fixture | Observed stock stdout or diagnostic |
| --- | --- |
| positive/lookalike_qualified | explicit custom / no automatic custom |
| positive/ignored_scalar_index | nested body / drop nested / after nested / iteration body / drop iteration / iteration body / drop iteration / outer body / drop second / drop first |
| positive/condition_temporaries | drop cond / then body / drop cond / else body / drop lhs / drop rhs / and body / drop lhs / short body / drop while / while body / drop while / while body / drop while / done |
| positive/lazy_temporaries | drop lhs / rhs eval / drop rhs / and body / drop lhs / rhs eval / drop rhs / or body / drop lhs / drop rhs / after let true / drop let / after plain true |
| positive/parameter_rebind | as parameter_paths, except `drop first replacement` precedes `drop second parameter` because the rebound local drops before parameters |
| negative/generic_explicit_ufcs | E0040 |
| negative/explicit_qualified | E0040 |

Additional native-domain boundaries (rejected before Drop admission, never counted as conformance): `positive/lookalike.rs` and `negative/explicit_ufcs.rs` use unqualified `Trait::method` paths the resolver does not support; `positive/parameter_paths.rs` uses a `mut` by-value parameter the parser does not support; `negative/move_field.rs` uses `String::from`; `unsupported/reference_field.rs` uses a nominal lifetime parameter.
