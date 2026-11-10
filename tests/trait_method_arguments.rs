//! Trait-owner arguments and a method's own arguments are separate ordered domains.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn projected_reference_aggregates_retain_declared_receiver_origins() {
    oracle::stock_native("fn main(){println!(\"hello\");}");
    oracle::stock_native(
        r#"
trait Container {
    type Item;
    fn get(&self, index: usize) -> Option<&Self::Item>;
    fn get_mut(&mut self) -> Option<&mut Self::Item>;
    fn retained(&self, other: &Self::Item) -> Option<&Self::Item>;
    fn pair<'a>(&'a self, other: &'a Self::Item) -> (&'a Self::Item, Option<&'a Self::Item>);
    fn first(&self) -> Option<&Self::Item> { self.get(0) }
}
struct Slot<T> { value: T }
impl<T> Container for Slot<T> {
    type Item = T;
    fn get(&self, index: usize) -> Option<&T> {
        if index == 0 { Some(&self.value) } else { None }
    }
    fn get_mut(&mut self) -> Option<&mut T> { Some(&mut self.value) }
    fn retained(&self, _other: &T) -> Option<&T> { Some(&self.value) }
    fn pair<'a>(&'a self, other: &'a T) -> (&'a T, Option<&'a T>) {
        (&self.value, Some(other))
    }
}
fn read<C: Container<Item = i32>>(c: &C) -> i32 {
    let extra = 7;
    let (first, second) = c.pair(&extra);
    *c.first().unwrap() + *first + *second.unwrap()
}
fn after_temporary<C: Container<Item = i32>>(c: &C) -> Option<&i32> {
    let temporary = 99;
    c.retained(&temporary)
}
fn bump<C: Container<Item = i32>>(c: &mut C) { *c.get_mut().unwrap() += 1; }
fn main() {
    let mut slot = Slot { value: 11 };
    println!("{} {} {}", read(&slot), *slot.get(0).unwrap(), *after_temporary(&slot).unwrap());
    bump(&mut slot);
    println!("{}", *slot.get(0).unwrap());
}
"#,
    );
}

#[test]
fn projected_reference_aggregates_reject_invalid_unused_implementations() {
    oracle::stock_native("fn main(){println!(\"hello\");}");
    for source in [
        "trait C{type Item;fn get(&self)->Option<&Self::Item>;}struct S;impl C for S{type Item=i32;fn get(&self)->Option<&i32>{let x=1;Some(&x)}}fn main(){}",
        "trait C{type Item;fn get(&self,other:&Self::Item)->Option<&Self::Item>;}struct S(i32);impl C for S{type Item=i32;fn get(&self,other:&i32)->Option<&i32>{Some(other)}}fn main(){}",
        "trait C{type Item;fn get(&mut self)->Option<&mut Self::Item>;}struct S(i32);impl C for S{type Item=i32;fn get(&mut self)->Option<&mut i32>{Some(&self.0)}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn projected_reference_aggregates_accept_source_closure_field_results() {
    oracle::stock_native(r#"fn main(){println!("hello");}"#);
    oracle::stock_native(
        r#"
struct Entry { name: String }
fn main() {
    let entries = vec![Entry { name: String::from("alpha") }, Entry { name: String::from("beta") }];
    let names: Vec<&String> = entries.iter().map(|entry| &entry.name).collect();
    println!("{} {} {}", names.len(), names[0], names[1]);
}
"#,
    );
}

#[test]
fn explicit_and_inferred_ufcs_method_arguments_match_stock() {
    oracle::stock_native(
        "trait Convert<A,B>{fn choose<U:Copy,V>(a:A,b:B,u:U,v:V)->U{u}}impl Convert<i32,bool> for i32{}impl Convert<bool,i32> for i32{}fn main(){assert_eq!(<i32 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8,'x'),7u8);assert_eq!(<i32 as Convert<bool,i32>>::choose(false,5,11i32,'y'),11);}",
    );
}

#[test]
fn result_context_can_infer_a_zero_argument_trait_method() {
    oracle::stock_native(
        "trait Factory{fn empty<T>()->Option<T>{None}}impl Factory for i32{}fn main(){let value:Option<u8>=<i32 as Factory>::empty();assert!(value.is_none());}",
    );
}

#[test]
fn ufcs_method_inference_never_changes_trait_owner_arity_order_or_bounds() {
    let declarations = "trait Convert<A,B>{fn choose<U:Copy,V>(a:A,b:B,u:U,v:V)->U{u}}impl Convert<i32,bool> for i32{}";
    for bad in [
        "<u8 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8,'x');",
        "<i32 as Convert<bool,i32>>::choose::<u8,char>(false,3,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<char,u8>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8,char,bool>(3,true,7u8,'x');",
        "<i32 as Convert<i32,bool>>::choose::<String,char>(3,true,String::new(),'x');",
        "<i32 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8);",
    ] {
        oracle::stock_and_native_reject(&format!("{declarations}fn bad(){{{bad}}}fn main(){{}}"));
    }
    oracle::stock_and_native_reject(
        "trait Bad{fn bad<U,V>(u:U,v:V)->U{v}}impl Bad for i32{}fn main(){}",
    );
}
