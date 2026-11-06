//! Static associated calls bind the written owner before their nested result.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn rc_trait_receiver_associated_owner_identity_matches_stock() {
    oracle::check(
        "rc-trait-receiver-associated-owner-identity",
        &[Case {
            name: "rc_clone_preserves_nested_owner_identity",
            source: r#"
use std::rc::Rc;
use std::cell::RefCell;
struct Account { balance: i32 }
fn main() {
    let a = Rc::new(RefCell::new(Account { balance: 42i32 }));
    let b = Rc::clone(&a);
    println!("{} {}", Rc::strong_count(&b), Rc::ptr_eq(&a, &b));
}
"#,
            expect: Expect::Run("2 true\n"),
        }],
    );
}

#[test]
fn trait_receiver_associated_owners_match_stock() {
    oracle::check(
        "trait-receiver-associated-owner",
        &[
            Case {
                name: "borrowed_mutable_and_value_receivers",
                source: r#"
struct Packet<T> { value: T }
trait Read { fn read(&self) -> u64; }
trait Write { fn write(&mut self, value: u64); }
trait Take { fn take(self) -> u64; }
impl<T> Read for Packet<T> { fn read(&self) -> u64 { 7u64 } }
impl<T> Write for Packet<T> { fn write(&mut self, value: u64) { println!("{}", value); } }
impl<T> Take for Packet<T> { fn take(self) -> u64 { 9u64 } }
fn main() {
    let mut a = Packet { value: 42u32 };
    println!("{}", Packet::read(&a));
    Packet::write(&mut a, 8u64);
    println!("{}", Packet::take(a));
}
"#,
                expect: Expect::Run("7\n8\n9\n"),
            },
            Case {
                name: "specialized_heads_share_only_source_receiver_equations",
                source: r#"
struct Packet<T> { value: T }
trait Read { fn read(&self) -> u64; }
impl Read for Packet<u32> { fn read(&self) -> u64 { 32u64 } }
impl Read for Packet<u64> { fn read(&self) -> u64 { 64u64 } }
fn main() {
    let a = Packet { value: 42u32 };
    let b = Packet { value: 42u64 };
    println!("{} {}", Packet::read(&a), Packet::read(&b));
}
"#,
                expect: Expect::Run("32 64\n"),
            },
            Case {
                name: "trait_and_method_actuals_keep_distinct_binders",
                source: r#"
struct Packet<T> { value: T }
trait Read<U> { fn read<V>(&self, fallback: U, extra: V) -> (U, V); }
impl<T> Read<T> for Packet<T> {
    fn read<V>(&self, fallback: T, extra: V) -> (T, V) { (fallback, extra) }
}
fn main() {
    let a = Packet { value: 42u32 };
    let pair = Packet::read::<u16>(&a, 7u32, 9u16);
    println!("{} {}", pair.0, pair.1);
}
"#,
                expect: Expect::Run("7 9\n"),
            },
            Case {
                name: "nested_generic_owner_clone",
                source: r#"
struct Packet<T> { value: T }
trait Duplicate { fn duplicate(&self) -> Self; }
impl<T: Copy> Duplicate for Packet<T> {
    fn duplicate(&self) -> Self { Self { value: self.value } }
}
fn main() {
    let a = Packet { value: Packet { value: 42u32 } };
    let b = Packet::duplicate(&a.value);
    println!("{}", b.value);
}
"#,
                expect: Expect::Run("42\n"),
            },
            Case {
                name: "specialized_heads_do_not_authorize_missing_application",
                source: r#"
struct Packet<T> { value: T }
trait Read { fn read(&self) -> u64; }
impl Read for Packet<u32> { fn read(&self) -> u64 { 32u64 } }
impl Read for Packet<u64> { fn read(&self) -> u64 { 64u64 } }
fn unused_bad() { let a = Packet { value: 42i32 }; let b = Packet::read(&a); }
fn main() {}
"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unavailable_impl_bound",
                source: r#"
struct Packet<T> { value: T }
struct NotCopy { value: u32 }
trait Duplicate { fn duplicate(&self) -> Self; }
impl<T: Copy> Duplicate for Packet<T> {
    fn duplicate(&self) -> Self { Self { value: self.value } }
}
fn unused_bad() {
    let a = Packet { value: NotCopy { value: 42u32 } };
    let b = Packet::duplicate(&a);
}
fn main() {}
"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "ambiguous_visible_receiver_traits",
                source: r#"
struct Packet<T> { value: T }
trait First { fn read(&self) -> u64; }
trait Second { fn read(&self) -> u64; }
impl<T> First for Packet<T> { fn read(&self) -> u64 { 1u64 } }
impl<T> Second for Packet<T> { fn read(&self) -> u64 { 2u64 } }
fn unused_bad() { let a = Packet { value: 42u32 }; let b = Packet::read(&a); }
fn main() {}
"#,
                expect: Expect::Invalid("E0034"),
            },
            Case {
                name: "wrong_receiver_mutability",
                source: r#"
struct Packet<T> { value: T }
trait Write { fn write(&mut self); }
impl<T> Write for Packet<T> { fn write(&mut self) {} }
fn unused_bad() { let a = Packet { value: 42u32 }; Packet::write(&a); }
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn static_owner_nested_returns_match_stock() {
    oracle::check(
        "static-owner-nested",
        &[
            Case {
                name: "nested_option_self_and_ordered_method_actuals",
                source: r#"
struct Packet<T> { value: T }
impl<T> Packet<T> {
    fn wrap(value: T) -> Option<Self> { Some(Self { value }) }
    fn pair<U>(value: T, extra: U) -> Option<(Self, U)> {
        Some((Self { value }, extra))
    }
}
fn main() {
    let a = Packet::<u64>::wrap(42);
    let b = Packet::<i32>::pair::<u16>(-7, 9);
    match a { Some(p) => println!("{}", p.value), None => println!("missing") }
    match b { Some((p, extra)) => println!("{} {}", p.value, extra), None => println!("missing") }
}
"#,
                expect: Expect::Run("42\n-7 9\n"),
            },
            Case {
                name: "written_owner_constrains_generic_null_argument",
                source: r#"
struct Pointer<T> { pointer: *mut T }
impl<T> Pointer<T> {
    fn wrap(pointer: *mut T) -> Option<Self> { Some(Self { pointer }) }
}
fn main() {
    let value = Pointer::<i32>::wrap(core::ptr::null_mut());
    match value { Some(p) => println!("{}", p.pointer as usize == 0usize), None => println!("missing") }
}
"#,
                expect: Expect::Run("true\n"),
            },
        ],
    );
}
