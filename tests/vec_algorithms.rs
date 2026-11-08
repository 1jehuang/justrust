//! Source-library Vec algorithms compared with stock Rust at workers 1 and 8.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/iterator_oracle.rs"]
mod iterator_oracle;
use iterator_oracle::closure_oracle::pinned_stock as stock_rustc;

#[test]
fn implicit_deref_mut_field_assignments_match_stock() {
    iterator_oracle::stock_native(
        r#"
use std::ops::{Deref,DerefMut};
struct Value{n:i32}
struct Owner{value:Value}
impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{&self.value}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Value{println!("owner deref");&mut self.value}}
struct Shadow{value:Value,n:i32}
impl Deref for Shadow{type Target=Value;fn deref(&self)->&Value{&self.value}}
impl DerefMut for Shadow{fn deref_mut(&mut self)->&mut Value{println!("shadow deref");&mut self.value}}
fn rhs(value:i32)->i32{println!("rhs {}",value);value}
fn main(){let mut owner=Owner{value:Value{n:4i32}};owner.n+=rhs(3i32);owner.n=rhs(11i32);println!("{}",owner.n);let mut shadow=Shadow{value:Value{n:2i32},n:5i32};shadow.n+=1i32;println!("{} {}",shadow.n,shadow.value.n);}
"#,
    );
}

#[test]
fn implicit_deref_mut_fields_reject_invalid_unused_programs() {
    let prefix = r#"
use std::ops::{Deref,DerefMut};
struct Value{n:i32}
struct Owner{value:Value}
impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{&self.value}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Value{&mut self.value}}
"#;
    for body in [
        "fn unused(){let owner=Owner{value:Value{n:1i32}};owner.n+=1i32;}fn main(){}",
        "fn unused(){let mut owner=Owner{value:Value{n:1i32}};let loan=&owner.n;owner.n=3i32;println!(\"{}\",loan);}fn main(){}",
        "fn unused(){let mut owner=Owner{value:Value{n:1i32}};owner.absent=3i32;}fn main(){}",
    ] {
        iterator_oracle::stock_and_native_reject(&format!("{prefix}{body}"));
    }
}

#[test]
fn overloaded_deref_field_method_receivers_match_stock() {
    let prefix = r#"
use std::ops::{Deref,DerefMut};
struct Value{entries:Vec<i32>}
struct Owner{value:Value}
impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{&self.value}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Value{&mut self.value}}
"#;
    iterator_oracle::stock_native(&format!(
        "{prefix}fn main(){{let mut owner=Owner{{value:Value{{entries:vec![1i32]}}}};owner.entries.push(2i32);println!(\"{{}}\",owner.entries.len());}}"
    ));
    for body in [
        "fn unused(){let owner=Owner{value:Value{entries:vec![1i32]}};owner.entries.push(2i32);}fn main(){}",
        "fn unused(){let mut owner=Owner{value:Value{entries:vec![1i32]}};owner.entries.push(owner.entries.len() as i32);}fn main(){}",
        "fn unused(){let mut owner=Owner{value:Value{entries:vec![1i32]}};let loan=&owner.entries;owner.entries.push(2i32);println!(\"{:?}\",loan);}fn main(){}",
    ] {
        iterator_oracle::stock_and_native_reject(&format!("{prefix}{body}"));
    }
}

#[test]
fn local_deref_noncopy_field_formatting_matches_stock() {
    iterator_oracle::stock_native(
        r#"
use std::ops::Deref;
struct Value{name:String}
struct Owner{value:Value}
impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{println!("deref");&self.value}}
struct Shadow{value:Value,name:String}
impl Deref for Shadow{type Target=Value;fn deref(&self)->&Value{println!("unexpected deref");&self.value}}
fn main(){let owner=Owner{value:Value{name:String::from("retained")}};let text=format!("{} {}",owner.name,owner.name);println!("{}",text);println!("{}",owner.name);let shadow=Shadow{value:Value{name:String::from("target")},name:String::from("stored")};println!("{}",shadow.name);}
"#,
    );
}

#[test]
fn local_deref_field_formatting_rejects_invalid_unused_programs() {
    let prefix = r#"
use std::ops::{Deref,DerefMut};
struct Value{name:String}
struct Owner{value:Value}
impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{&self.value}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Value{&mut self.value}}
"#;
    for body in [
        "fn unused(){let owner=Owner{value:Value{name:String::from(\"kept\")}};let _=format!(\"{}\",owner.absent);}fn main(){}",
        "fn unused(){let mut owner=Owner{value:Value{name:String::from(\"kept\")}};let loan=&mut *owner;let _=format!(\"{}\",owner.name);loan.name=String::from(\"changed\");}fn main(){}",
    ] {
        iterator_oracle::stock_and_native_reject(&format!("{prefix}{body}"));
    }
    iterator_oracle::stock_and_native_reject(
        r#"
mod inside{use std::ops::Deref;pub struct Value{name:String}pub struct Owner{value:Value}impl Deref for Owner{type Target=Value;fn deref(&self)->&Value{&self.value}}pub fn make()->Owner{Owner{value:Value{name:String::from("private")}}}}
fn unused(){let owner=inside::make();let _=format!("{}",owner.name);}fn main(){}
"#,
    );
}

#[test]
fn unchanged_sort_by_actual_program() {
    iterator_oracle::stock_native(include_str!("fixtures/full_programs/34_sort_by.rs"));
}

#[test]
fn explicit_borrow_coercions_with_source_initializers_match_stock() {
    for initializer in [
        "(0i32..4i32).map(|x|x).collect()",
        "make::<i32>()",
        "vec![1i32,2i32,3i32]",
    ] {
        let source = format!(
            "fn make<T>()->Vec<T>{{Vec::new()}}fn mutate<T:Copy+std::fmt::Display>(v:&mut[T]){{if v.len()>1usize{{let first=v[0usize];v[0usize]=v[1usize];v[1usize]=first;}}let mut i=0usize;while i<v.len(){{println!(\"{{}}\",v[i]);i+=1usize;}}}}fn read<T:Copy+std::fmt::Display>(v:&[T]){{let mut i=0usize;while i<v.len(){{println!(\"{{}}\",v[i]);i+=1usize;}}}}fn main(){{let data:Vec<i32>={initializer};let mut copied=data.clone();mutate(&mut copied);read(&copied);}}"
        );
        iterator_oracle::stock_native(&source);
    }
    iterator_oracle::stock_native(
        r#"
use std::ops::{Deref,DerefMut};
struct Wrapper<T>{values:[T;2]}
impl<T> Deref for Wrapper<T>{type Target=[T];fn deref(&self)->&[T]{&self.values}}
impl<T> DerefMut for Wrapper<T>{fn deref_mut(&mut self)->&mut[T]{&mut self.values}}
fn mutate<T:Copy>(values:&mut[T]){let first=values[0usize];values[0usize]=values[1usize];values[1usize]=first;}
fn read<T:Copy+std::fmt::Display>(values:&[T]){let mut i=0usize;while i<values.len(){println!("{}",values[i]);i+=1usize;}}
fn main(){let mut owner:Wrapper<i32>=Wrapper{values:[3i32,7i32]};mutate(&mut owner);read(&owner);}
"#,
    );
    iterator_oracle::stock_native(include_str!("fixtures/full_programs/16_sorting.rs"));
}

#[test]
fn shared_local_deref_method_receivers_match_stock() {
    iterator_oracle::stock_native(
        r#"
use std::ops::Deref;
struct Owner<T>{n:T}
struct Wrapper<T>{v:Owner<T>}
struct View<'a,T>{r:&'a T}
impl<T> Deref for Wrapper<T>{type Target=Owner<T>;fn deref(&self)->&Owner<T>{&self.v}}
impl<T> Owner<T>{fn view<'a>(&'a self)->View<'a,T>{View{r:&self.n}}}
fn show<T:std::fmt::Display>(view:View<'_,T>){println!("{}",view.r);}
fn main(){let wrapper=Wrapper{v:Owner{n:String::from("shared")}};let view=wrapper.view();show(view);let borrowed=&wrapper;let view=borrowed.view();show(view);}
"#,
    );
    iterator_oracle::stock_native(
        r#"
use std::rc::Rc;use std::cell::RefCell;
struct Account{balance:i32}
fn main(){let a=Rc::new(RefCell::new(Account{balance:42i32}));let b=Rc::clone(&a);let view=b.borrow();println!("{}",view.balance);drop(view);let view=a.try_borrow().unwrap();println!("{}",view.balance);}
"#,
    );
}

