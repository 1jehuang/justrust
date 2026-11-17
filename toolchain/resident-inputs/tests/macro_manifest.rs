use jr_resident_inputs::{capture, capture_with_macros};
use std::collections::BTreeMap;
use std::path::Path;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "jr-macro-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn args(&self) -> Vec<String> {
        vec![format!(
            "--extern=fixture={}",
            self.0.join("libfixture.so").display()
        )]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn malformed_macro_objects_and_loader_overrides_are_rejected() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("libfixture.so"), b"not an ELF object").unwrap();
    assert!(capture(&fixture.args(), &fixture.0).is_err());
    assert!(capture_with_macros(&fixture.args(), &fixture.0, &BTreeMap::new()).is_err());
    for key in ["LD_PRELOAD", "LD_AUDIT", "LD_DEBUG", "LD_ORIGIN_PATH"] {
        let env = BTreeMap::from([(key.into(), "untrusted".into())]);
        assert!(
            capture_with_macros(&fixture.args(), &fixture.0, &env)
                .unwrap_err()
                .contains("loader environment")
        );
    }
}

#[test]
#[ignore = "requires an explicitly supplied actual native macro shared object"]
fn actual_macro_and_mapped_runtime_content_identity() {
    let fixture = Fixture::new();
    let source = std::env::var_os("JUSTRUST_TEST_RESIDENT_MANIFEST_MACRO").unwrap();
    let library = fixture.0.join("libfixture.so");
    std::fs::copy(Path::new(&source), &library).unwrap();
    assert!(capture(&fixture.args(), &fixture.0).is_err());
    let env = BTreeMap::new();
    let manifest = capture_with_macros(&fixture.args(), &fixture.0, &env)
        .unwrap()
        .unwrap();
    manifest.verify().unwrap();
    assert!(manifest.files().any(|path| path == library));
    assert!(
        manifest
            .files()
            .any(|path| path.file_name().is_some_and(|name| name == "libc.so.6"))
    );
    assert!(manifest.files().any(|path| {
        path.file_name()
            .is_some_and(|name| name == "ld-linux-x86-64.so.2")
    }));
    let shadow = fixture.0.join("libc.so.6");
    std::fs::write(&shadow, b"not the mapped libc").unwrap();
    let env = BTreeMap::from([("LD_LIBRARY_PATH".into(), fixture.0.as_os_str().to_owned())]);
    assert!(
        capture_with_macros(&fixture.args(), &fixture.0, &env)
            .unwrap_err()
            .contains("shadows")
    );
    std::fs::remove_file(shadow).unwrap();
    let mut bytes = std::fs::read(&library).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&library, bytes).unwrap();
    assert!(manifest.verify().is_err());
}
