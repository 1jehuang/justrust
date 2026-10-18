# Original core exponent estimator probe

`estimator.rs` is the complete unchanged original Rust module
`library/core/src/num/imp/flt2dec/estimator.rs`, not an extracted or rewritten
function. Its SHA256 is
`2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5`.

It comes from the same retained source import described in CORE-BRINGUP.md:
source inventory revision `d907fba494adcabfdd642a18a7918b0c9847d35b`, upstream
revision `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, source version `1.101.0`,
local import commit `8f0a1aeef27a0bb43b0e405c86e8a9dcd2c2448c`.
These source revisions are distinct from the installed stock compiler used as
an external oracle. Each executed acceptance run records that compiler's version.

Copyright belongs to The Rust Project Contributors. LICENSE-MIT,
LICENSE-APACHE and COPYRIGHT are retained beside the original module.

Acceptance uses an authored client with an ordinary external `mod estimator;`
declaration and compares the same provider/client files on stock and native.
No name-specific replacement implements `estimate_scaling_factor`.
The positive mathematical domain has nonzero mantissas. For zero, the original
`mant - 1` operation follows the configured overflow behavior rather than an
invented totalized approximation.

The original test specification permits the estimate or an estimate one less
than the ideal base-10 exponent. Exact stock/native differential comparisons
are additional implementation parity checks, not a replacement specification
requiring the ideal rounded exponent in all cases.

This is a stepping stone toward library support: it does not compile core's
original numeric method provider, the core crate root, standard libraries,
const evaluation, a library artifact or a general Cargo project. The scoped
contract and validation status are tracked in LEADING-ZEROS.md at repository root.
