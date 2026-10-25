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
