# Native heap foundation

First bounded Linux x86-64 increment, 2026-10-18. This is not Box, Vec, String,
std::alloc compatibility, general Rust layout, or full Rust support.

## Primitive boundary

`unsafe { __justrust_linux_syscall6!(number, a, b, c, d, e, f) }` accepts exactly
seven `usize` expressions and returns the exact raw kernel word as `usize`.
The syscall number is followed by six Linux x86-64 ABI arguments. Arguments
are evaluated once, left to right, before the operation. The backend stages
words and loads rax/rdi/rsi/rdx/r10/r8/r9, accounting for rcx/r11 clobbers.
Negative errno is not translated into libc errno or an allocator result.
Lexical/module macro declarations shadow the primitive through ordinary checked
builtin selection. Parentheses are the only primitive delimiter in this slice.
Unsafe is required even in an unused function. Wrong arity and non-usize
arguments reject before any executable is published. Divergent arguments without
an exact usize type are outside this primitive's bounded signature.

The source expression, typed expression and CFG rvalue retain every operand.
Source name resolution, canonical call inventory, cloning and recipe inventory
walk operands. IR and CFG validation check arity and exact types, including
unused IR rows. CFG preflight, lowering, operand visits, ownership and borrow
analysis retain the ordinary graph-native facts. No node kinds, shared state,
side slots or graph-integrity ratchets were added or loosened.

## Ordinary Rust policy

`libraries/prelude/native_heap.rs` is embedded as the ordinary per-file SOURCE
input `<prelude>/__justrust_std/native_heap.rs`. Its explicit bootstrap namespace
is `std::native_heap` (also reachable through the core alias), not a replacement
for Rust's allocator API. All allocation, layout and error policy is Rust source:

- `null<T>()` and `is_null<T>(*const T)` use existing raw-pointer casts.
- `byte_add`/`byte_sub` operate on `*mut u8`, checking address arithmetic for
  unsigned overflow/underflow and returning null on failure. They do not prove
  allocation bounds, alignment, provenance or dereference validity. They are
  not `ptr::add`/`offset` method implementations and do not scale by a type layout.
- `valid_layout` requires nonzero size, size at most isize::MAX, and nonzero
  power-of-two alignment at most 4096. Larger alignment is an explicit unsupported
  layout and returns false, not an incorrectly aligned allocation.
- `allocate` uses anonymous private read/write mmap. Linux x86-64 page alignment
  satisfies the admitted alignments. Invalid layouts and raw kernel errors
  produce an empty null handle with zero length. Successful mappings are zeroed.
- `Allocation` has private address/length and an ordinary canonical source
  `impl Drop`. Canonical Copy/Drop exclusion makes it non-Copy, without a
  compiler special case or a private marker. `pointer` and `len` inspect a
  shared handle. `release` consumes it and reports munmap success. Explicit
  release and automatic normal-path destruction call the same source helper,
  which clears address/length before attempting munmap, preventing double free.
  Empty handles succeed without IO. Normal scope exit, moves, early returns,
  conditional ownership and replacement reuse canonical cleanup flags.

Only the actual syscall ABI is backend-specific. There are no compiler cases
for allocation functions, Allocation, Box, Vec, String or their methods.
Allocation ownership is ordinary checked nominal ownership and field privacy.
The compiler's frozen closed-root inventory excludes native library functions,
matching the instance planner. This fixes demanded generic library calls being
assigned an unrelated eager-library body/symbol.

## Memory transport

Dereferenced scalar loads/stores use 1/2/4/8-byte memory accesses for integer,
bool, char and float widths, with existing signed normalization. References
reborrowed from pointers use the same widths. This does not change private
local carrier slots or establish Rust aggregate layouts. 128-bit values retain
the existing two-limb transport. Before this increment a u8 access at the last
byte of a mapping read/wrote a qword and could fault or corrupt adjacent bytes.
The actual mapping-boundary regression caught that bug, rather than a synthetic
backend byte fixture.

General public aggregate layout, size_of/align_of, raw pointer methods,
over-aligned and zero-sized allocation, realloc, allocator traits, generic and
aggregate Drop glue and owning collections remain follow-up work. Existing
private aggregate layout limits still apply. SCC-born loans live across a loop
backedge remain an explicit unsupported boundary; the cleanup conflict control
requires a real BorrowConflict as well, not just that boundary. Unsafe users
remain responsible for valid addresses,
initialized scalar values, alignment and live allocation bounds.

## Validation

`tests/heap.rs` runs real native executables at workers 1 and 8 and checks
check/build agreement. Positive differential programs exercise:

1. mmap, zeroing, aligned u64 write/read, first/interior/last byte operations,
   consuming free, and an independent mincore ENOMEM check proving unmapping.
2. Invalid layouts, supported-layout kernel failure and exact raw ENOSYS.
3. Generic null tests and checked byte arithmetic on actual u64 array storage,
   including overflow and underflow without dereferencing invented addresses.
4. Once-only left-to-right evaluation of all seven syscall operands.
5. Adjacent signed/unsigned narrow scalars, bool, Unicode char, floats and u128,
   sentinel-byte preservation and 1/2/4-byte accesses ending at a page boundary.
6. A legitimate scalar-field Copy/Clone implementation, ensuring the owning
   handle's canonical Drop does not blanket-reject ordinary Copy types.
7. Real mapping presence before moves and absence after normal/early-return
   cleanup, plus preserved consuming explicit release.
8. Ordinary shared/mutable owner borrows and legal Copy field reads.
9. A raw-pointer-field source destructor counting exact-once calls across moves,
   conditional moves, loop continue, replacement and self-move.

A panic=abort native/stock control requires no destructor marker during abort.
Escaping owner borrows and replacement with later-use loans reject in both
compilers. Nested/partial-move owner shapes and Drop-carrying generic functions
remain explicit unsupported boundaries, not silent no-op destruction.

The stock oracle compiles the same Rust policy and program with a test-only
inline-assembly syscall adapter, at stock opt-level 0 and 2. That adapter is
never input to the native compiler. `JUSTRUST_HEAP_STOCK_REQUIRED=1` makes a missing
stock compiler a failure instead of silently counting skipped oracle cells.
Local oracle runs are necessary because hosted lacks /usr/bin/rustc.

Negative controls cover unsafe admission in used/unused bodies, primitive
signature, private handle forgery, double release and a user Copy implementation
(stock E0184). `tests/native_heap_authority.rs` checks ten used/unused malformed
IR variants and two malformed CFG variants. `tests/graph_integrity.rs` remains
a mandatory unchanged gate. Validation receipts are recorded in FINDINGS.md.
No compile-time speedup claim is made.