#[test]
fn format_operands_infer_partial_collect_types() {
    for source in [
        r#"fn main(){println!("{:?}",(0i32..3).map(|x|x+1).collect::<Vec<_>>());}"#,
        r#"fn main(){let text=format!("{:?}",(0i32..3).map(|x|x+1).collect::<Vec<_>>());println!("{}",text);}"#,
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        r#"fn unused(){println!("{:?}",(0i32..3).collect::<Vec<bool>>());}fn main(){}"#,
        r#"fn unused(){let _=format!("{:?}",(0i32..3).collect::<Vec<bool>>());}fn main(){}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(failure.resource_usage.first_failure.is_none());
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn comparator_sorting_vec_and_slices_match_stock() {
    for source in [
        r#"struct Item{key:i32,text:String}fn main(){let mut v=vec![Item{key:2,text:String::from("b")},Item{key:1,text:String::from("a")},Item{key:2,text:String::from("c")}];v.sort_unstable_by(|a,b|a.key.cmp(&b.key));for x in &v{println!("{}",x.key);}v.sort_unstable_by_key(|x|-x.key);for x in &v{println!("{}",x.key);}let mut e:Vec<Item>=Vec::new();e.sort_unstable_by(|a,b|a.key.cmp(&b.key));println!("{}",e.len());}"#,
        r#"struct Item{key:i32,text:String}fn stable(v:&mut[Item]){v.sort_by(|a,b|a.key.cmp(&b.key));}fn main(){let mut v=vec![Item{key:2,text:String::from("b")},Item{key:1,text:String::from("a")},Item{key:2,text:String::from("c")}];stable(v.as_mut_slice());for x in &v{println!("{} {}",x.key,x.text);}v.as_mut_slice().sort_by_key(|x|-x.key);for x in &v{println!("{} {}",x.key,x.text);}}"#,
        r#"fn order(v:&mut[i32]){let mut compared=false;v.sort_unstable_by(|a,b|{compared=true;b.cmp(a)});println!("{} {}",compared,v.len());v.sort_unstable_by_key(|x|*x);let mut i=0usize;while i<v.len(){println!("{}",v[i]);i+=1usize;}}fn main(){let mut v=vec![3i32,1,2,2];order(v.as_mut_slice());let mut e:Vec<i32>=Vec::new();order(e.as_mut_slice());let mut one=vec![4i32];order(one.as_mut_slice());v.sort_unstable();println!("{:?}",v);}"#,
        r#"fn main(){let mut called=false;let mut v=vec![2i32,1];v.sort_unstable_by(|a,b|{called=true;a.cmp(b)});println!("{} {:?}",called,v);}"#,
    ] {
        iterator_oracle::stock_native(source);
    }
}

#[test]
fn comparator_sorting_rejects_invalid_unused_programs() {
    for source in [
        "fn unused(){let mut v=vec![1i32];v.sort_unstable_by(|a,b|a<b);}fn main(){}",
        "fn unused(v:&mut[i32]){v.sort_by(|a,b|a<b);}fn main(){}",
        "struct Key;fn unused(v:&mut[i32]){v.sort_unstable_by_key(|_|Key);}fn main(){}",
        "fn unused(v:&mut[i32]){v.sort_unstable_by(|a,b|{*a=3;a.cmp(b)});}fn main(){}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn shared_slice_comparator_receiver_rejection_is_explicitly_unsupported() {
    // Stock rejects the shared receiver. Native currently stops earlier at an
    // inference boundary, which must not count as semantic rejection evidence.
    let source = "fn unused(v:&[i32]){v.sort_unstable_by(|a:&i32,b:&i32|a.cmp(b));}fn main(){}";
    comparator_boundary(
        source,
        "shared-receiver",
        Some("E0596"),
        "unsupported: nominal source constraints cannot progress",
    );
}

#[test]
fn comparator_function_items_remain_explicitly_unsupported() {
    comparator_boundary(
        "fn descending(a:&i32,b:&i32)->std::cmp::Ordering{b.cmp(a)}fn main(){let mut v=vec![1i32,3,2];v.sort_unstable_by(descending);v.as_mut_slice().sort_by(descending);}",
        "function-item",
        None,
        "unsupported: function items with reference-bearing signatures as callable values",
    );
}

fn comparator_boundary(source: &str, tag: &str, stock_error: Option<&str>, native_error: &str) {
    let root = std::env::temp_dir().join(format!("jr-sort-{tag}-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    fs::write(&input, source).unwrap();
    let stock = Command::new(iterator_oracle::closure_oracle::pinned_stock::stock_rustc())
        .args(["--edition=2024", "--emit=metadata"])
        .arg(&input)
        .arg("-o")
        .arg(root.join("stock.rmeta"))
        .output()
        .unwrap();
    if let Some(error) = stock_error {
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains(error));
    } else {
        assert!(
            stock.status.success(),
            "{}",
            String::from_utf8_lossy(&stock.stderr)
        );
    }
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.into(), workers, check)
                .err()
                .expect("unsupported comparator boundary must not produce an executable");
            assert!(failure.resource_usage.first_failure.is_none());
            let diagnostic = failure.error.to_string();
            assert!(
                diagnostic.contains(native_error),
                "workers={workers} check={check}: {diagnostic}"
            );
            if stock_error.is_some() {
                assert!(iterator_oracle::validate_native_rejection(&failure).is_err());
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_heap_exact_swap_preserves_pair_alias_and_mutable_element_origins() {
    for source in [
        r#"fn main(){let mut values:Vec<&str>=Vec::new();values.push("first");values.push("second");values.swap(0usize,1usize);println!("{:?}",values);values.swap(1usize,1usize);println!("{:?}",values);}"#,
        r#"fn main(){let first=String::from("local-a");let second=String::from("local-b");let mut values:Vec<&str>=Vec::new();values.push(first.as_str());values.push(second.as_str());values.swap(0usize,1usize);println!("{:?}",values);}"#,
        r#"fn main(){let mut first=1i32;let mut second=2i32;let mut values:Vec<&mut i32>=Vec::new();values.push(&mut first);values.push(&mut second);values.swap(0usize,1usize);println!("{} {}",values[0usize],values[1usize]);}"#,
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        r#"fn main(){let escaped:Vec<&str>;{let owner=String::from("local");let mut values:Vec<&str>=Vec::new();values.push(owner.as_str());values.push("static");values.swap(0usize,1usize);escaped=values;}println!("{:?}",escaped);}"#,
        r#"fn main(){let mut values:Vec<&str>=Vec::new();values.push("first");values.push("second");let shared=&values;shared.swap(0usize,1usize);}"#,
        r#"fn main(){let mut owner=1i32;let mut values:Vec<&mut i32>=Vec::new();values.push(&mut owner);values.push(&mut owner);values.swap(0usize,1usize);println!("{}",values[0usize]);}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn native_heap_reference_carriers_preserve_initialized_origins() {
    for source in [
        r#"fn main(){let mut values:Vec<&str>=Vec::new();values.push("ccc");values.push("a");values.push("bb");println!("{:?}",values);values.sort_by_key(|s|s.len());println!("{:?}",values);}"#,
        r#"fn observed<'a>(value:&'a str)->&'a str{println!("push {}",value);value}fn main(){let mut values:Vec<&str>=Vec::new();values.push(observed("first"));values.push(observed("second"));println!("{:?}",values);}"#,
        r#"fn append<'a>(values:&mut Vec<&'a str>,value:&'a str){values.push(value);}fn main(){let mut values:Vec<&str>=Vec::new();append(&mut values,"first");append(&mut values,"second");println!("{:?}",values);}"#,
        r#"fn observe(values:&mut[&'static str]){println!("{}",values[0usize]);}fn main(){let mut values=vec!["first","second"];observe(&mut values);println!("{}",values[1usize]);}"#,
        r#"fn main(){let mut values=vec!["first","second"];let view=values.as_mut_slice();println!("{:?}",view);}"#,
        r#"fn head_mut<'a>(values:&'a mut Vec<&'static str>)->&'a mut &'static str{&mut values[0usize]}fn main(){let mut values=vec!["first"];*head_mut(&mut values)="second";println!("{}",values[0usize]);}"#,
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn main(){let owner=String::from("local");let mut values=vec![owner.as_str(),"tail"];*head_mut(&mut values)="second";println!("{} {}",values[0usize],values[1usize]);}"#,
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn replace<'b>(values:&mut Vec<&'b str>,text:&'b str){*head_mut(values)=text;}fn main(){let owner=String::from("local");let mut values=vec![owner.as_str(),"tail"];replace(&mut values,owner.as_str());println!("{} {}",values[0usize],values[1usize]);}"#,
        r#"fn head<'a>(values:&'a Vec<&'static str>)->&'a &'static str{&values[0usize]}fn main(){let values=vec!["first","second"];println!("{}",*head(&values));}"#,
        r#"fn main(){let values=vec!["first","second"];for value in &values{println!("{}",value);}}"#,
        r#"fn main(){let owned=String::from("local");let mut values:Vec<&str>=Vec::new();values.push(owned.as_str());println!("{:?}",values);}"#,
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        r#"fn bad_slice<'a,T,U>(values:&'a mut Vec<T>)->&'a mut[U]{values.as_mut_slice()}fn main(){}"#,
        r#"fn observe(_: &mut[&str]){}fn main(){let values=vec!["first"];observe(&values);}"#,
        r#"fn main(){let escaped;{let mut values=vec!["local"];escaped=values.as_mut_slice();}println!("{:?}",escaped);}"#,
        r#"fn bad_mut<'a,T,U>(values:&'a mut Vec<T>)->&'a mut U{&mut values[0usize]}fn main(){}"#,
        r#"fn main(){let values=vec!["first"];let _=&mut values[0usize];}"#,
        r#"fn main(){let escaped;{let mut values=vec!["local"];escaped=&mut values[0usize];}println!("{}",escaped);}"#,
        r#"fn bad<'a,T,U>(values:&'a Vec<T>)->&'a U{&values[0usize]}fn main(){}"#,
        r#"fn main(){let first=1i32;let mut values:Vec<&i32>=Vec::new();values.push(&first);*values[0usize]=2i32;}"#,
        r#"fn change(values:&Vec<&str>){values.push("bad");}fn main(){let values:Vec<&str>=Vec::new();change(&values);}"#,
        r#"fn main(){let escaped:Vec<&str>;{let owned=String::from("local");let mut values:Vec<&str>=Vec::new();values.push(owned.as_str());escaped=values;}println!("{:?}",escaped);}"#,
        r#"fn main(){let escaped;{let values=vec!["local"];escaped=&values[0usize];}println!("{}",escaped);}"#,
        r#"fn main(){let escaped;{let values=vec!["local"];escaped=(&values).into_iter();}for value in escaped{println!("{}",value);}}"#,
        r#"fn main(){let values:Vec<&str>=Vec::new();values.push("x");}"#,
        r#"fn main(){let mut values:Vec<bool>=Vec::new();values.push("x");}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            if source.starts_with("fn bad<'a,T,U>")
                || source.starts_with("fn bad_mut<'a,T,U>")
                || source.starts_with("fn bad_slice<'a,T,U>")
            {
                assert!(
                    failure.error.to_string().contains("E0308"),
                    "workers={workers} check={check}: {}\n{source}",
                    failure.error
                );
            }
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}\n{source}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}\n{source}",
                failure.error
            );
        });
    }
}

#[test]
fn native_heap_reference_carrier_writes_reject_invalid_lifetimes() {
    // Run these write controls independently of unrelated slice rejections.
    for source in [
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn overwrite<'b>(values:&mut Vec<&'b str>,text:&str){*head_mut(values)=text;}fn main(){}"#,
        r#"fn head_mut<'a>(values:&'a mut Vec<&'static str>)->&'a mut &'static str{&mut values[0usize]}fn main(){let values=vec!["first"];*head_mut(&mut values)="second";}"#,
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn overwrite<'b>(values:&Vec<&'b str>,text:&'b str){*head_mut(values)=text;}fn main(){}"#,
        r#"fn head_mut<'a>(values:&'a mut Vec<&'static str>)->&'a mut &'static str{&mut values[0usize]}fn main(){let mut values=vec!["first"];let owner=String::from("local");*head_mut(&mut values)=owner.as_str();println!("{}",values[0usize]);}"#,
        r#"fn head_mut<'a>(values:&'a mut Vec<&'static str>)->&'a mut &'static str{&mut values[0usize]}fn main(){let mut values=vec!["first"];let owner=String::from("local");let slot=head_mut(&mut values);let again=&mut *slot;*again=owner.as_str();println!("{}",values[0usize]);}"#,
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn overwrite<'b>(values:&mut Vec<&'b str>){let owner=String::from("short");*head_mut(values)=owner.as_str();}fn main(){}"#,
        r#"fn head_mut<'a,'b>(values:&'a mut Vec<&'b str>)->&'a mut &'b str{&mut values[0usize]}fn main(){let escaped;{let owner=String::from("local");let mut values=vec![owner.as_str(),"tail"];*head_mut(&mut values)="second";escaped=values;}println!("{:?}",escaped);}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}\n{source}",
                failure.error
            );
            assert!(
                !failure
                    .error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("unsupported"),
                "workers={workers} check={check}: {}\n{source}",
                failure.error
            );
        });
    }
}

