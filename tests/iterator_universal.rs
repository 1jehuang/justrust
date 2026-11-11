//! Universal associated projections remain exact until closed instantiation.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

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
