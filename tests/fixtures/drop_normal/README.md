# Normal-path Drop stock corpus

This corpus was frozen and executed with **stock `/usr/bin/rustc` before any native compiler invocation**. It is not a claim that the experimental compiler supports these programs.

## Authoritative receipt

`receipts-v2/manifest.json` freezes all 28 source SHA-256 values, source classifications, compiler binary SHA-256, `rustc -Vv`, and flags. Its SHA-256 is `f2bfbd2897313741381b6f7203e73bb8a981cb29640bf4f9af0f76c515b7dcb6`.

Freeze: `2026-10-18T10:07:42.781985+00:00`. Observation: `2026-10-18T10:07:43.795331+00:00`.

Compiler: rustc 1.98.1, commit `48a229ceaefd4985c50990b14116b6d856af0985`, x86_64-unknown-linux-gnu. Flags: `--edition=2024 -C panic=abort -A warnings`.

`receipts-v2/stock.json` stores compile commands/status/stdout/stderr and execution commands/status/stdout/stderr. Byte streams are base64, not normalized text. A second stock execution reproduced exact runtime stdout, stderr, and status for all legal cases. All 24 legal programs compiled and ran with status zero. Four invalid programs were rejected with status one.

`receipts-v1/` is the historical first 26-source freeze, before adding the two abrupt call-argument fixtures. No existing source changed between freezes. Use v2 for integration.

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

The X32 file is byte-for-byte equal to `Fixture X32.full_source` in `tests/canonical_copy.rs`, including final newline. Its measured full-source SHA-256 is `77d24a043ea0f6d3e99e19ac05d82b229b4c56985791c70a57ed246d0ba5c453`. The historical fixture declares `28fc4d7000d2464e5f7f11cfb3467592099b3de0b6e8da40432963667b7e5de6`, which is not the hash of those full-source bytes. X32 retains the historical scalar projection and destructor mutation unchanged. X32's output alone cannot prove that mutation ran, so the other marker-bearing destructor fixtures are required.

## Invalid Rust, not unsupported Rust

| Fixture | Stock diagnostic |
| --- | --- |
| negative/copy_drop.rs | E0184, Copy cannot coexist with a destructor |
| negative/move_field.rs | E0509, non-Copy field cannot be moved out of a Drop owner |
| negative/explicit_method.rs | E0040, explicit destructor call prohibited |
| negative/explicit_ufcs.rs | E0040, explicit destructor call prohibited |

The field-move negative deliberately uses a non-Copy String field, outside the scalar owner admission slice. It must not be confused with legal scalar Copy field reads. The native harness expects an explicit unsupported owner/type diagnostic for this one case, **not native E0509 conformance**. D17 remains partial: non-Copy field-move conformance is blocked by the excluded owner shape. Native invalid-Rust controls are the other three negatives only.

## Integration harness

`tests/drop_normal.rs` consumes `receipts-v2/fixture-index.tsv`, exported directly from the observed stock receipt by `python3 experiments/drop_normal_oracle.py export`. It pins the index, manifest, receipt, compiler, and each source hash. Before any native invocation it rechecks the entire stock corpus in metadata and executable modes, then runs a bounded child for native CLI and observed CHECK/BUILD with workers 1 and 8. Successful BUILD artifacts are executed and compared byte-for-byte. Successful and rejected observed results must have no structured resource failure or poisoned budget. CHECK must publish no executable or machine-code node. Each process is bounded, including the child containing in-process observed calls.

Run the target in both bootstrap debug and release profiles only after coordinator authorization. No harness compilation or native execution had occurred when the stock receipt was frozen. Adversarial cleanup authority tests remain separately owned.

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

These fixtures do not exhaust unsupported boundaries, notably destructuring, pattern temporary scopes, borrow-based temporary lifetime extension, indexed replacement, unwind behavior, or malicious proof mutation. See `DROP-NORMAL.md` for the larger implementation contract. Native support and adversarial proof validation remain separate gates.
