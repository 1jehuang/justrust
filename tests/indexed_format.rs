//! Indexed formatting borrows the original place and evaluates indices once.
use justrust_engine::compiler;
use std::os::unix::fs::PermissionsExt;

#[path = "support/stock_rustc.rs"]
mod pinned_stock;

#[test]
fn divergent_non_native_formatting_is_fail_closed() {
    let directory = std::env::temp_dir().join(format!(
        "indexed-format-divergent-boundary-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    for (index,(source,valid)) in [
        ("struct Table{values:[i32;3]}fn make()->Table{Table{values:[7,11,24]}}fn main(){println!(\"{:>3} {}\",1i32,make().values[{return;0usize}]);}",true),
        ("struct Table{values:[i32;3]}struct Bad;fn make()->Table{Table{values:[7,11,24]}}fn main(){println!(\"{} {}\",make().values[{return;0usize}],Bad);}",false),
    ].into_iter().enumerate() {
        let input=directory.join(format!("case{index}.rs"));
        std::fs::write(&input,source).unwrap();
        let stock=std::process::Command::new(pinned_stock::stock_rustc()).env_remove("RUSTC_BOOTSTRAP").args(["--edition=2024","--emit=metadata"]).arg(&input).arg("-o").arg(directory.join("stock.rmeta")).output().unwrap();
        assert_eq!(stock.status.success(),valid,"{}",String::from_utf8_lossy(&stock.stderr));
        if !valid {assert!(String::from_utf8_lossy(&stock.stderr).contains("E0277"));}
        // A boundary rejection is not claimed as semantic-invalid evidence.
        // Neither stock-valid formatting nor a missing Display impl may be
        // accepted by skipping library trait obligations after divergence.
        for workers in [1,8] {for check in [true,false] {
            let failure=compiler::compile_observed(source.into(),workers,check).err().expect("divergent library formatting accepted without its obligations");
            assert!(failure.to_string().contains("unsupported: divergent non-native formatting operands"),"{failure}");
            if check {
                assert!(failure.report.as_ref().unwrap().node_evaluations.iter().all(|(key,_)|key.kind!=103), "CHECK demanded codegen: workers={workers}");
            }
        }}
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn indexed_format_matches_stock() {
    let source = include_str!("fixtures/indexed_format/transport.rs");
    let expected = include_bytes!("fixtures/indexed_format/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|error| panic!("workers={workers},check={check}: {error}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                let root = std::env::var_os("JCODE_SCRATCH_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                let path = root.join(format!("indexed-format-{}-{workers}", std::process::id()));
                std::fs::write(&path, bytes).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
                let output = std::process::Command::new(&path).output().unwrap();
                std::fs::remove_file(path).unwrap();
                assert!(output.status.success(), "{output:?}");
                assert_eq!(&output.stdout, expected);
            }
        }
    }
}

#[test]
fn indexed_format_keeps_borrow_live_across_later_arguments() {
    let source = include_str!("fixtures/indexed_format/conflict.rs");
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match compiler::compile(source.into(), workers, check) {
                Ok(_) => panic!("format borrow overlap accepted"),
                Err(error) => error.to_string(),
            };
            assert!(
                !error.to_ascii_lowercase().contains("unsupported"),
                "{error}"
            );
        }
    }
}
