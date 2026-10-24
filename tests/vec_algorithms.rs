//! Source-library Vec algorithms compared with stock Rust at workers 1 and 8.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

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
