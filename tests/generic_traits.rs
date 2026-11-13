//! Type-parameterized trait applications with real checked default/bound/ownership paths.
//! No new trait-argument inference is claimed: multi-application dot remains a distinct boundary.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[path = "support/iterator_oracle.rs"]
mod coherence_oracle;

#[test]
fn array_map_moves_each_element_once_in_index_order() {
    coherence_oracle::stock_native(include_str!("milestones/m2/09_enums_matching.rs"));
    coherence_oracle::stock_native(
        r#"
struct Noisy(i32);
impl Drop for Noisy { fn drop(&mut self) { println!("drop {}", self.0); } }
fn classify(n: i32) -> &'static str { if n < 0 { "neg" } else if n == 0 { "zero" } else { "pos" } }
fn main() {
    println!("{:?}", [-5, 0, 2, 8, 9].map(classify));
    let names = [String::from("a"), String::from("bcd")];
    println!("{:?}", names.map(|s| s.len()));
    let mut calls = Vec::new();
    let doubled = [1u8, 2u8, 3u8].map(|x| { calls.push(x); u32::from(x) * 2 });
    println!("{:?} {:?}", doubled, calls);
    let owned = [Noisy(1), Noisy(2), Noisy(3)].map(|n| { println!("map {}", n.0); n.0 * 10 });
    println!("{:?}", owned);
    let wrapped = [4i32, 5].map(|n| Noisy(n));
    println!("built {}", wrapped.len());
    let empty: [String; 0] = [];
    println!("{}", empty.map(|s| s.len()).len());
    let sum: i32 = [1i32, 2, 3].iter().map(|x| x * 2).sum();
    println!("{} {:?}", sum, Some(4i32).map(|x| x + 1));
}
"#,
    );
    for source in [
        r#"fn main(){let a=[String::from("a")];let _b=a.map(|s|s.len());println!("{}",a[0]);}"#,
        r#"fn unused(){let _b=[1i32,2].map(|x,y|x+y);}fn main(){}"#,
        r#"fn unused(){let _b:[bool;3]=[1i32,2].map(|x|x>0);}fn main(){}"#,
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn const_impl_owned_array_iteration_collection_milestone() {
    coherence_oracle::stock_native(include_str!("milestones/m2/03_collections.rs"));
    coherence_oracle::stock_native(
        r#"fn main(){let a=[String::from("a"),String::from("b")];let mut i=a.into_iter();println!("{} {}",i.next().unwrap(),i.next_back().unwrap());let empty:[String;0]=[];println!("{}",empty.into_iter().count());}"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"fn main(){let a=[String::from("a")];let _i=a.into_iter();println!("{}",a[0]);}"#,
    );
}

#[test]
fn generic_event_bus_milestone() {
    coherence_oracle::stock_native(include_str!("milestones/m3/10_generic_event_bus.rs"));
}

#[test]
fn const_length_array_impls_match_concrete_arrays_by_dot_call() {
    coherence_oracle::stock_native(
        r#"
use std::fmt::Display;
fn sum<const N: usize>(values: &[i32; N]) -> i32 { let mut sum = 0; let mut i = 0; while i < N { let v = &values[i]; sum += *v; i += 1; } sum }
fn render<T: Display, const N: usize>(values: &[T; N]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < N { out.push_str(&format!("<{}>", &values[i])); i += 1; }
    out
}
trait Total { fn total(&self) -> i32; }
impl<const N: usize> Total for [i32; N] {
    fn total(&self) -> i32 { sum(self) }
}
trait Show { fn show(&self) -> String; fn width(self) -> usize; }
impl<T: Display, const N: usize> Show for [T; N] {
    fn show(&self) -> String { render(self) }
    fn width(self) -> usize { N }
}
trait Three { fn three(&self) -> &'static str; }
impl<T> Three for [T; 3] { fn three(&self) -> &'static str { "three" } }
fn main() {
    println!("{} {}", [1, 2, 3].total(), [4i32, 5, 6, 7, 8].total());
    let words = [String::from("a"), String::from("bc")];
    println!("{} {}", words.show(), [1.5f64, 2.0, 3.25].show());
    println!("{} {}", words.width(), [true; 4].width());
    println!("{}", ['x', 'y', 'z'].three());
}
"#,
    );
    for source in [
        "trait Three{fn three(&self)->u8;}impl<T> Three for [T;3]{fn three(&self)->u8{3}}fn unused(){let _=[1i32,2,3,4].three();}fn main(){}",
        "use std::fmt::Display;trait Show{fn show(&self)->String;}impl<T:Display,const N:usize> Show for [T;N]{fn show(&self)->String{String::new()}}struct P;fn unused(){let _=[P,P].show();}fn main(){}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn const_impl_array_heads_keep_owned_element_and_extent_slots() {
    coherence_oracle::stock_native(
        r#"
const KNOWN_LENGTH: usize = 2;
fn const_value<const N: usize>() -> usize { N }
fn forwarded_const<const M: usize>() -> usize { const_value::<M>() }
trait ArrayExtent {
    type Element;
    fn extent(&self) -> usize;
}
impl<T, const N: usize> ArrayExtent for [T; N] {
    type Element = T;
    fn extent(&self) -> usize { N }
}
fn extent<A: ArrayExtent>(array: &A) -> usize { array.extent() }
fn element<A: ArrayExtent>(_: &A, value: A::Element) -> A::Element { value }
trait ArrayIdentity: Sized { fn identity(self) -> Self; }
impl<T, const N: usize> ArrayIdentity for [T; N] { fn identity(self) -> Self { self } }
fn identity<A: ArrayIdentity>(array: A) -> A { array.identity() }
fn main() {
    let numbers = [2i32, 4, 6];
    let flags = [true, false];
    let empty: [i32; 0] = [];
    let owned = [String::from("a"), String::from("b")];
    println!("{} {} {} {}", extent(&numbers), extent(&flags), extent(&empty), extent(&owned));
    println!("{} {}", element(&numbers, 9i32), element(&owned, String::from("owned")));
    let a = 17i32;
    let b = 23i32;
    let references = identity([(&a, &b), (&b, &a)]);
    println!("{} {}", *references[0].0, *references[1].0);
    let moved = identity([String::from("affine"), String::from("array")]);
    println!("{} {}", moved[0], moved[1]);
    let known: [i32; KNOWN_LENGTH] = [31, 32];
    println!("{} {} {}", extent(&known), const_value::<4>(), forwarded_const::<5>());
}
"#,
    );
}

#[test]
fn const_impl_array_heads_reject_changed_scope_kind_and_unbound_slots() {
    for source in [
        "trait A{} impl<T,const N:bool>A for [T;N]{}fn main(){}",
        "trait A{} impl<T,const N:usize>A for [T;M]{}fn main(){}",
        "trait A{} impl<T,const N:usize>A for T{}fn main(){}",
        "trait A{fn value(&self)->bool;}impl<T,const N:usize>A for [T;N]{fn value(&self)->bool{N}}fn main(){}",
        "trait A{type Item;}impl<T,const N:usize>A for [T;N]{type Item=T;}fn take<X:A>(_:X,_:X::Item){}fn unused(){take([1i32,2],true);}fn main(){}",
        "fn ping<const N:usize>(){}fn unused(){ping();}fn main(){}",
        "fn ping<const N:usize>(){}fn unused(){ping::<true>();}fn main(){}",
        "fn unused<const N:usize>(_: [i32;M]){}fn main(){}",
        "trait A{}impl<T,N>A for[T;N]{}fn main(){}",
        "trait A:Sized{fn identity(self)->Self;}impl<T,const N:usize>A for[T;N]{fn identity(self)->Self{self}}fn bad()->[&'static i32;1]{let local=1i32;[&local].identity()}fn main(){}",
        r#"trait A:Sized{fn identity(self)->Self;}impl<T,const N:usize>A for[T;N]{fn identity(self)->Self{self}}fn unused(){let a=[String::from("a")];let b=a.identity();println!("{}",a[0]);}fn main(){}"#,
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn const_impl_direct_array_inputs_infer_element_and_extent_in_source_order() {
    coherence_oracle::stock_native(
        r#"
fn const_first<const N: usize, T>(_: &[T; N]) -> usize { N }
fn type_first<T, const N: usize>(_: &[T; N]) -> usize { N }
fn same_extent<T, U, const N: usize>(_: &[T; N], _: &[U; N]) -> usize { N }
fn first<T, const N: usize>(array: &[T; N]) -> &T { &array[0] }
fn at<T, const N: usize>(array: &[T; N], index: usize) -> &T { &array[index] }
fn at_mut<T, const N: usize>(array: &mut [T; N], index: usize) -> &mut T { &mut array[index] }
fn nested<T, const N: usize, const M: usize>(array: &[[T; M]; N], row: usize, column: usize) -> &T { &array[row][column] }
fn forwarded<T, const N: usize>(array: &[T; N]) -> usize { const_first(array) }
fn main() {
    let numbers = [3i32, 5, 7];
    let flags = [true, false];
    let empty: [u8; 0] = [];
    let owned = [String::from("a"), String::from("b")];
    println!("{} {} {} {}", const_first(&numbers), type_first(&flags), const_first(&empty), type_first(&owned));
    println!("{} {} {}", const_first::<3, i32>(&numbers), type_first::<bool, 2>(&flags), same_extent(&flags, &owned));
    println!("{} {}", first(&numbers), forwarded(&owned));
    let mut changed = [11i32, 13];
    *at_mut(&mut changed, 1) = 17;
    println!("{} {}", at(&numbers, 2), at(&changed, 1));
    let rows = [[19i32, 23], [29, 31]];
    println!("{}", nested(&rows, 1, 0));
    println!("{} {}", first(&owned), at(&owned, 1));
}
"#,
    );
    for source in [
        "fn take<T,const N:usize>(_:&[T;N],_:&[T;N]){}fn unused(){take(&[1i32,2],&[3i32]);}fn main(){}",
        "fn take<const N:usize,T>(_:&[T;N]){}fn unused(){take::<3,i32>(&[1i32,2]);}fn main(){}",
        "fn take<const N:usize,T>(_:&[T;N]){}fn unused(){take::<true,i32>(&[1i32]);}fn main(){}",
        "fn take<T,const N:usize>(_:&[T;N],_:&[T;N]){}fn unused(){take(&[1i32],&[true]);}fn main(){}",
        "fn bad<T,const N:usize>(array:&[T;N],i:usize)->&mut T{&mut array[i]}fn main(){}",
        "fn at<T,const N:usize>(a:&[T;N])->&T{&a[0]}fn bad()->&'static i32{let array=[1i32];at(&array)}fn main(){}",
        "fn bad<T,const N:usize>(a:&mut[T;N]){let x=&mut a[0];let y=&a[1];let _pair=(x,y);}fn main(){}",
        "fn bad<T,const N:usize>(a:&mut[T;N],i:usize,j:usize){let x=&mut a[i];let y=&mut a[j];let _pair=(x,y);}fn main(){}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
    // A symbolic extent is not proof that an element exists. Even an unused
    // returned reference must perform the genuine runtime bounds check.
    use std::os::unix::fs::PermissionsExt;
    let trap_source = "fn at<T,const N:usize>(a:&[T;N],i:usize)->&T{&a[i]}fn main(){let a:[i32;0]=[];println!(\"before\");let _borrowed=at(&a,0);println!(\"after\");}";
    let scratch = Scratch::new();
    scratch.write("empty.rs", trap_source.as_bytes());
    let stock = scratch.path("empty-stock");
    let compiled = capture(
        Command::new(coherence_oracle::closure_oracle::pinned_stock::stock_rustc())
            .arg("--edition=2024")
            .arg("-Cpanic=abort")
            .arg(scratch.path("empty.rs"))
            .arg("-o")
            .arg(&stock),
        &scratch,
        30,
    );
    assert_eq!(compiled.code, 0, "{compiled:?}");
    let expected = capture_with_signal(&mut Command::new(&stock), &scratch, 5, Some(6));
    assert_eq!(expected.code, -6, "{expected:?}");
    assert_eq!(expected.stdout, b"before\n");
    for workers in [1, 8] {
        let checked = compiler::compile(trap_source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| key.kind != 103)
        );
        let built = compiler::compile(trap_source.into(), workers, false).unwrap();
        let path = scratch.path(&format!("empty-native-{workers}"));
        fs::write(&path, built.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = capture_with_signal(&mut Command::new(&path), &scratch, 5, Some(6));
        assert_eq!(actual.code, expected.code, "{actual:?}");
        assert!(
            String::from_utf8_lossy(&actual.stderr)
                .contains("index out of bounds: the len is 0 but the index is 0"),
            "{actual:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "{actual:?}");
    }
}

#[test]
fn user_const_generic_structs_impls_and_functions() {
    coherence_oracle::stock_native(
        r#"
struct Buf<T, const N: usize> { items: [T; N] }
impl<T: Copy + std::fmt::Debug, const N: usize> Buf<T, N> {
    fn len(&self) -> usize { N }
    fn get(&self, i: usize) -> T { self.items[i] }
    fn sum_with(&self, f: impl Fn(T) -> i64) -> i64 { let mut s = 0; for x in &self.items { s += f(*x); } s }
}
struct Matrix<const R: usize, const C: usize> { d: [[i32; C]; R] }
impl<const R: usize, const C: usize> Matrix<R, C> {
    fn transpose(&self) -> Matrix<C, R> {
        let mut d = [[0; R]; C];
        for i in 0..R { for j in 0..C { d[j][i] = self.d[i][j]; } }
        Matrix { d }
    }
}
struct Flags<const ON: bool>(u8);
impl<const ON: bool> Flags<ON> { fn on(&self) -> bool { ON } }
fn fill<const N: usize>(v: u8) -> [u8; N] { [v; N] }
fn total<T: Copy + Into<i64>, const N: usize>(values: &[T; N]) -> i64 {
    let mut s = 0; let mut i = 0;
    while i < N { s += values[i].into(); i += 1; }
    for x in values { s += (*x).into(); }
    s
}
fn first_last<const N: usize>(a: Buf<i32, N>) -> (i32, i32) { (a.get(0), a.get(N - 1)) }
fn main() {
    let b = Buf { items: [3i32, 1, 4, 1, 5] };
    println!("{} {} {}", b.len(), b.get(2), b.sum_with(|x| x as i64 * 2));
    let c: Buf<i32, 5> = b;
    println!("{:?} {}", first_last(c), Buf::<char, 2> { items: ['a', 'b'] }.get(1));
    let m = Matrix { d: [[1, 2, 3], [4, 5, 6]] };
    let t = m.transpose();
    println!("{} {} {}", t.d[2][1], t.d[0][1], t.d.len());
    println!("{} {}", Flags::<true>(0).on(), Flags::<false>(0).on());
    let on: Flags<true> = Flags(0);
    let off: Flags<false> = Flags(0);
    println!("{} {}", on.on(), off.on());
    println!("{:?} {:?}", fill::<3>(7), fill::<0>(1));
    println!("{} {}", total(&[1u8, 2, 3]), total(&[10i32; 4]));
}
"#,
    );
    for source in [
        "struct S<const N: usize>; fn unused() { let _: S<u8> = S; } fn main() {}",
        "struct S<T>(T); fn unused() { let _: S<3> = S(1); } fn main() {}",
        "struct S<const N: usize>([i32; N]); fn unused() { let _: S<1, 2> = S([0]); } fn main() {}",
        "struct S<const N: usize>([i32; N]); fn unused() { let _: S<3> = S([1, 2, 3, 4]); } fn main() {}",
        "fn take(_: [i32; 3]) {} fn unused() { take([1, 2, 3, 4]); } fn main() {}",
        "fn n<const N: usize>() -> usize { N } fn unused() { n::<1, 2>(); } fn main() {}",
        "struct S<const N: usize>(u8); impl<const N: usize> S<N> { fn f(&self) -> bool { N } } fn main() {}",
        "fn fill<T, const N: usize>(v: T) -> [T; N] { [v; N] } fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn inherent_method_arguments_follow_unique_trait_applications() {
    coherence_oracle::stock_native(
        r#"
trait Pick<T> { fn pick(self) -> T; }
struct Wrapped<T> { value: T }
impl<T> Pick<T> for Wrapped<T> { fn pick(self) -> T { self.value } }
struct Hub;
impl Hub {
    fn take<Q, R: Pick<Q>>(&self, value: R) -> Q { value.pick() }
}
fn main() {
    let hub = Hub;
    println!("{} {}", hub.take(Wrapped { value: 7i32 }), hub.take(Wrapped { value: true }));
}
"#,
    );
    for source in [
        r#"trait Pick<T>{fn pick(self)->T;} struct Hub; impl Hub{fn take<Q,R:Pick<Q>>(&self,r:R)->Q{r.pick()}} struct Missing; fn unused(){let hub=Hub; let _:i32=hub.take(Missing);} fn main(){}"#,
        r#"trait Pick<T>{fn pick(self)->T;} struct Wrapped<T>{value:T} impl<T>Pick<T> for Wrapped<T>{fn pick(self)->T{self.value}} struct Hub; impl Hub{fn take<Q,R:Pick<Q>>(&self,r:R)->Q{r.pick()}} fn unused(){let hub=Hub; let _:bool=hub.take(Wrapped{value:7i32});} fn main(){}"#,
        r#"trait Pick<T>{fn pick(self)->T;} struct Dual; impl Pick<i32>for Dual{fn pick(self)->i32{7}} impl Pick<bool>for Dual{fn pick(self)->bool{true}} struct Hub; impl Hub{fn take<Q,R:Pick<Q>>(&self,r:R)->Q{r.pick()}} fn unused(){let hub=Hub; let value=hub.take(Dual);} fn main(){}"#,
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn generic_parameter_projection_membership_can_follow_a_blanket_impl() {
    let protocol = r#"
trait MyInto {
    type Item;
    type IntoIter: Iterator<Item = Self::Item>;
    fn into_it(self) -> Self::IntoIter;
}
impl<I: Iterator> MyInto for I {
    type Item = I::Item;
    type IntoIter = I;
    fn into_it(self) -> I { self }
}
fn unused_identity<I: Iterator>(it: I) -> I { it.into_it() }
"#;
    coherence_oracle::stock_native(&format!(
        r#"{protocol}
fn total<I: Iterator<Item = i32>>(it: I) -> i32 {{
    let mut a = 0;
    let mut j = it.into_it();
    while let Some(x) = j.next() {{ a += x; }}
    a
}}
fn main() {{ println!("{{}}", total(vec![1, 2, 3].into_iter())); }}
"#
    ));
    coherence_oracle::stock_native(
        r#"
trait Read { type Item; fn read(self) -> Self::Item; }
impl<T: Copy> Read for &T {
    type Item = T;
    fn read(self) -> T { *self }
}
fn get<T: Copy>(value: &T) -> T { value.read() }
fn main() { println!("{}", get(&7i32)); }
"#,
    );
    coherence_oracle::stock_native(
        r#"
trait Echo { type Out; fn echo(self) -> Self::Out; }
impl<T> Echo for T { type Out = T; fn echo(self) -> T { self } }
fn identity<T>(value: T) -> T { value.echo() }
fn main() { println!("{} {}", identity(7i32), identity(true)); }
"#,
    );
    for body in [
        // Blanket membership still requires the Iterator bound, even unused.
        "fn unused<I>(it: I) -> I { it.into_it() }",
        // Normalization must preserve the exact associated Item equality.
        "fn unused<I: Iterator<Item = u8>>(it: I) -> i32 { let mut j = it.into_it(); j.next().unwrap() }",
    ] {
        coherence_oracle::stock_and_native_reject(&format!("{protocol}\n{body}\nfn main() {{}}"));
    }
}

#[test]
fn blanket_projection_normalization_composes_caller_equalities() {
    let protocol = r#"
trait MyInto {
    type Item;
    type IntoIter: Iterator<Item = Self::Item>;
    fn into_it(self) -> Self::IntoIter;
}
impl<I: Iterator> MyInto for I {
    type Item = I::Item;
    type IntoIter = I;
    fn into_it(self) -> I { self }
}
trait BorrowItem { type Item; }
impl<I: Iterator> BorrowItem for &mut I {
    type Item = I::Item;
}
"#;
    coherence_oracle::stock_native(&format!(
        r#"{protocol}
fn first<I: Iterator<Item = i32>>(it: I) -> i32 {{
    let mut j = it.into_it();
    let x: <I as MyInto>::Item = j.next().unwrap();
    let v: i32 = x;
    v
}}
fn borrowed<I: Iterator<Item = i32>>(it: &mut I) -> i32 {{
    let x: <&mut I as BorrowItem>::Item = it.next().unwrap();
    let v: i32 = x;
    v
}}
fn main() {{
    let mut it = vec![4, 5].into_iter();
    println!("{{}} {{}}", first(vec![1, 2].into_iter()), borrowed(&mut it));
}}
"#
    ));
    for body in [
        "fn unused<I: Iterator<Item = u8>>(it: I) -> i32 { let mut j = it.into_it(); let x: <I as MyInto>::Item = j.next().unwrap(); x }",
        "fn unused<I>(it: I) -> i32 { let mut j = it.into_it(); let x: <I as MyInto>::Item = j.next().unwrap(); x }",
    ] {
        coherence_oracle::stock_and_native_reject(&format!("{protocol}\n{body}\nfn main() {{}}"));
    }
}

#[test]
fn qualified_associated_types_require_direct_trait_ownership() {
    for source in [
        r#"
mod model {
    pub trait Parent<A> { type Item; }
    pub trait Child<A>: Parent<A> {}
}
use model::Child as Bound;
struct Value { n: i32 }
impl model::Parent<i32> for Value { type Item = i32; }
impl model::Child<i32> for Value {}
fn identity<I: Bound<i32>>(x: I::Item) -> I::Item { x }
fn main() { println!("{}", identity::<Value>(7)); }
"#,
        r#"
trait Parent { type Item; }
trait Child: Parent { type Item; }
struct Value { n: i32 }
impl Parent for Value { type Item = i32; }
impl Child for Value { type Item = bool; }
fn identity<I: Child>(x: <I as Child>::Item) -> <I as Child>::Item { x }
fn main() { println!("{}", identity::<Value>(true)); }
"#,
        r#"
trait Parent { type Item; fn next(&mut self) -> Option<Self::Item>; }
trait Child: Parent { fn back(&mut self) -> Option<Self::Item>; }
impl<I: Parent> Parent for &mut I {
    type Item = I::Item;
    fn next(&mut self) -> Option<I::Item> { (**self).next() }
}
impl<I: Child> Child for &mut I {
    fn back(&mut self) -> Option<I::Item> { (**self).back() }
}
struct Value { n: i32 }
impl Parent for Value {
    type Item = i32;
    fn next(&mut self) -> Option<i32> { Some(self.n) }
}
impl Child for Value { fn back(&mut self) -> Option<i32> { Some(self.n) } }
fn main() { let mut value = Value { n: 9 }; let mut it = &mut value; println!("{}", it.back().unwrap()); }
"#,
    ] {
        coherence_oracle::stock_native(source);
    }
    for source in [
        "trait Parent{type Item;}trait Child:Parent{}fn unused<I:Child>(x:<I as Child>::Item){}fn main(){}",
        "trait Parent{type Item;}trait Child:Parent{}struct Unused<I:Child>{value:<I as Child>::Item}fn main(){}",
        "mod m{pub trait Parent{type Item;}pub trait Child:Parent{}}use m::Child as Bound;fn unused<I:Bound>(x:<I as Bound>::Item){}fn main(){}",
        "trait Parent{type Item;}trait Child:Parent{type Item;}fn unused<I:Child>(x:I::Item){}fn main(){}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn builtin_callable_bounds_are_disjoint_from_non_callable_constructors() {
    for source in [
        r#"trait Pattern{fn matches(&mut self,c:char)->bool;}
impl Pattern for char{fn matches(&mut self,c:char)->bool{*self==c}}
impl<F:FnMut(char)->bool> Pattern for F{fn matches(&mut self,c:char)->bool{self(c)}}
fn main(){let mut p=|c:char|c=='!';println!("{} {}",p.matches('!'),p.matches('x'));let mut q=',';println!("{}",q.matches(','));}"#,
        r#"trait Tag{fn tag(&self)->i32;}
impl<F:FnOnce()> Tag for F{fn tag(&self)->i32{1}}
impl Tag for char{fn tag(&self)->i32{2}}
impl Tag for &str{fn tag(&self)->i32{3}}
impl<T> Tag for (T,){fn tag(&self)->i32{4}}
impl<T> Tag for [T;2]{fn tag(&self)->i32{5}}
impl<T> Tag for *const T{fn tag(&self)->i32{6}}
fn show<T:Tag>(x:T)->i32{x.tag()}
fn main(){let p=0usize as *const i32;println!("{} {} {} {} {} {}",show(||{}),show('x'),show("é"),show((true,)),show([1,2]),show(p));}"#,
    ] {
        coherence_oracle::stock_native(source);
    }
    for source in [
        "trait Tag{}impl<F:FnMut(char)->bool> Tag for F{}impl Tag for fn(char)->bool{}fn main(){}",
        "trait Tag{}impl<F:FnMut(char)->bool> Tag for F{}impl<T> Tag for &T{}fn main(){}",
        "trait Tag{}impl<F:FnOnce()> Tag for F{}impl<F:FnMut()> Tag for F{}fn main(){}",
        // A user trait with the same spelling is not a builtin call capability.
        "trait FnMut{}impl FnMut for char{}trait Tag{}impl<F:FnMut> Tag for F{}impl Tag for char{}fn main(){}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn conditional_blanket_impls_use_orphan_aware_disjointness() {
    for source in [
        r#"
trait Mark {}
trait Tag { fn tag(self) -> usize; }
struct A { n: usize }
impl Mark for A {}
impl<T: Mark> Tag for T { fn tag(self) -> usize { 7 } }
impl<T> Tag for Vec<T> { fn tag(self) -> usize { self.len() } }
fn main() { println!("{} {}", A { n: 1 }.tag(), vec![1, 2, 3].tag()); }
"#,
        r#"
trait Tag { fn tag(self) -> i32; }
struct A { n: i32 }
impl<T: std::fmt::Display> Tag for T { fn tag(self) -> i32 { 7 } }
impl Tag for A { fn tag(self) -> i32 { self.n } }
fn main() { println!("{} {}", A { n: 42 }.tag(), 1i32.tag()); }
"#,
        r#"
struct A { n: u8 }
trait Tag { fn tag(self) -> i32; }
impl<T: From<A>> Tag for T { fn tag(self) -> i32 { 7 } }
impl Tag for u8 { fn tag(self) -> i32 { 42 } }
fn main() { println!("{} {}", 1u8.tag(), A { n: 1 }.tag()); }
"#,
    ] {
        coherence_oracle::stock_native(source);
    }
    for source in [
        // Upstream may add Display for Vec. Absence now proves nothing.
        "trait Foo {} impl<T: std::fmt::Display> Foo for T {} impl Foo for Vec<u8> {} fn main() {}",
        "trait Foo {} impl<T: From<u16>> Foo for T {} impl Foo for u8 {} fn main() {}",
        // The bound really is satisfied, including when both impls are unused.
        "struct A { n: i32 } trait Mark {} impl Mark for A {} trait Foo {} impl<T: Mark> Foo for T {} impl Foo for A {} fn main() {}",
        // Downstream can implement Mark for a reference to its own type.
        "trait Mark {} trait Foo {} impl<T: Mark> Foo for T {} impl<U> Foo for &U {} fn main() {}",
        "trait A {} trait B {} trait Foo {} impl<T: A> Foo for T {} impl<T: B> Foo for T {} fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Final28 stock JSON SHA256 e62a91d49ac9adf66a9d2186f7d999d52ca4b574a0c03ec86a860f3726858c09
// Original11 measured baseline source bytes/order retained unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
    InvalidUnsupported,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    outcome: Outcome,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "concrete_ufcs",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rigid_forward",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T> Take<T> for S{fn take(self,x:T)->T{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){if forward(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "distinct_applications",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl Get<i32> for S{fn get(self)->i32{42}}impl Get<bool> for S{fn get(self)->bool{true}}fn main(){if <S as Get<bool>>::get(S){if <S as Get<i32>>::get(S)==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "input_inferred_method",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}impl Take<bool> for S{fn take(self,x:bool)->bool{x}}fn main(){if S.take(true){if S.take(42i32)==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "closure_inferred_open_argument",
            source: r#"trait Conv<U>{fn conv<F:Fn(i32)->U>(self,f:F)->U;}struct S(i32);impl<U> Conv<U> for S{fn conv<F:Fn(i32)->U>(self,f:F)->U{f(self.0)}}fn main(){let b=S(4).conv(|x|x>3);let n=S(5).conv(|x|x as u8*2);if b{println!("{}",n);}}"#,
            stdout: Some("10\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "uninferable_open_argument",
            source: r#"trait Make<U>{fn make(self)->usize;}struct S;impl<U> Make<U> for S{fn make(self)->usize{0}}fn unused(){let _n=S.make();}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0282]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "closure_open_argument_wrong_result",
            source: r#"trait Conv<U>{fn conv<F:Fn(i32)->U>(self,f:F)->U;}struct S(i32);impl<U> Conv<U> for S{fn conv<F:Fn(i32)->U>(self,f:F)->U{f(self.0)}}fn unused(){let _b:bool=S(4).conv(|x|x+1);}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "ambiguous_application",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl Get<i32> for S{fn get(self)->i32{42}}impl Get<bool> for S{fn get(self)->bool{true}}fn main(){let x=S.get();}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0283]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "wrong_impl_signature",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:bool)->i32{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_trait_argument",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}fn forward<U:Take>(u:U){}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0107]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_application_bound",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){let x=forward(S,true);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "universal_bad_body",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T> Take<T> for S{fn take(self,x:T)->T{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "universal_noncopy_move",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}fn forward<T,U:Take<T>>(u:U,x:T)->T{let y=x;u.take(x)}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "overlapping_same_application",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl<T> Get<T> for S{fn get(self)->T{loop{}}}impl Get<i32> for S{fn get(self)->i32{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0119]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "default_method",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{x}}struct S;impl Take<i32> for S{}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "supertrait_forward",
            source: r#"trait Parent<T>{fn take(self,x:T)->T;}trait Child<T>:Parent<T>{}struct S;impl<T> Parent<T> for S{fn take(self,x:T)->T{x}}impl<T> Child<T> for S{}fn forward<T,U:Child<T>>(u:U,x:T)->T{u.take(x)}fn main(){if forward(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_parameter_bound",
            source: r#"trait Take<T:Copy>:Sized{fn take(self,x:T)->T{let y=x;x}}struct S;impl<T:Copy> Take<T> for S{}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "two_ordered_trait_arguments",
            source: r#"trait Take<T:Copy,U:Copy>{fn take(self,x:T,y:U)->T;}struct S;impl Take<i32,bool> for S{fn take(self,x:i32,y:bool)->i32{if y{x}else{0}}}fn main(){if <S as Take<i32,bool>>::take(S,42,true)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_argument_only_impl_parameter",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct Carrier{n:i32}impl<T> Take<T> for Carrier{fn take(self,x:T)->T{x}}fn main(){if <Carrier as Take<i32>>::take(Carrier{n:7},42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_trait_application",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn main(){macro_rules! take{($x:literal)=>{<S as Take<i32>>::take(S,$x)};}if take!(42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_trait_alias",
            source: r#"mod m{pub trait Take<T>{fn take(self,x:T)->T;}pub struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}}use m::Take as Alias;fn main(){if <m::S as Alias<i32>>::take(m::S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "universal_noncopy_forward",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;struct Cell{n:i32}impl<T> Take<T> for S{fn take(self,x:T)->T{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){let cell=forward(S,Cell{n:42});if cell.n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "associated_function_application",
            source: r#"trait Take<T>{fn take(x:T)->T;}struct S;impl<T> Take<T> for S{fn take(x:T)->T{x}}fn main(){if <S as Take<i32>>::take(42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_argument_bound_violation",
            source: r#"trait Take<T:Copy>{fn take(self,x:T)->T;}struct S;struct Cell{n:i32}impl Take<Cell> for S{fn take(self,x:Cell)->Cell{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unconstrained_impl_parameter",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T,U> Take<T> for S{fn take(self,x:T)->T{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0207]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "excess_trait_arity",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32,bool> for S{fn take(self,x:i32)->i32{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0107]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_required_item",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unused_bad_default",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_impl_return",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->bool{true}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_supertrait_application",
            source: r#"trait Parent<T>{fn take(self,x:T)->T;}trait Child<T>:Parent<T>{}struct S;impl Parent<bool> for S{fn take(self,x:bool)->bool{x}}impl Child<i32> for S{}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unused_default_noncopy_move",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{let y=x;x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
    ]
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "jr-generic-traits-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let mut file = new_file(&self.path(name));
        file.write_all(bytes).unwrap();
        drop(file);
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Debug)]
struct Captured {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    capture_with_signal(command, scratch, seconds, None)
}
fn capture_with_signal(
    command: &mut Command,
    scratch: &Scratch,
    seconds: u64,
    expected_signal: Option<i32>,
) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            // Retry only Linux pre-spawn ETXTBSY, never an executed child.
            Err(error) if error.raw_os_error() == Some(26) && retries < 20 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("spawn {command:?}: {error}"),
        }
    };
    let mut child = Running(child);
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "output bound: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    let code = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        let signal = status.signal();
        assert_eq!(
            signal, expected_signal,
            "unexpected signal: {command:?}: {status:?}"
        );
        -signal.expect("missing termination signal")
    });
    Captured {
        code,
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn runtime(path: &Path, expected: &str, scratch: &Scratch) {
    assert!(!expected.is_empty(), "runtime oracle cannot be empty");
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch, 10);
    assert_eq!(version.code, 0);
    eprintln!(
        "GENERIC TRAITS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Invalid => partition.1 += 1,
            Outcome::Unsupported => partition.2 += 1,
            Outcome::InvalidUnsupported => partition.3 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=generic_traits_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
        if let Some(expected) = case.stdout {
            counts.0 += 1;
            assert!(case.stock_diagnostic.is_none());
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            counts.1 += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains(case.stock_diagnostic.expect("measured invalid category")),
                "{}:{diagnostic}",
                case.name
            );
            assert!(!image.exists());
        }
    }
    assert_eq!(counts, (14, 17));
    assert_eq!(partition, (14, 15, 0, 2));
}
#[test]
fn stock_nested_generic_traits_oracle_edition2024_actual_toolchain() {
    stock_all();
}
#[test]
fn source_trait_binders_arguments_defaults_and_bounds_remain_genuine() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed =
            frontend::parse_prepared(&prepared).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let program = &parsed.program;
        assert!(!program.traits.is_empty());
        for declaration in &program.traits {
            assert!(!declaration.type_params.is_empty());
            for parameter in &declaration.type_params {
                assert!(!program.names.spelling(*parameter).unwrap().is_empty());
            }
        }
        let mut applications = 0;
        for implementation in &program.impls {
            if let Some(frontend::TraitPath::Source(path)) = &implementation.trait_path {
                assert!(path.segments.iter().any(|segment| !segment.args.is_empty()));
                applications += 1;
            }
        }
        assert!(applications > 0);
        if case.name == "trait_parameter_bound" || case.name == "two_ordered_trait_arguments" {
            assert!(
                program
                    .traits
                    .iter()
                    .any(|declaration| !declaration.bounds.is_empty())
            );
        }
        if case.name == "two_ordered_trait_arguments" {
            assert_eq!(program.traits[0].type_params.len(), 2);
        }
        if matches!(case.name, "default_method" | "trait_parameter_bound") {
            assert!(
                program
                    .traits
                    .iter()
                    .flat_map(|declaration| &declaration.items)
                    .any(|item| item.default.is_some())
            );
        }
        count += 1;
    }
    assert_eq!(count, 14);
}
#[test]
fn checked_trait_applications_keep_rigid_argument_identity_and_universal_ownership() {
    let mut count = 0;
    let mut rigid = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        // Complete public pipeline checks trait defaults and unused impl methods too.
        compiler::compile_observed(case.source.to_owned(), 1, true)
            .unwrap_or_else(|e| panic!("{}:{e}", case.name));
        count += 1;
        // The prelude-free legacy resolver has no `Fn`; the full pipeline
        // above already checked this case.
        if case.name == "closure_inferred_open_argument" {
            continue;
        }
        count -= 1;
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let signatures = frontend::signature_table(&program).unwrap();
        if case.name == "macro_trait_application" {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let mut own_rigid = 0;
        for index in 0..program.functions.len() {
            frontend::check_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} function{index}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            if program.functions[index].type_params.is_empty() {
                function.validate().unwrap();
            }
            for expression in &function.exprs {
                if let ir::ExprKind::Call {
                    target: ir::Callee::TraitItem(call),
                    ..
                } = &expression.kind
                {
                    assert!(
                        !call.trait_ref.args.is_empty(),
                        "{} must not erase trait application arguments",
                        case.name
                    );
                    assert!(
                        call.method_args.is_empty()
                            || case.name == "closure_inferred_open_argument",
                        "trait arguments are not method arguments"
                    );
                    if matches!(call.trait_ref.self_ty, ir::Ty::Param(_)) {
                        assert_eq!(call.trait_ref.args.len(), 1);
                        assert!(matches!(call.trait_ref.args[0], ir::Ty::Param(_)));
                        own_rigid += 1;
                    }
                }
            }
        }
        if matches!(
            case.name,
            "rigid_forward" | "supertrait_forward" | "universal_noncopy_forward"
        ) {
            assert!(
                own_rigid > 0,
                "{} lost real rigid trait-argument proof transport",
                case.name
            );
            rigid += 1;
        }
        count += 1;
    }
    assert_eq!(count, 14);
    assert_eq!(rigid, 3);
}
#[test]
fn bad_application_arity_signatures_bounds_overlap_and_unused_generic_bodies_are_invalid() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(error) => error,
            Ok(_) => panic!("{} invalid accepted", case.name),
        };
        let message = error.to_string();
        assert!(!message.is_empty());
        assert!(
            !message.contains("unsupported:"),
            "{} ordinary error must not become Unsupported:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 15);
}
#[test]
fn multiple_trait_applications_do_not_claim_new_dot_argument_inference() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.outcome,
            Outcome::Unsupported | Outcome::InvalidUnsupported
        )
    }) {
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        frontend::resolve(&parsed).unwrap();
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(error) => error,
            Ok(_) => panic!("{} unadmitted trait-argument inference accepted", case.name),
        };
        let message = error.to_string();
        assert!(message.contains("unsupported:"), "{}:{message}", case.name);
        assert!(
            message.contains("multiple trait applications require explicit UFCS; trait argument inference is unsupported"),
            "{}:{message}",
            case.name
        );
        match case.outcome {
            Outcome::Unsupported => {
                assert!(case.stdout.is_some());
                assert!(case.stock_diagnostic.is_none());
                counts.0 += 1;
            }
            Outcome::InvalidUnsupported => {
                assert!(case.stdout.is_none());
                assert!(matches!(
                    case.stock_diagnostic,
                    Some("error[E0283]" | "error[E0282]")
                ));
                counts.1 += 1;
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(counts, (0, 2));
}
#[test]
fn native_generic_traits_workers1_8_check_build_no_compiler_fallback() {
    // jr-rust --profile emits telemetry, not optimization. The coordinator runs
    // this same target under debug and release BOOTSTRAP compiler profiles.
    // Neither run asserts a nonexistent optimized native code-generation mode.
    stock_all();
    let mut gates = 0;
    let mut modes = (0, 0, 0, 0, 0, 0, 0, 0);
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.write("client.rs", case.source.as_bytes());
                let image = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !diagnostic.contains("panicked at"),
                    "{}:{diagnostic}",
                    case.name
                );
                assert_eq!(
                    result.code,
                    i32::from(case.outcome != Outcome::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.outcome != Outcome::Runtime {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    if matches!(
                        case.outcome,
                        Outcome::Unsupported | Outcome::InvalidUnsupported
                    ) {
                        assert!(
                            diagnostic.contains("unsupported:"),
                            "{} explicit boundary:{diagnostic}",
                            case.name
                        );
                    } else {
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                    }
                }
                if !check && case.outcome == Outcome::Runtime {
                    runtime(
                        &image,
                        case.stdout.expect("runtime source oracle"),
                        &scratch,
                    );
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path} {}",
                            case.name
                        );
                    }
                }
                match (case.outcome, check) {
                    (Outcome::Runtime, true) => modes.0 += 1,
                    (Outcome::Runtime, false) => modes.1 += 1,
                    (Outcome::Invalid, true) => modes.2 += 1,
                    (Outcome::Invalid, false) => modes.3 += 1,
                    (Outcome::Unsupported, true) => modes.4 += 1,
                    (Outcome::Unsupported, false) => modes.5 += 1,
                    (Outcome::InvalidUnsupported, true) => modes.6 += 1,
                    (Outcome::InvalidUnsupported, false) => modes.7 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 124);
    assert_eq!(modes, (28, 28, 30, 30, 0, 0, 4, 4));
    eprintln!(
        "NATIVE GENERIC TRAITS:124 cells, modes{modes:?};28 nonempty-output main images;4 stock-invalid/native-Unsupported cells are distinct boundaries, not ordinary-invalid parity or full Rust support"
    );
}

#[test]
fn generic_function_item_callable_output() {
    coherence_oracle::stock_native(
        r#"
fn make<T, F: FnOnce() -> Vec<T>>(f: F) -> Vec<T> { f() }
fn pair<T, F: Fn() -> T>(f: F) -> (T, T) { (f(), f()) }
fn seeded<T: Clone>(x: &T) -> Vec<T> { let mut v = make(Vec::new); v.push(x.clone()); v }
fn empties<T>() -> usize { let (a, b): (Vec<T>, Vec<T>) = pair(Vec::new); a.len() + b.len() }
fn main() {
    println!("{:?} {:?}", seeded(&5), seeded(&String::from("s")));
    println!("{}", empties::<String>());
}
"#,
    );
    for source in [
        "fn make<T, F: FnOnce() -> Vec<T>>(f: F) -> Vec<T> { f() } fn unused<T>() { let v: Vec<T> = make(Vec::new); let a = v; let b = v; drop((a, b)); } fn main() {}",
        "fn make<T, F: FnOnce() -> Vec<T>>(f: F) -> Vec<T> { f() } fn unused<T>() -> Vec<u8> { make::<T, _>(Vec::new) } fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn trait_impl_producers_blanket_into_and_indexed_comparison_operands() {
    coherence_oracle::stock_native(
        r#"
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Dog { name: String }
impl From<&str> for Dog { fn from(s: &str) -> Dog { Dog { name: s.to_string() } } }
fn id<T>(_: &T) -> usize { 1 }
fn main() {
    let d: Dog = "rex".into();
    let e = Dog::from("ace");
    println!("{} {}", id(&d), id(&e));
    let dogs = vec![e.clone(), d.clone()];
    println!("{} {} {}", d == dogs[1], d > dogs[0], dogs[0] == e);
    let mut hm = HashMap::new();
    hm.insert(d.clone(), 3);
    println!("{:?} {:?}", hm.get(&Dog::from("rex")), dogs.iter().map(|d| &d.name).collect::<Vec<_>>());
}
"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"#[derive(PartialEq)] struct D(i32); fn unused() { let v = vec![D(1)]; let d = D(1); let _ = d == v[0usize] == true; } fn main() {}"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"struct Dog; fn unused() { let d: Dog = "rex".into(); } fn main() {}"#,
    );
}

#[test]
fn generic_trait_methods_with_projected_results() {
    coherence_oracle::stock_native(
        r#"
#[derive(Debug)]
enum HexError { Odd, Bad(char) }
trait FromHex: Sized {
    type Error;
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>;
}
impl FromHex for Vec<u8> {
    type Error = HexError;
    fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> {
        let bytes = hex.as_ref();
        if bytes.len() % 2 != 0 { return Err(HexError::Odd); }
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut v = 0u8;
            for b in &bytes[i..i + 2] {
                let d = match *b { b'0'..=b'9' => b - b'0', b'a'..=b'f' => b - b'a' + 10, c => return Err(HexError::Bad(c as char)) };
                v = v * 16 + d;
            }
            out.push(v);
            i += 2;
        }
        Ok(out)
    }
}
struct P(u8);
impl FromHex for P { type Error = HexError; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> { Ok(P(hex.as_ref()[0])) } }
fn main() {
    println!("{:?}", Vec::<u8>::from_hex("0aff"));
    println!("{:?}", Vec::<u8>::from_hex(b"abc"));
    println!("{:?}", <Vec<u8> as FromHex>::from_hex(String::from("zz")));
    println!("{:?}", P::from_hex(b"a").map(|p| p.0));
    let r = P::from_hex("b");
    println!("{}", r.is_ok());
    println!("{:?}", decode(b"0a"));
    let q: Result<P, HexError> = FromHex::from_hex("c");
    println!("{:?}", q.map(|p| p.0));
}
fn decode<T: AsRef<[u8]>>(data: T) -> Result<Vec<u8>, HexError> {
    FromHex::from_hex(data)
}
"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"trait FromHex: Sized { type Error; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>; }
impl FromHex for Vec<u8> { type Error = String; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> { let _ = hex; Err(5u32) } }
fn main() {}"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"trait FromHex: Sized { type Error; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>; }
impl FromHex for Vec<u8> { fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error> { let _ = hex; Ok(Vec::new()) } }
fn main() {}"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"trait FromHex: Sized { type Error; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>; }
struct P; impl FromHex for P { type Error = (); fn from_hex<T: AsRef<[u8]>>(_: T) -> Result<Self, ()> { Ok(P) } }
fn unused() { let _ = P::from_hex(5u32); } fn main() {}"#,
    );
    coherence_oracle::stock_and_native_reject(
        r#"trait FromHex: Sized { type Error; fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Self::Error>; }
struct P; impl FromHex for P { type Error = (); fn from_hex<T: AsRef<[u8]>>(_: T) -> Result<Self, ()> { Ok(P) } }
fn unused() { let _: Result<u8, ()> = FromHex::from_hex("a"); } fn main() {}"#,
    );
}

#[test]
fn reference_casts_to_unsized_slices() {
    coherence_oracle::stock_native(
        r#"
fn fill(out: &mut [u8]) -> Result<(), u8> {
    for (i, b) in out.iter_mut().enumerate() { *b = i as u8 + 1; }
    Ok(())
}
fn arr() -> Result<[u8; 3], u8> {
    let mut out = [0_u8; 3];
    fill(&mut out as &mut [u8])?;
    Ok(out)
}
fn main() {
    println!("{:?}", arr());
    let a = [4u8, 5];
    let s = &a as &[u8];
    let v = vec![7u16, 8, 9];
    let t = &v as &[u16];
    let w = String::from("hi");
    let st = &w as &str;
    println!("{:?} {:?} {}", s, t, st);
}
"#,
    );
    for source in [
        "fn unused() { let a = [1u8, 2]; let _ = &a as &[u16]; } fn main() {}",
        "fn unused() { let a = [1u8, 2]; let _ = &a as &mut [u8]; } fn main() {}",
        "fn unused() { let a = 5u8; let _ = &a as &[u8]; } fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn inline_reference_literals_for_generic_parameters() {
    coherence_oracle::stock_native(
        r#"
fn len<T: AsRef<[u8]>>(d: T) -> usize { d.as_ref().len() }
fn id<T>(t: T) -> T { t }
fn main() {
    println!("{} {} {}", len(b"0a"), len(&[1u8, 2, 3]), len("hey"));
    println!("{:?} {:?} {:?}", id(b"xy"), id(&[true, false]), id(&['a', 'b']));
    let n = len(b"abcd");
    println!("{}", n);
}
"#,
    );
    for source in [
        "fn len<T: AsRef<[u8]>>(d: T) -> usize { d.as_ref().len() } fn unused() { let _ = len(&[1u16, 2]); } fn main() {}",
        "fn len<T: AsRef<[u8]>>(d: T) -> usize { d.as_ref().len() } fn unused() { let _ = len(&[1u8, 300]); } fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}

#[test]
fn large_scalar_arrays_in_generic_impls() {
    coherence_oracle::stock_native(
        r#"
fn fill(data: &[u8], out: &mut [u8]) -> Result<(), u8> {
    if data.len() > out.len() { return Err(1); }
    for i in 0..data.len() { out[i] = data[i]; }
    Ok(())
}
trait FromBytes: Sized { fn from_bytes<T: AsRef<[u8]>>(b: T) -> Result<Self, u8>; }
macro_rules! imp { ($($n:expr)+) => {$(
    impl FromBytes for [u8; $n] {
        fn from_bytes<T: AsRef<[u8]>>(b: T) -> Result<Self, u8> {
            let mut out = [0_u8; $n];
            fill(b.as_ref(), &mut out as &mut [u8])?;
            Ok(out)
        }
    }
)+} }
imp! { 2 100 4096 32768 65536 4294967296 }
fn main() {
    let a: Result<[u8; 2], u8> = FromBytes::from_bytes(b"hi");
    let b: Result<[u8; 100], u8> = FromBytes::from_bytes("abc");
    let c: Result<[u8; 4096], u8> = FromBytes::from_bytes([7u8; 9]);
    let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
    let d: Result<[u8; 2], u8> = FromBytes::from_bytes("toolong");
    println!("{:?} {} {} {} {} {:?}", a, b[0], b[3], c[8], c[9], d);
    let mut big = [1u32; 1000];
    big[999] = 5;
    let s: u32 = big.iter().sum();
    println!("{}", s);
}
"#,
    );
    for source in [
        "fn unused() { let big = [0u8; 1000]; big[3] = 1; } fn main() {}",
        "fn unused() -> u8 { let big: [u8; 1000]; big[0] } fn main() {}",
    ] {
        coherence_oracle::stock_and_native_reject(source);
    }
}
