//! Source-library Vec algorithms compared with stock Rust at workers 1 and 8.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/iterator_oracle.rs"]
mod iterator_oracle;

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
        r#"fn head_mut<'a>(values:&'a mut Vec<&'static str>)->&'a mut &'static str{&mut values[0usize]}fn main(){let mut values=vec!["first"];*head_mut(&mut values)="second";println!("{}",values[0usize]);}"#,
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
                    "workers={workers} check={check}: {}",
                    failure.error
                );
            }
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
