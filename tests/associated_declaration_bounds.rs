//! General associated declaration obligations, specialized through native calls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn applied_bound_carries_ordered_arguments_and_associated_equalities() {
    oracle::stock_native(
        r#"
trait Read<A> { type Item; fn read(self) -> Self::Item; }
trait Factory<A> { type Output: Read<A, Item = A>; fn make(self) -> Self::Output; }
struct Value(i32);
impl Read<i32> for Value { type Item = i32; fn read(self) -> i32 { self.0 } }
struct Source;
impl Factory<i32> for Source { type Output = Value; fn make(self) -> Value { Value(42) } }
fn extract<F: Factory<i32>>(factory: F) -> i32 { factory.make().read() }
fn main() { println!("{}", extract(Source)); }
"#,
    );
}

#[test]
fn inherited_associated_declared_bounds_apply_without_a_repeated_where_clause() {
    oracle::stock_native(
        r#"
trait Read { fn read(self) -> i32; }
trait Factory { type Output: Read; fn make(self) -> Self::Output; }
trait Child: Factory {}
struct Value;
impl Read for Value { fn read(self) -> i32 { 17 } }
struct Source;
impl Factory for Source { type Output = Value; fn make(self) -> Value { Value } }
impl Child for Source {}
fn extract<F: Child>(factory: F) -> i32 { factory.make().read() }
fn main() { println!("{}", extract(Source)); }
"#,
    );
}

#[test]
fn unused_assignments_cannot_evade_declared_membership_or_equalities() {
    for source in [
        "trait Read {} trait Factory {type Output: Read;} struct S; impl Factory for S {type Output=i32;} fn main(){}",
        "trait Read<A>{type Item;} trait Factory {type Output: Read<i32,Item=bool>;} struct V;impl Read<i32> for V {type Item=i32;}struct S;impl Factory for S {type Output=V;}fn main(){}",
        "trait Read<A>{} trait Factory {type Output:Read<i32>;}struct V;impl Read<bool> for V{}struct S;impl Factory for S{type Output=V;}fn main(){}",
        "trait Read {} trait Factory {type Output:Read;}struct S<T>{value:T}impl<T> Factory for S<T>{type Output=T;}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn closed_declared_projection_keeps_same_self_trait_arguments_distinct() {
    oracle::stock_native(
        r#"
trait Read { fn read(self) -> i32; }
trait Factory<A> { type Output: Read; fn make(self) -> Self::Output; }
struct First;
struct Second;
impl Read for First { fn read(self) -> i32 { 19 } }
impl Read for Second { fn read(self) -> i32 { 23 } }
struct Source;
impl Factory<i32> for Source { type Output = First; fn make(self) -> First { First } }
impl Factory<bool> for Source { type Output = Second; fn make(self) -> Second { Second } }
fn first<F: Factory<i32>>(factory: F) -> i32 { factory.make().read() }
fn second<F: Factory<bool>>(factory: F) -> i32 { factory.make().read() }
fn main() { println!("{}", first(Source) + second(Source)); }
"#,
    );
}

#[test]
fn composite_equality_rhs_supplies_inference_evidence() {
    // `C: Container<Item = Vec<T>>` infers `T` from the normalized projection.
    oracle::stock_native(
        r#"
trait Container { type Item; fn get(&self) -> Self::Item; }
struct W<T>(Vec<T>);
impl<T: Clone> Container for W<T> { type Item = Vec<T>; fn get(&self) -> Vec<T> { self.0.clone() } }
fn count<T, C: Container<Item = Vec<T>>>(c: &C) -> usize { c.get().len() }
fn main() { let v: Vec<u8> = vec![7, 2]; let w = W(v); println!("{}", count(&w)); }
"#,
    );
    oracle::stock_and_native_reject(
        r#"
trait Container { type Item; }
struct W;
impl Container for W { type Item = Vec<u8>; }
fn count<T, C: Container<Item = Option<T>>>(_c: &C) {}
fn main() { count(&W); }
"#,
    );
}

#[test]
fn associated_copy_equality_does_not_grant_rhs_parameter_copy() {
    oracle::stock_and_native_reject(
        r#"
trait Source<A> { type Item: Copy; fn get(self) -> Self::Item; }
struct S;
impl Source<u8> for S { type Item = (i32, bool); fn get(self) -> Self::Item { (37, true) } }
fn duplicate<T: Source<A, Item = U>, U, A>(source: T) -> (U, U) {
    let value: U = source.get(); (value, value)
}

fn main() {
    let ((a, first), (b, second)) = duplicate::<S, (i32, bool), u8>(S);
    println!("{} {} {}", a + b, first, second);
}
"#,
    );
    for source in [
        "trait Source {type Item;} struct Token; struct S; impl Source for S{type Item=Token;} fn duplicate<T:Source<Item=U>,U>(value:U)->(U,U){(value,value)} fn main(){}",
        "trait Copy {} trait Source {type Item:Copy;} struct Token;impl Copy for Token{} struct S;impl Source for S{type Item=Token;}fn duplicate<T:Source<Item=U>,U>(value:U)->(U,U){(value,value)}fn main(){}",
        "trait Source<A>{type Item;}trait Other<A>{type Item:Copy;}fn duplicate<T:Source<u8,Item=U>+Other<bool>,U>(value:U)->(U,U){(value,value)}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn trait_where_projection_keeps_original_owner_occurrence() {
    oracle::stock_native(
        r#"trait Source where Self::Out: Copy { type Out; fn get(self) -> Self::Out; }
fn dup<T: Source>(x: T) -> (T::Out, T::Out) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(1u8)); println!("ok"); }
"#,
    );
    // The original projected obligation applies even to unused implementations.
    for source in [
        "trait Source where Self::Out:Copy{type Out;fn get(self)->Self::Out;}struct S;impl Source for S{type Out=String;fn get(self)->String{String::new()}}fn main(){}",
        "trait Source where Self::Out:Copy{type Out;fn get(self)->Self::Out;}struct Wrap<T>(T);impl<T> Source for Wrap<T>{type Out=T;fn get(self)->T{self.0}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
