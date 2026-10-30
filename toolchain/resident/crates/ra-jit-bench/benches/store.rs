//! The store's hot paths: the key a function is kept under, a record of a pack, the check of a
//! sealed pack when it is first read, and a read of the store that finds a value or does not.
//!
//! Every benchmark does the same work on every run, so that tango can pair the binary of a
//! change with the binary of its base.

use std::hint::black_box;

use hir_ty_codegen::bench::entry_key;
use ra_jit_store::bench::{Salted, SealedStore, check_pack, encode_record, key_of};
use tango_bench::{IntoBenchmarks, benchmark_fn, tango_benchmarks};

/// A name as long as a mangled generic function's.
const NAME: &str = "_ZN4core4iter6traits8iterator8Iterator4fold17h0123456789abcdefE\
                    <alloc::vec::into_iter::IntoIter<u32>, usize, closure>";

/// What the store holds when the benchmarks read it: a thousand functions of 512 bytes.
const COUNT: usize = 1000;
const LEN: usize = 512;

fn benchmarks() -> impl IntoBenchmarks {
    [
        benchmark_fn("key/entry", |b| {
            let program = key_of(7);
            b.iter(move || entry_key(black_box(NAME), false, &program))
        }),
        benchmark_fn("key/salted", |b| {
            let salted = Salted::new(b"a build", "a toolchain");
            let key = key_of(1);
            b.iter(move || salted.key(black_box(&key)))
        }),
        benchmark_fn("pack/encode_record_1k", |b| {
            let key = key_of(2);
            let value = vec![0x5a; 1024];
            b.iter(move || encode_record(&key, black_box(&value)))
        }),
        benchmark_fn("pack/check_1000x512", |b| {
            let store = SealedStore::new("check", COUNT, LEN).expect("a store");
            b.iter(move || {
                let (name, bytes) = store.pack();
                check_pack(name, black_box(bytes))
            })
        }),
        benchmark_fn("store/get_hit", |b| {
            let store = SealedStore::new("hit", COUNT, LEN).expect("a store");
            let mut at = 0;
            b.iter(move || {
                at = (at + 1) % COUNT;
                let key = store.keys()[at];
                store.get(black_box(&key))
            })
        }),
        benchmark_fn("store/has_miss", |b| {
            let store = SealedStore::new("miss", COUNT, LEN).expect("a store");
            let key = key_of(u64::MAX);
            b.iter(move || store.has(black_box(&key)))
        }),
    ]
}

tango_benchmarks!(benchmarks());
