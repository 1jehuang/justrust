//! Ordinary source-library Vec indexing, compared with stock Rust.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

fn output(source: &str, workers: usize, tag: &str) -> Vec<u8> {
    let compiled = compiler::compile(source.into(), workers, false)
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    let path = std::env::temp_dir().join(format!(
        "jr-vec-index-{}-{workers}-{tag}",
        std::process::id()
    ));
    fs::write(&path, compiled.executable.unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::new(&path).output().unwrap();
    fs::remove_file(path).unwrap();
    assert!(result.status.success(), "{result:?}");
    result.stdout
}

fn compare(source: &str, expected: &str, tag: &str) {
    let root = std::env::temp_dir().join(format!("jr-vec-oracle-{}-{tag}", std::process::id()));
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
        assert_eq!(output(source, workers, tag), stock.stdout);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vec_shared_index_places() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(11);
    values.push(22);
    values.push(33);
    let i: usize = 2;
    let a = values[0];
    let b = values[i];
    let r = &values[1];
    let s = &values[i];
    println!("{} {} {} {} {}", a, b, r, s, values[1]);
    let text = format!("{}:{}", values[0], values[i]);
    println!("{}", text);
}
"#,
        "11 33 22 33 22\n11:33\n",
        "shared",
    );
}

#[test]
fn vec_index_format_borrows_noncopy_once() {
    compare(
        r#"
fn next(counter: &mut usize) -> usize {
    *counter += 1;
    0
}
fn main() {
    let mut words: Vec<String> = Vec::new();
    words.push(String::from("hello"));
    let mut count: usize = 0;
    println!("{}", words[next(&mut count)]);
    let text = format!("{}", words[next(&mut count)]);
    println!("{} {} {}", text, words[0], count);
}
"#,
        "hello\nhello hello 2\n",
        "noncopy",
    );
}

#[test]
fn vec_out_of_bounds_read_panics() {
    for source in [
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; let _x = v[i]; }",
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; v[i] = 9; }",
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; let _x = &mut v[i]; }",
    ] {
        for workers in [1, 8] {
            let compiled = compiler::compile(source.into(), workers, false).unwrap();
            let path = std::env::temp_dir()
                .join(format!("jr-vec-bounds-{}-{workers}", std::process::id()));
            fs::write(&path, compiled.executable.unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let result = Command::new(&path).output().unwrap();
            fs::remove_file(path).unwrap();
            assert_eq!(result.status.signal(), Some(6), "{result:?}");
        }
    }
}

#[test]
fn vec_mutable_index_places() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(11);
    values.push(22);
    let i: usize = 1;
    values[0] = 5;
    values[i] = values[0] + 3;
    values[0] += 7;
    values[i] *= 2;
    let element = &mut values[0];
    *element += 1;
    let other = &mut values[i];
    *other -= 2;
    println!("{} {}", values[0], values[i]);
}
"#,
        "13 14\n",
        "mutable",
    );
}

#[test]
fn vec_immutable_index_assignment_is_rejected() {
    for source in [
        "fn main() { let v: Vec<i32> = Vec::new(); v[0] = 1; }",
        "fn main() { let v: Vec<i32> = Vec::new(); v[0] += 1; }",
        "fn main() { let v: Vec<i32> = Vec::new(); let r = &mut v[0]; *r = 1; }",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile(source.into(), workers, true).is_err(),
                "{source}"
            );
            assert!(
                compiler::compile(source.into(), workers, false).is_err(),
                "{source}"
            );
        }
    }
}

