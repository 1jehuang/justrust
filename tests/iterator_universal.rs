//! Universal associated projections remain exact until closed instantiation.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn unchanged_iterator_adapters_program() {
    oracle::stock_native(include_str!("fixtures/full_programs/36_iter_adapters.rs"));
}

#[test]
fn overlapping_windows_and_projected_indexing() {
    oracle::stock_native(
        "fn main(){let v=vec![2i32,5,9,14];let differences:Vec<i32>=v.windows(2).map(|w|w[1]-w[0]).collect();println!(\"{:?}\",differences);println!(\"{} {} {}\",v.windows(1).count(),v.windows(4).count(),v.windows(5).count());let empty:Vec<i32>=Vec::new();println!(\"{}\",empty.windows(1).count());let s:&[i32]=&v;let mut w=s.windows(2);let first=w.next().unwrap();let last=w.next_back().unwrap();let middle=w.next().unwrap();println!(\"{} {} {} {} {} {} {}\",first[0],first[1],last[0],last[1],middle[0],middle[1],w.next_back().is_none());}",
    );
}

#[test]
fn zero_sized_windows_panic_at_construction() {
    use std::os::unix::fs::PermissionsExt;
    let source = "fn main(){let v=vec![1i32,2];println!(\"before\");let _=v.windows(0);println!(\"after\");}";
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-zero-windows-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    std::fs::write(&input, source).unwrap();
    let stock_image = root.join("stock");
    let stock = std::process::Command::new(oracle::closure_oracle::pinned_stock::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(input)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let stock = std::process::Command::new(stock_image).output().unwrap();
    assert!(!stock.status.success());
    assert_eq!(stock.stdout, b"before\n");
    for workers in [1, 8] {
        let checked = justrust_engine::compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(!checked.report.published_kind(103));
        let built = justrust_engine::compiler::compile(source.into(), workers, false).unwrap();
        let image = root.join(format!("native-{workers}"));
        std::fs::write(&image, built.executable.unwrap()).unwrap();
        std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
        let native = std::process::Command::new(image).output().unwrap();
        assert!(!native.status.success());
        assert_eq!(native.stdout, stock.stdout);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inferred_borrowed_filter_operands() {
    oracle::stock_native(
        "fn main(){let v:Vec<i32>=(1..=6).collect();let evens:Vec<i32>=v.iter().copied().filter(|x|x%2==0).collect();println!(\"{:?}\",evens);}",
    );
}

#[test]
fn unique_operator_rhs_accepts_deref_and_unsizing_coercions() {
    oracle::stock_native(
        "fn main(){let t=String::from(\"tail\");let s=String::from(\"head\")+&t;println!(\"{} {}\",s,t);let v=vec![1i32,2,3];println!(\"{}\",v.iter().fold(String::new(),|acc,x|acc+&x.to_string()));}",
    );
    oracle::stock_native(
        "struct Count;impl std::ops::Add<&[i32]> for Count{type Output=usize;fn add(self,rhs:&[i32])->usize{rhs.len()}}fn main(){let a=[3i32,4,5];println!(\"{}\",Count+&a);let v=vec![1i32,2];println!(\"{}\",Count+&v);}",
    );
}

#[test]
fn provided_projected_ordering_bound_is_proved() {
    oracle::stock_native(
        "fn main(){let v=vec![2i32,9,4];println!(\"{:?}\",v.iter().copied().map(|x|x+1).max());}",
    );
    for source in [
        "struct NoOrd;fn unused(){let v=vec![NoOrd];let _=v.into_iter().max();}fn main(){}",
        "trait Pick{type Item;fn pick(self,x:Self::Item)->Self::Item where Self:Sized,Self::Item:Ord{x}}struct N;struct P;impl Pick for P{type Item=N;}fn unused(){let _=P.pick(N);}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let _:bool=v.windows(2).map(|w|w[0]).next().unwrap();}fn main(){}",
        "fn unused(){let s=String::new()+&true;}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let w=v.windows(2);drop(v);let _=w.count();}fn main(){}",
        "fn unused(){let v=vec![1i32,2];let _=v.iter().copied().filter(|x|x%true==0).count();}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn provided_iterator_default_checks_universally_and_runs_closed() {
    oracle::stock_native(
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn last(mut self)->Option<Self::Item> where Self:Sized{let mut out=None;while let Some(x)=self.next(){out=Some(x);}out}}struct Counter{n:i32}impl It for Counter{type Item=i32;fn next(&mut self)->Option<i32>{if self.n<3{self.n+=1;Some(self.n)}else{None}}}fn main(){println!(\"{}\",Counter{n:0}.last().unwrap());}",
    );
}

#[test]
fn rigid_generic_projection_identity_is_not_an_eager_concrete_guess() {
    oracle::stock_native(
        "trait Has{type Out;}struct S;impl Has for S{type Out=i32;}fn identity<I:Has>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){println!(\"{}\",identity::<S>(42));}",
    );
}

#[test]
fn unused_universal_body_and_unproved_projection_reject() {
    for source in [
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out)->bool{x}fn main(){}",
        "trait Has{type Out;}fn bad<I>(x:<I as Has>::Out)-><I as Has>::Out{x}fn main(){}",
        "trait Has{type Out;}fn bad<I:Has>(x:<I as Has>::Out){let first=x;let second=x;}fn main(){}",
        "trait It{type Item;fn next(&mut self)->Option<Self::Item>;fn bad(self)->Self::Item where Self:Sized{true}}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}
