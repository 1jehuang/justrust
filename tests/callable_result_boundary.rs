//! Reference-bearing closure results are an explicit admission boundary, not a move proof.
use std::process::Command;

#[test]
fn reference_result_carriers_fail_before_adapter_publication() {
    let dir = std::env::temp_dir().join(format!(
        "jr-callable-result-boundary-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in [
        (
            "root",
            "fn main(){let mut n=1i32;let r=&mut n;let f=move||r;}",
        ),
        (
            "tuple",
            "fn main(){let mut n=1i32;let r=&mut n;let f=move||(r,);}",
        ),
        (
            "array",
            "fn main(){let mut n=1i32;let r=&mut n;let f=move||[r];}",
        ),
    ] {
        let path = dir.join(format!("{name}.rs"));
        std::fs::write(&path, source).unwrap();
        let stock = Command::new("rustc")
            .args(["--edition=2024", "-Awarnings", "--emit=metadata"])
            .arg(&path)
            .arg("-o")
            .arg(dir.join(format!("{name}.rmeta")))
            .output()
            .unwrap();
        assert!(stock.status.success(), "stock {name}: {stock:?}");
        for workers in [1, 8] {
            for check in [true, false] {
                let executable = dir.join(format!("{name}-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&path).arg("--workers").arg(workers.to_string());
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&executable);
                }
                let native = command.output().unwrap();
                assert!(
                    !native.status.success(),
                    "unsupported result admitted: {native:?}"
                );
                let message = String::from_utf8_lossy(&native.stderr);
                assert!(message.contains("unsupported: closure reference results require checked capture carrier contracts"), "{name}: {native:?}");
                assert!(
                    !message.contains("reference contract has missing or extra positions"),
                    "malformed adapter was reached: {native:?}"
                );
                assert!(!executable.exists(), "unsupported result emitted an image");
            }
        }
    }
}
