//! Canonical per-place environments preserve disjoint field ownership.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};
#[test]
fn disjoint_capture_places_match_stock() {
    oracle::check(
        "closure_places",
        &[
            Case {
                name: "shared_field",
                source: "struct P{a:i32,b:i32}fn main(){let mut p=P{a:7,b:1};let f=||p.a;p.b=9;assert_eq!(f(),7);assert_eq!(p.b,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "independent_mutable_fields",
                source: "struct P{a:i32,b:i32}fn main(){let mut p=P{a:1,b:2};let mut f=||{p.a+=1;p.a};let mut g=||{p.b+=2;p.b};assert_eq!(f(),2);assert_eq!(g(),4);assert_eq!(f(),3);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mixed_field_modes",
                source: "struct P{a:i32,b:i32,c:i32}fn main(){let mut p=P{a:1,b:2,c:3};let mut f=||{p.a+=1;p.a+p.b};p.c=8;assert_eq!(f(),4);assert_eq!(f(),5);assert_eq!(p.c,8);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "nested_field",
                source: "struct P{a:i32,b:i32}struct Q{p:P,n:i32}fn main(){let mut q=Q{p:P{a:7,b:2},n:1};let f=||q.p.a;q.p.b=8;q.n=9;assert_eq!(f(),7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "move_field_remainder",
                source: "struct Item{i:i32}struct P{a:Item,b:Item}fn main(){let p=P{a:Item{i:7},b:Item{i:9}};let f=move ||p.a;let b=p.b;let a=f();assert_eq!(a.i+b.i,16);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "same_mutable_field_conflicts",
                source: "struct P{a:i32,b:i32}fn main(){let mut p=P{a:1,b:2};let mut f=||{p.a+=1;};p.a=8;f();}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "whole_place_overlaps_field",
                source: "struct P{a:i32,b:i32}fn read(p:&P)->i32{p.a+p.b}fn main(){let mut p=P{a:1,b:2};let f=||crate::read(&p);p.b=8;let x=f();}",
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
#[test]
fn borrowed_capture_drop_order_matches_stock() {
    oracle::check(
        "closure_drop_order",
        &[Case {
            name: "borrow_drops_after_environment",
            source: "struct D{i:i32}impl Drop for D{fn drop(&mut self){println!(\"{}\",self.i);}}fn main(){let d=D{i:7};{let f=||d.i;assert_eq!(f(),7);}println!(\"after\");}",
            expect: Expect::Run("after\n7\n"),
        }],
    );
}
