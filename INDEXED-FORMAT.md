# Indexed native format operands

Status: implemented for the frozen scope. Stock-differential native, alias, malformed-operand and unchanged graph-integrity gates pass. Full Rust remains open.

Native scalar format arguments such as `println!("{}", slice[index])` must borrow the actual checked element place, not snapshot its value before the remaining arguments evaluate. Support constant/runtime indices into local-rooted shared/mutable arrays and slices, using the existing indexed-borrow authority. Evaluate each source index once, retain the ordinary loan, and preserve argument order. A mutable source permits a shared format borrow but does not grant new mutability. Never synthesize raw or static provenance.

Reuse the checked indexed-place helper and its local index temporaries. A checked shared-reference block may be a native format operand only when its referent is a supported native scalar. IR and CFG must agree on dereferencing that reference at formatting time. Existing library formatting, non-Copy element restrictions, call identity, runtime failure identity and unrelated format grammar remain unchanged.

Acceptance: frozen stock1.98.1 shared/mut array/slice and side-effect-index fixtures at workers1/8 CHECK/BUILD/run; alias-conflicting later operands rejected; non-Copy operands follow existing library formatting or explicit unsupported behavior without an accidental element move; forged nonreference/mutable-reference format operands rejected; unchanged graph-integrity gate. This is separate from mutable range admission.
