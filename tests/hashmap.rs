//! Native-source std::collections::HashMap, checked against stock rustc at
//! workers 1 and 8 (CHECK and BUILD). Iteration order is unspecified, so
//! accepted programs print only order-independent output.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::{Case, Expect};

#[test]
fn hashmap_matches_stock() {
    closure_oracle::check(
        "hashmap",
        &[
            Case {
                name: "scalar_api",
                source: r#"use std::collections::HashMap;
fn main() {
    let mut m: HashMap<u32, u64> = HashMap::new();
    println!("{} {}", m.len(), m.is_empty());
    for i in 0..10u32 { *m.entry(i % 3).or_insert(0) += i as u64; }
    println!("{:?} {:?} {:?}", m.get(&0), m.get(&2), m.get(&7));
    if let Some(v) = m.get_mut(&1) { *v *= 10; }
    println!("{:?} {}", m.insert(1, 5), m.contains_key(&1));
    println!("{:?} {:?} {} {}", m.remove(&0), m.remove(&0), m.len(), m.is_empty());
}"#,
                expect: Expect::Run(
                    "0 true\nSome(18) Some(15) None\nSome(120) true\nSome(18) None 2 false\n",
                ),
            },
            Case {
                name: "string_and_derived_keys",
                source: r#"use std::collections::HashMap;
#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum Cat { A, B }
fn main() {
    let mut s: HashMap<String, i32> = HashMap::new();
    s.insert(String::from("blue"), 10);
    s.insert(String::from("red"), 50);
    println!("{:?} {}", s.get(&String::from("red")), s.contains_key(&String::from("x")));
    let mut c: HashMap<Cat, u32> = HashMap::new();
    *c.entry(Cat::A).or_insert(1) += 1;
    *c.entry(Cat::B).or_insert(7) += 0;
    *c.entry(Cat::A).or_insert(1) += 1;
    println!("{:?} {:?} {}", c.get(&Cat::A), c.get(&Cat::B), c.len());
}"#,
                expect: Expect::Run("Some(50) false\nSome(3) Some(7) 2\n"),
            },
            Case {
                name: "key_without_hash_in_unused_fn",
                source: "use std::collections::HashMap; struct K; fn unused(){ let mut m: HashMap<K,u32> = HashMap::new(); m.insert(K,1); } fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "get_then_insert_in_unused_fn",
                source: "use std::collections::HashMap; fn unused(){ let mut m: HashMap<u32,u32> = HashMap::new(); let r = m.get(&1); m.insert(2,3); println!(\"{:?}\", r); } fn main(){}",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "two_get_mut_in_unused_fn",
                source: "use std::collections::HashMap; fn unused(){ let mut m: HashMap<u32,u32> = HashMap::new(); let a = m.get_mut(&1); let b = m.get_mut(&2); println!(\"{:?}{:?}\", a, b); } fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "insert_through_immutable_in_unused_fn",
                source: "use std::collections::HashMap; fn unused(){ let m: HashMap<u32,u32> = HashMap::new(); m.insert(1,2); } fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
        ],
    );
}
