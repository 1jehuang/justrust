//! Parent-local callable variables are solved before authoritative closure checking.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

#[test]
fn local_callable_signatures_match_stock() {
    let dir = std::env::temp_dir().join(format!("jr-callable-infer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let programs = [
        (
            "nested_block",
            "fn main(){let f={let g=|x|x;g};println!(\"{}\",f(9u8));}",
        ),
        ("unit", "fn main(){let f=||{};f();println!(\"unit\");}"),
        (
            "written_reborrow",
            "fn main(){let f=|mut n:i32|->i32{let r=&mut n;let q:&mut i32=r;*q+=1;*r};println!(\"{}\",f(1));}",
        ),
        (
            "formal_reborrow",
            "fn bump(r:&mut i32){*r+=1;}fn main(){let f=|mut n:i32|{let r=&mut n;crate::bump(r);*r};println!(\"{}\",f(1));}",
        ),
        (
            "later_call",
            "fn main(){let f=|x|x+1;println!(\"{}\",f(2));}",
        ),
        (
            "result_context",
            "fn main(){let f=|x|x;let value:u8=f(7);println!(\"{}\",value);}",
        ),
        (
            "bound_context",
            "fn apply<F:Fn(i32)->i32>(f:F)->i32{f(3)}fn main(){let f=|x|x+1;println!(\"{}\",apply(f));}",
        ),
        (
            "return_context",
            "fn main(){let f=|x|{return x;};println!(\"{}\",f(5i64));}",
        ),
        (
            "shadowed",
            "fn main(){let x=100;let f=|x|{let x=x+2;x};println!(\"{} {}\",x,f(5));}",
        ),
        // Associated function values whose owner arguments are inferred.
        (
            "owner_inferred_fn_item",
            "use std::collections::BTreeMap;fn main(){let mut m=BTreeMap::new();m.entry(7u8).or_insert_with(Vec::new).push(3usize);m.entry(7u8).or_insert_with(Vec::new).push(4usize);let f=String::new;let s:String=f();let v:Vec<Vec<i32>>=(0..3).map(|_|Vec::new()).collect();println!(\"{} {:?} {:?} {:?}\",m.len(),m.get(&7u8),s,v);}",
        ),
        // Function items as values through generic adapters.
        (
            "fn_item_adapter",
            "fn inc(x:i32)->i32{x+1} fn main(){let v:Vec<i32>=vec![1,2].into_iter().map(inc).collect();let f=inc;println!(\"{:?} {}\",v,f(1));}",
        ),
        // Items whose parameters are references.
        (
            "fn_item_reference_parameter",
            "fn slen(s:&str)->usize{s.len()} fn neg(x:&i32)->bool{*x<0} fn dbl(x:&i32)->i32{*x*2} fn ap<F:Fn(&str)->usize>(f:F)->usize{f(\"abc\")} fn main(){let v=vec![1,-2,3];let d:Vec<i32>=v.iter().map(dbl).collect();let m=v.into_iter().filter(neg).count();let r:Vec<Result<i32,String>>=vec![Ok(1),Err(\"e\".to_string()),Ok(3)];let k=r.into_iter().filter(Result::is_ok).count();println!(\"{} {:?} {} {} {}\",ap(slen),d,m,k,\"ab1\".chars().all(char::is_alphanumeric));}",
        ),
        // Explicit generic arguments on a function value.
        (
            "fn_item_explicit_arguments",
            "fn id<T>(x:T)->T{x} fn main(){let f=id::<u8>;let g=Vec::<i32>::new;let w=g();println!(\"{} {:?}\",f(3),w);}",
        ),
        // Generic items whose arguments are inferred from the callable bound.
        (
            "fn_item_inferred_arguments",
            "fn id<T>(x:T)->T{x} fn main(){let a:Vec<i32>=vec![1,2].into_iter().map(id).collect();let b:Vec<Box<i32>>=vec![1,2].into_iter().map(Box::new).collect();let c:Vec<String>=vec![\"a\",\"b\"].into_iter().map(String::from).collect();vec![String::new()].into_iter().for_each(drop);println!(\"{:?} {:?} {:?}\",a,b,c);}",
        ),
    ];
    for (name, text) in programs {
        // The native compiler captures its source directory; keep each
        // program's directory free of earlier stock/native executables.
        let out = dir.join(format!("{name}-out"));
        let dir = dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let source = dir.join(format!("{name}.rs"));
        std::fs::write(&source, text).unwrap();
        let stock = out.join(format!("{name}-stock"));
        let output = Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(&source)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(output.status.success(), "stock {name}: {output:?}");
        let expected = Command::new(stock).output().unwrap();
        for workers in [1, 8] {
            let checked = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .args(["--check", "--workers"])
                .arg(workers.to_string())
                .output()
                .unwrap();
            assert!(
                checked.status.success(),
                "CHECK {name} workers{workers}: {checked:?}"
            );
            let executable = out.join(format!("{name}-{workers}"));
            let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .arg("--workers")
                .arg(workers.to_string())
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "BUILD {name} workers{workers}: {built:?}"
            );
            let actual = Command::new(executable).output().unwrap();
            assert_eq!(actual.status.code(), expected.status.code(), "{name}");
            assert_eq!(actual.stdout, expected.stdout, "{name}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unresolved_or_conflicting_callable_signatures_are_rejected() {
    let dir = std::env::temp_dir().join(format!("jr-callable-infer-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text, diagnostic) in [
        ("unresolved", "fn main(){let f=|x|x;}", "E0282"),
        ("unresolved_owner", "fn main(){let f=Vec::new;}", "E0282"),
        ("arity", "fn main(){let f=|x|x+1;f();}", "E0057"),
        (
            "fn_item_unresolved_arguments",
            "fn id<T>(x:T)->T{x} fn main(){let f=id;}",
            "E0282",
        ),
        (
            "fn_item_argument_count",
            "fn id<T>(x:T)->T{x} fn main(){let f=id::<u8,u8>;}",
            "E0107",
        ),
        (
            "conflict",
            "fn main(){let f=|x|x;f(1i32);f(true);}",
            "E0308",
        ),
        (
            "move",
            "fn main(){let f=|r:&mut i32|{let q=r;*r};}",
            "E0382",
        ),
    ] {
        let source = dir.join(format!("{name}.rs"));
        std::fs::write(&source, text).unwrap();
        let stock = Command::new("rustc")
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&source)
            .arg("-o")
            .arg(dir.join(format!("{name}.rmeta")))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "stock accepted {name}");
        assert!(
            String::from_utf8_lossy(&stock.stderr).contains(diagnostic),
            "stock {name}: {stock:?}"
        );
        for workers in [1, 8] {
            let native = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .args(["--check", "--workers"])
                .arg(workers.to_string())
                .output()
                .unwrap();
            assert!(!native.status.success(), "native accepted {name}");
            assert!(
                {
                    let message = String::from_utf8_lossy(&native.stderr);
                    message.contains(diagnostic)
                        || (diagnostic == "E0382"
                            && message.contains("initialization/move analysis")
                            && message.contains("ReadNotDefinitelyInitialized"))
                },
                "native {name}: {native:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
