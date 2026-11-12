//! Universal associated projections remain exact until closed instantiation.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn copy_vec_extend_method_selects_by_iterator_item() {
    oracle::stock_native(
        r#"
fn main() {
    let mut e = vec![1];
    e.extend(vec![2, 3]);
    e.extend([4].iter());
    let tail: Vec<i32> = vec![5];
    e.extend(tail);
    let more = [6, 7];
    e.extend(&more);
    let mut c: Vec<char> = Vec::new();
    c.extend("ab".chars());
    let mut s = vec![String::from("x")];
    s.extend(vec![String::from("y")]);
    println!("{:?} {:?} {:?}", e, c, s);
}
"#,
    );
    oracle::stock_and_native_reject(
        "fn unused() { let mut e: Vec<i32> = vec![1]; let s: Vec<u32> = vec![2]; e.extend(s); } fn main() {}",
    );
    oracle::stock_and_native_reject(
        "fn unused() { let mut e: Vec<String> = Vec::new(); let s = vec![String::new()]; e.extend(s.iter()); } fn main() {}",
    );
}

#[test]
fn borrowed_copy_vec_extension_and_partition_match_stock() {
    oracle::stock_native(
        r#"
use std::iter::Extend;
#[derive(Clone, Copy, Debug)]
struct Pair { left: i32, right: i32 }
fn main() {
    let values = vec![1i64, 2, 3, 4];
    let (small, big): (Vec<i64>, Vec<i64>) = values.iter().partition(|&&x| x < 3);
    println!("{:?} {:?}", small, big);
    let pairs = [Pair { left: 3, right: 7 }, Pair { left: 5, right: 11 }];
    let mut out: Vec<Pair> = vec![Pair { left: 1, right: 2 }];
    <Vec<Pair> as Extend<&Pair>>::extend(&mut out, pairs.iter());
    let empty: [Pair; 0] = [];
    <Vec<Pair> as Extend<&Pair>>::extend(&mut out, empty.iter());
    println!("{:?} {} {}", out, pairs[0].left, pairs[1].right);
}
"#,
    );
    oracle::stock_and_native_reject(
        r#"use std::iter::Extend; fn unused() { let source = vec![String::from("a")]; let mut target: Vec<String> = Vec::new(); <Vec<String> as Extend<&String>>::extend(&mut target, source.iter()); } fn main() {}"#,
    );
    oracle::stock_and_native_reject(
        r#"use std::iter::Extend; fn unused() { let source = vec![1u32]; let mut target: Vec<i32> = Vec::new(); <Vec<i32> as Extend<&u32>>::extend(&mut target, source.iter()); } fn main() {}"#,
    );
}

#[test]
fn blanket_into_iterator_for_user_generic_and_borrowed_iterators() {
    oracle::stock_native(
        r#"
struct Counter { n: i32, end: i32 }
impl Iterator for Counter {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        if self.n < self.end { self.n += 1; Some(self.n) } else { None }
    }
}
fn total<I: Iterator<Item = i32>>(it: I) -> i32 {
    let mut sum = 0;
    for x in it { sum += x; }
    sum
}
fn prefix<I: Iterator<Item = i32>>(it: &mut I) -> i32 {
    let mut sum = 0;
    for x in it { sum += x; if x == 2 { break; } }
    sum
}
struct Bag { values: Vec<i32> }
impl IntoIterator for Bag {
    type Item = i32;
    type IntoIter = std::vec::IntoIter<i32>;
    fn into_iter(self) -> Self::IntoIter { self.values.into_iter() }
}
fn main() {
    let mut sum = 0;
    for x in (Counter { n: 0, end: 4 }) { sum += x; }
    println!("{} {}", sum, total(Counter { n: 0, end: 5 }));
    let mut c = Counter { n: 0, end: 5 };
    println!("{} {}", prefix(&mut c), c.next().unwrap());
    for x in &mut c { sum += x; }
    println!("{} {}", sum, c.next().is_none());
    for (i, x) in (Counter { n: 0, end: 4 }).enumerate() {
        sum += i as i32 * x;
    }
    for x in (Counter { n: 0, end: 4 }).map(|x| x * 3).filter(|x| *x % 2 == 0) {
        sum += x;
    }
    for x in (Bag { values: vec![7, 8] }) { sum += x; }
    println!("{} {}", sum, total(vec![2, 3, 4].into_iter()));
}
"#,
    );
}

