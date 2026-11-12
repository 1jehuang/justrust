//! General associated declaration obligations, specialized through native calls.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn declaration_predicates_keep_projected_nominal_fields_logical() {
    oracle::stock_native(
        r#"
trait Family { type Owned; }
struct Source;
impl Family for Source { type Owned = i32; }
struct Holder<T: Family> { value: T::Owned }
trait Tagged {}
impl Tagged for Holder<Source> {}
fn unused<T>() where Holder<Source>: Tagged {}
fn main() { println!("41"); }
"#,
    );
    for source in [
        // A logical nominal application still requires its exact declared bound.
        r#"trait Family { type Owned; }
struct Holder<T: Family> { value: T::Owned }
trait Tagged {}
fn unused<T>() where Holder<T>: Tagged {}
fn main() {}"#,
        // A direct projected predicate cannot invent source trait membership.
        r#"trait Family { type Owned; }
struct Other;
trait Tagged {}
fn unused<T>() where <Other as Family>::Owned: Tagged {}
fn main() {}"#,
        // Declaration predicates do not authorize invalid physical field types.
        r#"trait Family { type Owned; }
struct Source;
impl Family for Source { type Owned = str; }
struct Holder<T: Family> { value: [T::Owned; 1] }
trait Tagged {}
fn unused<T>() where Holder<Source>: Tagged {}
fn main() {}"#,
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn unused_declaration_domains_require_original_nominal_bounds() {
    for declaration in [
        "trait Broken<T> where Holder<T>: Tagged {}",
        "trait Broken<T> {} struct Marker; impl<T> Broken<T> for Marker where Holder<T>: Tagged {}",
        "struct Broken<T> where Holder<T>: Tagged { value: T }",
    ] {
        let source = format!(
            "trait Family {{ type Owned; }} struct Holder<T: Family> {{ value: T::Owned }} trait Tagged {{}} {declaration} fn main() {{}}"
        );
        oracle::stock_and_native_reject(&source);
    }
    for declaration in [
        "trait Valid<T: Family> where Holder<T>: Tagged {}",
        "trait Valid<T> {} struct Marker; impl<T: Family> Valid<T> for Marker where Holder<T>: Tagged {}",
        "struct Valid<T: Family> where Holder<T>: Tagged { value: T }",
        "trait Valid where Holder<Source>: Tagged {}",
        "trait Valid {} struct Marker; impl Valid for Marker where Holder<Source>: Tagged {}",
        "struct Valid where Holder<Source>: Tagged { value: Source }",
    ] {
        let source = format!(
            "trait Family {{ type Owned; }} struct Holder<T: Family> {{ value: T::Owned }} trait Tagged {{}} struct Source; impl Family for Source {{ type Owned = i32; }} impl Tagged for Holder<Source> {{}} {declaration} fn main() {{ println!(\"41\"); }}"
        );
        oracle::stock_native(&source);
    }
}

#[test]
fn associated_subject_blanket_bounds_format_and_dispatch() {
    oracle::stock_native(
        r#"
use std::fmt::Debug;
trait C { type Item; }
struct Number; struct Flag;
impl C for Number { type Item = i32; }
impl C for Flag { type Item = bool; }
fn show<T:C>(x:&T::Item)->String where T::Item:Debug {
    println!("{x:?}"); format!("{x:?}")
}
fn main() { println!("{} {}",show::<Number>(&7),show::<Flag>(&true)); }
"#,
    );
    oracle::stock_native(
        r#"
use std::fmt::Debug;
trait C { type Item: Debug; }
struct Source<T> { value:T }
impl<T:Debug> C for Source<T> { type Item=T; }
fn show<T:C>(x:&T::Item)->String { format!("{x:?}") }
fn main() { println!("{} {}",show::<Source<i32>>(&11),show::<Source<bool>>(&false)); }
"#,
    );
    oracle::stock_native(
        r#"
trait C { type Item; }
trait Read<A> { fn read(&self)->A; }
trait Wrap<A> { fn wrapped(&self)->A; }
impl<X:Read<A>,A> Wrap<A> for X { fn wrapped(&self)->A { <X as Read<A>>::read(self) } }
struct Number; impl C for Number { type Item=i32; }
impl Read<i32> for i32 { fn read(&self)->i32 { *self+1 } }
fn show<T:C>(x:&T::Item)->i32 where T::Item:Read<i32> { <T::Item as Wrap<i32>>::wrapped(x) }
fn main() { println!("{}",show::<Number>(&41)); }
"#,
    );
}

#[test]
fn unused_associated_subject_blankets_require_exact_caller_bounds() {
    for source in [
        r#"trait C {type Item;} fn unused<T:C>(x:&T::Item)->String {format!("{x:?}")} fn main(){}"#,
        r#"use std::fmt::Debug; trait C{type Item;type Other;} fn unused<T:C>(x:&T::Item)->String where T::Other:Debug {format!("{x:?}")} fn main(){}"#,
        r#"trait C{type Item;} trait Read<A>{fn read(&self)->A;} trait Wrap{fn wrapped(&self)->i32;} impl<X:Read<i32>> Wrap for X{fn wrapped(&self)->i32{<X as Read<i32>>::read(self)}} fn unused<T:C>(x:&T::Item)->i32 where T::Item:Read<bool>{<T::Item as Wrap>::wrapped(x)} fn main(){}"#,
        r#"trait C{type Item;} trait Read{fn read(&self)->i32;} trait Wrap{fn wrapped(&self)->i32;} impl<X:Read> Wrap for X{fn wrapped(&self)->i32{Read::read(self)}} fn unused<T:C>(x:&T::Item)->i32{Wrap::wrapped(x)} fn main(){}"#,
    ] {
        oracle::stock_and_native_reject(source);
    }
}

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
fn composite_equality_keeps_lexical_integer_result_context() {
    oracle::stock_native(
        r#"
trait Container { type Item; fn get(&self) -> Self::Item; }
struct W<T>(Vec<T>);
impl<T: Clone> Container for W<T> { type Item = Vec<T>; fn get(&self) -> Vec<T> { self.0.clone() } }
fn count<T, C: Container<Item = Vec<T>>>(c: &C) -> usize { c.get().len() }
fn main() {
    let v: Vec<u8> = { let v = vec![7, 2]; { let v = v; v } };
    println!("{}", count(&W(v)));
}
"#,
    );
    for source in [
        "fn unused(){let v:Vec<u8>={let v=vec![256,2];v};}fn main(){}",
        "fn unused(){let v:Vec<u8>={let v=vec![7u16,2];v};}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
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
fn explicit_copy_equality_infers_shared_reference_to_noncopy_referent() {
    // Exact controls_18 source, including its non-Copy referent. U is inferred
    // from Source::Out, not from the Copy bound or the checked call's actuals.
    oracle::stock_native(
        r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let text = String::new(); let _ = dup(Wrap(&text)); println!("ok"); }
"#,
    );
    // Both &String and &i32 are Copy shared references. Equal carrier shape
    // cannot replace the exact associated RHS required by the annotation.
    oracle::stock_and_native_reject(
        r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let text = String::new(); let _: (&i32, &i32) = dup(Wrap(&text)); println!("ok"); }
"#,
    );
}

#[test]
fn projected_owner_reference_origins_do_not_escape_or_duplicate_mutable_borrows() {
    for source in [
        r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn escape() -> &'static i32 { let value = 7; let (first, _) = dup(Wrap(&value)); first }
fn main() { println!("{}", escape()); }
"#,
        r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn unused_escape() -> &'static i32 { let value = 7; let (first, _) = dup(Wrap(&value)); first }
fn main() { println!("ok"); }
"#,
        r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let mut value = 7; let (first, second) = dup(Wrap(&mut value)); *first = 1; *second = 2; }
"#,
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