#[test]
fn native_heap_lifetime_constraints_are_checked_without_codegen() {
    for source in [
        r#"fn append<'a>(values:&mut Vec<&'a str>,value:&'a str){values.push(value);}fn main(){let mut values:Vec<&str>=Vec::new();append(&mut values,"first");append(&mut values,"second");println!("{:?}",values);}"#,
        r#"fn main(){let owned=String::from("local");let mut values:Vec<&str>=Vec::new();values.push(owned.as_str());println!("{:?}",values);}"#,
        r#"fn observed<'a>(value:&'a str)->&'a str{println!("push {}",value);value}fn main(){let mut values:Vec<&str>=Vec::new();values.push(observed("first"));values.push(observed("second"));println!("{:?}",values);}"#,
    ] {
        for workers in [1, 8] {
            let checked = compiler::compile(source.into(), workers, true)
                .unwrap_or_else(|error| panic!("CHECK workers={workers}: {error}\n{source}"));
            assert!(checked.executable.is_none());
            assert!(
                checked
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(key, _)| key.kind != 103)
            );
        }
    }
    // Same mandatory escape as the stock/BUILD oracle above. CHECK must reject
    // it semantically before the separate raw-slot executable-proof boundary.
    let source = r#"fn main(){let escaped:Vec<&str>;{let owned=String::from("local");let mut values:Vec<&str>=Vec::new();values.push(owned.as_str());escaped=values;}println!("{:?}",escaped);}"#;
    for workers in [1, 8] {
        let failure = compiler::compile_observed(source.into(), workers, true)
            .err()
            .unwrap_or_else(|| panic!("CHECK accepted an escaped native heap element borrow"));
        assert!(failure.resource_usage.first_failure.is_none());
        assert!(
            !failure.error.to_string().contains("unsupported"),
            "workers={workers}: {}",
            failure.error
        );
    }
}

