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

#[test]
fn move_reference_carriers_match_stock() {
    oracle::check(
        "move_reference_carriers",
        &[
            Case {
                name: "owned_guard_and_shared_tuple_origin",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!(\"drop {}\",self.n);}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!(\"{}\",f());println!(\"{}\",pair.1);}",
                expect: Expect::Run("drop 7\n10\n4\n"),
            },
            Case {
                name: "retained_reducer_smaller_source",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!(\"{}\",f());}",
                expect: Expect::Run("10\n"),
            },
            Case {
                name: "copied_reference_keeps_original_target",
                source: "fn main(){let a=3i32;let b=9i32;let mut r=&a;let f=move||*r;r=&b;assert_eq!(f(),3);assert_eq!(*r,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "shared_carrier_merges_disjoint_pointee_fields",
                source: "fn main(){let pair=(3i32,4i32);let r=&pair;let f=move||(*r).0+(*r).1;assert_eq!(f(),7);assert_eq!(f(),7);assert_eq!(pair.1,4);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_carrier_merges_read_and_write",
                source: "fn main(){let mut pair=(3i32,4i32);let r=&mut pair;let mut f=move||{(*r).0+=(*r).1;(*r).0};assert_eq!(f(),7);assert_eq!(f(),11);assert_eq!(pair.1,4);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "uncalled_owned_guard_cleanup",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!(\"drop {}\",self.n);}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!(\"created\");drop(f);println!(\"tail {}\",pair.1);}",
                expect: Expect::Run("created\ndrop 7\ntail 4\n"),
            },
            Case {
                name: "unused_dangling_move_reference",
                source: "fn unused(){let f={let pair=(3i32,4i32);let r=&pair.0;move||*r};let value=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0597"),
            },
            Case {
                name: "unused_live_copied_reference_conflict",
                source: "fn unused(){let mut pair=(3i32,4i32);let r=&pair.0;let f=move||*r;pair.0=9;let value=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0506"),
            },
            Case {
                name: "unused_mutable_carrier_moved",
                source: "fn unused(){let mut n=3i32;let r=&mut n;let mut f=move||{*r+=1;*r};let value=*r;let result=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0382"),
            },
            Case {
                name: "unused_noncopy_pointee_move",
                source: "struct Item{n:i32}fn unused(){let item=Item{n:3};let r=&item;let f=move||{let owned=*r;owned.n};let value=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0507"),
            },
            Case {
                name: "unused_guard_capture_reused",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){}}fn unused(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};drop(guard);let value=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0382"),
            },
            Case {
                name: "unused_fnonce_called_twice",
                source: "struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){}}fn unused(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};let a=f();let b=f();}fn main(){}",
                expect: Expect::InvalidAfterParse("E0382"),
            },
        ],
    );
}
