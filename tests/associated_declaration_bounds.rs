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