#[test]
fn user_sum_and_operators_milestone() {
    oracle::stock_native(include_str!("milestones/m2/14_traits_operators.rs"));
    oracle::stock_native(
        r#"struct Total(i32);impl std::iter::Sum<i32> for Total{fn sum<I:Iterator<Item=i32>>(it:I)->Total{let mut n=0;for x in it{n+=x;}Total(n)}}impl<'a> std::iter::Sum<&'a i32> for Total{fn sum<I:Iterator<Item=&'a i32>>(it:I)->Total{let mut n=0;for x in it{n+=*x;}Total(n)}}fn total<I:Iterator<Item=i32>>(it:I)->Total{it.sum()}fn main(){let v=vec![2i32,3,4];let a:Total=v.iter().sum();let b=total(v.into_iter());println!("{} {}",a.0,b.0);}"#,
    );
    oracle::stock_native(
        r#"struct Gather<T> { values: Vec<T> }
impl<T> std::iter::Sum<T> for Gather<T> {
    fn sum<I: Iterator<Item = T>>(mut input: I) -> Gather<T> {
        let mut values = Vec::new();
        for item in &mut input { values.push(item); }
        Gather { values }
    }
}
fn gather<T, I: Iterator<Item = T>>(input: I) -> Gather<T> { input.sum() }
fn main() {
    let numbers: Gather<i32> = gather(vec![2i32, 3, 4].into_iter());
    println!("{} {}", numbers.values.len(), numbers.values[1]);
    let words: Gather<String> = vec![String::from("a"), String::from("bb")].into_iter().sum();
    println!("{} {}", words.values.len(), words.values[1]);
}"#,
    );
    for source in [
        "struct Total;impl std::iter::Sum for Total{fn sum<I:Iterator<Item=Total>+Clone>(_:I)->Total{Total}}fn main(){}",
        "struct Total;impl std::iter::Sum<u8> for Total{fn sum<I:Iterator<Item=u8>>(_:I)->Total{Total}}fn unused(){let _:Total=(0i32..2).sum();}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn borrowed_iterator_forwards_nth_and_double_ended_state() {
    oracle::stock_native(
        r#"
struct Steps { front: i32, back: i32 }
impl Iterator for Steps {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        if self.front < self.back { let n = self.front; self.front += 1; Some(n) } else { None }
    }
    fn nth(&mut self, n: usize) -> Option<i32> {
        println!("nth {n}");
        self.next()
    }
}
impl DoubleEndedIterator for Steps {
    fn next_back(&mut self) -> Option<i32> {
        if self.front < self.back { self.back -= 1; Some(self.back) } else { None }
    }
}
fn select<I: Iterator<Item = i32>>(mut it: I) -> i32 { it.nth(2).unwrap() }
fn main() {
    let mut s = Steps { front: 1, back: 5 };
    println!("{}", select(&mut s));
    for x in (&mut s).rev() { println!("{x}"); }
    println!("{} {}", s.front, s.back);
}
"#,
    );
}

