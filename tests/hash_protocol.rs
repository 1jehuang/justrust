//! std-shaped Hash/Hasher protocol: generic `Hash::hash<H: Hasher>`, provided
//! `write_*` methods over `write(&[u8])`, and derived `Hash` generic over the
//! hasher. Hash VALUES of std's DefaultHasher are unspecified, so every run
//! case prints the byte stream a user `Hasher` records, which stock and native
//! must agree on exactly.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

const RECORDER: &str = "use std::hash::{Hash,Hasher};struct Rec{bytes:Vec<u8>}impl Hasher for Rec{fn finish(&self)->u64{self.bytes.len() as u64}fn write(&mut self,b:&[u8]){let mut i=0;while i<b.len(){self.bytes.push(b[i]);i+=1;}}}fn show<T:Hash>(v:&T){let mut r=Rec{bytes:Vec::new()};v.hash(&mut r);let mut i=0;while i<r.bytes.len(){print!(\"{} \",r.bytes[i]);i+=1;}println!(\"| {}\",r.finish());}";

macro_rules! with_recorder {
    ($body:literal) => {
        concat!(
            "use std::hash::{Hash,Hasher};struct Rec{bytes:Vec<u8>}impl Hasher for Rec{fn finish(&self)->u64{self.bytes.len() as u64}fn write(&mut self,b:&[u8]){let mut i=0;while i<b.len(){self.bytes.push(b[i]);i+=1;}}}fn show<T:Hash>(v:&T){let mut r=Rec{bytes:Vec::new()};v.hash(&mut r);let mut i=0;while i<r.bytes.len(){print!(\"{} \",r.bytes[i]);i+=1;}println!(\"| {}\",r.finish());}",
            $body
        )
    };
}

#[test]
fn recorder_prefix_is_the_shared_harness() {
    assert!(with_recorder!("").starts_with(RECORDER));
}

#[test]
fn primitive_and_tuple_byte_streams_match_stock() {
    oracle::check(
        "hash_protocol_primitives",
        &[Case {
            name: "primitives",
            source: with_recorder!(
                "fn main(){show(&7u8);show(&258u16);show(&-2i32);show(&1u64);show(&true);show(&'A');show(&(1u8,2u16));}"
            ),
            expect: Expect::Run(
                "7 | 1\n2 1 | 2\n254 255 255 255 | 4\n1 0 0 0 0 0 0 0 | 8\n1 | 1\n65 0 0 0 | 4\n1 2 0 | 3\n",
            ),
        }],
    );
}

#[test]
fn derived_hash_is_generic_over_the_hasher() {
    oracle::check(
        "hash_protocol_derive",
        &[
            Case {
                name: "struct_and_enum",
                source: with_recorder!(
                    "#[derive(Hash)]struct P{x:u8,y:u16}#[derive(Hash)]enum E{A,B(u8),C{v:u8}}fn main(){show(&P{x:1,y:2});show(&E::A);show(&E::B(9));show(&E::C{v:3});}"
                ),
                expect: Expect::Run(
                    "1 2 0 | 3\n0 0 0 0 0 0 0 0 | 8\n1 0 0 0 0 0 0 0 9 | 9\n2 0 0 0 0 0 0 0 3 | 9\n",
                ),
            },
            Case {
                name: "generic_param_named_like_binder",
                source: with_recorder!(
                    "#[derive(Hash)]struct W<H>{h:H}fn main(){show(&W{h:5u8});}"
                ),
                expect: Expect::Run("5 | 1\n"),
            },
            Case {
                name: "manual_std_form_impl",
                source: with_recorder!(
                    "struct K(u8);impl Hash for K{fn hash<H:Hasher>(&self,state:&mut H){state.write_u8(self.0);state.write_u8(self.0);}}fn main(){show(&K(4));}"
                ),
                expect: Expect::Run("4 4 | 2\n"),
            },
        ],
    );
}

