//! Stock-matching borrowed capture environment controls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};
#[test]
fn argument_borrow_aggregate_results() {
    oracle::check(
        "argument_borrow_aggregate_results",
        &[
            Case {
                name: "noncopy_argument_field_call_and_tuple",
                source: "struct Item{name:String,count:i32}fn main(){let items=vec![Item{name:String::from(\"apple\"),count:7}];let mut rows=items.iter().map(|i|(i.name.as_str(),&i.count));let row=rows.next().unwrap();assert_eq!(row.0,\"apple\");assert_eq!(*row.1,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "nested_shared_argument_borrow_results",
                source: "struct Item{name:String,count:i32}fn main(){let item=Item{name:String::from(\"pear\"),count:9};let items=vec![&item];let mut rows=items.iter().map(|i|(i.name.as_str(),&i.count));let row=rows.next().unwrap();assert_eq!(row.0,\"pear\");assert_eq!(*row.1,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_owned_argument_borrow_escape",
                source: "struct Item{name:String}fn unused(){let items=vec![Item{name:String::from(\"apple\")}];let mut rows=items.into_iter().map(|i|(i.name.as_str(),i.name.as_str()));let row=rows.next().unwrap();}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_mutation_while_argument_result_live",
                source: "struct Item{name:String}fn unused(){let mut items=vec![Item{name:String::from(\"apple\")}];let mut rows=items.iter().map(|i|(i.name.as_str(),i.name.as_str()));let row=rows.next().unwrap();items.clear();println!(\"{}\",row.0);}fn main(){}",
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}
#[test]
fn borrowed_capture_environments() {
    oracle::check(
        "closure_borrows",
        &[
            Case {
                name: "captured_borrow_result",
                source: "fn main(){let n=7i32;let f=||&n;assert_eq!(*f(),7);assert_eq!(*f(),7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_borrow_tuple_result",
                source: "fn main(){let a=7i32;let b=9i32;let f=||(&a,&b);let (x,y)=f();assert_eq!(*x+*y,16);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_result_disjoint_parent_field",
                source: "struct Pair{left:i32,right:i32}fn main(){let mut pair=Pair{left:7,right:9};let f=||&pair.left;let result=f();pair.right=11;assert_eq!(*result,7);assert_eq!(pair.right,11);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_iterator_result_chain",
                source: "fn main(){let bytes=[2u8,3u8];let f=||bytes.iter().cloned();let mut first=f();let mut second=f();assert_eq!(first.next(),Some(2u8));assert_eq!(first.next(),Some(3u8));assert_eq!(first.next(),None);assert_eq!(second.next(),Some(2u8));}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_slice_reference_read_result",
                source: "fn inspect(bytes:&[u8]){let f=||bytes.iter().cloned();let mut result=f();assert_eq!(result.next(),Some(2u8));assert_eq!(result.next(),Some(3u8));}fn main(){let bytes=[2u8,3u8];inspect(&bytes);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_nested_slice_reference_read_result",
                source: "fn inspect(bytes:&&[u8]){let f=||bytes.iter().cloned();let mut result=f();assert_eq!(result.next(),Some(2u8));}fn main(){let values=[2u8,3u8];let bytes:&[u8]=&values;inspect(&bytes);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_nested_reference_borrow_result",
                source: "fn main(){let values=[2u8,3u8];let bytes:&[u8]=&values;let f=||&bytes;let result=f();assert_eq!((*result)[0],2u8);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "captured_result_keeps_parent_loan",
                source: "fn main(){let mut n=7i32;let f=||&n;let result=f();n=9;println!(\"{}\",result);}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "unused_local_reference_result_escape",
                source: "fn unused(){let f=||{let local=7i32;&local};let result=f();}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_moved_owned_capture_result_escape",
                source: "fn unused(){let values=[2u8,3u8];let f=move||&values;let result=f();}fn main(){}",
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "shared_read",
                source: "fn main(){let n=7i32;let f=|x:i32|n+x;assert_eq!(f(2),9);assert_eq!(f(3),10);assert_eq!(n,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_write",
                source: "fn main(){let mut n=7i32;let mut f=||{n+=1;n};assert_eq!(f(),8);assert_eq!(f(),9);assert_eq!(n,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "generic_fnmut",
                source: "fn twice<F:FnMut()->i32>(mut f:F)->i32{f()+f()}fn main(){let mut n=7i32;let f=||{n+=1;n};assert_eq!(twice(f),17);assert_eq!(n,9);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "move_owned_mutation",
                source: "fn main(){let n=7i32;let mut f=move ||{let x=n+1;x};assert_eq!(f(),8);assert_eq!(n,7);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "fnmut_not_fn",
                source: "fn apply<F:Fn()->i32>(f:F)->i32{f()}fn main(){let mut n=7i32;let f=||{n+=1;n};let x=apply(f);}",
                expect: Expect::Invalid("E0525"),
            },
            Case {
                name: "immutable_callable",
                source: "fn main(){let mut n=7i32;let f=||{n+=1;};f();}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "live_shared_capture",
                source: "fn main(){let mut n=7i32;let f=||n;n=9;let x=f();}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "live_mut_capture",
                source: "fn main(){let mut n=7i32;let mut f=||{n+=1;n};let x=n;let y=f();}",
                expect: Expect::Invalid("E0503"),
            },
            Case {
                name: "unused_borrow_conflict",
                source: "fn unused(){let mut n=7i32;let f=||n;n=9;let x=f();}fn main(){}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "unused_mutable_call",
                source: "fn unused(){let mut n=7i32;let f=||{n+=1;};f();}fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "escaping_borrow",
                source: "fn main(){let f={let n=7i32;||n};let x=f();}",
                expect: Expect::Invalid("E0373"),
            },
        ],
    );
}
