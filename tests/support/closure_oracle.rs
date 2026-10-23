//! Closure-specific stock differential acceptance with explicit engine workers.
//! Unlike the general oracle, every native case exercises CHECK and BUILD at
//! workers 1 and 8. CHECK must publish no image and demand no code generation.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Individual test targets may exercise only one outcome.
pub enum Expect {
    Run(&'static str),
    Invalid(&'static str),
}
pub struct Case {
    pub name: &'static str,
    pub source: &'static str,
    pub expect: Expect,
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn check(tag: &str, cases: &[Case]) {
    let compiler = std::env::var_os("JUSTRUST_REAL_RUSTC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rustc"));
    check_with_compiler(tag, cases, &compiler);
}

pub fn check_with_compiler(tag: &str, cases: &[Case], compiler: &Path) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "jr-closure-oracle-{tag}-{}-{stamp}",
            std::process::id()
        ));
    fs::create_dir_all(&root).unwrap();
    let scratch = Scratch(root);
    for (index, case) in cases.iter().enumerate() {
        let source = scratch.0.join(format!("case-{index}.rs"));
        let stock_image = scratch.0.join(format!("stock-{index}"));
        fs::write(&source, case.source).unwrap();
        let stock = Command::new(compiler)
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "--color=never",
                "--crate-name=oracle",
                "-Awarnings",
            ])
            .arg(&source)
            .arg("-o")
            .arg(&stock_image)
            .output()
            .expect("closure acceptance requires the stock Rust compiler");
        let stderr = String::from_utf8_lossy(&stock.stderr);
        match case.expect {
            Expect::Run(expected) => {
                assert!(
                    stock.status.success(),
                    "{}: stock rejected: {stderr}",
                    case.name
                );
                let ran = Command::new(&stock_image).output().unwrap();
                assert!(
                    ran.status.success(),
                    "{}: stock runtime: {ran:?}",
                    case.name
                );
                assert_eq!(
                    ran.stdout,
                    expected.as_bytes(),
                    "{}: stock stdout",
                    case.name
                );
            }
            Expect::Invalid(fragment) => {
                assert!(
                    !stock.status.success() && stderr.contains(fragment),
                    "{}: expected stock {fragment}: {stderr}",
                    case.name
                );
            }
        }
        for workers in [1, 8] {
            for check_only in [true, false] {
                let image = scratch
                    .0
                    .join(format!("native-{index}-{workers}-{check_only}"));
                let compiled =
                    justrust_engine::compiler::compile(case.source.into(), workers, check_only);
                match (case.expect, compiled) {
                    (Expect::Run(expected), Ok(compiled)) => {
                        assert_eq!(
                            compiled.executable.is_none(),
                            check_only,
                            "{} workers={workers} CHECK={check_only}",
                            case.name
                        );
                        if check_only {
                            assert!(!image.exists(), "CHECK wrote an image");
                            assert!(
                                compiled
                                    .report
                                    .node_evaluations
                                    .iter()
                                    .all(|(key, _)| !matches!(
                                        key.kind,
                                        103 | 105 | 130 | 131 | 133
                                    )),
                                "{}: CHECK demanded code generation",
                                case.name
                            );
                        } else {
                            fs::write(&image, compiled.executable.unwrap()).unwrap();
                            fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
                            let ran = Command::new(&image).output().unwrap();
                            assert!(
                                ran.status.success(),
                                "{} workers={workers}: {ran:?}",
                                case.name
                            );
                            assert_eq!(
                                ran.stdout,
                                expected.as_bytes(),
                                "{} workers={workers}",
                                case.name
                            );
                        }
                    }
                    (Expect::Run(_), Err(error)) => panic!(
                        "{} workers={workers} CHECK={check_only}: native rejected: {error}",
                        case.name
                    ),
                    (Expect::Invalid(_), Ok(_)) => panic!(
                        "{} workers={workers} CHECK={check_only}: accepted invalid closure",
                        case.name
                    ),
                    (Expect::Invalid(_), Err(error)) => {
                        assert!(
                            !error
                                .to_string()
                                .to_ascii_lowercase()
                                .contains("unsupported"),
                            "{} workers={workers} CHECK={check_only}: invalid closure reached unsupported boundary: {error}",
                            case.name
                        );
                        assert!(!image.exists(), "rejection wrote an image");
                    }
                }
            }
        }
    }
}