#[test]
fn blanket_into_iterator_does_not_allow_conflicting_or_unproved_impls() {
    for source in [
        "struct C { n: i32 } impl Iterator for C { type Item = i32; fn next(&mut self) -> Option<i32> { None } } impl IntoIterator for C { type Item = i32; type IntoIter = C; fn into_iter(self) -> C { self } } fn main() {}",
        "fn unused<I>(it: I) { for _ in it {} } fn main() {}",
        "fn unused<I: Iterator<Item = u8>>(it: I) { for x in it { let _: bool = x; } } fn main() {}",
        "fn unused<I: Iterator<Item = i32>>(it: &mut I) { for x in it { let _: bool = x; } } fn main() {}",
        "fn unused<I: Iterator<Item = i32>, J: Iterator>(a: &mut I, b: &mut J) { for x in b { let _: i32 = x; } } fn main() {}",
        "fn unused<I: Iterator<Item = i32>>(it: &mut I) -> &'static mut I { <&mut I as IntoIterator>::into_iter(it) } fn main() {}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn unchanged_iterator_adapters_program() {
    oracle::stock_native(include_str!("fixtures/full_programs/36_iter_adapters.rs"));
}

#[test]
fn overlapping_windows_and_projected_indexing() {
    oracle::stock_native(
        "fn main(){let v=vec![2i32,5,9,14];let differences:Vec<i32>=v.windows(2).map(|w|w[1]-w[0]).collect();println!(\"{:?}\",differences);println!(\"{} {} {}\",v.windows(1).count(),v.windows(4).count(),v.windows(5).count());let empty:Vec<i32>=Vec::new();println!(\"{}\",empty.windows(1).count());let s:&[i32]=&v;let mut w=s.windows(2);let first=w.next().unwrap();let last=w.next_back().unwrap();let middle=w.next().unwrap();println!(\"{} {} {} {} {} {} {}\",first[0],first[1],last[0],last[1],middle[0],middle[1],w.next_back().is_none());}",
    );
}

#[test]
fn zero_sized_windows_panic_at_construction() {
    use std::os::unix::fs::PermissionsExt;
    let source = "fn main(){let v=vec![1i32,2];println!(\"before\");let _=v.windows(0);println!(\"after\");}";
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-zero-windows-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    std::fs::write(&input, source).unwrap();
    let stock_image = root.join("stock");
    let stock = std::process::Command::new(oracle::closure_oracle::pinned_stock::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(input)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let stock = std::process::Command::new(stock_image).output().unwrap();
    assert!(!stock.status.success());
    assert_eq!(stock.stdout, b"before\n");
    for workers in [1, 8] {
        let checked = justrust_engine::compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(!checked.report.published_kind(103));
        let built = justrust_engine::compiler::compile(source.into(), workers, false).unwrap();
        let image = root.join(format!("native-{workers}"));
        std::fs::write(&image, built.executable.unwrap()).unwrap();
        std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
        let native = std::process::Command::new(image).output().unwrap();
        assert!(!native.status.success());
        assert_eq!(native.stdout, stock.stdout);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inferred_borrowed_filter_operands() {
    oracle::stock_native(
        "fn main(){let v:Vec<i32>=(1..=6).collect();let evens:Vec<i32>=v.iter().copied().filter(|x|x%2==0).collect();println!(\"{:?}\",evens);}",
    );
}

#[test]
fn unique_operator_rhs_accepts_deref_and_unsizing_coercions() {
    oracle::stock_native(
        "fn main(){let t=String::from(\"tail\");let s=String::from(\"head\")+&t;println!(\"{} {}\",s,t);let v=vec![1i32,2,3];println!(\"{}\",v.iter().fold(String::new(),|acc,x|acc+&x.to_string()));}",
    );
    oracle::stock_native(
        "struct Count;impl std::ops::Add<&[i32]> for Count{type Output=usize;fn add(self,rhs:&[i32])->usize{rhs.len()}}fn main(){let a=[3i32,4,5];println!(\"{}\",Count+&a);let v=vec![1i32,2];println!(\"{}\",Count+&v);}",
    );
}

#[test]
fn provided_projected_ordering_bound_is_proved() {
    oracle::stock_native(
        "fn main(){let v=vec![2i32,9,4];println!(\"{:?}\",v.iter().copied().map(|x|x+1).max());}",
    );
    for source in [
        "struct NoOrd;fn unused(){let v=vec![NoOrd];let _=v.into_iter().max();}fn main(){}",
        "trait Pick{type Item;fn pick(self,x:Self::Item)->Self::Item where Self:Sized,Self::Item:Ord{x}}struct N;struct P;impl Pick for P{type Item=N;}fn unused(){let _=P.pick(N);}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let _:bool=v.windows(2).map(|w|w[0]).next().unwrap();}fn main(){}",
        "fn unused(){let s=String::new()+&true;}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let w=v.windows(2);drop(v);let _=w.count();}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let _=v.iter().copied().filter(|x|x%true==0).count();}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn provided_iterator_default_checks_universally_and_runs_closed() {
    oracle::stock_native(
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn last(mut self)->Option<Self::Item> where Self:Sized{let mut out=None;while let Some(x)=self.next(){out=Some(x);}out}}struct Counter{n:i32}impl It for Counter{type Item=i32;fn next(&mut self)->Option<i32>{if self.n<3{self.n+=1;Some(self.n)}else{None}}}fn main(){println!(\"{}\",Counter{n:0}.last().unwrap());}",
    );
}

#[test]
fn rigid_generic_projection_identity_is_not_an_eager_concrete_guess() {
    oracle::stock_native(
        "trait Has{type Out;}struct S;impl Has for S{type Out=i32;}fn identity<I:Has>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){println!(\"{}\",identity::<S>(42));}",
    );
}

#[test]
fn unused_universal_body_and_unproved_projection_reject() {
    for source in [
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out)->bool{x}fn main(){}",
        "trait Has{type Out;}fn bad<I>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){}",
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out){let first=x;let second=x;}fn main(){}",
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn bad(self)->Self::Item where Self:Sized{true}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn peekable_over_references_and_producer_slot_writes() {
    oracle::stock_native(
        "struct P<I, T> { it: I, peeked: Option<T> } impl<T, I: Iterator<Item = T>> P<I, T> { fn fill(&mut self) { if let None = self.peeked { self.peeked = self.it.next(); } } } fn fill<T, I: Iterator<Item = T>>(it: &mut I, slot: &mut Option<T>) { *slot = it.next(); } fn put<T>(slot: &mut Option<T>, v: T) { *slot = Some(v); } fn main() { let v: Vec<i64> = vec![1, 2, 10, 11, 20]; let mut p: P<std::slice::Iter<i64>, &i64> = P { it: v.iter(), peeked: None }; p.fill(); println!(\"{:?}\", p.peeked); let mut it = v.iter(); let mut s: Option<&i64> = None; fill(&mut it, &mut s); println!(\"{:?}\", s); let x = 5i64; let mut o: Option<&i64> = None; put(&mut o, &x); let t = o.take(); let u = o.replace(&x); println!(\"{:?} {:?} {:?}\", o, t, u); let mut pk = v.iter().peekable(); while let Some(x) = pk.next() { if let Some(&&n) = pk.peek() { if n % 10 == 0 { print!(\"{x}->{n} \"); } } } println!(); }\n",
    );
    for source in [
        "fn fill<T, I: Iterator<Item = T>>(it: &mut I, slot: &mut Option<T>) { *slot = it.next(); } fn unused() { let mut s: Option<&i64> = None; { let v: Vec<i64> = vec![1]; let mut it = v.iter(); fill(&mut it, &mut s); } println!(\"{:?}\", s); } fn main() {}\n",
        "fn main() {\n    let mut v: Vec<i64> = vec![1, 2, 3];\n    let mut p = v.iter().peekable();\n    let r = p.peek();\n    v.push(4);\n    println!(\"{:?}\", r);\n}\n",
        "fn unused() {\n    let r;\n    {\n        let v: Vec<i64> = vec![1];\n        let mut p = v.iter().peekable();\n        r = *p.peek().unwrap();\n    }\n    println!(\"{}\", r);\n}\nfn main() {}\n",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn non_closure_opaque_iterator_returns_match_stock() {
    oracle::stock_native(
        r#"
fn tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase())
}
fn bumped(v: &[u8]) -> impl Iterator<Item = u8> { v.iter().copied().map(|x| x + 1) }
fn main() {
    for w in tokens("It was, the BEST") { println!("{w}"); }
    let all: Vec<String> = tokens("a-b c").collect();
    println!("{all:?}");
    let v = vec![1u8, 2];
    println!("{:?}", bumped(&v).collect::<Vec<_>>());
}
"#,
    );
}

#[test]
fn non_closure_opaque_iterator_borrow_errors_match_stock() {
    oracle::stock_and_native_reject(
        r#"
fn words(s: &str) -> impl Iterator<Item = &str> + '_ { s.split(' ') }
fn main() { let it; { let owned = String::from("a b"); it = words(&owned); } for w in it { println!("{w}"); } }
"#,
    );
    oracle::stock_and_native_reject(
        r#"
fn bumped(v: &[u8]) -> impl Iterator<Item = u8> + '_ { v.iter().map(|x| *x + 1) }
fn main() { let v = vec![1u8, 2]; let it = bumped(&v); drop(v); for w in it { println!("{w}"); } }
"#,
    );
}

const RIGID_CONTAINER: &str = r#"
trait Container { type Item; fn get(&self, i: usize) -> Option<&Self::Item>; fn len(&self) -> usize; fn own(&self) -> Self::Item where Self::Item: Clone { self.get(0).unwrap().clone() } }
struct Stack<T> { items: Vec<T> }
impl<T> Container for Stack<T> { type Item = T; fn get(&self, i: usize) -> Option<&T> { self.items.get(i) } fn len(&self) -> usize { self.items.len() } }
struct Bytes(Vec<u8>);
impl Container for Bytes { type Item = u8; fn get(&self, i: usize) -> Option<&u8> { self.0.get(i) } fn len(&self) -> usize { self.0.len() } }
"#;

#[test]
fn rigid_reference_projections_in_generic_callers_match_stock() {
    for body in [
        // `&C::Item` results stay rigid in the universal body and are read
        // through Option without a complete physical cell table.
        r#"fn present<C: Container>(c: &C) -> bool { c.get(0).is_some() }
fn first<C: Container>(c: &C) -> bool { let _o = c.get(0); true }
fn main() { println!("{} {}", present(&Stack { items: vec![1i32] }), present(&Bytes(vec![]))); println!("{}", first(&Bytes(vec![3]))); }"#,
        // A closure parameter typed by a caller projection, instantiated on a
        // closed implementation.
        r#"fn show<C: Container>(c: &C) -> String where C::Item: std::fmt::Debug + Clone { let x = c.own(); let g = |y: &C::Item| format!("{y:?}"); g(&x) }
fn main() { println!("{}", show(&Bytes(vec![7, 2]))); println!("{}", show(&Stack { items: vec![String::from("s")] })); }"#,
        r#"fn render<C: Container>(c: &C) -> Vec<String> where C::Item: std::fmt::Debug { let mut v = Vec::new(); for i in 0..c.len() { if let Some(x) = c.get(i) { v.push(format!("{x:?}")); } } v }
fn main() { println!("{:?}", render(&Stack { items: vec![1i64, 2] })); println!("{:?}", render(&Bytes(vec![9]))); }"#,
        r#"fn pick<C: Container>(c: &C) -> Option<&C::Item> { let r = c.get(1); r.or(c.get(0)) }
fn count<C: Container>(c: &C) -> usize where C::Item: std::fmt::Debug { let mut n = 0; let f = |x: &C::Item| format!("{x:?}").len(); if let Some(x) = pick(c) { n += f(x); } n }
fn main() { let s = Stack { items: vec![10u32, 200] }; println!("{:?} {}", pick(&s), count(&s)); let e: Stack<String> = Stack { items: vec![] }; println!("{:?} {}", pick(&e), count(&e)); }"#,
    ] {
        oracle::stock_native(&format!("{RIGID_CONTAINER}{body}"));
    }
}

#[test]
fn rigid_projection_obligations_reject_unimplemented_or_mismatched_items() {
    for (body, code) in [
        (
            "struct Plain; fn show<C: Container>(c: &C) -> bool { c.get(0).is_some() } fn main() { println!(\"{}\", show(&Plain)); }",
            "E0277",
        ),
        (
            "fn want<C: Container<Item = u8>>(c: &C) -> bool { c.get(0).is_some() } fn main() { println!(\"{}\", want(&Stack { items: vec![1i32] })); }",
            "E0271",
        ),
        (
            "fn unused<C: Container>(c: &C) -> bool { let x: &u8 = c.get(0).unwrap(); *x > 0 } fn main() {}",
            "E0308",
        ),
    ] {
        let source = format!("{RIGID_CONTAINER}{body}");
        oracle::stock_and_native_reject_observed(&source, |workers, check, failure| {
            assert!(
                failure.to_string().contains(code),
                "workers={workers} check={check}: expected {code}: {failure}"
            );
        });
    }
}

#[test]
fn rigid_reference_actual_trait_calls_in_closures_match_stock() {
    // `c.get(i)` inside a closure argument is a TraitItem call whose method
    // actual is a reference; it is admitted through the selected provider's
    // carrier contract, and chained adapters stay within the work budget.
    for body in [
        r#"fn count<C: Container>(c: &C) -> usize { (0..c.len()).filter_map(|i| c.get(i)).count() }
fn main() { println!("{} {}", count(&Stack { items: vec![1i32, 2] }), count(&Bytes(vec![]))); }"#,
        r#"fn show_all<C: Container>(c: &C) -> Vec<String> where C::Item: std::fmt::Debug { (0..c.len()).filter_map(|i| c.get(i)).map(|x| format!("{x:?}")).collect() }
fn main() { let s = Stack { items: vec![1, 2] }; println!("{:?} {:?}", show_all(&s), show_all(&Bytes(vec![7]))); }"#,
    ] {
        oracle::stock_native(&format!("{RIGID_CONTAINER}{body}"));
    }
    for body in [
        "fn firsts<C: Container>(c: &C) -> Vec<&C::Item> { (0..c.len()).filter_map(|i| c.get(i)).collect() } fn main() { let r; { let s = Stack { items: vec![1, 2] }; r = firsts(&s); } println!(\"{}\", r.len()); }",
        "fn firsts<C: Container>(c: &C) -> Vec<&C::Item> { (0..c.len()).filter_map(|i| c.get(i)).collect() } fn main() { let s = Stack { items: vec![1, 2] }; let r = firsts(&s); drop(s); println!(\"{}\", r.len()); }",
    ] {
        oracle::stock_and_native_reject(&format!("{RIGID_CONTAINER}{body}"));
    }
}

#[test]
fn vec_of_mut_refs_element_receivers() {
    oracle::stock_native(
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut b=S{n:4}; let mut v: Vec<&mut S> = vec![&mut a, &mut b]; v[0].bump(); v[1].bump(); v[1].n += 3; drop(v); println!(\"{:?} {:?}\", a, b); }",
    );
    for source in [
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; let x = &mut v[0]; let y = &mut v[0]; x.bump(); y.bump(); }",
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; let x = &mut v[0]; println!(\"{}\", v.len()); x.bump(); }",
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; v[0].bump(); println!(\"{:?}\", a); v[0].bump(); }",
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; let m: S = *v[0]; println!(\"{:?}\", m); }",
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; { let mut b=S{n:2}; v[0] = &mut b; } v[0].bump(); }",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn opaque_loop_then_closure_filter_loop_match_stock() {
    oracle::stock_native(
        r#"
use std::collections::BTreeMap;
fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()) }
fn main() {
    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (ln, line) in "Alpha beta\nbeta Gamma longer".lines().enumerate() { for w in tokens(line) { index.entry(w).or_default().push(ln + 1); } }
    for (w, ls) in index.iter().filter(|(w, _)| w.len() > 4) { println!("{w}: {ls:?}"); }
}
"#,
    );
    oracle::stock_and_native_reject(
        r#"
use std::collections::BTreeMap;
fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(|c: char| !c.is_alphanumeric()).map(|w| w.to_lowercase()) }
fn main() {
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for w in tokens("a b") { index.insert(w, 1); }
    let it = index.iter().filter(|(w, _)| w.len() > 0);
    index.insert(String::new(), 0);
    for (w, n) in it { println!("{w}{n}"); }
}
"#,
    );
}

#[test]
fn take_over_slice_of_reference_pairs_matches_stock() {
    oracle::stock_native(
        r#"
fn main() {
    let a = String::from("alpha"); let b = String::from("be");
    let n = 3usize; let m = 7usize;
    let top: Vec<(&String, &usize)> = vec![(&a, &n), (&b, &m), (&a, &m)];
    println!("{:?}", top.iter().take(2).map(|(w, c)| format!("{w}={c}")).collect::<Vec<_>>());
    for (w, c) in top.iter().skip(1) { println!("{} {}", w.len(), c); }
}
"#,
    );
    for source in [
        "fn main(){ let shown; { let a=String::from(\"x\"); let n=1usize; let top: Vec<(&String,&usize)> = vec![(&a,&n)]; shown = top.iter().take(1).map(|(w, _)| *w).collect::<Vec<&String>>(); } println!(\"{shown:?}\"); }",
        "fn main(){ let mut a=String::from(\"x\"); let n=1usize; let top: Vec<(&String,&usize)> = vec![(&a,&n)]; let mut it = top.iter().take(1); a.push('y'); println!(\"{:?}\", it.next()); }",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn fold_over_opaque_iterator_with_closure_matches_stock() {
    oracle::stock_native(
        r#"
use std::collections::HashMap;
fn toks(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').map(|w| w.to_lowercase()) }
fn main() {
    let n = toks("a Bb ccc").fold(0usize, |acc, w| acc + w.len());
    let m: HashMap<String, usize> = toks("x Y x").fold(HashMap::new(), |mut m, w| { *m.entry(w).or_insert(0) += 1; m });
    let mut v: Vec<_> = m.into_iter().collect(); v.sort();
    println!("{n} {v:?}");
}
"#,
    );
    for source in [
        "fn toks(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').map(|w| w.to_lowercase()) } fn main(){ let it = toks(\"a b\"); let n = it.fold(0usize, |a, w| a + w.len()); let k = it.count(); println!(\"{n}{k}\"); }",
        "fn toks(s: &str) -> impl Iterator<Item = String> + '_ { s.split(' ').map(|w| w.to_lowercase()) } fn main(){ let n: usize = toks(\"a b\").fold(0usize, |a, w| a + w); println!(\"{n}\"); }",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn vec_of_mut_refs_option_element_access() {
    oracle::stock_native(
        "struct S { n: i32 } fn main(){ let mut a=S{n:1}; { let mut v: Vec<&mut S> = vec![&mut a]; if let Some(s) = v.last_mut() { s.n += 1; } } println!(\"{}\", a.n); }",
    );
    oracle::stock_native(
        "fn main(){ let mut x=1i32; { let mut w: Vec<&mut i32> = vec![&mut x]; if let Some(r) = w.get_mut(0) { **r += 5; } } println!(\"{}\", x); }",
    );
    oracle::stock_native(
        "#[derive(Debug)] struct S { n: i32 } impl S { fn bump(&mut self) { self.n += 1; } } fn main(){ let mut a=S{n:1}; let mut b=S{n:4}; let mut x=1i32; let mut y=2i32; { let mut v: Vec<&mut S> = vec![&mut a, &mut b]; if let Some(s) = v.last_mut() { s.bump(); } if let Some(s) = v.get_mut(0) { s.n += 10; } println!(\"{}\", v.get_mut(5).is_none()); v.swap(0, 1); v[0].bump(); } { let mut w: Vec<&mut i32> = vec![&mut x, &mut y]; if let Some(r) = w.last_mut() { **r += 5; } if let Some(r) = w.get_mut(0) { **r *= 3; } } println!(\"{:?} {:?} {} {}\", a, b, x, y); }",
    );
    for source in [
        "struct S { n: i32 } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; let p = v.last_mut().unwrap(); let q = v.get_mut(0).unwrap(); p.n += 1; q.n += 1; }",
        "struct S { n: i32 } fn main(){ let mut a=S{n:1}; let mut v: Vec<&mut S> = vec![&mut a]; let p = v.last_mut().unwrap(); println!(\"{}\", v.len()); p.n += 1; }",
        "fn main(){ let mut a=1i32; let r; { let mut v: Vec<&mut i32> = vec![&mut a]; r = v.last_mut().unwrap(); } **r += 1; }",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