#[test]
fn native_heap_nested_affine_reborrow_preserves_aliases_moves_and_lifetimes() {
    for source in [
        r#"fn main(){let mut a=1i32;let mut v=vec![&mut a];let first=&mut v[0usize];let second=&mut v[0usize];**first=2i32;**second=3i32;}"#,
        r#"fn main(){let mut a=1i32;let mut v=vec![&mut a];let moved=v[0usize];*moved=2i32;println!("{}",v[0usize]);}"#,
        r#"fn main(){let mut outer=1i32;let mut v=vec![&mut outer];{let mut short=2i32;v[0usize]=&mut short;}println!("{}",v[0usize]);}"#,
        r#"fn main(){let mut a=1i32;let first=&mut a;let values=vec![first,first];println!("{}",values[0usize]);}"#,
        // Replacing the slot does not erase the collection's borrowed lifetime.
        r#"fn main(){let mut a=1i32;let mut b=2i32;let mut v=vec![&mut a];{let mut short=3i32;v[0usize]=&mut short;v[0usize]=&mut b;}println!("{}",v[0usize]);}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
    iterator_oracle::stock_native(
        r#"fn main(){let mut a=1i32;let mut b=5i32;let mut v:Vec<&mut i32>=vec![&mut a,&mut b];*v[0]+=10;*v[1]=7;println!("{} {}",v[0],v[1]);}"#,
    );
    iterator_oracle::stock_native(
        r#"fn main(){let mut outer=1i32;let mut replacement=2i32;let mut v=vec![&mut outer];v[0usize]=&mut replacement;println!("{}",v[0usize]);}"#,
    );
}

#[test]
fn static_string_vector_seed_replays_all_closed_inputs() {
    for source in [
        "fn main(){let values=vec![\"ccc\",\"a\",\"bb\"];println!(\"{:?}\",values);}",
        "fn main(){let values=vec![\"éx\"];println!(\"{}\",values[0]);}",
        "fn main(){let e;e=vec![1u32];let z;z=vec![0u8;3];println!(\"{} {:?}\",e[0],z);}",
        "fn main(){let mut value=1i32;let e=vec![&mut value];let s=2i32;let f=vec![&s];println!(\"{} {}\",*e[0],*f[0]);}",
        "fn main(){let mut a=1i32;let mut b=5i32;let mut v:Vec<&mut i32>=vec![&mut a,&mut b];*v[0]+=10;*v[1]=7;println!(\"{} {}\",v[0],v[1]);}",
        "struct P{x:i32}fn main(){let mut v=vec![P{x:1}];v[0].x*=5;let mut w=vec![[1i32,2]];w[0][1]+=10;w[0][0]=3;let r=w[0];println!(\"{} {:?}\",v[0].x,r);}",
        "fn main(){let mut v=vec![vec![1,2],vec![3,4]];v[0][1]+=5;let i=1;v[i][0]=7;v[i][i]*=3;let j=0usize;println!(\"{} {} {} {}\",v[0][1],v[1][0],v[i][1],v[j][j]);}",
        "fn main(){let mut a=1i32;let mut b=5i32;let mut v:Vec<&mut i32>=vec![&mut a,&mut b];let i=1;*v[i]+=10;*v[i-1]=3;let x=*v[i];println!(\"{} {} {}\",x,v[0],*v[i]);}",
        "fn main(){let v=vec![vec![1.5,2.0]];let mut m=[[0;3];2];let i=1;m[i][2]=5;m[i][0]+=m[i][2];println!(\"{} {:?}\",v[0][1]+1.0,m);}",
        "fn main(){let mut v=vec![[1i32,2],[3,4]];let i=1;v[i][0]=9;v[0][i]*=4;println!(\"{} {} {}\",v[0][1],v[i][0],v[1][i]);let w=vec![(1u8,true)];println!(\"{} {}\",w[0].0,w[0].1);}",
        "#[derive(Clone,Copy)]struct P{x:i32}fn main(){let mut v=vec![[P{x:1},P{x:2}]];let i=1;v[0][i].x+=5;v[0][0].x=7;println!(\"{} {}\",v[0][1].x,v[0][0].x);}",
        "fn main(){let m=[[1i64,2],[3,4]];let r=&m;let i=1usize;let x=r[i][0];let y=r[0][i];let mut n=[[1u8;2];2];n[1][i]=5;n[0][i]+=1;println!(\"{} {} {:?}\",x,y,n);}",
        "fn main(){let o=Some(3i32);let mut s=0;for x in o.iter(){s+=*x;}let n:Option<i32>=None;println!(\"{} {}\",s,n.iter().count());}",
        "struct Holder<T>{value:T}fn keep<T>(value:T,_capacity:usize)->Holder<T>{println!(\"called\");Holder{value}}fn main(){let held=keep(\"abc\",2usize);println!(\"{}\",held.value);}",
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        "fn main(){let _:Vec<bool>=vec![\"a\",\"b\"];}",
        "fn main(){let _=vec![\"a\",true];}",
        "fn main(){let escaped;{let value=String::from(\"local\");escaped=vec![value.as_str()];}println!(\"{}\",escaped[0]);}",
        "fn main(){let escaped;{let mut value=1i32;escaped=vec![&mut value];}println!(\"{}\",*escaped[0]);}",
        "fn main(){let mut a=1i32;let v:Vec<&mut i32>=vec![&mut a];let i=0;*v[i]=2;}",
        "fn main(){let a=1i32;let mut v:Vec<&i32>=vec![&a];let i=0;*v[i]=2;}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn nested_shared_str_len_preserves_builtin_and_method_authority() {
    for source in [
        "fn main(){let mut values=vec![\"ccc\",\"a\",\"bb\"];values.sort_by_key(|s|s.len());println!(\"{:?}\",values);}",
        "fn main(){let text=\"éx\";let a=&text;let b=&a;println!(\"{} {} {}\",text.len(),a.len(),b.len());}",
        "trait Custom{fn len(&self)->usize;}impl Custom for &&str{fn len(&self)->usize{99}}fn main(){let text=\"abc\";let nested=&text;println!(\"{} {}\",text.len(),nested.len());}",
        "fn observe<'a>(value:&'a &'static str)->&'a &'static str{println!(\"receiver\");value}fn main(){let text=\"éx\";println!(\"len {}\",observe(&text).len());}",
        "fn f(x:&'static str)->&'static str{x}fn main(){let r;{r=f(\"lit\");}println!(\"{}\",r);}",
        "fn f<'a>(x:&'a str,y:&'static str)->&'a str{if x.len()>1{y}else{x}}fn main(){let s=String::from(\"hi\");println!(\"{}\",f(&s,\"z\"));}",
        "fn f(x:&'static str)->usize{x.len()}fn h(t:&'static str)->usize{let u=t;f(u)}fn main(){const C:&str=\"abc\";println!(\"{} {}\",h(\"x\"),f(C));}",
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        "fn f(x:&'static i32)->i32{*x}fn main(){let a=3;println!(\"{}\",f(&a));}",
        "fn f(x:&'static str)->usize{x.len()}fn main(){let s=String::from(\"hi\");println!(\"{}\",f(&s));}",
        "fn f(x:&'static str)->usize{x.len()}fn h(t:&str)->usize{f(t)}fn main(){println!(\"{}\",h(\"x\"));}",
    ] {
        iterator_oracle::stock_and_native_reject(source);
    }
    for source in [
        "fn main(){let text=\"abc\";let nested=&text;let _=nested.len(0);}",
        "fn main(){let text=\"abc\";let nested=&text;let _=nested.len::<i32>();}",
        "fn main(){let text=\"abc\";let nested=&text;let _:bool=nested.len();}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn cloned_vec_owner_survives_slice_coercion_before_equality() {
    for source in [
        "fn slice(v:&mut[i32]){v[0]=7;}fn main(){let data=vec![3i32,1];let mut a=data.clone();slice(&mut a);let b=data.clone();println!(\"{}\",a==b);a.push(9);println!(\"{} {}\",a.len(),a.pop().unwrap());}",
        "fn slice(v:&mut[i32]){v[0]=7;}fn main(){let data:Vec<i32>=(0..3).map(|i|i).collect();let mut a=data.clone();slice(&mut a);let b=data.clone();println!(\"{}\",a==b);a.push(9);println!(\"{} {}\",a.len(),a.pop().unwrap());}",
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        "fn slice(_: &mut[bool]){}fn main(){let data=vec![3i32,1];let mut a=data.clone();slice(&mut a);a.push(9);}",
        "fn slice(_: &mut[i32]){}fn main(){let data=vec![3i32,1];let a=data.clone();slice(&mut a);println!(\"{}\",a==data);}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn vec_equality_preserves_elements_order_and_trait_bounds() {
    for source in [
        "fn require_eq<T:Eq>(_:&T){}fn main(){let a=vec![1i32,2];let b=vec![1i32,2];let short=vec![1i32];let empty:Vec<i32>=Vec::new();require_eq(&a);println!(\"{} {} {} {}\",a==b,a!=short,empty==Vec::<i32>::new(),a.len());}",
        "struct Probe(i32);impl PartialEq for Probe{fn eq(&self,other:&Self)->bool{println!(\"cmp {} {}\",self.0,other.0);self.0==other.0}}fn main(){let a=vec![Probe(1),Probe(2),Probe(3)];let b=vec![Probe(1),Probe(9),Probe(3)];println!(\"{}\",a==b);let short=vec![Probe(1)];println!(\"{}\",a==short);}",
        "struct Left(i32);struct Right(i32);impl PartialEq<Right> for Left{fn eq(&self,other:&Right)->bool{self.0==other.0}}fn main(){let a=vec![Left(1),Left(2)];let b=vec![Right(1),Right(2)];let c=vec![Right(1),Right(3)];println!(\"{} {}\",a==b,a==c);}",
    ] {
        iterator_oracle::stock_native(source);
    }
    for source in [
        "struct NoEq;fn unused(){let a=vec![NoEq];let b=vec![NoEq];let _equal=a==b;}fn main(){}",
        "fn main(){let a=vec![1i32];let b=vec![true];println!(\"{}\",a==b);}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "workers={workers} check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn cloned_iterator_for_loop_matches_stock() {
    for source in [
        "fn main(){let values=vec![1i32,2i32];for value in values.iter().cloned(){println!(\"{}\",value);}}",
        "#[derive(Clone)]struct Item{n:u32,text:String}fn main(){let values=vec![Item{n:1,text:String::from(\"a\")},Item{n:2,text:String::from(\"b\")}];for value in values.iter().cloned(){println!(\"{} {}\",value.n,value.text);}for value in values.iter(){println!(\"{} {}\",value.n,value.text);}}",
        "#[derive(Clone)]struct Item{n:u32}impl Drop for Item{fn drop(&mut self){println!(\"drop {}\",self.n);}}fn main(){let values=vec![Item{n:1},Item{n:2}];for value in values.iter().cloned(){println!(\"item {}\",value.n);break;}println!(\"after\");}",
        "#[derive(Clone)]struct Z;impl Drop for Z{fn drop(&mut self){println!(\"drop\");}}fn main(){let empty:Vec<Z>=Vec::new();for _ in empty.iter().cloned(){panic!();}let values=vec![Z,Z];for _ in values.iter().cloned(){println!(\"item\");}}",
    ] {
        iterator_oracle::stock_native(source);
    }
}

#[test]
fn cloned_struct_slice_elements_preserve_original_field_lifetimes() {
    use iterator_oracle::closure_oracle::{self, Case, Expect};
    closure_oracle::check(
        "cloned-struct-slice-carriers",
        &[
            Case {
                name: "cloned_field_survives_descriptor_and_vec",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let value=1i32;let escaped;{let values=vec![Item{value:&value}];let mut result=None;for item in values.iter().cloned(){result=Some(item);}escaped=result.unwrap();}println!(\"{}\",escaped.value);}",
                expect: Expect::Run("1\n"),
            },
            Case {
                name: "original_cloned_field_cannot_escape_referent",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let escaped;{let value=1i32;let values=vec![Item{value:&value}];let mut result=None;for item in values.iter().cloned(){result=Some(item);}escaped=result.unwrap();}println!(\"{}\",escaped.value);}",
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "cloned_field_keeps_original_referent_loan",
                source: "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let mut value=1i32;let values=vec![Item{value:&value}];let mut result=None;for item in values.iter().cloned(){result=Some(item);}let escaped=result.unwrap();value=2i32;println!(\"{}\",escaped.value);}",
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}

#[test]
fn cloned_iterator_for_loop_preserves_bounds_moves_and_loans() {
    for source in [
        "struct Item;fn unused(values:Vec<Item>){for _ in values.iter().cloned(){}}fn main(){}",
        "fn unused(values:Vec<i32>){for _ in values.into_iter().cloned(){}}fn main(){}",
        "fn main(){let mut values=vec![1i32,2];for value in values.iter().cloned(){values.push(value);}}",
        "fn main(){let values=vec![1i32,2];let iter=values.iter().cloned();for _ in iter{}for _ in iter{}}",
        "#[derive(Clone)]struct Item<'a>{value:&'a i32}fn main(){let escaped;{let value=1i32;let values=vec![Item{value:&value}];let mut result=None;for item in values.iter().cloned(){result=Some(item);}escaped=result.unwrap();}println!(\"{}\",escaped.value);}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                failure.resource_usage.first_failure.is_none(),
                "resource failure for source={source:?}, workers={workers}, check={check}: {}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "unsupported rejection for source={source:?}, workers={workers}, check={check}: {}",
                failure.error
            );
        });
    }
}

fn compare(source: &str, expected: &str, tag: &str) {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("jr-vec-algorithms-{}-{tag}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let stock = Command::new("rustc")
        .args(["--edition", "2024", "-C", "panic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let stock = Command::new(root.join("stock")).output().unwrap();
    assert!(stock.status.success());
    assert_eq!(stock.stdout, expected.as_bytes());
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
        let built = compiler::compile(source.into(), workers, false).unwrap();
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(&binary).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, stock.stdout);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vec_comparison_algorithms() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(3); values.push(1); values.push(2); values.push(2); values.push(3);
    values.sort(); values.dedup();
    println!("{} {} {} {}", values.len(), values[0], values[1], values[2]);
    println!("{} {}", values.contains(&2), values.contains(&4));
    match values.binary_search(&2) { Ok(i) => println!("found {}", i), Err(i) => println!("missing {}", i) }
    match values.binary_search(&4) { Ok(i) => println!("found {}", i), Err(i) => println!("missing {}", i) }
    let mut empty: Vec<i32> = Vec::new();
    empty.sort(); empty.dedup();
    println!("{} {}", empty.len(), empty.contains(&0));
}
"#,
        "3 1 2 3\ntrue false\nfound 1\nmissing 3\n0 false\n",
        "integers",
    );
}

#[test]
fn vec_stable_noncopy_sort_and_dedup() {
    compare(
        r#"
struct Item { key: i32, order: i32 }
impl PartialEq for Item {
    fn eq(&self, other: &Item) -> bool { self.key == other.key }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Item) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Item) -> std::cmp::Ordering { self.key.cmp(&other.key) }
}
fn main() {
    let mut values: Vec<Item> = Vec::new();
    values.push(Item { key: 2, order: 0 });
    values.push(Item { key: 1, order: 1 });
    values.push(Item { key: 2, order: 2 });
    values.push(Item { key: 1, order: 3 });
    values.sort();
    let a = &values[0]; let b = &values[1];
    let c = &values[2]; let d = &values[3];
    println!("{} {} {} {}", a.order, b.order, c.order, d.order);
    values.dedup();
    let first = &values[0]; let second = &values[1];
    println!("{} {} {}", values.len(), first.order, second.order);
    println!("{}", values.contains(&Item { key: 2, order: 99 }));
}
"#,
        "1 3 0 2\n2 1 0\ntrue\n",
        "noncopy",
    );
}

#[test]
fn vec_sort_by_and_sort_by_key_are_stable() {
    compare(
        r#"
struct Item { key: i32, order: i32 }
fn main() {
    let mut v: Vec<Item> = Vec::new();
    v.push(Item { key: 2, order: 0 });
    v.push(Item { key: 1, order: 1 });
    v.push(Item { key: 2, order: 2 });
    v.push(Item { key: 1, order: 3 });
    v.sort_by(|a, b| b.key.cmp(&a.key));
    let (a, b, c, d) = (v[0].order, v[1].order, v[2].order, v[3].order);
    println!("{} {} {} {}", a, b, c, d);
    v.sort_by_key(|x| x.key);
    let (a, b, c, d) = (v[0].order, v[1].order, v[2].order, v[3].order);
    println!("{} {} {} {}", a, b, c, d);
    let mut n = vec![5i64, -3, 4, -1];
    n.sort_by_key(|x| x.abs());
    println!("{:?}", n);
    let mut e: Vec<i32> = Vec::new();
    e.sort_by(|a, b| a.cmp(b));
    println!("{}", e.len());
}
"#,
        "0 2 1 3\n1 3 0 2\n[-1, -3, 4, 5]\n0\n",
        "sort-by",
    );
}

#[test]
fn owned_into_iter_moves_elements_and_frees_remainder() {
    compare(
        "fn main(){let v=vec![1u32,2,3];let mut t=0u32;for x in v {t+=x;}println!(\"{}\",t);\
let s=vec![String::from(\"a\"),String::from(\"b\"),String::from(\"c\")];for x in s {println!(\"{}\",x);}\
let w=vec![String::from(\"x\"),String::from(\"y\"),String::from(\"z\")];let mut it=w.into_iter();\
println!(\"{:?} {:?}\",it.next(),it.next_back());let e:Vec<u8>=Vec::new();for _ in e {println!(\"never\");}}\n",
        "6\na\nb\nc\nSome(\"x\") Some(\"z\")\n",
        "into-iter",
    );
}

#[test]
fn owned_for_over_user_drop_elements_matches_stock_order() {
    compare(
        "struct D{n:u32}\nimpl Drop for D { fn drop(&mut self){ println!(\"drop {}\", self.n); } }\n\
fn main(){ let w=vec![D{n:1},D{n:2},D{n:3}]; for d in w { if d.n==2 { break; } println!(\"got {}\", d.n); } println!(\"end\"); }\n",
        "got 1\ndrop 1\ndrop 2\ndrop 3\nend\n",
        "into-iter-drop",
    );
}

#[test]
fn refcell_guards_track_borrows() {
    compare(
        "use std::cell::RefCell;\n\
fn main(){let c=RefCell::new(5u32);{let mut w=c.borrow_mut();*w+=1;}let r1=c.borrow();let r2=c.borrow();println!(\"{} {}\",*r1,*r2);\
let v=RefCell::new(vec![1u32]);let n=v.borrow().len();let mut g=v.borrow_mut();g.push(2u32);println!(\"{} {}\",n,g.len());}\n",
        "6 6\n1 2\n",
        "refcell",
    );
    compare(
        "use std::cell::RefCell;\nstruct L{v:RefCell<u32>}\n\
fn main(){let c=RefCell::new(1u32);*c.borrow_mut()=7;*c.borrow_mut()+=1;let l=L{v:RefCell::new(2)};*l.v.borrow_mut()*=5;\
let a=c.borrow();let b=l.v.borrow();println!(\"{} {}\",*a,*b);}\n",
        "8 10\n",
        "refcell-temp-assign",
    );
    compare(
        "use std::cell::RefCell;\n\
fn main(){let c:RefCell<Vec<u32>>=RefCell::new(Vec::new());c.borrow_mut().push(3u32);c.borrow_mut().push(4);\
println!(\"{} {:?}\",c.borrow().len(),c.borrow().last());}\n",
        "2 Some(4)\n",
        "refcell-temp-receiver",
    );
    compare(
        "use std::cell::RefCell;use std::rc::Rc;\nstruct L{n:u32,s:RefCell<Vec<String>>}\n\
fn main(){let l=Rc::new(L{n:3,s:RefCell::new(Vec::new())});l.s.borrow_mut().push(\"a\".to_string());\
l.s.borrow_mut().push(\"b\".to_string());for x in l.s.borrow().iter(){println!(\"{x}\");}\
let c=RefCell::new(vec![1u32,2]);let g=c.borrow();for x in g.iter(){println!(\"{x}\");}println!(\"{}\",l.n);}\n",
        "a\nb\n1\n2\n3\n",
        "deref-field-receiver",
    );
}

#[test]
fn shared_ledger_fallible_borrows_match_stock() {
    iterator_oracle::stock_native(include_str!("fixtures/full_programs/38_shared_ledger.rs"));
}

#[test]
fn generic_owner_method_chains_replay_consumed_borrowed_values() {
    iterator_oracle::stock_native(
        r#"struct Owner<T>{n:T}
struct View<'a,T>{r:&'a T}
struct Slot<T>{v:T}
impl<T> Owner<T>{fn slot<'a>(&'a self)->Slot<View<'a,T>>{Slot{v:View{r:&self.n}}}}
impl<T> Slot<T>{fn take(self)->T{self.v}}
fn consume<'a,T:std::fmt::Display>(v:View<'a,T>){println!("{}",v.r);}
fn main(){let owner=Owner{n:7i32};let row=owner.slot().take();consume(row);let text=Owner{n:String::from("shared")};let row=text.slot().take();consume(row);}"#,
    );
}

#[test]
fn generic_owner_method_chains_replay_free_call_receiver() {
    iterator_oracle::stock_native(
        r#"struct View<'a,T>{r:&'a T}
struct Slot<T>{v:T}
impl<T> Slot<T>{fn take(self)->T{self.v}}
fn make<'a,T>(x:&'a T)->Slot<View<'a,T>>{Slot{v:View{r:x}}}
fn consume<'a,T:std::fmt::Display>(v:View<'a,T>){println!("{}",v.r);}
fn main(){let n=13i32;let row=make(&n).take();consume(row);let text=String::from("fresh");let row=make(&text).take();consume(row);}"#,
    );
}

#[test]
fn refcell_fallible_borrow_state_transitions_match_stock() {
    iterator_oracle::stock_native(
        r#"use std::cell::RefCell;
fn main(){let c=RefCell::new(vec![1i32]);let a=c.try_borrow().unwrap();let b=c.try_borrow().unwrap();
println!("{} {} {}",a.len(),b.len(),c.try_borrow_mut().is_err());drop(a);println!("{}",c.try_borrow_mut().is_err());drop(b);
{let mut w=c.try_borrow_mut().unwrap();w.push(2);println!("{} {}",c.try_borrow().is_err(),c.try_borrow_mut().is_err());}
println!("{:?}",*c.try_borrow().unwrap());}"#,
    );
}

#[test]
fn refcell_fallible_errors_and_early_return_release_match_stock() {
    iterator_oracle::stock_native(
        r#"use std::cell::{RefCell,BorrowError,BorrowMutError};
fn reader(c:&RefCell<i32>)->Result<i32,BorrowError>{let g=c.try_borrow()?;Ok(*g)}
fn writer(c:&RefCell<i32>)->Result<(),BorrowMutError>{let mut g=c.try_borrow_mut()?;*g+=1;Ok(())}
fn main(){let c=RefCell::new(3i32);let g=c.borrow();let e=writer(&c).unwrap_err();println!("{} {:?}",e,e);drop(g);writer(&c).unwrap();
let w=c.borrow_mut();let e=reader(&c).unwrap_err();println!("{} {:?}",e,e);drop(w);println!("{}",reader(&c).unwrap());}"#,
    );
}

#[test]
fn refcell_fallible_guard_return_tied_to_caller_matches_stock() {
    iterator_oracle::stock_native(
        r#"use std::cell::{RefCell,Ref,RefMut,BorrowError,BorrowMutError};
fn read<'a,T>(c:&'a RefCell<T>)->Result<Ref<'a,T>,BorrowError>{c.try_borrow()}
fn edit<'a,T>(c:&'a RefCell<T>)->Result<RefMut<'a,T>,BorrowMutError>{c.try_borrow_mut()}
fn main(){let c=RefCell::new(String::from("a"));{let mut g=edit(&c).unwrap();g.push_str("b");}let g=read(&c).unwrap();println!("{}",*g);}"#,
    );
}

