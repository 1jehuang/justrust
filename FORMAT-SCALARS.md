# Scalar `{}` formatting in print macros

Native milestone: `print!`, `println!`, `eprint!`, `eprintln!` with bare `{}`
arguments for all integer widths (including i128/u128 extremes), bool, char and
shared str. Arguments are evaluated once, left to right, into CFG snapshots
before any format bytes are written. Place arguments retain shared loans until
output, preventing later argument mutations from silently changing borrowed
values. Dynamic-index and temporary-rooted place arguments remain explicitly
unsupported. Escaped `{{` and `}}` remain literal braces.

The AST and typed IR retain the argument expressions. CFG format operands are
ordinary graph-owned values, visible to validation, ownership and borrow
visitors. Native x86-64 emission converts integer magnitudes by two-limb decimal
division, encodes scalar chars as UTF-8, and writes descriptors directly through
Linux syscalls, retrying interrupted/partial writes. No runtime host compiler,
external formatter, whole-program node or shared compiler state is introduced.

`tests/format_scalars.rs` checks the frozen 20-source stock corpus: seven positive
sources and the shared-str boundary now execute identically to stock, five
invalid sources reject, and the remaining seven boundaries reject explicitly.
The first full-program corpus source `01_hello.rs` now checks, builds and runs
with stdout equal to its frozen stock oracle, and is ratcheted to `pass`.
Backend tests additionally execute all 12 integer types at boundaries, Unicode
encoding boundaries, evaluation order, all four streams/newline combinations,
empty descriptors, failed writes and large writes.

This is not full Rust formatting or full Rust support. Debug/specifiers,
width/precision, positional/named fields, floats, arbitrary references and
user-defined Display remain unsupported. Primitive associated constants remain
a separate path-resolution gap. Full-program corpus entries beyond `01_hello`
still depend on other compiler features.
