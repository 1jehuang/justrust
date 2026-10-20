# Genuine usize and runtime array indexing

Status: implemented bounded native slice, validated by 647 hosted tests and strict all-target lint. Full Rust support remains incomplete.

The target remains Linux x86-64. usize is a genuine unsigned 64-bit scalar with
range 0 through u64::MAX, Copy semantics and full-width argument/local/result
transport. Decimal magnitude is retained before contextual typing. Explicit
usize literals and forward expected-type contexts do not convert i32 values.
Backward numeric inference remains an explicit unsupported boundary until a
numeric inference solver exists. Existing i32 signed 32-bit behavior stays intact.

Native usize add/sub use carry/borrow overflow checks, multiplication checks the
high product, division/remainder reject zero and use unsigned instructions, and
comparisons/bounds use unsigned 64-bit flags. Unary minus rejects; unary not is
full-width bitwise complement. Source arithmetic and builtin indexing failures
now use source-aware panic-abort diagnostics. See `RUNTIME-BOUNDS.md` for the
bounded indexing contract. Synthetic descriptor faults remain diagnostic 101;
unwinding remains unsupported.

Runtime indexing has no dynamic Place projection. Checked IR keeps literal
Index and ConstantIndex and adds DynamicIndex with expression index, plus a
local-rooted static-array AssignIndex. CFG adds IndexCopy, IndexAssign and
IndexInspect. Array type/length/element layout come from the validated static
array Place, not duplicated mutable metadata. Index operands must have usize
type. The array-length limit remains4096, but the runtime index accepts the
entire64-bit range and traps when index >= actual length before scale/address.

Reads evaluate base address/effects once, retain a live place rather than eagerly
copying the array, then evaluate the index once. Readiness and bounds checks run
at the atomic operation after index effects, so the index may initialize or
replace the source array. Earlier static indexed-base effects and initialization
checks run BEFORE the next dynamic index expression, retaining a live place
rather than snapshotting its subarray. Writes evaluate and snapshot the full RHS before LHS address/index effects. The
index operand is also snapshotted. The final write cannot observe later operand
source mutations. Abrupt transfers preserve effects without a reachable access.

Copy element reads, including Copy aggregates and unit, are admitted. Non-Copy
indexed moves remain rejected. Only direct ignored `let _ = a[i]` values use
non-consuming IndexInspect with readiness and runtime bounds checks; non-Copy
dynamic match is explicitly unsupported in this slice. Initial dynamic non-Copy
subfield/payload extraction and nested dynamic bases or assignment targets are explicitly
unsupported, not implemented through a guessed static child or parent move.

Ownership checks the exact initialized array subtree after ordered index effects.
Unknown-index replacement requires a mutable, initialized array, preserves init
bits and invalidates every contained enum selection proof. It never grants a
proof for a guessed child, restores missing initialization or introduces symbolic
index equality. Enclosing enum facts outside the array stay intact. Existing
bounded preflight, paths, fixed point, state bytes and diagnostics remain gates.

Validation must cover full-width high-bit values and transport, unsigned overflow,
index4294967296 rejection without truncation, zero-length bounds, base/index/RHS
side-effect ordering, index-initialized storage, snapshots, Copy aggregates,
non-Copy wildcard/replacement, invalid i32 indices, proof invalidation, structural
resource limits and preserved old corpora. Standard Rust is an external oracle,
not this compiler's implementation. Full Rust support remains incomplete.