#[test]
fn real_reference_wrapper_return_tied_to_caller_matches_stock() {
    iterator_oracle::stock_native(
        r#"struct Owner{value:i32,token:()}struct View<'a>{value:*const i32,life:&'a ()}
impl Owner{fn view<'a>(&'a self)->View<'a>{View{value:&self.value as *const i32,life:&self.token}}}
fn view<'a>(o:&'a Owner)->View<'a>{o.view()}
fn main(){let o=Owner{value:7,token:()};let v=view(&o);println!("{} {:?}",unsafe{*v.value},v.life);}"#,
    );
}

#[test]
fn refcell_fallible_borrows_reject_invalid_unused_programs() {
    for source in [
        "use std::cell::RefCell;fn unused(c:&RefCell<i32>){let mut r=c.try_borrow().unwrap();*r=4;}fn main(){}",
        "use std::cell::{RefCell,Ref};fn unused()->Ref<'static,i32>{let c=RefCell::new(1);c.try_borrow().unwrap()}fn main(){}",
        "use std::cell::{RefCell,RefMut};fn unused()->RefMut<'static,i32>{let c=RefCell::new(1);c.try_borrow_mut().unwrap()}fn main(){}",
        "use std::cell::{RefCell,Ref,BorrowError};fn unused()->Result<Ref<'static,i32>,BorrowError>{let c=RefCell::new(1);c.try_borrow()}fn main(){}",
        "use std::cell::RefCell;fn unused(){let c=RefCell::new(1i32);let g=c.try_borrow().unwrap();drop(c);println!(\"{}\",*g);}fn main(){}",
        "use std::cell::RefCell;fn unused(){let c=RefCell::new(1i32);let mut g=c.try_borrow_mut().unwrap();let moved=c;*g=2;drop(moved);}fn main(){}",
        "struct Owner{value:i32,token:()}struct View<'a>{value:*const i32,life:&'a ()}impl Owner{fn view<'a>(&'a self)->View<'a>{View{value:&self.value as *const i32,life:&self.token}}}fn unused()->View<'static>{let o=Owner{value:7,token:()};o.view()}fn main(){}",
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(failure.resource_usage.first_failure.is_none());
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn field_reads_through_nested_references() {
    compare(
        "struct It{id:u32,q:u32}\nfn f(i:&&It)->u32{i.q}\n\
fn main(){let v=vec![It{id:2,q:0},It{id:1,q:4}];let n=v.iter().filter(|i|i.q==0).count();\
let e:Vec<&It>=v.iter().filter(|i|i.q==4).collect();let r=&v[0];println!(\"{} {} {} {}\",n,e.len(),f(&r),v[1].id);}\n",
        "1 1 0 1\n",
        "nested-ref-field",
    );
}

#[test]
fn collect_into_partially_annotated_vec() {
    compare(
        "fn main(){let w=vec![(1u32,2u64)];let x:Vec<_>=w.into_iter().collect();println!(\"{:?}\",x);\
let v=vec![1u32,5];let y:Vec<_>=v.iter().map(|a|a+1).collect();println!(\"{:?}\",y);}\n",
        "[(1, 2)]\n[2, 6]\n",
        "collect-vec-infer",
    );
}

#[test]
fn index_autoderefs_through_guards_and_boxes() {
    compare(
        "use std::cell::RefCell;\nfn mk()->Box<Vec<u32>>{Box::new(vec![5,6])}\n\
fn main(){let c=RefCell::new(vec![3u32,4]);let x=c.borrow()[1];c.borrow_mut().push(x);let i=2;\
let y=c.borrow()[i]+c.borrow()[0];c.borrow_mut().push(9);println!(\"{} {} {:?} {:?}\",x,y,c.borrow(),c.borrow());\
let b:Box<Vec<u32>>=Box::new(vec![7,8]);println!(\"{} {}\",b[1],mk()[0]);}\n",
        "4 7 [3, 4, 4, 9] [3, 4, 4, 9]\n8 5\n",
        "index-autoderef",
    );
}

#[test]
fn rc_shares_ownership_and_drops_once() {
    compare(
        "use std::rc::Rc;\nstruct N{v:u32}\nimpl Drop for N{fn drop(&mut self){println!(\"drop {}\",self.v);}}\n\
fn main(){let a=Rc::new(N{v:7});println!(\"{}\",Rc::strong_count(&a));{let b=a.clone();let c=b.clone();println!(\"{} {}\",Rc::strong_count(&a),c.v);}\
println!(\"{} {}\",Rc::strong_count(&a),a.v);let s=Rc::new(String::from(\"hi\"));println!(\"{}\",s.len());\
let x=Rc::new(5u32);let y=x.clone();println!(\"{} {} {}\",*x,*y,Rc::ptr_eq(&x,&y));let bx=Box::new(9u32);println!(\"{}\",*bx);}\n",
        "1\n3 7\n1 7\n2\n5 5 true\n9\ndrop 7\n",
        "rc",
    );
}

#[test]
fn mutable_method_autoderef_through_box_matches_stock() {
    compare(
        "fn main(){let mut b=Box::new(vec![1u32]);b.push(2);b.push(3);println!(\"{} {:?}\",b.len(),b.pop());\
let r=std::rc::Rc::new(vec![4u32,5]);println!(\"{}\",r.len());}\n",
        "3 Some(3)\n2\n",
        "box-autoderef-mut",
    );
}

#[test]
fn rc_dyn_coercion_preserves_aliases_counts_and_last_owner_drop() {
    compare(
        r#"
use std::rc::Rc;
use std::cell::RefCell;
trait Observer { fn notify(&self, delta:u32)->u32; }
struct Tail { id:u32 }
impl Drop for Tail { fn drop(&mut self){println!("tail {}",self.id);} }
struct Log { value:RefCell<u32>, tail:Tail }
impl Observer for Log {
    fn notify(&self,delta:u32)->u32 {let mut value=self.value.borrow_mut();*value+=delta;*value}
}
impl Drop for Log {fn drop(&mut self){println!("log {}",self.value.borrow());}}
fn attempt(owner:Rc<dyn Observer>,fail:bool)->Result<u32,u32> {
    if fail {return Err(owner.notify(3));}
    Ok(owner.notify(1))
}
fn erase(owner:Rc<Log>)->Rc<dyn Observer>{owner}
fn main(){
    let typed=Rc::new(Log{value:RefCell::new(4),tail:Tail{id:7}});
    let object:Rc<dyn Observer>=typed.clone();
    let alias=object.clone();
    println!("{} {} {}",Rc::strong_count(&typed),Rc::strong_count(&object),Rc::ptr_eq(&object,&alias));
    println!("{} {}",object.notify(2),typed.value.borrow());
    println!("{:?} {:?}",attempt(alias,true),attempt(object.clone(),false));
    println!("{} {}",Rc::strong_count(&typed),typed.value.borrow());
    drop(typed);
    println!("{}",Rc::strong_count(&object));
    drop(object);
    let typed=Rc::new(Log{value:RefCell::new(11),tail:Tail{id:8}});
    {let object=erase(typed.clone());println!("{}",object.notify(1));}
    println!("{}",Rc::strong_count(&typed));
}
"#,
        "3 3 true\n6 6\nErr(9) Ok(10)\n2 10\n1\nlog 10\ntail 7\n12\n1\nlog 12\ntail 8\n",
        "rc-dyn-alias-drop",
    );
}

#[test]
fn rc_dyn_zero_sized_pointee_drops_once_per_allocation() {
    compare(
        r#"
use std::rc::Rc;
trait Read {fn read(&self)->u32;}
struct Z;
impl Read for Z {fn read(&self)->u32{42}}
impl Drop for Z {fn drop(&mut self){println!("drop Z");}}
fn main(){
    let typed=Rc::new(Z);
    let object:Rc<dyn Read>=typed.clone();
    let alias=object.clone();
    let other:Rc<dyn Read>=Rc::new(Z);
    println!("{} {} {}",object.read(),Rc::ptr_eq(&object,&alias),Rc::ptr_eq(&object,&other));
    drop(object);drop(typed);
    println!("{}",Rc::strong_count(&alias));
    drop(alias);drop(other);
}
"#,
        "42 true false\n1\ndrop Z\ndrop Z\n",
        "rc-dyn-zst-drop",
    );
}

#[test]
fn rc_drop_preserves_original_pointee_address_for_both_last_owner_kinds() {
    compare(
        r#"
use std::rc::Rc;
use std::cell::RefCell;
trait Read{fn read(&self)->u32;}
struct N{address:RefCell<usize>}
impl Read for N{fn read(&self)->u32{7}}
impl Drop for N{
    fn drop(&mut self){
        let recorded=*self.address.borrow();
        let here=self as *const N as usize;
        println!("{}",recorded==here);
    }
}
fn main(){
    let typed=Rc::new(N{address:RefCell::new(0)});
    let address=&*typed as *const N as usize;
    *typed.address.borrow_mut()=address;
    let object:Rc<dyn Read>=typed.clone();
    drop(object);drop(typed);
    let typed=Rc::new(N{address:RefCell::new(0)});
    let address=&*typed as *const N as usize;
    *typed.address.borrow_mut()=address;
    let object:Rc<dyn Read>=typed.clone();
    drop(typed);drop(object);
}
"#,
        "true\ntrue\n",
        "rc-drop-address",
    );
}

#[test]
fn iterator_max_by_matches_stock() {
    compare(
        "fn main(){let v=vec![3i32,9,2,9,1];let x=v.clone().into_iter().max_by(|a,b|a.cmp(b));\
let y=v.clone().into_iter().min_by(|a,b|a.cmp(b));let e:Vec<i32>=Vec::new();println!(\"{:?} {:?} {:?}\",x,y,e.into_iter().max_by(|a,b|a.cmp(b)));}\n",
        "Some(9) Some(1) None\n",
        "max-by",
    );
    compare(
        "trait S{fn a(&self)->f64;}\nstruct C{r:f64}\nimpl S for C{fn a(&self)->f64{self.r}}\n\
fn id(x:&String)->&String{x}\n\
fn main(){let v=vec![1.0f64,3.0,2.0];let b=v.iter().max_by(|a,b|a.partial_cmp(b).unwrap()).unwrap();\
let m=v.iter().min_by(|a,b|a.partial_cmp(b).unwrap()).unwrap();println!(\"{} {}\",b,m);\
let w:Vec<Box<dyn S>>=vec![Box::new(C{r:1.0}),Box::new(C{r:3.0})];\
let x=w.iter().max_by(|a,b|{let x:&Box<dyn S>=a;let y:&Box<dyn S>=b;x.a().partial_cmp(&y.a()).unwrap()}).unwrap();println!(\"{}\",x.a());\
let mut s=String::from(\"a\");let mut r=id(&s);let mut i=0;while i<2{r=id(&s);i+=1;}println!(\"{}\",r);s.push('x');println!(\"{}\",s);}\n",
        "3 1\n3\na\nax\n",
        "max-by-ref",
    );
    compare(
        "trait S{fn a(&self)->f64;}\nstruct C{r:f64}\nimpl S for C{fn a(&self)->f64{self.r}}\n\
fn main(){let v:Vec<Box<dyn S>>=vec![Box::new(C{r:1.0}),Box::new(C{r:3.0})];\
let b=v.iter().max_by(|a,b|a.a().partial_cmp(&b.a()).unwrap()).unwrap();let r=&b;let rr=&r;println!(\"{} {}\",b.a(),rr.a());}\n",
        "3 3\n",
        "max-by-ref-box-receiver",
    );
    compare(
        "fn main(){let v=vec![3u32,9,2,9,1];let m=v.iter().max_by_key(|x|**x%5).unwrap();\
let n=v.iter().min_by_key(|x|**x).unwrap();let e:Vec<u32>=Vec::new();println!(\"{} {} {:?}\",m,n,e.iter().max_by_key(|x|**x));\
let s=vec![String::from(\"ab\"),String::from(\"abcd\"),String::from(\"xy\")];\
println!(\"{} {}\",s.iter().max_by_key(|s|s.len()).unwrap(),s.iter().min_by_key(|s|s.len()).unwrap());}\n",
        "9 1 None\nabcd ab\n",
        "max-by-key",
    );
}

#[test]
fn vec_reference_coerces_to_slice_parameter() {
    compare(
        "trait S{fn a(&self)->f64;}\nstruct C{r:f64}\nimpl S for C{fn a(&self)->f64{self.r}}\n\
fn t(s:&[Box<dyn S>])->f64{s.iter().map(|x|x.a()).sum()}\nfn first(s:&[u32])->u32{s[0]}\nfn n(s:&[String])->usize{s.len()}\n\
fn main(){let v:Vec<Box<dyn S>>=vec![Box::new(C{r:1.5}),Box::new(C{r:2.0})];println!(\"{}\",t(&v));\
let w=vec![4u32,5];let x=vec![String::from(\"a\")];println!(\"{} {}\",first(&w),n(&x));}\n",
        "3.5\n4 1\n",
        "vec-deref-slice",
    );
}

#[test]
fn str_equality_and_borrow_match_stock() {
    compare(
        "use std::borrow::Borrow;\nfn eqk<K: Borrow<Q>, Q: ?Sized + PartialEq>(k: &K, q: &Q) -> bool { <K as Borrow<Q>>::borrow(k) == q }\n\
fn main(){let s=String::from(\"blue\");let a=\"x\";let b=String::from(\"x\");\
println!(\"{} {} {} {} {}\",eqk::<String,str>(&s,\"blue\"),eqk::<String,str>(&s,\"red\"),a==b,b==a,\"x\"==\"y\");}\n",
        "true false true true false\n",
        "str-borrow",
    );
}

#[test]
fn closed_method_arguments_mut_receiver_match_stock() {
    iterator_oracle::stock_native(
        r#"
struct Owner{n:i64}
struct Output{n:i64}
impl Drop for Output{fn drop(&mut self){println!("owned {}",self.n);}}
struct View<'a>{r:&'a i64}
impl Owner{
    fn make(&mut self,label:&str,n:i64)->Output{self.n+=n;println!("{} {}",label,self.n);Output{n:self.n}}
    fn view<'a>(&'a mut self,n:i64)->View<'a>{self.n+=n;View{r:&self.n}}
}
fn consume<T>(value:T){drop(value);}
fn examine<T>(value:&T){}
fn main(){
    let mut owner=Owner{n:10i64};
    let out=owner.make("deposit",7i64);examine(&out);consume(out);
    let row=owner.view(3i64);println!("view {}",row.r);consume(row);
    println!("final {}",owner.n);
}
"#,
    );
}

