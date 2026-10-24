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
                name: "entry_or_default",
                source: r#"use std::collections::HashMap;
fn main() {
    let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
    m.entry(1).or_default().push(10);
    m.entry(1).or_default().push(11);
    let e = m.entry(2).or_default();
    let n = e.len();
    let mut c: HashMap<u8, u64> = HashMap::new();
    *c.entry(5).or_default() += 4;
    *c.entry(5).or_default() += 4;
    let mut s: HashMap<u8, String> = HashMap::new();
    s.entry(1).or_default().push_str("ab");
    let mut f: HashMap<u8, f64> = HashMap::new();
    *f.entry(1).or_default() += 1.5;
    let mut o: HashMap<u8, Option<u8>> = HashMap::new();
    let slot = o.entry(1).or_default();
    let none = slot.is_none();
    println!("{:?} {} {:?} {}", m.get(&1), n, c.get(&5), m.len());
    println!("{:?} {:?} {} {:?}", s.get(&1), f.get(&1), none, <Vec<u8> as Default>::default());
}"#,
                expect: Expect::Run("Some([10, 11]) 0 Some(8) 2\nSome(\"ab\") Some(1.5) true []\n"),
            },
            Case {
                name: "or_default_requires_default_in_unused_fn",
                source: "use std::collections::HashMap; struct N; fn unused(){ let mut m: HashMap<u32,N> = HashMap::new(); let _ = m.entry(1).or_default(); } fn main(){}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "btreemap_ordered_entry_insert_remove",
                source: r#"use std::collections::BTreeMap;
fn main() {
    let mut m: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    m.entry(3).or_default().push(30);
    m.entry(1).or_default().push(10);
    m.entry(3).or_default().push(31);
    for (k, v) in &m { println!("{} {:?}", k, v); }
    let mut n: BTreeMap<i32, i32> = BTreeMap::new();
    let a = n.insert(2, 20);
    let b = n.insert(2, 21);
    n.insert(-7, 1);
    for (k, v) in [(9i32, 90i32), (0, 0)] { n.insert(k, v); }
    let mut keys = Vec::new();
    for (k, _) in &n { keys.push(*k); }
    *n.entry(0).or_insert(5) += 1;
    println!("{:?} {:?} {:?} {:?} {}", a, b, keys, n.get(&0), n.contains_key(&-7));
    println!("{:?} {:?} {}", n.remove(&-7), n.remove(&100), n.len());
}"#,
                expect: Expect::Run(
                    "1 [10]\n3 [30, 31]\nNone Some(20) [-7, 0, 2, 9] Some(1) true\nSome(1) None 3\n",
                ),
            },
            Case {
                name: "btreemap_key_without_ord_in_unused_fn",
                source: "use std::collections::BTreeMap; struct K; fn unused(){ let mut m: BTreeMap<K,u32> = BTreeMap::new(); m.insert(K,1); } fn main(){}",
                expect: Expect::Invalid("E0277"),
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