#[test]
fn provided_write_methods_dispatch_like_core() {
    // Records one entry per `write` call (its length) and marks calls that
    // reach the overridden write_u32/write_u64. Stock: each unsigned write_*
    // calls `write` once at native width; signed delegate to unsigned;
    // usize/isize never route through write_u64. Expected output is the
    // stock rustc result for this exact source.
    oracle::check(
        "hash_protocol_dispatch",
        &[Case {
            name: "write_dispatch",
            source: "use std::hash::{Hash,Hasher};struct D{log:Vec<u32>}impl Hasher for D{fn finish(&self)->u64{self.log.len() as u64}fn write(&mut self,b:&[u8]){self.log.push(b.len() as u32);}fn write_u32(&mut self,_i:u32){self.log.push(1000);}fn write_u64(&mut self,_i:u64){self.log.push(2000);}}fn show<T:Hash>(v:&T){let mut d=D{log:Vec::new()};v.hash(&mut d);let mut i=0;while i<d.log.len(){print!(\"{} \",d.log[i]);i+=1;}println!(\"| {}\",d.finish());}#[derive(Hash)]enum E{A,B(u8)}fn main(){show(&1u8);show(&1u16);show(&1u32);show(&1u64);show(&1u128);show(&1usize);show(&-1i32);show(&-1i64);show(&-1isize);show(&-1i128);show(&(1u16,2u32));show(&E::A);show(&E::B(3));show(&true);show(&'A');}",
            expect: Expect::Run(
                "1 | 1\n2 | 1\n1000 | 1\n2000 | 1\n16 | 1\n8 | 1\n1000 | 1\n2000 | 1\n8 | 1\n16 | 1\n2 1000 | 2\n8 | 1\n8 1 | 2\n1 | 1\n1000 | 1\n",
            ),
        }],
    );
}

#[test]
fn forwarded_open_method_actuals_keep_caller_environment() {
    // resolve_trait_call: a concrete-Self trait call whose method actual is
    // the caller's own type parameter (open) resolves in the caller's
    // environment; a fully closed call keeps the closed dispatch path; the
    // same forwarding without the required bound is still rejected.
    oracle::check(
        "trait_call_open_method_actuals",
        &[
            Case {
                name: "forwarded_open_and_closed_actuals",
                source: "trait Sink{fn put(&mut self,v:u32);}trait Emit{fn emit<S:Sink>(&self,s:&mut S);}struct V(u32);impl Emit for V{fn emit<S:Sink>(&self,s:&mut S){s.put(self.0);}}struct W{v:V}impl Emit for W{fn emit<S:Sink>(&self,s:&mut S){<V as Emit>::emit::<S>(&self.v,s);self.v.emit(s);}}struct Log(Vec<u32>);impl Sink for Log{fn put(&mut self,v:u32){self.0.push(v);}}fn main(){let mut l=Log(Vec::new());W{v:V(7)}.emit(&mut l);V(3).emit::<Log>(&mut l);println!(\"{:?}\",l.0);}",
                expect: Expect::Run("[7, 7, 3]\n"),
            },
            Case {
                name: "forwarded_actual_missing_bound",
                source: "trait Sink{fn put(&mut self,v:u32);}trait Emit{fn emit<S:Sink>(&self,s:&mut S);}struct V(u32);impl Emit for V{fn emit<S:Sink>(&self,s:&mut S){s.put(self.0);}}fn unused<S>(s:&mut S){V(1).emit(s);}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn non_std_hash_forms_reject() {
    oracle::check(
        "hash_protocol_invalid",
        &[
            Case {
                name: "non_generic_hash_method",
                source: "use std::hash::{Hash,Hasher};use std::hash::DefaultHasher;struct K;impl Hash for K{fn hash(&self,_s:&mut DefaultHasher){}}fn main(){}",
                expect: Expect::Invalid("E0049"),
            },
            Case {
                name: "derive_with_non_hash_field",
                source: "struct N;#[derive(Hash)]struct S{n:N}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
