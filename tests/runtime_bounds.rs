//! Original indexing locations and actual unsigned operands reach the native
//! abort diagnostic. The oracle and native executable use identical abort policy.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

fn output(executable: &std::path::Path) -> (Vec<u8>, String) {
    let child = Command::new("sh")
        .args(["-c", "ulimit -c 0; exec \"$1\"", "bounds-oracle"])
        .arg(executable)
        .env("RUST_BACKTRACE", "0")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.signal(), Some(6), "{result:?}");
    let stderr = String::from_utf8(result.stderr).unwrap();
    let identity = format!("thread 'main' ({pid})");
    assert!(stderr.contains(&identity), "{stderr}");
    (
        result.stdout,
        stderr.replace(&identity, "thread 'main' (<PID>)"),
    )
}

fn source_bounds_match_stock(selected: &str) {
    let dir = std::env::temp_dir().join(format!(
        "jr-source-bounds-{}-{selected}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut matched = false;
    for (name, source) in [
        (
            "associated_const_slice_index",
            "struct Limits{x:i32}impl Limits{const INDEX:usize=2;}fn pick(a:&[i32]){\n    let _r=&a[Limits::INDEX];\n}fn main(){let a=[7,8];pick(&a);}",
        ),
        (
            "receiver",
            "fn main(){let first=[7,8];let second=[9];let mut s:&[i32]=&first;\n    let _r=s[{s=&second;1}];\n}",
        ),
        (
            "borrow",
            "struct S{x:i32} fn pick(a:&[S;2],i:usize){\n    let _r=&a[i];\n}\nfn main(){let a=[S{x:7},S{x:8}];pick(&a,2);}",
        ),
        (
            "mutborrow",
            "fn pick(a:&mut [i32],i:usize){\n    let _r=&mut a[i];\n}\nfn main(){let mut a=[7,8];pick(&mut a,2);}",
        ),
        (
            "write",
            "fn idx()->usize{println!(\"index\");2} fn rhs()->i32{println!(\"rhs\");9} fn main(){let mut a=[7,8];\n    a[idx()]=rhs();\n}",
        ),
        (
            "slicewrite",
            "fn idx()->usize{println!(\"index\");2} fn rhs()->i32{println!(\"rhs\");9} fn put(a:&mut [i32]){\n    a[idx()]=rhs();\n}fn main(){let mut a=[7,8];put(&mut a);}",
        ),
        (
            "nestedborrow",
            "fn idx(tag:i32,i:usize)->usize{println!(\"{}\",tag);i} fn main(){let a=[[1,2],[3,4]];\n    let _r=&a[idx(1,2)][idx(2,0)];\n}",
        ),
        (
            "nestedwrite",
            "fn idx(tag:i32,i:usize)->usize{println!(\"{}\",tag);i} fn rhs()->i32{println!(\"rhs\");9}fn main(){let mut a=[[1,2],[3,4]];\n    a[idx(1,0)][idx(2,2)]=rhs();\n}",
        ),
        (
            "constant_slicewrite",
            "fn rhs()->i32{println!(\"rhs\");9} fn put(a:&mut [i32]){\n    a[2]=rhs();\n}fn main(){let mut a=[7,8];put(&mut a);}",
        ),
        (
            "constant_slicecompound",
            "fn rhs()->i32{println!(\"rhs\");9} fn put(a:&mut [i32]){\n    a[2]+=rhs();\n}fn main(){let mut a=[7,8];put(&mut a);}",
        ),
        (
            "nestedwrite_outer",
            "fn idx(tag:i32,i:usize)->usize{println!(\"{}\",tag);i} fn rhs()->i32{println!(\"rhs\");9}fn main(){let mut a=[[1,2],[3,4]];\n    a[idx(1,2)][idx(2,0)]=rhs();\n}",
        ),
        (
            "constant_sliceborrow",
            "struct S{x:i32}fn pick(a:&[S]){\n    let _r=&a[2];\n}fn main(){let a=[S{x:7},S{x:8}];pick(&a);}",
        ),
        (
            "noncopy_mutborrow",
            "struct S{x:i32}fn pick(a:&mut [S],i:usize){\n    let _r=&mut a[i];\n}fn main(){let mut a=[S{x:7},S{x:8}];pick(&mut a,2);}",
        ),
        (
            "format_slice",
            "fn pick(a:&[i32],i:usize){\n    println!(\"{}\",a[i]);\n}fn main(){let a=[7,8];pick(&a,2);}",
        ),
        (
            "format_constant_slice",
            "fn pick(a:&[i32]){\n    println!(\"{}\",a[2]);\n}fn main(){let a=[7,8];pick(&a);}",
        ),
        (
            "ignored",
            "struct S{x:i32}fn pick(a:[S;2],i:usize){\n    let _=a[i];\n}fn main(){pick([S{x:1},S{x:2}],2);}",
        ),
        (
            "array",
            "fn pick(a:[i32;2],i:usize)->i32{\n    a[i]\n}\nfn main(){let _x=pick([7,8],2);}",
        ),
        (
            "slice",
            "fn pick(a:&[i32],i:usize)->i32{\n    a[i]\n}\nfn main(){let a=[7,8];let _x=pick(&a,18446744073709551615usize);}",
        ),
    ] {
        if name != selected {
            continue;
        }
        matched = true;
        eprintln!(
            "bounds case {name}: pinned stock CHECK/BUILD + 3 native opt modes x CHECK/BUILD/run x workers 1/8"
        );
        let path = dir.join(format!("{name}.rs"));
        std::fs::write(&path, source).unwrap();
        let metadata = dir.join(format!("stock-check-{name}.rmeta"));
        let check = Command::new(stock_rustc::stock_rustc())
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "-Coverflow-checks=yes",
                "--emit=metadata",
            ])
            .arg(&path)
            .arg("-o")
            .arg(&metadata)
            .output()
            .unwrap();
        assert!(check.status.success(), "stock CHECK {name}: {check:?}");
        assert!(
            metadata.is_file(),
            "stock CHECK produced no metadata: {name}"
        );
        let stock = dir.join(format!("stock-{name}"));
        let build = Command::new(stock_rustc::stock_rustc())
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "-Cpanic=abort", "-Coverflow-checks=yes"])
            .arg(&path)
            .arg("-o")
            .arg(&stock)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let expected = output(&stock);
        for mode in ["off", "adaptive", "final"] {
            for workers in [1, 8] {
                let native = dir.join(format!("native-{name}-{mode}-{workers}"));
                let check_directory = dir.join(format!("check-{name}-{mode}-{workers}"));
                std::fs::create_dir(&check_directory).unwrap();
                let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                    .current_dir(&check_directory)
                    .arg(&path)
                    .args(["--edition", "2024", "--check", "--opt", mode, "--workers"])
                    .arg(workers.to_string())
                    .output()
                    .unwrap();
                assert!(
                    check.status.success(),
                    "native CHECK {name}/{mode}/{workers}: {check:?}"
                );
                assert!(
                    std::fs::read_dir(&check_directory)
                        .unwrap()
                        .next()
                        .is_none(),
                    "native CHECK published output: {name}/{mode}/{workers}"
                );
                let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                    .arg(&path)
                    .args(["--edition", "2024", "--opt", mode, "--workers"])
                    .arg(workers.to_string())
                    .arg("-o")
                    .arg(&native)
                    .output()
                    .unwrap();
                assert!(build.status.success(), "{name}/{mode}/{workers}: {build:?}");
                assert_eq!(output(&native), expected, "{name}/{mode}/{workers}");
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert!(matched, "unknown bounds source case: {selected}");
}

#[test]
fn source_bounds_associated_const_slice_index() {
    source_bounds_match_stock("associated_const_slice_index");
}
#[test]
fn source_bounds_receiver() {
    source_bounds_match_stock("receiver");
}
#[test]
fn source_bounds_borrow() {
    source_bounds_match_stock("borrow");
}
#[test]
fn source_bounds_mutborrow() {
    source_bounds_match_stock("mutborrow");
}
#[test]
fn source_bounds_write() {
    source_bounds_match_stock("write");
}
#[test]
fn source_bounds_slicewrite() {
    source_bounds_match_stock("slicewrite");
}
#[test]
fn source_bounds_nestedborrow() {
    source_bounds_match_stock("nestedborrow");
}
#[test]
fn source_bounds_nestedwrite() {
    source_bounds_match_stock("nestedwrite");
}
#[test]
fn source_bounds_constant_slicewrite() {
    source_bounds_match_stock("constant_slicewrite");
}
#[test]
fn source_bounds_constant_slicecompound() {
    source_bounds_match_stock("constant_slicecompound");
}
#[test]
fn source_bounds_nestedwrite_outer() {
    source_bounds_match_stock("nestedwrite_outer");
}
#[test]
fn source_bounds_constant_sliceborrow() {
    source_bounds_match_stock("constant_sliceborrow");
}
#[test]
fn source_bounds_noncopy_mutborrow() {
    source_bounds_match_stock("noncopy_mutborrow");
}
#[test]
fn source_bounds_format_slice() {
    source_bounds_match_stock("format_slice");
}
#[test]
fn source_bounds_format_constant_slice() {
    source_bounds_match_stock("format_constant_slice");
}
#[test]
fn source_bounds_remaining_ignored() {
    source_bounds_match_stock("ignored");
}
#[test]
fn source_bounds_remaining_array() {
    source_bounds_match_stock("array");
}
#[test]
fn source_bounds_remaining_slice() {
    source_bounds_match_stock("slice");
}
