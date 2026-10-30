# What breaks ra-jit's codegen: a sweep of 93 crates and 7 binaries

Branch `ra-jit-breakers`, off `ra-jit-spike`. Everything below was measured on one 8-core, 31 GB
VM with `nightly-2026-05-10` (`harness/toolchain`). Each number is from a single run unless it
says otherwise, and each is followed by the command that printed it. "ra-jit-14" is
`cargo build -p ra-jit --release` at commit `4219d84566`, the end of the first pass;
"ra-jit-30" is the same at `1c293327c1`, the end of the second.

## Summary

| | before (as found) | first pass (ra-jit-14) | second pass (ra-jit-30) | command |
|---|---|---|---|---|
| crates with a result (the other 7 are under "Not tested") | | 86 of 93 | 86 of 93 | `cratesweep.sh ~/sweep/out15 $(cat versions.txt)` |
| crates whose tests behave as rustc's | not measured on the whole list, see below | 84 of 86 | 86 of 86 | `sweepreport.py ~/sweep/out15` |
| tests run and compared | | 10746 | 10746 | same |
| functions compiled by ra-jit | | 41597 of 41692 (95 left to rustc) | 41682 of 41697 (15 left, all generic-array) | same |
| binaries, command lines the same | 25 of 29 on the 3 that ran, 4 did not run | 68 of 68 over 7 binaries | 68 of 68 | `bintest.py <name>` |
| tokio `--features full`, tests differing | | 8 of 1018 | 0 of 1018 (2 of 3273 functions left) | `cratetest.py vendor/tokio --features full` |
| `harness/corpus` | 661 of 662, MATCH | 661 of 662, MATCH | 661 of 662, MATCH | `difftest.py corpus --bin runner --crate cases` |
| `harness/stress.py` | 99 of 99 | 99 of 99, 0 restarts | 99 of 99, 0 restarts | `stress.py` |
| reduced breakers | | 13 of 13 right, 3 open | 25 of 25 right, 0 open | `breakers.py` |

"Before" has no single number for the 93 crates: I stopped the first sweep with the
unmodified ra-jit after 29 crates, to restart it with the first fixes in. In those 29, ra-jit
crashed on futures, pin-project-lite and tokio and failed to link serde_json. Of those 29: 4965 tests, 9039 of 9539 functions, 7 crates failing
(`sweepreport.py ~/sweep/out1`). A full sweep with the first four fixes in gave 40986 of 41212
functions and 6 failing crates (`sweepreport.py ~/sweep/out2`); the final one is the table
above.

The second pass took the 95 functions and the two differing crates of the first pass one by
one; the rows "15 to 26" below are what came out of it. What is left is 15 functions of
generic-array, in one cause, and 2 of tokio's with `--features full`.

## On stable 1.99

The sweep again with `rustc 1.99.0` and the collated branch (`f296858ee3` plus the three fixes
below), one run, Cranelift: `RA_JIT_BIN=.. CLEAN=1 JOBS=4 harness/cratesweep.sh ~/sweep/out99
$(versions)`, then `sweepreport.py ~/sweep/out99`.

| | result |
|---|---|
| crates whose tests behave as rustc's | 86 of 86 |
| tests run and compared | 10745 |
| functions compiled by ra-jit, as the sweep ran | 41958 of 41987 (29 left to rustc) |
| the same after the fixes below, rerun on the four crates they touch | 15 left, all generic-array |

What the 29 were:

| cause | functions | crates | what was done |
|---|---|---|---|
| `spin_loop` reaches `_mm_pause`, an LLVM intrinsic with no body, on 1.99 | 5 | crossbeam-channel, futures-util | a one-instruction helper; corpus case `spin_hint` |
| A member of the workspace and the registry's copy of the same version (ahash, through hashbrown's dev-dependency) were given each other's features, so the copy named a dependency it does not have | 1 | ahash | features are keyed by the package's directory too |
| The test names nothing of the crate, so the crate is not linked, but the standard library's own `hashbrown` rlib made the harness think it was | 8 | hashbrown | the harness's check; not ra-jit's |
| The next trait solver on `GenericArray<T, N>` (as above) | 15 | generic-array | open |