/// G4 corpus 06/16: index autoderef through `&Vec`/`&mut Vec` (Index and
/// IndexMut on `*r`), constant-index operands through `&[T]` in generic
/// comparisons, and `Vec<String>::join`.
#[test]
fn reference_index_autoderef_and_join_match_stock() {
    compare(
        "fn insertion(v: &mut Vec<i32>) {
    for i in 1..v.len() {
        let key = v[i];
        let mut j = i;
        while j > 0 && v[j - 1] > key {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = key;
    }
    v[0] += 100;
}
fn read(v: &Vec<i32>, i: usize) -> i32 { v[i] }
fn same<T: PartialEq + Copy>(v: &[T]) -> bool { v[0] == v[1] }
fn below<T: PartialOrd + Copy>(v: &[T], p: T) -> bool { v[0] < p }
fn main() {
    let mut v = vec![5, 2, 9, 1];
    insertion(&mut v);
    println!(\"{} {} {} {}\", v[0], read(&v, 1), v[2], v[3]);
    println!(\"{} {} {}\", same(&[1, 1, 2]), same(&[2.5, 3.0]), below(&[1u8, 2], 2));
    let parts: Vec<String> = vec![String::from(\"a\"), String::from(\"bc\")];
    println!(\"[{}]\", parts.join(\", \"));
}
",
        "101 2 5 9\ntrue false true\n[a, bc]\n",
        "ref-autoderef",
    );
}

#[test]
fn index_assignment_through_shared_vec_reference_is_rejected() {
    let source = "fn f(v: &Vec<i32>) { v[0] = 1; } fn main() { let v = vec![1]; f(&v); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
    let source = "fn main() { let v = vec![String::from(\"a\")]; let s: String = v.join(1); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[path = "support/closure_oracle.rs"]
mod range_receiver_oracle;

#[test]
fn vec_element_method_receivers_match_stock() {
    use range_receiver_oracle::{Case, Expect};
    range_receiver_oracle::check(
        "vec-element-method-receiver",
        &[
            Case {
                name: "noncopy_clone_and_mutable_receiver_index_once",
                source: "#[derive(Clone)]struct Item{value:String}impl Item{fn size(&self)->usize{self.value.len()}fn clear(&mut self){self.value.clear();}}fn next(n:&mut usize)->usize{*n+=1;0}fn main(){let mut items=vec![Item{value:String::from(\"abc\")}];let mut n=0usize;let copy=items[next(&mut n)].clone();println!(\"{} {} {}\",copy.size(),items[0].size(),n);items[next(&mut n)].clear();println!(\"{} {} {}\",copy.size(),items[0].size(),n);}",
                expect: Expect::Run("3 3 1\n3 0 2\n"),
            },
            Case {
                name: "custom_index_and_index_mut_receivers_evaluate_once",
                source: "use std::ops::{Index,IndexMut};struct Item{v:i32}impl Item{fn get(&self)->i32{self.v}fn bump(&mut self){self.v+=1;}}struct Items{item:Item}impl Index<usize> for Items{type Output=Item;fn index(&self,i:usize)->&Item{assert_eq!(i,0);&self.item}}impl IndexMut<usize> for Items{fn index_mut(&mut self,i:usize)->&mut Item{assert_eq!(i,0);&mut self.item}}fn next(n:&mut usize)->usize{*n+=1;0}fn main(){let mut items=Items{item:Item{v:4}};let mut n=0usize;let a=items[next(&mut n)].get();items[next(&mut n)].bump();println!(\"{} {} {}\",a,items[0].get(),n);}",
                expect: Expect::Run("4 5 2\n"),
            },
            Case {
                name: "builtin_array_and_source_slice_receiver_fallbacks",
                source: "struct Item{v:i32}impl Item{fn get(&self)->i32{self.v}fn bump(&mut self){self.v+=1;}}fn main(){let mut items=[Item{v:4}];items[0].bump();let a=items[0].get();let s:&[Item]=&items;let b=s.len();println!(\"{} {}\",a,b);}",
                expect: Expect::Run("5 1\n"),
            },
            Case {
                name: "noncopy_index_value_still_cannot_move",
                source: "struct Item{v:String}fn main(){let items=vec![Item{v:String::from(\"a\")}];let item=items[0];println!(\"{}\",item.v);}",
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "immutable_owner_cannot_acquire_index_mut_receiver",
                source: "struct Item{v:i32}impl Item{fn bump(&mut self){self.v+=1;}}fn main(){let items=vec![Item{v:1}];items[0].bump();}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "index_mut_carrier_borrow_precedes_method_arguments",
                source: "struct Item{v:usize}impl Item{fn set(&mut self,n:usize){self.v=n;}}fn main(){let mut items=vec![Item{v:0}];items[0].set(items.len());}",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "indexed_receiver_reference_retains_owner_borrow",
                source: "struct Item{v:i32}impl Item{fn value(&self)->&i32{&self.v}}fn main(){let mut items=vec![Item{v:1}];let r=items[0].value();items.push(Item{v:2});println!(\"{}\",r);}",
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}

#[test]
fn vec_range_index_method_receivers_match_stock() {
    use range_receiver_oracle::{Case, Expect};
    range_receiver_oracle::check(
        "vec-range-receiver",
        &[
            Case {
                name: "collect_len_and_runtime_bounds",
                source: "fn main(){let c=vec!['a','b','c','d'];let s:String=c[1..3].iter().collect();let (a,b)=(0usize,2usize);let t:String=c[a..b].iter().collect();let n=c[..3].len();let m=c[2..].iter().count();let u:String=c[..=1].iter().collect();println!(\"{} {} {} {} {}\",s,t,n,m,u);}",
                expect: Expect::Run("bc ab 3 2 ab\n"),
            },
            Case {
                name: "out_of_range_receiver_panics_in_unused_fn",
                source: "fn unused(){let c=vec![1i32];let _=c[0..5].len();}fn main(){println!(\"ok\");}",
                expect: Expect::Run("ok\n"),
            },
            Case {
                name: "owner_mutation_while_subslice_iterator_live",
                source: "fn main(){let mut c=vec![1i32,2,3];let mut it=c[0..2].iter();c.push(4);println!(\"{:?}\",it.next());}",
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}