#[test]
fn reference_free_leaf_reborrow_with_copy_token_matches_stock() {
    iterator_oracle::stock_native(
        r#"
struct Owner<'a,T>{token:&'a (),value:T}
fn leaf<'a,'b,T>(owner:&'b mut Owner<'a,T>)->&'b mut T{&mut owner.value}
fn main(){
    let token=();
    let mut text=Owner{token:&token,value:String::from("account")};
    leaf(&mut text).push_str(" updated");
    println!("{}",text.value);
    let mut numbers=Owner{token:&token,value:vec![2i32,3i32]};
    leaf(&mut numbers).push(5i32);
    println!("{:?}",numbers.value);
}
"#,
    );
}

#[test]
fn reference_free_leaf_reborrow_drop_leaf_matches_stock() {
    iterator_oracle::stock_native(
        r#"
struct Owner<'a,T>{token:&'a (),value:T}
fn leaf<'a,'b,T>(owner:&'b mut Owner<'a,T>)->&'b mut T{&mut owner.value}
struct Notice{value:i32}
impl Drop for Notice{fn drop(&mut self){println!("drop {}",self.value);}}
fn main(){let token=();let mut owner=Owner{token:&token,value:Notice{value:7i32}};let r=leaf(&mut owner);*r=Notice{value:11i32};println!("value {}",owner.value.value);}
"#,
    );
}

