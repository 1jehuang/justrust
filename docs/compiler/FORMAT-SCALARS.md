# Native scalar Display and primitive structural Debug in print macros

Native milestone: `print!`, `println!`, `eprint!`, `eprintln!` with bare `{}`
arguments for all integer widths (including i128/u128 extremes), bool, char and
shared str. Arguments are evaluated once, left to right, into CFG snapshots
before any format bytes are written. Place arguments retain shared loans until
output, preventing later argument mutations from silently changing borrowed
values. Dynamic-index and temporary-rooted place arguments remain explicitly
unsupported. Escaped `{{` and `}}` remain literal braces.

Bare `{:?}` additionally supports bool, every integer width, unit, and recursive
tuples/fixed arrays containing only those types. Singleton tuples retain their
trailing comma. Empty arrays still validate their element's capability. These
aggregates are structurally Copy, and place arguments keep real shared loans
rather than acquiring implicit movement or user-defined formatting permissions.
Mixed Display/Debug modes are retained per argument through source plans, IR and
CFG, with one immutable admissibility function and exact arity validation.
This is an explicit native primitive operation, not canonical `std::fmt` trait
dispatch. A user trait named `Debug` grants no capability.

The AST and typed IR retain the argument expressions. CFG format operands are
ordinary graph-owned values, visible to validation, ownership and borrow
visitors. Native x86-64 emission converts integer magnitudes by two-limb decimal
division, encodes scalar chars as UTF-8, and writes descriptors directly through
Linux syscalls, retrying interrupted/partial writes. No runtime host compiler,
external formatter, whole-program node or shared compiler state is introduced.

`tests/format_scalars.rs` checks the frozen 20-source stock corpus: seven positive
sources plus shared-str and primitive-Debug boundaries now execute identically
to stock, five invalid sources reject, and six boundaries reject explicitly.
The first full-program corpus source `01_hello.rs` now checks, builds and runs
with stdout equal to its frozen stock oracle, and is ratcheted to `pass`.
Backend tests additionally execute all 12 integer types at boundaries, Unicode
encoding boundaries, evaluation order, all four streams/newline combinations,
empty descriptors, failed writes and large writes.

`tests/format_debug.rs` compares all integer widths, structural nesting, unit,
empty arrays, evaluation order, Copy reuse, streams and workers 1/8 against the
stock compiler. It also proves stock accepts char/string Debug and floating
Display/Debug while native compilation rejects them explicitly. Authority tests
reject malformed modes/counts, illegal types even in unused bodies and unused IR
rows, and preserve immutable retained-source modes after candidate edits.

This is not full Rust formatting or full Rust support. Char/string Debug still
needs exact escaping and Unicode printable semantics. Float formatting needs a
native correctly rounded shortest-decimal conversion including subnormals,
signed zero, infinities and NaN. Width/precision, alternate/pretty Debug,
positional/named fields, arbitrary references, nominal/derived/user formatting,
and source-backed `std::fmt` trait dispatch remain separate unsupported work.
No host compiler or external formatter participates in native runtime output.
