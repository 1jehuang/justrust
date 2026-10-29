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
        ("arity", "fn main(){let f=|x|x+1;f();}", "E0057"),
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
