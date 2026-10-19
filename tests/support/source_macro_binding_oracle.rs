//! Stock/native harness scoped to the macro binding feature.
#![allow(dead_code)]
use justrust_engine::compiler::{self, source};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
#[path = "stock_rustc.rs"]
mod stock;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    root: PathBuf,
    options: source::SourceOptions,
    extern_args: Vec<String>,
    dependency_failure: Option<Output>,
}
impl Fixture {
    pub fn new(user: &str, modules: &[(&str, &str)], dependency: Option<&str>) -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "macro-bindings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("main.rs"), user).unwrap();
        for (path, contents) in modules {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let mut options = source::SourceOptions::default();
        let mut extern_args = Vec::new();
        let mut dependency_failure = None;
        if let Some(dependency) = dependency {
            let input = root.join("dep.rs");
            let output = root.join("libdep.rlib");
            fs::write(&input, dependency).unwrap();
            let built = Command::new(stock::stock_rustc())
                .args([
                    "--edition=2024",
                    "--crate-type=lib",
                    "--crate-name=dep",
                    "-Cpanic=abort",
                ])
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .output()
                .unwrap();
            if !built.status.success() {
                dependency_failure = Some(built);
            }
            options.externs.push(source::ExternCrate {
                name: "dep".into(),
                root: input,
                cfg: source::CfgSet::native_x86_64_linux(),
            });
            extern_args.extend(["--extern".into(), format!("dep={}", output.display())]);
        }
        Self {
            root,
            options,
            extern_args,
            dependency_failure,
        }
    }
    pub fn stock(&self, metadata: bool) -> Output {
        if let Some(output) = &self.dependency_failure {
            return output.clone();
        }
        let mut command = Command::new(stock::stock_rustc());
        command
            .args(["--edition=2024", "-Cpanic=abort"])
            .args(&self.extern_args);
        if metadata {
            command.arg("--emit=metadata");
        }
        command
            .arg(self.root.join("main.rs"))
            .arg("-o")
            .arg(self.root.join("stock"));
        command.output().unwrap()
    }
    pub fn native(
        &self,
        workers: usize,
        check: bool,
    ) -> Result<compiler::Compilation, compiler::Failure> {
        compiler::compile_file_observed_with_options(
            self.root.join("main.rs"),
            workers,
            check,
            self.options.clone(),
        )
    }
    pub fn assert_positive(&self) {
        let built = self.stock(false);
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let expected = Command::new(self.root.join("stock")).output().unwrap();
        assert!(
            expected.status.success(),
            "{}",
            String::from_utf8_lossy(&expected.stderr)
        );
        for workers in [1, 8] {
            let checked = self
                .native(workers, true)
                .unwrap_or_else(|e| panic!("CHECK workers={workers}: {e}"));
            assert!(checked.executable.is_none());
            assert!(
                checked
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(key, _)| key.kind != 103)
            );
            let built = self
                .native(workers, false)
                .unwrap_or_else(|e| panic!("BUILD workers={workers}: {e}"));
            let image = built.executable.unwrap();
            assert_eq!(&image[..4], b"\x7fELF");
            let path = self.root.join(format!("native-{workers}"));
            fs::write(&path, image).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let actual = Command::new(path).output().unwrap();
            assert_eq!(actual.status.code(), expected.status.code());
            assert_eq!(actual.stdout, expected.stdout);
            assert_eq!(actual.stderr, expected.stderr);
        }
    }
    pub fn assert_negative(&self) {
        let stock = self.stock(true);
        assert!(!stock.status.success(), "invalid control accepted by stock");
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(
                    self.native(workers, check).is_err(),
                    "invalid control accepted workers={workers} check={check}"
                );
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub fn stock_native(source: &str) {
    Fixture::new(source, &[], None).assert_positive();
}
pub fn stock_and_native_reject(source: &str) {
    Fixture::new(source, &[], None).assert_negative();
}
