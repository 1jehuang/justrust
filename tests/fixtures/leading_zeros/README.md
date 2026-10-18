# Frozen leading-zero corpus

Generated and independently classified with the Python standard-library runner:

```sh
python3 experiments/leading_zeros_oracle.py \
  --output-dir NEW_STOCK_RECEIPT_DIRECTORY \
  --freeze-dir NEW_FROZEN_CORPUS_DIRECTORY
```

Both directories must not exist. The runner invokes `/usr/bin/rustc -Vv`, stock
metadata checking, stock executable compilation, and every successful executable.
The full raw receipt and exact source files remain in the output directory.
`stock-receipt.json` is the same evidence with its output-directory prefix replaced
by `$OUTPUT`. Commands, complete outputs, exit statuses, source hashes and output
hashes are retained. It does not need the original machine's scratch directory.

`manifest.tsv` columns are case name, semantic category, observed stock
classification, and SHA256 of the exact `.rs` bytes. `.stdout` files contain the
complete expected executable stdout. Reformatting sources changes this corpus and
requires new stock classification. Native test code is deliberately separate.

The corpus has 30 cases: 20 valid, 8 invalid, and 2 stock-legal boundaries that are
not asserted invalid by native tests. It assumes a 64-bit Linux x86-64 target.
Each of the twelve primitive integer types gets its own compact source. Expected
counts are independently calculated by Python integer bit length on the declared
width's bit pattern. Extra sources exhaust all 256 unsigned/signed byte patterns
using a shift-count oracle, visit all 128 single-bit positions, nest method calls,
exercise a once-only effectful receiver and a divergent receiver, and preserve an
ordinary nominal method with the same spelling and a different return type.

The estimator client imports the separately retained original provider as a real
external module. Its nine point checks and seven sweeps over exponents -1074
through 1024 compare the original approximation with exact integer arithmetic
using independently computed `(mantissa - 1).bit_length()`. They do not replace the
provider with a rewritten function, claim exact decimal logarithms, or include
zero mantissas outside its intended domain. Original provider SHA256:
`2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5`.

`tests/leading_zeros.rs` reclassifies every frozen case with stock before spawning
any native checks. It tests public CLI CHECK/BUILD plus observed compilation at
workers 1 and 8, executes both CLI-produced and observed executable bytes, and
compares complete stdout, stderr and exit status. Public text compilation is used
for standalone cases, and public file compilation for the unmodified external
estimator module. Children are bounded and terminated on timeout. Invalid native
cases must have
receiver/method or type-mismatch diagnostics, never resource/infrastructure errors,
and the observed public resource ledger must show no budget failure. The test needs
`/usr/bin/rustc` and `sha256sum` on the test host, and does not download a toolchain.
