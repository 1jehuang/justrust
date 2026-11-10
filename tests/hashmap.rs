//! Native-source std::collections::HashMap, checked against stock rustc at
//! workers 1 and 8 (CHECK and BUILD). Iteration order is unspecified, so
//! accepted programs print only order-independent output.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::{Case, Expect};

#[test]
fn map_debug_matches_stock() {
    closure_oracle::check(
        "map-debug",
        &[
            Case {
                name: "ordered_keys_single_hash_entry_and_empty_maps",
                source: r#"use std::collections::{BTreeMap,HashMap};fn main(){let mut m:BTreeMap<i32,&str>=BTreeMap::new();m.insert(3,"c");m.insert(1,"a");m.insert(2,"b");println!("{m:?}");let mut h:HashMap<&str,i32>=HashMap::new();h.insert("only",7);println!("{h:?}");let a:BTreeMap<i32,String>=BTreeMap::new();let b:HashMap<String,i32>=HashMap::new();println!("{a:?} {b:?}");}"#,
                expect: Expect::Run("{1: \"a\", 2: \"b\", 3: \"c\"}\n{\"only\": 7}\n{} {}\n"),
            },
            Case {
                name: "ordered_map_value_needs_debug_in_unused_fn",
                source: r#"use std::collections::BTreeMap;struct Value;fn unused(m:BTreeMap<i32,Value>){println!("{m:?}");}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "hash_map_value_needs_debug_in_unused_fn",
                source: r#"use std::collections::HashMap;struct Value;fn unused(m:HashMap<i32,Value>){println!("{m:?}");}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn entry_and_modify_matches_stock() {
    closure_oracle::check(
        "entry-and-modify",
        &[
            Case {
                name: "occupied_only_and_consuming_capture",
                source: r#"use std::collections::{HashMap,BTreeMap};fn main(){let mut h:HashMap<&str,i32>=HashMap::new();let mut calls=0;h.entry("a").and_modify(|v|{calls+=1;*v+=100;}).or_insert(7);h.entry("a").and_modify(|v|{calls+=1;*v+=2;}).or_insert(99);println!("{} {}",h["a"],calls);let mut b:BTreeMap<i32,String>=BTreeMap::new();b.insert(1,"left".to_string());let suffix=String::from("!");b.entry(1).and_modify(move|v|v.push_str(&suffix)).or_insert("bad".to_string());let mut missed=0;b.entry(2).and_modify(|v|{missed+=1;v.push_str("bad");}).or_insert("right".to_string());println!("{} {} {}",b[&1],b[&2],missed);}"#,
                expect: Expect::Run("9 1\nleft! right 0\n"),
            },
            Case {
                name: "wrong_value_parameter_in_unused_fn",
                source: r#"use std::collections::HashMap;fn unused(){let mut m:HashMap<i32,i32>=HashMap::new();m.entry(1).and_modify(|v:&mut String|v.clear());}fn main(){}"#,
                expect: Expect::Invalid("E0631"),
            },
            Case {
                name: "non_unit_callback_in_unused_fn",
                source: r#"use std::collections::BTreeMap;fn unused(){let mut m:BTreeMap<i32,i32>=BTreeMap::new();m.entry(1).and_modify(|v|{*v+=1;7});}fn main(){}"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn entry_default_only_evaluates_when_vacant() {
    closure_oracle::check(
        "entry-default-vacant-only",
        &[
            Case {
                name: "default_side_effects_are_skipped_for_both_occupied_maps",
                source: r#"use std::collections::{HashMap,BTreeMap};struct Value(i32);impl Default for Value{fn default()->Self{println!("default");Value(7)}}
fn main(){let mut hash:HashMap<i32,Value>=HashMap::new();hash.entry(1).or_default().0+=1;hash.entry(1).or_default().0+=1;
let mut tree:BTreeMap<i32,Value>=BTreeMap::new();tree.entry(1).or_default().0+=2;tree.entry(1).or_default().0+=2;
println!("{} {}",hash.get(&1).unwrap().0,tree.get(&1).unwrap().0);}"#,
                expect: Expect::Run("default\ndefault\n9 11\n"),
            },
            Case {
                name: "occupied_default_must_not_construct_or_drop_a_throwaway_value",
                source: r#"use std::collections::{HashMap,BTreeMap};struct Value(i32);impl Default for Value{fn default()->Self{println!("construct");Value(7)}}impl Drop for Value{fn drop(&mut self){println!("drop {}",self.0)}}
fn main(){let mut hash:HashMap<i32,Value>=HashMap::new();hash.insert(1,Value(11));hash.entry(1).or_default().0+=1;
let mut tree:BTreeMap<i32,Value>=BTreeMap::new();tree.insert(1,Value(21));tree.entry(1).or_default().0+=1;
println!("{} {}",hash.get(&1).unwrap().0,tree.get(&1).unwrap().0);}"#,
                expect: Expect::Run("12 22\ndrop 22\ndrop 12\n"),
            },
        ],
    );
}

#[test]
fn hashmap_entry_copy_effects_preserve_origin_and_alias_checks() {
    closure_oracle::check(
        "hashmap-copy-entry-effects",
        &[
            Case {
                name: "local_copy_key_entry_roundtrip",
                source: r#"use std::collections::HashMap;fn main(){let text=String::from("key");let mut map:HashMap<&str,usize>=HashMap::new();*map.entry(text.as_str()).or_insert(0)+=1;*map.entry(text.as_str()).or_insert(0)+=1;println!("{}",map.get(text.as_str()).unwrap());}"#,
                expect: Expect::Run("2\n"),
            },
            Case {
                name: "copy_entry_mutable_slot_keeps_map_borrowed",
                source: r#"use std::collections::HashMap;fn main(){let mut map:HashMap<&str,usize>=HashMap::new();let slot=map.entry("key").or_insert(0);map.insert("other",1);println!("{}",slot);}"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "copy_entry_key_cannot_outlive_original_owner",
                source: r#"use std::collections::HashMap;fn main(){let mut map:HashMap<&str,usize>=HashMap::new();{let owner=String::from("local");*map.entry(owner.as_str()).or_insert(0)+=1;}println!("{}",map.len());}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "copy_entry_return_cannot_escape_local_map_in_unused_function",
                source: r#"use std::collections::HashMap;fn unused()->&'static mut usize{let mut map:HashMap<&str,usize>=HashMap::new();map.entry("key").or_insert(0)}fn main(){}"#,
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

#[test]
fn btree_entry_identity_value_keeps_nested_origin_and_alias_checks() {
    closure_oracle::check(
        "btree-identity-entry-effects",
        &[
            Case {
                name: "entry_keeps_noncopy_vector_shared_carriers",
                source: r#"use std::collections::BTreeMap;fn main(){let owner=String::from("value");let mut map:BTreeMap<usize,Vec<&str>>=BTreeMap::new();map.entry(0).or_default().push(owner.as_str());println!("{}",map.get(&0).unwrap().len());}"#,
                expect: Expect::Run("1\n"),
            },
            Case {
                name: "identity_value_cannot_escape_nested_original_owner",
                source: r#"use std::collections::BTreeMap;fn main(){let mut map:BTreeMap<usize,Vec<&str>>=BTreeMap::new();{let owner=String::from("value");map.entry(0).or_default().push(owner.as_str());}println!("{}",map.len());}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "identity_value_keeps_map_exclusively_borrowed",
                source: r#"use std::collections::BTreeMap;fn main(){let mut map:BTreeMap<usize,Vec<&str>>=BTreeMap::new();let value=map.entry(0).or_default();map.insert(1,Vec::new());println!("{}",value.len());}"#,
                expect: Expect::Invalid("E0499"),
            },
        ],
    );
}

#[test]
fn hashmap_matches_stock() {
    closure_oracle::check(
        "hashmap",
        &[
            Case {
                name: "shared_native_field_reference_iteration",
                source: r#"use std::collections::{HashMap,BTreeMap};fn main(){let mut map:HashMap<&str,&str>=HashMap::new();map.insert("key","value");for(key,value)in &map{println!("{} {}",key,value);}let mut tree:BTreeMap<&str,&str>=BTreeMap::new();tree.insert("other","entry");for(key,value)in &tree{println!("{} {}",key,value);}}"#,
                expect: Expect::Run("key value\nother entry\n"),
            },
            Case {
                name: "shared_native_field_wrong_family_in_unused_fn",
                source: r#"use std::collections::HashMap;fn bad<'a,K:std::hash::Hash+Eq,V>(map:&'a HashMap<K,V>)->&'a V{for(key,_)in map{return key;}panic!("empty")}fn main(){}"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "shared_native_field_reference_cannot_escape",
                source: r#"use std::collections::HashMap;fn unused()->&'static str{let text=String::from("local");let mut map:HashMap<u32,&str>=HashMap::new();map.insert(1u32,text.as_str());for(_,value)in &map{return *value;}""}fn main(){}"#,
                expect: Expect::Invalid("E0515"),
            },
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
                name: "btreemap_borrowed_string_vectors",
                source: r#"use std::collections::BTreeMap;
fn main() {
    let text = String::from("the cat on the mat cat");
    let mut by_len: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for word in text.split_whitespace() {
        let entry = by_len.entry(word.len()).or_default();
        if !entry.contains(&word) { entry.push(word); }
    }
    for (length, words) in &by_len { println!("{}: {:?}", length, words); }
}"#,
                expect: Expect::Run("2: [\"on\"]\n3: [\"the\", \"cat\", \"mat\"]\n"),
            },
            Case {
                name: "hashmap_borrowed_string_vectors",
                source: r#"use std::collections::HashMap;
fn main() {
    let mut m: HashMap<usize, Vec<&str>> = HashMap::new();
    m.entry(1).or_default().push("a");
    m.entry(1).or_default().push("b");
    println!("{} {:?}", m.len(), m.get(&1));
}"#,
                expect: Expect::Run("1 Some([\"a\", \"b\"])\n"),
            },
            Case {
                name: "borrowed_map_value_cannot_escape_owner",
                source: "use std::collections::BTreeMap; fn unused() -> &'static str { let text = String::from(\"temporary\"); let mut map: BTreeMap<usize, Vec<&str>> = BTreeMap::new(); map.entry(1).or_default().push(text.as_str()); for (_, values) in &map { return values[0]; } \"\" } fn main() {}",
                expect: Expect::Invalid("E0515"),
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
                name: "owned_for_loop_consumes_map",
                source: "use std::collections::HashMap; fn main(){ let mut m: HashMap<u32,usize> = HashMap::new(); m.insert(1,1); m.insert(2,2); m.insert(3,3); let mut t = 0usize; let mut n = 0usize; for (k, v) in m { t += k as usize * v; n += 1; } println!(\"{} {}\", t, n); }",
                expect: Expect::Run("14 3\n"),
            },
            Case {
                name: "borrowed_str_lookups_on_string_keys",
                source: r#"use std::collections::HashMap;
use std::borrow::Borrow;
fn eqb<K: Borrow<Q>, Q: ?Sized + Eq>(k: &K, q: &Q) -> bool { k.borrow() == q }
fn main() {
    let mut scores = HashMap::new();
    scores.insert(String::from("blue"), 10);
    scores.insert(String::from("red"), 50);
    if let Some(s) = scores.get("blue") { println!("blue = {}", s); }
    if let Some(s) = scores.get_mut("red") { *s += 1; }
    println!("{:?} {:?}", scores.remove("blue"), scores.remove("blue"));
    println!("{} {} {:?}", scores.contains_key("blue"), scores.len(), scores.get(&String::from("red")));
    println!("{} {}", eqb(&String::from("a"), "a"), eqb(&3, &4));
}"#,
                expect: Expect::Run("blue = 10\nSome(10) None\nfalse 1 Some(51)\ntrue false\n"),
            },
            Case {
                name: "integer_literal_borrowed_keys_follow_key_type",
                source: "use std::collections::HashMap; use std::borrow::Borrow; fn g<K: Borrow<Q>, Q: ?Sized + Eq>(_k: &K, _q: &Q) -> u8 { 1 } fn main(){ let mut m: HashMap<u32, u64> = HashMap::new(); m.insert(0,1); println!(\"{:?} {}\", m.get(&0), m.contains_key(&0)); println!(\"{:?}\", m.remove(&7)); if let Some(v) = m.get_mut(&0) { *v += 1; } println!(\"{:?} {}\", m.get(&0), g(&5u32, &0)); }",
                expect: Expect::Run("Some(1) true\nNone\nSome(2) 1\n"),
            },
            Case {
                name: "unrelated_borrowed_key_in_unused_fn",
                source: "use std::collections::HashMap; fn unused(){ let m: HashMap<String,u32> = HashMap::new(); let _ = m.get(&5u8); } fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "insert_through_immutable_in_unused_fn",
                source: "use std::collections::HashMap; fn unused(){ let m: HashMap<u32,u32> = HashMap::new(); m.insert(1,2); } fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "map_index_operator",
                source: r#"use std::collections::{HashMap,BTreeMap};fn main(){let mut c:HashMap<String,usize>=HashMap::new();c.insert(String::from("a"),3);let mut n:HashMap<u32,u64>=HashMap::new();n.insert(7,9);let mut t:BTreeMap<u32,u8>=BTreeMap::new();t.insert(2,5);println!("{} {} {} {}",c["a"],c[&String::from("a")],n[&7],t[&2]);}"#,
                expect: Expect::Run("3 3 9 5\n"),
            },
            Case {
                name: "map_index_str_key_reference",
                source: r#"use std::collections::HashMap;fn main(){let mut c:HashMap<&str,usize>=HashMap::new();c.insert("a",5);println!("{} {}",c.get("a").unwrap(),c[&"a"]);}"#,
                expect: Expect::Run("5 5\n"),
            },
            Case {
                name: "map_index_wrong_key_in_unused_fn",
                source: r#"use std::collections::HashMap;fn unused(c:&HashMap<String,u32>)->u32{c[&5u8]}fn main(){}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "generic_for_over_vec_reference",
                source: r#"fn count<K>(v:&Vec<K>)->usize{let mut n=0;for _k in v{n+=1;}n}fn main(){println!("{}",count(&vec![1u8,2,3]));}"#,
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "debug_option_of_borrowed_str_vectors",
                source: r#"fn f(v:&Vec<&str>)->usize{let x=v[0];x.len()+v[1].len()}fn main(){let v=vec!["a","bc"];let o=Some(&v);println!("{:?} {}",o,f(&v));}"#,
                expect: Expect::Run("Some([\"a\", \"bc\"]) 3\n"),
            },
            Case {
                name: "generic_call_with_inline_vec_argument",
                source: r#"fn count<K>(v:&Vec<K>)->usize{let mut n=0;for _k in v{n+=1;}n}fn main(){let o=Some(vec!["x"]);println!("{} {:?}",count(&vec![1u8,2,3]),o);}"#,
                expect: Expect::Run("3 Some([\"x\"])\n"),
            },
            Case {
                name: "into_iter_on_vector_reference_yields_element_references",
                source: r#"fn first<'a>(v:&'a Vec<u8>)->Option<&'a u8>{let mut it=v.into_iter();it.next()}fn main(){let v=vec![3u8,4];let r=&v;let it=r.into_iter();println!("{:?} {}",first(&v),it.count());}"#,
                expect: Expect::Run("Some(3) 2\n"),
            },
            Case {
                name: "vector_reference_infers_generic_slice_element",
                source: r#"fn first<K>(v:&[K])->Option<&K>{let mut it=v.iter();it.next()}fn main(){let v=vec![3u8,4];let r=first(&v);let a=[97u8];let n:Option<&u8>=first(&a);println!("{:?} {:?}",r,n);}"#,
                expect: Expect::Run("Some(3) Some(97)\n"),
            },
            Case {
                name: "generic_slice_iterator_next_returns_element_reference",
                source: r#"fn first<'a,K>(v:&'a Vec<K>)->Option<&'a K>{let mut it=v.iter();it.next()}fn second<K>(v:&Vec<K>)->Option<&K>{let mut it=v.into_iter();it.next();it.next()}fn main(){let v=vec![3u8,4];println!("{:?} {:?}",first(&v),second(&v));}"#,
                expect: Expect::Run("Some(3) Some(4)\n"),
            },
            Case {
                name: "generic_slice_iterator_item_keeps_owner_borrowed",
                source: r#"fn f<K:Copy>(v:&mut Vec<K>,k:K)->bool{let mut it=v.iter();let x=it.next();v.push(k);x.is_some()}fn main(){}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "generic_option_reference_copied_and_cloned",
                source: r#"fn f<K:Copy>(v:&[K])->Option<K>{let mut it=v.iter();it.next().copied()}fn g<K:Clone>(o:Option<&K>)->Option<K>{o.cloned()}fn h<K:Copy>(v:&Vec<K>)->Option<K>{v.first().copied()}fn main(){let v=vec![5u8,6];println!("{:?} {:?} {:?}",f(&v),g(Some(&7u8)),h(&v));}"#,
                expect: Expect::Run("Some(5) Some(7) Some(5)\n"),
            },
            Case {
                name: "generic_first_reference_keeps_vector_borrowed",
                source: r#"fn f<'a,K:Copy>(v:&'a mut Vec<K>,k:K)->Option<&'a K>{let x=v.first();v.push(k);x}fn main(){}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "clone_vector_of_borrowed_str_and_slice_refs",
                source: r#"fn main(){let v:Vec<&str>=vec!["a","b"];let w=v.clone();let a=[1u8,2];let s:&[u8]=&a;let t=s.clone();println!("{} {:?} {}",w.len(),w,t.len());}"#,
                expect: Expect::Run("2 [\"a\", \"b\"] 2\n"),
            },
            Case {
                name: "constant_index_through_reference_returns_local_borrow_in_unused_fn",
                source: r#"fn unused()->&'static str{let s=String::from("t");let v=vec![s.as_str()];let r=&v;return r[0];}fn main(){}"#,
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "generic_map_loop_returns_key_as_value_in_unused_fn",
                source: r#"use std::collections::HashMap;use std::hash::Hash;fn bad<'a,K:Hash+Eq,V>(map:&'a HashMap<K,V>)->&'a V{for(key,_)in map{return key;}panic!("empty")}fn main(){}"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn lazy_entry_callbacks_preserve_vacancy_and_key_authority() {
    closure_oracle::check(
        "lazy-entry-callback-vacancy",
        &[
            Case {
                name: "both_maps_call_factories_only_for_vacant_keys",
                source: r#"use std::collections::{HashMap,BTreeMap};
fn main(){let mut calls=0;let mut h:HashMap<i32,i32>=HashMap::new();
h.entry(1).or_insert_with(||{calls+=1;7});h.entry(1).or_insert_with(||{calls+=100;99});
h.entry(2).or_insert_with_key(|k|{calls+=1;*k+10});h.entry(2).or_insert_with_key(|k|{calls+=100;*k});
println!("{} {} {}",h.get(&1).unwrap(),h.get(&2).unwrap(),calls);
let mut t:BTreeMap<i32,i32>=BTreeMap::new();
t.entry(3).or_insert_with(||{calls+=1;17});t.entry(3).or_insert_with(||{calls+=100;99});
t.entry(4).or_insert_with_key(|k|{calls+=1;*k+20});t.entry(4).or_insert_with_key(|k|{calls+=100;*k});
println!("{} {} {}",t.get(&3).unwrap(),t.get(&4).unwrap(),calls);}"#,
                expect: Expect::Run("7 12 2\n17 24 4\n"),
            },
            Case {
                name: "factory_moves_noncopy_value_only_on_vacancy",
                source: r#"use std::collections::{HashMap,BTreeMap};fn main(){
let mut h:HashMap<i32,String>=HashMap::new();let a=String::from("first");h.entry(1).or_insert_with(move||a);
let b=String::from("ignored");h.entry(1).or_insert_with(move||b);
let mut t:BTreeMap<i32,String>=BTreeMap::new();let c=String::from("sorted");t.entry(2).or_insert_with(move||c);
println!("{} {}",h.get(&1).unwrap(),t.get(&2).unwrap());}"#,
                expect: Expect::Run("first sorted\n"),
            },
            Case {
                name: "unused_factory_result_must_match_value_type",
                source: "use std::collections::HashMap;fn unused(){let mut h:HashMap<i32,i32>=HashMap::new();h.entry(1).or_insert_with(||true);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_key_factory_cannot_mutate_shared_key",
                source: "use std::collections::BTreeMap;fn unused(){let mut t:BTreeMap<i32,i32>=BTreeMap::new();t.entry(1).or_insert_with_key(|k|{*k=2;3});}fn main(){}",
                expect: Expect::Invalid("E0594"),
            },
        ],
    );
}

#[test]
fn receiver_bound_lazy_callback_results_remain_nonexecuting_hints() {
    closure_oracle::check(
        "receiver-bound-lazy-callback-results",
        &[
            Case {
                name: "direct_and_compound_lazy_assignments_keep_rhs_before_factory",
                source: r#"use std::collections::{HashMap,BTreeMap};
fn main(){let mut h:HashMap<i32,i32>=HashMap::new();
*h.entry(1).or_insert_with(||{println!("hash factory");4})+={println!("hash rhs");3};
*h.entry(1).or_insert_with(||{println!("wrong hash factory");100})+={println!("occupied rhs");2};
let mut t:BTreeMap<i32,i32>=BTreeMap::new();
*t.entry(2).or_insert_with_key(|key|{println!("tree factory {}",key);*key})={println!("tree rhs");20};
*t.entry(2).or_insert_with(||{println!("wrong tree factory");100})+=1;
println!("{} {}",h.get(&1).unwrap(),t.get(&2).unwrap());}"#,
                expect: Expect::Run(
                    "hash rhs\nhash factory\noccupied rhs\ntree rhs\ntree factory 2\n9 21\n",
                ),
            },
            Case {
                name: "direct_assignment_preserves_noncopy_callback_and_value",
                source: r#"use std::collections::{HashMap,BTreeMap};fn main(){
let mut h:HashMap<i32,String>=HashMap::new();let seed=String::from("seed");
*h.entry(1).or_insert_with(move||seed)=String::from("replacement");
let skipped=String::from("skipped");*h.entry(1).or_insert_with(move||{println!("wrong factory");skipped})=String::from("final");
let mut t:BTreeMap<i32,String>=BTreeMap::new();let value=String::from("tree seed");
*t.entry(2).or_insert_with(move||value)=String::from("tree final");
println!("{} {}",h.get(&1).unwrap(),t.get(&2).unwrap());}"#,
                expect: Expect::Run("final tree final\n"),
            },
            Case {
                name: "source_schema_result_is_not_specific_to_entry_names",
                source: r#"struct Holder<T>{value:T}impl<T> Holder<T>{fn callback<F:FnOnce()->T>(&mut self,_factory:F)->&mut T{&mut self.value}}
fn main(){let mut holder=Holder{value:7i32};*holder.callback(||{println!("wrong callback");100})+=5;println!("{}",holder.value);}"#,
                expect: Expect::Run("12\n"),
            },
            Case {
                name: "bad_callback_is_still_rejected_in_unused_compound_assignment",
                source: "use std::collections::HashMap;fn unused(){let mut h:HashMap<i32,i32>=HashMap::new();*h.entry(1).or_insert_with(||true)+=1;}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "owned_guard_field_assignment_keeps_rhs_and_guard_destruction_order",
                source: r#"use std::cell::RefCell;struct Value{count:i32}fn main(){let cell=RefCell::new(Value{count:1});
cell.borrow_mut().count+={println!("rhs");cell.borrow_mut().count+=1;2};
assert_eq!(cell.borrow().count,4);cell.borrow_mut().count=7;assert_eq!(cell.borrow().count,7);}"#,
                expect: Expect::Run("rhs\n"),
            },
            Case {
                name: "shared_guard_cannot_gain_mutability_from_implicit_field_hint",
                source: "use std::cell::RefCell;struct Value{count:i32}fn unused(){let cell=RefCell::new(Value{count:1});cell.borrow().count+=1;}fn main(){}",
                expect: Expect::Invalid("E0594"),
            },
            Case {
                name: "bad_key_callback_is_still_rejected_in_unused_direct_assignment",
                source: "use std::collections::BTreeMap;fn unused(){let mut h:BTreeMap<i32,String>=BTreeMap::new();*h.entry(1).or_insert_with_key(|_|false)=String::from(\"value\");}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "callback_result_hints_cannot_choose_ambiguous_trait_methods",
                source: "struct Slot(i32);trait Left{fn callback<F:FnOnce()->i32>(&mut self,f:F)->&mut i32;}trait Right{fn callback<F:FnOnce()->i32>(&mut self,f:F)->&mut i32;}impl Left for Slot{fn callback<F:FnOnce()->i32>(&mut self,_f:F)->&mut i32{&mut self.0}}impl Right for Slot{fn callback<F:FnOnce()->i32>(&mut self,_f:F)->&mut i32{&mut self.0}}fn unused(){let mut slot=Slot(0);*slot.callback(||1)+=2;}fn main(){}",
                expect: Expect::Invalid("E0034"),
            },
            Case {
                name: "shared_inherent_receiver_cannot_fall_through_to_mutable_trait",
                source: "struct Slot(i32);impl Slot{fn callback<F:FnOnce()->i32>(&self,_f:F)->&i32{&self.0}}trait Mutable{fn callback<F:FnOnce()->i32>(&mut self,f:F)->&mut i32;}impl Mutable for Slot{fn callback<F:FnOnce()->i32>(&mut self,_f:F)->&mut i32{&mut self.0}}fn unused(){let mut slot=Slot(0);*slot.callback(||1)=3;}fn main(){}",
                expect: Expect::Invalid("E0594"),
            },
            Case {
                name: "unknown_result_slot_cannot_be_fabricated_from_receiver",
                source: "struct Slot;impl Slot{fn callback<F:FnOnce(),T>(&mut self,_f:F)->&mut T{panic!(\"unused\")}}fn unused(){let mut slot=Slot;let _value=slot.callback(||{});}fn main(){}",
                expect: Expect::Invalid("E0282"),
            },
        ],
    );
}

#[test]
fn receiver_bound_callback_shared_inventory_matches_stock() {
    closure_oracle::check(
        "receiver-bound-callback-shared-inventory",
        &[Case {
            name: "unchanged_rc_refcell_inventory",
            source: r#"use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct Item { name: String, available: i32, reserved: i32 }
struct Inventory { items: HashMap<String, Rc<RefCell<Item>>>, events: Vec<String> }
impl Inventory {
    fn add(&mut self, name: &str, quantity: i32) {
        let key = name.to_string();
        let item = self.items.entry(key).or_insert_with(|| Rc::new(RefCell::new(Item {
            name: name.to_string(), available: 0, reserved: 0,
        })));
        item.borrow_mut().available += quantity;
    }
    fn reserve(&mut self, name: &str, quantity: i32) -> Result<(), String> {
        let key = name.to_string();
        let item = self.items.get(&key).ok_or_else(|| format!("unknown {}", name))?;
        let mut item = item.try_borrow_mut().map_err(|_| format!("busy {}", name))?;
        if quantity <= 0 || quantity > item.available {
            return Err(format!("unavailable {}: {}", name, quantity));
        }
        item.available -= quantity;
        item.reserved += quantity;
        self.events.push(format!("reserved {} {}", quantity, item.name));
        Ok(())
    }
    fn report(&self, names: &[&str]) {
        let mut total = 0;
        for name in names {
            let key = name.to_string();
            let item = self.items.get(&key).unwrap().borrow();
            println!("{}: {} free, {} reserved", item.name, item.available, item.reserved);
            total += item.available + item.reserved;
        }
        for event in &self.events { println!("{}", event); }
        println!("total {} events {}", total, self.events.len());
    }
}
fn main() {
    let mut inventory = Inventory { items: HashMap::new(), events: Vec::new() };
    inventory.add("bolts", 12);
    inventory.add("washers", 20);
    inventory.add("bolts", 8);
    inventory.reserve("bolts", 7).unwrap();
    inventory.reserve("washers", 9).unwrap();
    println!("rejected: {}", inventory.reserve("bolts", 30).unwrap_err());
    println!("rejected: {}", inventory.reserve("nuts", 1).unwrap_err());
    inventory.report(&["bolts", "washers"]);
}
"#,
            expect: Expect::Run(
                "rejected: unavailable bolts: 30\nrejected: unknown nuts\nbolts: 13 free, 7 reserved\nwashers: 11 free, 9 reserved\nreserved 7 bolts\nreserved 9 washers\ntotal 40 events 2\n",
            ),
        }],
    );
}
