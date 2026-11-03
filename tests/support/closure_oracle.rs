//! Closure-specific stock differential acceptance with explicit engine workers.
//! Unlike the general oracle, every native case exercises CHECK and BUILD at
//! workers 1 and 8. CHECK must publish no image and demand no code generation.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "stock_rustc.rs"]
pub mod pinned_stock;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Individual test targets may exercise only one outcome.
pub enum Expect {
    Run(&'static str),
    Invalid(&'static str),
    /// Requires original input parsing and complete declaration facts.
    /// Header parsing alone does not establish body syntax admission.
    InvalidAfterParse(&'static str),
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

// Recognize compiler-owned diagnostic categories, not words in source names or
// user identifiers. Source diagnostics can prepend path:line:column (or just
// line:column), so strip only an actual numeric location before classification.
fn diagnostic_head(diagnostic: &str) -> &str {
    let head = diagnostic.lines().next().unwrap_or(diagnostic);
    if let Some((location, message)) = head.split_once(": ") {
        let mut parts = location.rsplit(':');
        if parts
            .next()
            .is_some_and(|part| part.parse::<usize>().is_ok())
            && parts
                .next()
                .is_some_and(|part| part.parse::<usize>().is_ok())
        {
            return message;
        }
    }
    head
}

fn nonsemantic_diagnostic(diagnostic: &str) -> bool {
    let head = diagnostic_head(diagnostic);
    [
        "infrastructure:",
        "Infrastructure:",
        "ResourceLimit:",
        "source planning ResourceLimit (",
        "contract:",
        "unsupported:",
        "Unsupported:",
        "UnsupportedBorrowing:",
        "UnsupportedResource:",
        "unsupported ",
        "engine worker panicked:",
        "provider outputs panicked:",
        "provider start panicked:",
        "provider future poll panicked:",
        "provider future drop panicked:",
        "cannot spawn worker:",
    ]
    .iter()
    .any(|prefix| head.starts_with(prefix))
        || matches!(
            head,
            "engine execution aborted" | "task future missing during poll"
        )
}

/// A failed native run is evidence of invalid Rust only with a healthy resource
/// ledger and complete graph capture. Pre-graph syntax failures may lack a
/// report, but a reserved infrastructure/unsupported diagnostic is never proof.
/// Error currently exposes text, not a typed diagnostic category, so only its
/// reserved prefixes are examined. Resource words in user names are irrelevant.
pub fn validate_invalid_failure(
    failure: &justrust_engine::compiler::Failure,
) -> Result<(), &'static str> {
    if failure.resource_usage.first_failure.is_some() {
        return Err("resource exhaustion is not semantic rejection");
    }
    if failure.resource_usage.poisoned {
        return Err("poisoned resource ledger is not semantic rejection");
    }
    if let Some(report) = &failure.report {
        if !report.capture_complete {
            return Err("incomplete graph capture is not semantic rejection");
        }
        if report.events_dropped != 0 {
            return Err("dropped graph events are not semantic rejection");
        }
        if report
            .node_errors
            .iter()
            .any(|(_, error)| nonsemantic_diagnostic(&error.0))
        {
            return Err("nonsemantic graph failure is not invalid Rust evidence");
        }
    }
    if nonsemantic_diagnostic(&failure.error.0) {
        return Err("nonsemantic diagnostic is not invalid Rust evidence");
    }
    Ok(())
}

/// Also inspect typed ownership diagnostics: their rendered headline can look
/// semantic even when the actual analysis is incomplete or unsupported.
pub fn validate_native_rejection(
    failure: &justrust_engine::compiler::Failure,
) -> Result<(), &'static str> {
    use justrust_engine::compiler::ownership::ErrorKind;
    validate_invalid_failure(failure)?;
    for (_, analysis) in &failure.analyses {
        if !analysis.complete || analysis.stats.diagnostics_suppressed != 0 {
            return Err("incomplete ownership analysis is not semantic rejection");
        }
        if analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                ErrorKind::UnsupportedBorrowing(_)
                    | ErrorKind::ResourceLimit
                    | ErrorKind::InvalidCfg(_)
            )
        }) {
            return Err("nonsemantic ownership failure is not invalid Rust evidence");
        }
    }
    Ok(())
}

pub fn validate_semantic_rejection(
    failure: &justrust_engine::compiler::Failure,
) -> Result<(), &'static str> {
    validate_native_rejection(failure)?;
    let report = failure
        .report
        .as_ref()
        .ok_or("semantic rejection needs graph evidence")?;
    let parsed = report
        .node_ids
        .iter()
        .zip(&report.node_evaluations)
        .any(|(node, (key, _))| {
            key.kind == 121
                && key.params == [0]
                && report
                    .events
                    .iter()
                    .any(|event| event.kind == "publish" && event.node == Some(*node))
        });
    let declarations =
        report
            .node_ids
            .iter()
            .zip(&report.node_evaluations)
            .any(|(node, (key, _))| {
                key.kind == 101
                    && key.params.is_empty()
                    && report
                        .events
                        .iter()
                        .any(|event| event.kind == "publish" && event.node == Some(*node))
            });
    if !parsed || !declarations {
        return Err(
            "original source and declaration facts were not both published; parser rejection is not semantic invalid evidence",
        );
    }
    Ok(())
}

pub fn check(tag: &str, cases: &[Case]) {
    // The hosted bootstrap compiler is not the language oracle. Only the
    // independently pinned stock lookup (JR_STOCK_RUSTC) selects this binary.
    let compiler = PathBuf::from(pinned_stock::stock_rustc());
    check_with_compiler(tag, cases, &compiler);
}

pub fn check_with_compiler(tag: &str, cases: &[Case], compiler: &Path) {
    pinned_stock::verify_compiler(compiler)
        .unwrap_or_else(|error| panic!("{tag}: differential oracle identity: {error}"));
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
            Expect::Invalid(fragment) | Expect::InvalidAfterParse(fragment) => {
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
                let compiled = justrust_engine::compiler::compile_observed(
                    case.source.into(),
                    workers,
                    check_only,
                );
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
                    (Expect::Invalid(_) | Expect::InvalidAfterParse(_), Ok(_)) => panic!(
                        "{} workers={workers} CHECK={check_only}: accepted invalid closure",
                        case.name
                    ),
                    (expect @ (Expect::Invalid(_) | Expect::InvalidAfterParse(_)), Err(error)) => {
                        let validation = if matches!(expect, Expect::InvalidAfterParse(_)) {
                            validate_semantic_rejection(&error)
                        } else {
                            validate_native_rejection(&error)
                        };
                        validation.unwrap_or_else(|reason| {
                            panic!(
                                "{} workers={workers} CHECK={check_only}: {reason}: {error}",
                                case.name
                            )
                        });
                        assert!(!image.exists(), "rejection wrote an image");
                    }
                }
            }
        }
    }
}