crossbeam-channel's `tick` test binary differed once in the rerun with the same test results
printed and nothing from valgrind, and matched in three runs after; it measures intervals of
time.

tokio with `--features full` on 1.99 (`cratetest.py vendor/tokio --features full`): 155 test binaries, 1018 tests,
all matching, and no function left to rustc (296,088 of 296,088, counting a function once for each binary it is in).

## How it was run

- Libraries: `harness/cratesweep.sh <out> <crate-version>...` downloads each crate from
  crates.io and runs `harness/cratetest.py` on it, which builds every test binary of the
  crate with cargo, builds it again with ra-jit's object linked in front, runs both and
  compares the result of every test by name. `sweepreport.py <out>` adds up the JSON files.
  The final run: `RA_JIT_BIN=~/sweep/ra-jit-30 CLEAN=1 JOBS=4 harness/cratesweep.sh ~/sweep/out15
  $(cat ~/sweep/versions.txt)`, 60 minutes of wall time including base64's 1800 s timeout
  (`stat` of the log's birth and last write), with `breakers.py` just before it and nothing
  else on the machine.
- Binaries: `harness/bintest.py <name>` (new) clones the repository named in
  `harness/breakers/bins.json`, builds it with cargo and with ra-jit, runs each command line
  listed there with both and compares status, stdout and stderr.
- Reduced cases: `harness/breakers.py` (new) builds `harness/breakers/cases` with both and
  compares each case's output, then runs `cratetest.py` on the crates under
  `harness/breakers` that need a test of their own. It fails on any difference and on any
  function skipped that is not listed as open.

Two things in the harness itself had to be fixed before the numbers were right, and are in
the branch: `cratetest.py` counted a test binary that does not link the crate at all as the
crate's failure, and integration tests were loaded without dev-dependencies (below).

## The list, and why each is on it

| stress | crates |
|---|---|
| generics, trait objects, iterator adaptors | serde_json, itertools, nalgebra, either, anyhow, thiserror, petgraph, im, nom, winnow |
| const eval, const generics, type-level numbers | generic-array, typenum, arrayvec, heapless, bitvec, bitflags, hex-literal, static_assertions |
| unsafe, raw pointers, hand-rolled layouts | bumpalo, hashbrown, smallvec, slab, indexmap, tinyvec, bytes, triomphe, zerocopy, bytemuck, lru, smol_str |
| atomics, threads, lock-free | crossbeam-utils, crossbeam-queue, crossbeam-channel, crossbeam-epoch, arc-swap, once_cell, rayon, rayon-core, parking_lot, dashmap, thread_local, lazy_static |
| async, pinning | futures, futures-util, tokio, smol, async-channel, pin-project-lite |
| SIMD and CPU intrinsics | memchr, simd-json, aho-corasick, sha2, blake3, crc32fast, ahash, half |
| FFI, C layout, syscalls | memmap2, socket2, libc through most of the above |
| 128-bit integers, floats, formatting | num-bigint, num-traits, num-complex, ryu, rust_decimal, ordered-float, libm, fastrand, rand, rand_core, siphasher, fnv, byteorder |
| panics, unwinding, drop order | scopeguard, anyhow, crossbeam-queue (`drops`), rayon-core (`stack_overflow_crash`) |
| `dyn Any`, `TypeId` | downcast-rs, anyhow |
| text, tables, large statics | unicode-segmentation, unicode-width, unicode-normalization, regex-syntax, regex-automata, bstr, base64, percent-encoding, url, uuid, time, chrono, textwrap, strsim, smawk, camino, log |
| compression (tight loops, big frames) | adler2, miniz_oxide, flate2 |
| big binaries | ripgrep, fd, bat, just, hyperfine, zoxide, tokei |

Inline asm, thread locals, statics with interior pointers, closures and fn pointers are
already in `harness/corpus` (661 functions) and came up again only through the crates above.

## Binaries

| binary | command lines the same | functions from ra-jit | codegen ms |
|---|---|---|---|
| fd | 10 of 10 | 148 of 148 | 3329 |
| just | 12 of 12 | 822 of 822 | 8093 |
| hyperfine | 7 of 7 | 161 of 161 | 1767 |
| zoxide | 9 of 9 | 131 of 131 | 1381 |
| ripgrep | 12 of 12 | 1820 of 1820 | 4113 |
| bat | 10 of 10 | 321 of 321 | 5806 |
| tokei | 8 of 8 | 141 of 141 | 4870 |

`RA_JIT_BIN=~/sweep/ra-jit-30 harness/bintest.py <name> --json ~/sweep/bins30/<name>.json` for
each (`~/sweep/bins30.log`); the function counts and times are the sums of `compiled`,
`roots` and `codegen_ms` over the crates in that JSON. The counts are higher than with
ra-jit-14 (fd 143, just 553, ripgrep 1742, ...) because the second pass stopped taking generic
helpers in `const _` blocks for rustc's symbol, so more functions are ra-jit's to compile.

The first run with the unmodified ra-jit (`~/sweep/bins1.log`) gave fd 9 of 10, ripgrep 9 of
12, zoxide 9 of 9, hyperfine 7 of 7, and no result for just, tokei and bat. Two of those were
ra-jit: fd returned status 1 where rustc's build returns 130 (cause 7 below), and tokei had 0
functions compiled (cause 2). The rest was the new harness, which did not yet pick the right
binary of a package with several, and compared `argv[0]` in help texts. I did not separate
the two more finely than that.

## Root causes, by how many crates each touched

Fixed. Each has a commit and a case that fails without it (27 has the sweep's async-channel
entry instead). The first pass:

| # | cause | crates | effect before | reduced case | commit |
|---|---|---|---|---|---|
| 1 | Integration tests were loaded without dev-dependencies, so nothing from them resolved; and a macro that failed to expand at the end of a block was lowered as an empty block, so the function was compiled to return `()` | arc-swap (traced). crossbeam-queue, futures and pin-project-lite also failed in the first sweep and pass since; I did not trace those three one by one | arc-swap: 4 tests differ, and its `random` test binary segfaulted | `breakers/devdep` | `e1302a63a4` |
| 2 | A library and a binary, bench or test of the same name were taken for one crate | tokei, serde_json, ahash | 0 functions compiled; link failure; wrong `main` | `breakers/devdep` (`only_main`), tokei in `bins.json` | `414fa60d2f`, `c826da5e7e` |
| 3 | `format_args!` ends in `<Arguments>::new`, which is unstable; a stable `new` from a blanket trait in scope (yansi's `Paint`, through pretty_assertions) was picked instead | im (85 functions), any crate with pretty_assertions | every function that formats: `HasErrors` | `format_blanket_new` | `ffd6b4acd9` |
| 4 | `llvm.x86.pclmulqdq` (128, 256, 512) missing | crc32fast, simd-json | 10 functions left | `clmul` | `ea7d92397e` |
| 5 | `llvm.x86.xgetbv` missing | sha2, libm | 3 functions left | `xgetbv` | `1fbc416a11` |
| 6 | A method of `impl<T> Trait for T` in a crate rustc built no code of was named and compiled with nothing for `T` | downcast-rs, pin-project-lite | 10 functions reported as failures | `breakers/blanket` | `4219d84566` |
| 7 | Const eval read a niche-encoded enum's variant from the niche's start instead of the first niche variant | fd | wrong exit status | `niche_eq` | `99a4eebbd8` |
| 8 | `simd_cast`, `simd_as` missing | serde_json (zmij) | functions left | `simd_cast` | `1d53ea1572` |
| 9 | A large temporary and the binding it initializes each got a stack slot | rayon-core | `stack_overflow_crash` differs | `big_frame` | `52856e3d60` |
| 10 | `&[x; N]` of a constant was built on the stack instead of promoted | adler2 | 2 tests differ | `big_promoted` | `89ff27cf61` |
| 11 | In a macro, `1 as $ty << n` was parsed as the start of generic arguments | rand | 6 tests differ | `cast_then_shift` | `d74cf4ee84` |
| 12 | A macro call's location included the comments and attributes before it | simd-json | 3 tests differ (panic line numbers) | `macro_item_location` | `357e80d227` |
| 13 | Unsizing a struct whose last field is an associated type of a parameter | heapless | 22 functions left | `unsize_assoc_field` | `35715f72aa` |
| 14 | Const eval had no `bswap` or `bitreverse` | bitvec | 5 functions left | `const_bswap` | `35715f72aa` |

On 6: these methods are generic, they were never functions ra-jit could compile alone (11 of
them were listed, 10 failed and one happened to compile because it never touched `T`). The fix
stops counting them as roots, which is why the total number of functions went from 41703
(`sweepreport.py ~/sweep/out3`, the sweep before this fix) to 41692 while the number compiled
went up. Their instances are compiled where they are used, as before.

The second pass, in the order fixed. "crates" and "effect" are from the ra-jit-14 sweep
(`sweepreport.py ~/sweep/out6`) and the focused reruns of each crate noted in the commits.

| # | cause | crates | effect before | reduced case | commit |
|---|---|---|---|---|---|
| 15 | `llvm.x86.vcvtps2ph` (F16C) missing | half | 3 functions left | `f16c` | `ac5e2c9348` |
| 16 | Comparing two references to a struct that ends in a slice read each as one scalar; it is a pointer and a length | triomphe | 2 functions left | `unsized_tail_cmp` | `ac5e2c9348` |
| 17 | A generic helper in a `const _` block was taken for the function rustc has a symbol of, compiled with nothing for `T` and defined twice when two blocks shared a path | pin-project-lite | 9 functions left | `breakers/blanket` (`const_helpers`) | `ac5e2c9348` |
| 18 | A future had a field and a word of drop flags for every local held across an await; it is laid out by rustc_abi's coroutine layout now, a variant per await, locals overlapping, flags as many bytes as they need | futures (4 tests), tokio `full` (8 tests) | size assertions differ | `future_size`, `future_states` | `ce3225e7c4`, `4a5fa659af` |
| 19 | `safe` is a keyword only before `fn` and `static` in an extern block, and was taken for one before `!` too | bitvec | 21 functions left | `safe_macro` | `0ff9bec81b` |
| 20 | A macro written by a macro can have several parameters of one spelling from different expansion steps; bindings were kept by name alone | petgraph (`defmac!`) | 3 functions left | `macro_same_name_vars` | `de38e5c594` |
| 21 | An impl in a named `const` block (proptest-derive) was not an impl of the crate; and the impls of a dev-dependency nothing names were known where rustc never loads it (criterion's serde_json made `assert_eq!(1.5, s.parse().unwrap())` ambiguous) | im (6), ryu (2), itertools (9) | 17 functions left | `named_const_impl`, `breakers/devdep` (`unloaded`) | `efa3750e9c` |
| 22 | A projection in a path's type (`OVector<T, D>::from_element`) was not known to be of a trait its self type implements by the where clause | nalgebra | 3 functions left | `path_where_clause` | `f9e0547f20` |
| 23 | An impl in a function body of the workspace was an impl of its crate only where its type or trait was declared in the same block | zerocopy | 1 function left (`into` with no body) | `impl_in_fn` | `dd79e2f4ec` |
| 24 | An `$e:expr`, `$t:ty` or `$p:path` passed on to another macro was spliced as its tokens, so `0` matched a literal rule and a type of several tokens was not one `$x:tt` | bitvec | 20 functions left | `fragment_token` | `ac42999014` |
| 25 | A blanket impl in a function was given to the solver twice, through the block and through the crate: ambiguous, and the associated const fell back to the trait's declaration | zerocopy (`trailing_slice_layout`) | const eval `IncompleteExpr` | `blanket_in_fn` | `f395f5f19d` |
| 26 | A function generic only over lifetimes was compiled by ra-jit instead of calling rustc's copy, with a frame several times the size: `Debug` for a nested value overflowed a 2 MiB stack | simd-json (1 test) | `limit_fits_in_a_default_thread_stack` aborts | `lifetime_only_callee` | `40992f988e` |
| 27 | An instance that names types of two crates is compiled from the one that sees the other; "sees" was every transitive dependency, where the impls known are those of the crates the source names (as 21) | async-channel | 5 tests' functions left (`poll` with no body) | none reduced, see below | `1c293327c1` |

On 27: a reduction in `breakers/devdep` (a dev-dependency's generic `block_on` polling the
crate's future) compiled fine before the fix, so there is no case for it; async-channel's
tests in the sweep are the regression test (`async_channel 54/54` in `~/sweep/out15`).

Open. 15 functions in one crate in the sweep, and 2 in tokio with `full` (above).

| cause | functions | crates | reduced | difficulty |
|---|---|---|---|---|
| The next trait solver does not resolve `into_array`/`fold` on `GenericArray<T, N>` with `N` a typenum (`UnresolvedMethod`, 13) or normalize `IntoIter`/`GenericArray<i32, _>` (`HasPlaceholder`, 2) | 15 | generic-array | no | Open-ended. rustc itself with `RUSTFLAGS=-Znext-solver cargo test --no-run` on generic-array fails with `E0275` overflow evaluating `<N as ArrayLength>`, so at least part of this is the next solver's, which rust-analyzer shares. Each reduction I tried (a `GenericArray<u8, U4>` built by `from_array`, `into_array` on it, `fold` over it) compiled fine on its own. Days, upstream. |

## Not tested, and why

| crate | reason | checked with |
|---|---|---|
| bstr | cargo cannot build the tests: `src/unicode/data/*Test.txt` are not in the published crate | `cargo test --no-run` in `harness/vendor/bstr` |
| blake3 | same: `reference_impl` is not in the published crate | `cargo test --no-run` |
| rust_decimal | a dev-dependency needs the MySQL client library (`mysqlclient-sys` build script fails) | `cargo test --no-run` |
| smol | `rustix` does not compile on this nightly (`rustc_layout_scalar_valid_range_*` attributes) | `cargo test --no-run` |
| tokio | the tests do not build with default features | `cargo test --no-run`; see below for `--features full` |
| static_assertions | has no functions and no tests that run | sweep log: 1 test binary, 0 tests |
| base64 (unit tests only) | did not finish in the 1800 s the sweep allows; its integration tests are the same (107 of 107 functions) | see below |

tokio with `--features full` (`RA_JIT_BIN=~/sweep/ra-jit-30 cratetest.py harness/vendor/tokio
--features full --json ~/sweep/tokio-full30.json`, one run, after the sweep): MATCH, 155 test
binaries, 1018 tests, none differing (8 differed with ra-jit-14, all on the size of a future),
3271 of 3273 functions from ra-jit (`sweepreport.py` on that JSON). The 2 functions left are
`io_join::method_delegation` and `io_split::method_delegation` (`layout of Runtime:
HasErrorType`: rust-analyzer does not infer a type in `Runtime` here; not reduced). 18
further test binaries, the library's unit tests among them, do not build with cargo under
`full` alone (they want `test-util`), so tokio's unit tests are not covered. These numbers
are not in the totals above.

base64: the sweep reports it as failed. Its two integration test binaries match; the unit
test binary ran past the 1800 s limit of `cratetest.py` and was killed, so it has no result.
I saw its tests passing while it ran but did not find out which build is the slow one or
whether it would finish. It is counted among the 7 without a result, not among the 86.

## Every crate

`harness/sweepreport.py ~/sweep/out15 --md`. "codegen ms" is the time ra-jit spent in codegen
over the crate's test binaries, not wall time.

| crate | result | test binaries | tests | functions from ra-jit | codegen ms | left to rustc, or differing |
|---|---|---|---|---|---|---|
| adler2 | same | 1 | 6 | 23/23 | 683 |  |
| ahash | same | 3 | 38 | 185/185 | 2824 |  |
| aho_corasick | same | 1 | 163 | 856/856 | 2675 |  |
| anyhow | same | 15 | 73 | 215/215 | 27402 |  |
| arc_swap | same | 4 | 52 | 123/123 | 15245 |  |
| arrayvec | same | 4 | 52 | 66/66 | 1345 |  |
| async_channel | same | 3 | 41 | 54/54 | 2039 |  |
| bitflags | same | 1 | 54 | 824/824 | 2287 |  |
| bitvec | same | 1 | 141 | 264/264 | 7635 |  |
| bumpalo | same | 1 | 5 | 33/33 | 465 |  |
| bytemuck | same | 10 | 33 | 73/73 | 3310 |  |
| byteorder | same | 1 | 680 | 1140/1140 | 1932 |  |
| bytes | same | 13 | 1055 | 1561/1561 | 11484 |  |
| camino | same | 2 | 9 | 713/713 | 1890 |  |
| chrono | same | 4 | 291 | 1847/1847 | 6204 |  |
| crc32fast | same | 2 | 12 | 108/108 | 2928 |  |
| crossbeam_channel | same | 15 | 391 | 628/628 | 21475 |  |
| crossbeam_epoch | same | 2 | 40 | 128/128 | 1648 |  |
| crossbeam_queue | same | 3 | 23 | 29/29 | 2075 |  |
| crossbeam_utils | same | 7 | 62 | 423/423 | 14919 |  |
| dashmap | same | 1 | 20 | 44/44 | 852 |  |
| downcast_rs | same | 3 | 32 | 152/152 | 2210 |  |
| either | same | 1 | 8 | 13/13 | 1482 |  |
| fastrand | same | 3 | 32 | 77/77 | 3144 |  |
| flate2 | same | 6 | 80 | 369/369 | 4363 |  |
| fnv | same | 1 | 1 | 8/8 | 472 |  |
| futures_util | same | 1 | 1 | 50/50 | 3675 |  |
| futures | same | 31 | 84 | 106/106 | 16778 |  |
| generic_array | same | 1 | 3 | 4/19 | 4500 | 13 mir: UnresolvedMethod; 1 layout of Alias; 1 layout of GenericArray<i32, |
| half | same | 1 | 45 | 301/301 | 1503 |  |
| hashbrown | same | 7 | 113 | 406/406 | 7122 |  |
| heapless | same | 3 | 227 | 431/431 | 5228 |  |
| hex_literal | same | 2 | 8 | 11/11 | 1052 |  |
| im | same | 1 | 106 | 124/124 | 11933 |  |
| indexmap | same | 5 | 183 | 281/281 | 9123 |  |
| itertools | same | 14 | 450 | 809/809 | 27692 |  |
| lazy_static | same | 3 | 1 | 1/1 | 909 |  |
| libm | same | 1 | 116 | 1200/1200 | 2920 |  |
| log | same | 3 | 33 | 243/243 | 2138 |  |
| lru | same | 1 | 59 | 68/68 | 791 |  |
| memchr | same | 1 | 142 | 718/718 | 3263 |  |
| memmap2 | same | 1 | 27 | 129/129 | 1158 |  |
| miniz_oxide | same | 1 | 29 | 237/237 | 1232 |  |
| nalgebra | same | 1 | 21 | 70/70 | 11676 |  |
| nom | same | 16 | 268 | 899/899 | 27030 |  |
| num_bigint | same | 9 | 168 | 3733/3733 | 11579 |  |
| num_complex | same | 1 | 92 | 401/401 | 1456 |  |
| num_traits | same | 2 | 67 | 3985/3985 | 4169 |  |
| once_cell | same | 1 | 5 | 25/25 | 1388 |  |
| ordered_float | same | 3 | 72 | 369/369 | 3265 |  |
| parking_lot | same | 3 | 91 | 325/325 | 2562 |  |
| percent_encoding | same | 1 | 18 | 51/51 | 606 |  |
| petgraph | same | 24 | 346 | 830/830 | 31381 |  |
| pin_project_lite | same | 4 | 21 | 81/81 | 4578 |  |
| rand | same | 1 | 104 | 298/298 | 1913 |  |
| rand_core | same | 4 | 11 | 36/36 | 1521 |  |
| rayon_core | same | 8 | 112 | 481/481 | 11188 |  |
| rayon | same | 17 | 333 | 700/700 | 49835 |  |
| regex_automata | same | 1 | 204 | 2265/2265 | 3403 |  |
| regex_syntax | same | 1 | 147 | 985/985 | 3614 |  |
| ryu | same | 9 | 47 | 169/169 | 4363 |  |
| scopeguard | same | 1 | 8 | 14/14 | 423 |  |
| serde_json | same | 8 | 144 | 765/765 | 19653 |  |
| sha2 | same | 2 | 14 | 165/165 | 4165 |  |
| simd_json | same | 8 | 473 | 1895/1895 | 25872 |  |
| siphasher | same | 1 | 13 | 148/148 | 701 |  |
| slab | same | 3 | 47 | 102/102 | 3297 |  |
| smallvec | same | 1 | 63 | 81/81 | 1209 |  |
| smawk | same | 5 | 21 | 23/23 | 4257 |  |
| smol_str | same | 3 | 26 | 162/162 | 3379 |  |
| socket2 | same | 1 | 10 | 267/267 | 1410 |  |
| strsim | same | 2 | 96 | 133/133 | 1448 |  |
| textwrap | same | 3 | 163 | 312/312 | 8134 |  |
| thiserror | same | 13 | 56 | 262/262 | 19494 |  |
| thread_local | same | 1 | 9 | 26/26 | 727 |  |
| time | same | 1 | 1 | 845/845 | 1071 |  |
| tinyvec | same | 2 | 19 | 25/25 | 2055 |  |
| triomphe | same | 1 | 63 | 79/79 | 972 |  |
| typenum | same | 2 | 1762 | 2089/2089 | 7670 |  |
| unicode_normalization | same | 1 | 13 | 62/62 | 2050 |  |
| unicode_segmentation | same | 2 | 23 | 351/351 | 3234 |  |
| unicode_width | same | 2 | 35 | 107/107 | 2738 |  |
| url | same | 2 | 67 | 487/487 | 4744 |  |
| uuid | same | 1 | 62 | 323/323 | 1672 |  |
| winnow | same | 1 | 270 | 833/833 | 10026 |  |
| zerocopy | same | 3 | 140 | 1348/1348 | 13558 |  |

## Checks

At `1c293327c1` unless it says otherwise.

- `cargo fmt --all -- --check`: clean.
- `cargo clippy -p hir-ty -p ra-jit --all-targets -- --cap-lints warn`: exit 0, with the
  warnings the base branch already has (deprecated `*_imm` Cranelift builders, `eprintln!`,
  elidable `'db`); I did not diff the whole warning list against the base.
- `cargo xtask tidy`: exit 0.
- `cargo test -p <crate>` at `40992f988e` (`~/sweep/tests29.log`; the one commit after
  touches only `codegen/home.rs`): hir-ty 4, mbe 7 + 33, hir-def 511, syntax-bridge 9,
  span 9, hir 0, ide 1342, ide-completion 764, ide-diagnostics 744, parser 330 passed,
  0 failed. `tt`'s unit tests do not compile (`SyntaxContext::is_root` is not a method on
  this branch), and do not on `ra-jit-spike` either (checked in a worktree of it): not this
  branch's.
- `harness/breakers.py` with ra-jit-30: 25 of 25 right, 0 open, 0 functions skipped
  (`~/sweep/breakers30.log`).
- `harness/difftest.py corpus --bin runner --crate cases` with ra-jit-30:
  661 of 662 functions, MATCH. The one left is the `TypeId` of a function pointer type rustc
  has no symbol for, as on the base branch.
- `harness/stress.py` with ra-jit-30: 99 of 99 right, 0 restarts (`~/sweep/stress30.log`).

## Unverified or open

- The dx benchmark (1839 of 1839 functions) was not rerun on this branch. The Dioxus target
  directory was wiped for disk space, and a rebuild did not fit beside the sweeps. Causes 2,
  3, 6, 17, 21 and 27 change which functions ra-jit names, which impls it knows and which
  crate it compiles an instance from, so that number should be rechecked before this is
  merged anywhere.
- Cause 27 has no reduced case: the reduction I wrote compiled before the fix too. The
  async-channel sweep entry is its only regression test.
- Timing: only ra-jit's codegen time per crate was recorded. I did not measure ra-jit's wall
  time against cargo's per crate.
- The crate versions are the ones listed at the end of this report. The binaries are the
  default branch of each repository as cloned during the session, not pinned to a commit.
- macOS and aarch64 were not run. `xgetbv`, `clmul` and `f16c` are x86-64 only and return a
  constant elsewhere.
## Versions

serde_json-1.0.154 itertools-0.15.0 nalgebra-0.35.0 either-1.19.0 anyhow-1.0.104
thiserror-2.0.21 generic-array-1.4.5 typenum-1.20.1 arrayvec-0.7.8 heapless-0.9.3
bumpalo-3.20.3 hashbrown-0.17.1 smallvec-1.16.3 crossbeam-utils-0.8.23 crossbeam-queue-0.3.14
crossbeam-channel-0.5.17 crossbeam-epoch-0.9.21 slab-0.4.12 indexmap-2.14.2 tinyvec-1.13.3
bytes-1.12.1 arc-swap-1.9.2 once_cell-1.21.4 rayon-1.12.0 rayon-core-1.13.0 parking_lot-0.12.5
futures-util-0.3.34 futures-0.3.34 tokio-1.53.3 smol-2.0.2 async-channel-2.5.0 pin-project-
lite-0.2.17 memchr-2.8.3 bytemuck-1.25.2 simd-json-0.18.1 aho-corasick-1.1.5 sha2-0.11.1
blake3-1.8.7 crc32fast-1.5.2 memmap2-0.9.11 socket2-0.6.5 num-bigint-0.5.1 ryu-1.0.23
rust_decimal-1.43.0 num-traits-0.2.19 fastrand-2.5.0 scopeguard-1.2.0 downcast-rs-2.0.2
thread_local-1.1.10 unicode-segmentation-1.13.3 regex-syntax-0.8.11 regex-automata-0.4.18
bstr-1.13.1 smol_str-0.3.6 base64-0.23.1 percent-encoding-2.3.2 url-2.5.8 uuid-1.28.0
time-0.3.55 chrono-0.4.45 nom-8.0.0 winnow-1.0.4 log-0.4.34 bitflags-2.13.2 textwrap-0.16.4
strsim-0.11.1 fnv-1.0.7 ahash-0.8.12 byteorder-1.5.0 static_assertions-1.1.0 lazy_static-1.5.1
bitvec-1.1.1 petgraph-0.8.3 im-15.1.0 ordered-float-5.5.0 half-2.7.1 num-complex-0.4.6
libm-0.2.16 adler2-2.0.1 miniz_oxide-0.9.1 flate2-1.1.10 lru-0.18.5 dashmap-6.2.1
triomphe-0.1.17 zerocopy-0.8.63 rand-0.10.3 rand_core-0.10.1 siphasher-1.0.4 hex-literal-1.1.0
unicode-width-0.2.2 smawk-0.3.3 unicode-normalization-0.1.25 camino-1.2.6
