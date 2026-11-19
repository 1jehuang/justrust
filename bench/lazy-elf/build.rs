// Build an instrumented copy only inside this benchmark's OUT_DIR. Fail loudly
// if production moves a hook so measurements never silently compare two controls.
use std::{env, fs, path::PathBuf};
fn hook(s: &mut String, from: &str, to: &str) {
    assert_eq!(s.matches(from).count(), 1, "stale benchmark hook: {from}");
    *s = s.replacen(from, to, 1);
}
fn main() {
    let dst = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let path = "../../src/remote_artifact_prediction.rs";
    let unwind = "../../src/remote_artifact_unwind.rs";
    for p in [path, unwind] {
        println!("cargo:rerun-if-changed={p}");
    }
    let mut s = fs::read_to_string(path).unwrap();
    fs::copy(unwind, dst.join("remote_artifact_unwind.rs")).unwrap();
    hook(&mut s, "//! Negotiated", "// Negotiated");
    hook(&mut s, "//! local", "// local");
    hook(
        &mut s,
        "hints: Option<&unwind::Unwind>,",
        "hints: Option<&unwind::Unwind>, mut rows: Option<crate::rows::Rows>,",
    );
    hook(
        &mut s,
        "if source != LITERAL {\n            referenced",
        "let source = rows.as_mut().map_or(source, |r| r.choose(basis, pos, block, source));\n        if source != LITERAL {\n            referenced",
    );
    hook(
        &mut s,
        "compressed: &Path) -> Result<()> {",
        "compressed: &Path, lazy: bool) -> Result<()> {",
    );
    hook(
        &mut s,
        "    input.rewind()?;",
        "    let rows = lazy.then(|| crate::rows::Rows::detect(&basis, &mut input, size));\n    input.rewind()?;",
    );
    hook(
        &mut s,
        "        hints.as_ref(),",
        "        hints.as_ref(), rows,",
    );
    // Existing encode unit tests are also built against the instrumented copy.
    for variable in ["input", "recipe"] {
        let call = format!("&mut {variable}, None).unwrap()");
        hook(
            &mut s,
            &call,
            &format!("&mut {variable}, None, None).unwrap()"),
        );
    }
    fs::write(dst.join("prediction.rs"), s).unwrap();
}
