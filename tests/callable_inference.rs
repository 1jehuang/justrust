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
            "let_bound_closure_annotated_reference",
            "fn thing(x:impl FnOnce(&u32)){x(&4)} fn t<'a,F:Fn(&'a u32)>(f:F,x:&'a u32){f(x)} fn main(){let f=|y:&u32|println!(\"{}\",y);thing(f);let g=|y|println!(\"{}\",y);t(g,&5);let h=|_|println!(\"v\");thing_v(h);} fn thing_v(x:impl FnOnce(u32)){x(1)}",
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
        (
            "str_length_fn_item",
            "fn main(){let n:Vec<usize>=vec![\"ab\",\"é\",\"\"].into_iter().map(str::len).collect();let f=str::len;println!(\"{:?} {}\",n,f(\"éé\"));}",
        ),
        (
            "fn_item_generic_parse_adapter",
            "fn main(){let a:Vec<Result<i32,std::num::ParseIntError>>=vec![\"1\",\"2\"].into_iter().map(str::parse::<i32>).collect();let b:Result<Vec<i32>,std::num::ParseIntError>=\"3 4\".split_whitespace().map(str::parse::<i32>).collect();println!(\"{:?} {:?}\",a,b);}",
        ),
        (
            "fn_item_result_error_inference",
            "fn main(){let a:Result<Vec<i32>,_>=\"1 2\".split_whitespace().map(str::parse::<i32>).collect();let b:Result<Vec<i32>,_>=\"1 x\".split_whitespace().map(str::parse::<i32>).collect();println!(\"{:?} {}\",a,b.is_err());}",
        ),
        (
            "partial_nested_nominal_return",
            "fn value()->Result<Vec<i32>,String>{Ok(vec![2,3])}fn main(){let r:Result<Vec<i32>,_>=value();println!(\"{:?}\",r);}",
        ),
        (
            "nested_nominal_impl_inference",
            "struct Wrap<T>(T);trait Take<U>{fn take(self)->U;}impl<T> Take<T> for Wrap<(T,)>{fn take(self)->T{self.0.0}}fn main(){let n:u8=Wrap((7,)).take();println!(\"{}\",n);}",
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
            .args(["--edition=2024", "-Awarnings", "-C", "panic=abort"])
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
            "let_bound_closure_not_general",
            "fn thing(x:impl FnOnce(&u32)){} fn unused(){let f=|_|();thing(f);} fn main(){}",
            "E0277",
        ),
        (
            "let_bound_closure_not_general_mixed",
            "fn thing(x:impl FnOnce(&u32,&u32,u32)){} fn unused(){let f=|_,y:&u32,z|();thing(f);} fn main(){}",
            "E0277",
        ),
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
            "fn_item_wrong_arity",
            "fn inc(x:i32)->i32{x+1} fn ap<F:Fn(i32,i32)->i32>(f:F)->i32{f(1,2)} fn unused(){ap(inc);} fn main(){}",
            "E0277",
        ),
        (
            "fn_item_wrong_argument",
            "fn inc(x:i32)->i32{x+1} fn ap<F:Fn(bool)->i32>(f:F)->i32{f(true)} fn unused(){ap(inc);} fn main(){}",
            "E0631",
        ),
        (
            "non_function_generic_arguments",
            "const K:i32=1; fn unused(){let f=K::<u8>;} fn main(){}",
            "E0107",
        ),
        (
            "non_generic_function_arguments",
            "fn inc(x:i32)->i32{x+1} fn unused(){let f=inc::<u8>;} fn main(){}",
            "E0107",
        ),
        (
            "fn_item_wrong_result_error",
            "fn main(){let r:Result<Vec<i32>,String>=vec![\"1\"].into_iter().map(str::parse::<i32>).collect();println!(\"{:?}\",r);}",
            "E0277",
        ),
        (
            "partial_wrong_concrete_child",
            "fn value()->Result<Vec<i32>,String>{Ok(vec![2,3])}fn unused(){let _:Result<Vec<u8>,_>=value();}fn main(){}",
            "E0308",
        ),
        (
            "nested_nominal_missing_impl",
            "struct Sink<T>(T);fn unused(){let _:Sink<i32>=vec![1,2].into_iter().collect();}fn main(){}",
            "E0277",
        ),
        (
            "nested_nominal_impl_ambiguity",
            "struct Wrap<T>(T);trait Take<U>{fn take(self)->U;}impl Take<u8> for Wrap<Vec<u8>>{fn take(mut self)->u8{self.0.pop().unwrap()}}impl Take<u16> for Wrap<Vec<u16>>{fn take(mut self)->u16{self.0.pop().unwrap()}}fn main(){let n=Wrap(Vec::new()).take();println!(\"{}\",n);}",
            "E0282",
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
            .args(["--edition=2024", "--emit=metadata", "-C", "panic=abort"])
            .arg(&source)
            .arg("-o")
            .arg(dir.join(format!("{name}.rmeta")))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "stock accepted {name}");
        assert!(
            String::from_utf8_lossy(&stock.stderr).contains(diagnostic)
                || (name == "fn_item_wrong_arity"
                    && String::from_utf8_lossy(&stock.stderr).contains("E0593"))
                || (name == "fn_item_wrong_argument"
                    && String::from_utf8_lossy(&stock.stderr).contains("E0277")),
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
                        || (matches!(name, "fn_item_wrong_arity" | "fn_item_wrong_argument")
                            && (message.contains(
                                "associated projection has no applicable implementation",
                            ) || message.contains("E0308: mismatched types")))
                        || (name == "nested_nominal_impl_ambiguity"
                            && message.contains(
                                "unsupported: nominal source constraints cannot progress",
                            ))
                        || (name == "non_function_generic_arguments"
                            && message.contains("unsupported generic application expression"))
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