#[test]
fn string_field_drop_after_leaf_reborrow_matches_stock() {
    iterator_oracle::stock_native(
        r#"
struct Owner<'a,T>{token:&'a (),value:T}
fn leaf<'a,'b,T>(owner:&'b mut Owner<'a,T>)->&'b mut T{&mut owner.value}
struct Notice{value:String}
impl Drop for Notice{fn drop(&mut self){println!("drop {}",self.value);}}
fn main(){let token=();let mut notice=Owner{token:&token,value:Notice{value:String::from("old")}};let r=leaf(&mut notice);r.value=String::from("new");println!("{}",notice.value.value);}
"#,
    );
}

#[test]
fn reference_free_leaf_reborrow_excludes_noncopy_carrier_referents() {
    let source = r#"struct Owner<'a,T>{token:&'a (),value:T}fn leaf<'a,'b,T>(o:&'b mut Owner<'a,T>)->&'b mut T{&mut o.value}fn escape<'a>(token:&'a (),r:&'a i32)->&'static mut Vec<&'a i32>{let mut o=Owner{token,value:vec![r]};leaf(&mut o)}fn main(){}"#;
    let root = std::env::temp_dir().join(format!(
        "jr-reference-free-exclusion-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    fs::write(&input, source).unwrap();
    let stock = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "--emit=metadata"])
        .arg(&input)
        .arg("-o")
        .arg(root.join("out.rmeta"))
        .output()
        .unwrap();
    assert!(
        !stock.status.success() && String::from_utf8_lossy(&stock.stderr).contains("E0515"),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    for workers in [1, 8] {
        let checked = compiler::compile_observed(source.into(), workers, true)
            .err()
            .expect("escaping owner must fail CHECK");
        assert!(checked.resource_usage.first_failure.is_none());
        iterator_oracle::validate_native_rejection(&checked).unwrap();
        // BUILD also reaches a deliberately unsupported closed generic family.
        // That is a fail-closed exclusion, not semantic rejection evidence.
        let failure = compiler::compile_observed(source.into(), workers, false)
            .err()
            .unwrap_or_else(|| {
                panic!("carrier-bearing non-Copy leaf must not use reference-free admission")
            });
        assert!(failure.resource_usage.first_failure.is_none());
        let report = failure.report.as_ref().expect("completed graph report");
        assert!(
            report.node_errors.iter().any(|(_, error)| {
                let text = format!("{error:?}");
                text.contains("instance crate::leaf (")
                    && text.contains(
                        "unsupported original carrier body requires a complete closed family proof",
                    )
            }),
            "workers={workers}: {failure}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reference_free_leaf_reborrow_rejects_overlap_and_owner_moves() {
    for source in [
        r#"struct Owner<'a,T>{token:&'a (),value:T}fn leaf<'a,'b,T>(o:&'b mut Owner<'a,T>)->&'b mut T{&mut o.value}fn main(){let token=();let mut o=Owner{token:&token,value:String::from("x")};let a=leaf(&mut o);let b=leaf(&mut o);println!("{} {}",&*a,&*b);}"#,
        r#"struct Owner<'a,T>{token:&'a (),value:T}fn leaf<'a,'b,T>(o:&'b mut Owner<'a,T>)->&'b mut T{&mut o.value}fn main(){let token=();let mut o=Owner{token:&token,value:String::from("x")};let a=leaf(&mut o);drop(o);println!("{}",&*a);}"#,
        r#"struct Owner<'a,T>{token:&'a (),value:T}fn leaf<'a,'b,T>(o:&'b mut Owner<'a,T>)->&'b mut T{&mut o.value}fn escape<'a>(token:&'a ())->&'static mut &'a i32{let n=7;let mut o=Owner{token,value:&n};leaf(&mut o)}fn main(){}"#,
    ] {
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}

#[test]
fn vec_drop_may_dangle_copy_reference_elements() {
    for source in [
        r#"fn main(){let mut values=Vec::new();let late=5i32;values.push(&late);println!("{}",values[0usize]);}"#,
        r#"fn keep<'a>(values:&mut Vec<&'a i32>,value:&'a i32){values.push(value);}fn main(){let mut values=Vec::new();let late=7i32;keep(&mut values,&late);println!("{}",values.len());}"#,
    ] {
        iterator_oracle::stock_native(source);
    }
    {
        let source = r#"struct D<'a>(&'a i32);impl Drop for D<'_>{fn drop(&mut self){println!("{}",self.0);}}fn main(){let mut values=Vec::new();let late=5i32;values.push(D(&late));}"#;
        iterator_oracle::stock_and_native_reject_observed(source, |workers, check, failure| {
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "workers={workers} check={check}: {}",
                failure.error
            );
        });
    }
}
