//! Per-source-node external observations. These are evidence, not cache permission.
//!
//! No ambient environment is consumed by source preparation: cfg, optimization,
//! link snapshots and filters are supplied explicitly. The project driver may
//! attach frozen discovery evidence. Tool identity, scripts and proc macros
//! still require separate coverage.
use super::{CfgSet, SourceOptions, SourceReader};
use std::io;
use std::path::{Path, PathBuf};

const MAX_OBSERVATIONS: usize = 4096;
const MAX_IDENTITY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Canonical(PathBuf),
    IsFile(bool),
    Exists(bool),
    Bytes {
        limit: usize,
        length: usize,
        digest: [u8; 32],
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Observation {
    path: PathBuf,
    value: Value,
}

/// Immutable evidence retained by the existing per-file graph fact.
/// Private fields prevent callers from asserting observation completeness.
#[derive(Clone, Debug)]
pub struct SourceInputs {
    project: Option<std::sync::Arc<crate::project::inputs::DiscoveryInputs>>,
    observations: Vec<Observation>,
    options: Option<[u8; 32]>,
    complete: bool,
}
impl crate::engine::Fact for SourceInputs {}
impl SourceInputs {
    pub fn project_inputs(&self) -> Option<&crate::project::inputs::DiscoveryInputs> {
        self.project.as_deref()
    }
    pub(super) fn untracked() -> Self {
        Self {
            project: None,
            observations: Vec::new(),
            options: None,
            complete: false,
        }
    }

    /// Number of successful external operations retained, including absent paths.
    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }

    /// Recheck only this source node's observations, never authorize artifact reuse.
    /// Errors, unsupported readers, exhausted recording limits and option changes
    /// all reject. This check is not atomic with later filesystem access and does
    /// not protect against concurrent mutation. A future cache must pin a snapshot.
    pub fn matches_current_source(&self, options: &SourceOptions) -> bool {
        if !self.complete || self.options.is_none() || self.options != option_identity(options) {
            return false;
        }
        if self
            .project
            .as_ref()
            .is_some_and(|project| !project.matches_current())
        {
            return false;
        }
        let mut reader = super::FsReader;
        self.observations.iter().all(|o| match &o.value {
            Value::Canonical(expected) => {
                reader.canonicalize(&o.path).is_ok_and(|v| &v == expected)
            }
            Value::IsFile(expected) => reader.is_file(&o.path).is_ok_and(|v| v == *expected),
            Value::Exists(expected) => reader.exists(&o.path).is_ok_and(|v| v == *expected),
            Value::Bytes {
                limit,
                length,
                digest,
            } => reader
                .read(&o.path, *limit)
                .is_ok_and(|v| v.len() == *length && blake3::hash(&v).as_bytes() == digest),
        })
    }

    /// Source evidence alone never proves complete compilation input identity.
    /// In particular no arbitrary build-script/proc-macro execution is tracked.
    pub fn shareability_blockers(&self) -> &'static [&'static str] {
        &[
            "project discovery evidence, if supplied, excludes registry planning IO and unsupported configuration sources",
            "compiler, embedded libraries and external tool identity are not sealed",
            "attached reviewed source-probe receipts require separate validation",
            "build scripts and procedural macros require controlled input tracking",
            "filesystem observations are not an atomic immutable snapshot",
            "global private artifact sharing is disabled",
        ]
    }
}

pub(super) struct TrackingReader<'a> {
    inner: &'a mut dyn SourceReader,
    inputs: SourceInputs,
    remaining: usize,
    cwd: Option<PathBuf>,
}
impl<'a> TrackingReader<'a> {
    pub(super) fn new(inner: &'a mut dyn SourceReader, options: &SourceOptions) -> Self {
        let cwd = std::env::current_dir().ok();
        let complete = inner.host_filesystem() && cwd.is_some();
        Self {
            inner,
            inputs: SourceInputs {
                project: options.project_inputs.clone(),
                observations: Vec::new(),
                options: option_identity(options),
                complete,
            },
            remaining: MAX_IDENTITY_BYTES,
            cwd,
        }
    }
    pub(super) fn finish(self) -> SourceInputs {
        self.inputs
    }
    fn record<T>(&mut self, path: &Path, result: &io::Result<T>, value: impl FnOnce(&T) -> Value) {
        let Ok(result) = result else {
            self.inputs.complete = false;
            return;
        };
        if !self.inputs.complete {
            return;
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.as_ref().unwrap().join(path)
        };
        let value = value(result);
        let size = path.as_os_str().len().saturating_add(match &value {
            Value::Canonical(p) => p.as_os_str().len(),
            _ => 64,
        });
        if self.inputs.observations.len() >= MAX_OBSERVATIONS || size > self.remaining {
            self.inputs.complete = false;
            return;
        }
        self.remaining -= size;
        self.inputs.observations.push(Observation { path, value });
    }
}
impl SourceReader for TrackingReader<'_> {
    fn canonicalize(&mut self, p: &Path) -> io::Result<PathBuf> {
        let r = self.inner.canonicalize(p);
        self.record(p, &r, |v| Value::Canonical(v.clone()));
        r
    }
    fn is_file(&mut self, p: &Path) -> io::Result<bool> {
        let r = self.inner.is_file(p);
        self.record(p, &r, |v| Value::IsFile(*v));
        r
    }
    fn exists(&mut self, p: &Path) -> io::Result<bool> {
        let r = self.inner.exists(p);
        self.record(p, &r, |v| Value::Exists(*v));
        r
    }
    fn read(&mut self, p: &Path, limit: usize) -> io::Result<Vec<u8>> {
        let r = self.inner.read(p, limit);
        self.record(p, &r, |v| Value::Bytes {
            limit,
            length: v.len(),
            digest: *blake3::hash(v).as_bytes(),
        });
        r
    }
}

/// Length-framed and bounded, unlike formatting Debug output or joining strings.
struct Identity {
    hash: blake3::Hasher,
    remaining: usize,
    valid: bool,
}
impl Identity {
    fn bytes(&mut self, bytes: &[u8]) {
        if !self.valid {
            return;
        }
        let size = bytes.len().saturating_add(8);
        if size > self.remaining {
            self.valid = false;
            return;
        }
        self.remaining -= size;
        self.hash.update(&(bytes.len() as u64).to_le_bytes());
        self.hash.update(bytes);
    }
    fn count(&mut self, n: usize) {
        self.bytes(&(n as u64).to_le_bytes());
    }
    fn flag(&mut self, b: bool) {
        self.bytes(&[u8::from(b)]);
    }
    fn strings(&mut self, v: &[String]) {
        self.count(v.len());
        self.valid &= v.len() <= MAX_OBSERVATIONS;
        for s in v.iter().take(MAX_OBSERVATIONS) {
            if !self.valid {
                break;
            }
            self.bytes(s.as_bytes());
        }
    }
    fn cfg(&mut self, cfg: &CfgSet) {
        self.flag(cfg.debug_assertions());
        self.count(cfg.names.len());
        self.valid &= cfg.names.len() <= MAX_OBSERVATIONS;
        for name in cfg.names.iter().take(MAX_OBSERVATIONS) {
            if !self.valid {
                return;
            }
            self.bytes(name.as_bytes());
        }
        self.count(cfg.values.len());
        self.valid &= cfg.values.len() <= MAX_OBSERVATIONS;
        for (key, values) in cfg.values.iter().take(MAX_OBSERVATIONS) {
            if !self.valid {
                return;
            }
            self.bytes(key.as_bytes());
            self.count(values.len());
            self.valid &= values.len() <= MAX_OBSERVATIONS;
            for value in values.iter().take(MAX_OBSERVATIONS) {
                if !self.valid {
                    return;
                }
                self.bytes(value.as_bytes());
            }
        }
    }
}
fn option_identity(options: &SourceOptions) -> Option<[u8; 32]> {
    // Exhaustive destructuring makes new public configuration require a decision.
    let SourceOptions {
        project_inputs,
        edition,
        native_link,
        cfg,
        prelude,
        opt,
        overflow_checks,
        externs,
        library,
        test_filter,
        cap_lints_allow,
    } = options;
    let mut id = Identity {
        hash: blake3::Hasher::new(),
        remaining: MAX_IDENTITY_BYTES,
        valid: true,
    };
    id.bytes(b"justrust-source-input-v2");
    match project_inputs {
        Some(project) => {
            id.flag(true);
            id.bytes(project.identity()?);
        }
        None => id.flag(false),
    }
    id.bytes(&[match edition {
        super::Edition::E2015 => 0,
        super::Edition::E2018 => 1,
        super::Edition::E2021 => 2,
        super::Edition::E2024 => 3,
    }]);
    let crate::compiler::c_abi::elf::LinkOptions {
        emit_object,
        objects,
        libraries,
    } = native_link;
    id.flag(*emit_object);
    id.count(objects.len());
    id.valid &= objects.len() <= MAX_OBSERVATIONS;
    for object in objects.iter().take(MAX_OBSERVATIONS) {
        if !id.valid {
            break;
        }
        id.bytes(object);
    }
    id.strings(libraries);
    id.cfg(cfg);
    id.flag(*prelude);
    id.bytes(&[match opt {
        crate::compiler::ssa::OptMode::Off => 0,
        crate::compiler::ssa::OptMode::Adaptive => 1,
        crate::compiler::ssa::OptMode::Final => 2,
    }]);
    id.flag(*overflow_checks);
    id.count(externs.len());
    id.valid &= externs.len() <= MAX_OBSERVATIONS;
    for super::ExternCrate {
        name,
        root,
        cfg,
        cap_lints_allow,
    } in externs.iter().take(MAX_OBSERVATIONS)
    {
        if !id.valid {
            break;
        }
        id.bytes(name.as_bytes());
        id.bytes(root.as_os_str().as_encoded_bytes());
        id.cfg(cfg);
        id.flag(*cap_lints_allow);
    }
    id.flag(*library);
    let super::TestFilter {
        filters,
        skip,
        exact,
        ignored,
        include_ignored,
    } = test_filter;
    id.strings(filters);
    id.strings(skip);
    id.flag(*exact);
    id.flag(*ignored);
    id.flag(*include_ignored);
    id.flag(*cap_lints_allow);
    id.valid.then(|| *id.hash.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{self, expansion_budget::ExpansionBudget, source};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "jr-source-inputs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str, text: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, text).unwrap();
            p
        }
        fn prepare(&self, options: &SourceOptions) -> source::FileUnit {
            source::prepare_root_unit(
                &self.0.join("main.rs"),
                options,
                &ExpansionBudget::default(),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn content_changes_same_length_and_missing_files_invalidate() {
        let f = Fixture::new();
        let path = f.file("main.rs", "fn main() { let x = 1i32; }");
        let options = SourceOptions::default();
        let unit = f.prepare(&options);
        assert_eq!(unit.inputs.observation_count(), 3);
        assert!(unit.inputs.matches_current_source(&options));
        f.file("main.rs", "fn main() { let x = 2i32; }");
        assert!(!unit.inputs.matches_current_source(&options));
        f.file("main.rs", "fn main() { let x = 1i32; }");
        assert!(unit.inputs.matches_current_source(&options));
        std::fs::remove_file(&path).unwrap();
        assert!(!unit.inputs.matches_current_source(&options));
        std::fs::create_dir(path).unwrap();
        assert!(!unit.inputs.matches_current_source(&options));
    }

    #[test]
    fn negative_module_lookup_and_child_contents_are_distinct_inputs() {
        let f = Fixture::new();
        f.file("main.rs", "mod child; fn main() { child::value(); }");
        f.file("child.rs", "pub fn value() {} ");
        let options = SourceOptions::default();
        let parent = f.prepare(&options);
        assert!(parent.terminal.is_none());
        assert_eq!(parent.inputs.observation_count(), 5);
        let child =
            source::prepare_child_unit(&parent, 0, &options, &ExpansionBudget::default()).unwrap();
        assert!(parent.inputs.matches_current_source(&options));
        assert!(child.inputs.matches_current_source(&options));
        f.file("child.rs", "pub fn changed() {}");
        // Fine-grained: the parent only depends on discovery, not child bytes.
        assert!(parent.inputs.matches_current_source(&options));
        assert!(!child.inputs.matches_current_source(&options));
        std::fs::create_dir(f.0.join("child")).unwrap();
        f.file("child/mod.rs", "pub fn value() {}");
        assert!(!parent.inputs.matches_current_source(&options));
    }

    #[test]
    fn symlink_retarget_with_identical_bytes_changes_canonical_identity() {
        let f = Fixture::new();
        f.file("a.rs", "fn main() {}");
        f.file("b.rs", "fn main() {}");
        std::os::unix::fs::symlink("a.rs", f.0.join("main.rs")).unwrap();
        let options = SourceOptions::default();
        let unit = f.prepare(&options);
        assert!(unit.inputs.matches_current_source(&options));
        std::fs::remove_file(f.0.join("main.rs")).unwrap();
        std::os::unix::fs::symlink("b.rs", f.0.join("main.rs")).unwrap();
        assert!(!unit.inputs.matches_current_source(&options));
    }

    #[test]
    fn configuration_fields_are_framed_and_all_affect_identity() {
        let original = SourceOptions::default();
        let base = option_identity(&original).unwrap();
        type Change = Box<dyn Fn(&mut SourceOptions)>;
        let changes: Vec<Change> = vec![
            Box::new(|o| o.edition = source::Edition::E2021),
            Box::new(|o| o.native_link.emit_object = true),
            Box::new(|o| o.native_link.objects.push(vec![1, 2])),
            Box::new(|o| o.native_link.libraries.push("native".into())),
            Box::new(|o| {
                o.cfg.names.insert("custom".into());
            }),
            Box::new(|o| o.cfg.set_debug_assertions(false)),
            Box::new(|o| {
                o.cfg.insert_user("mode=\"custom\"").unwrap();
            }),
            Box::new(|o| o.prelude = false),
            Box::new(|o| o.opt = compiler::ssa::OptMode::Final),
            Box::new(|o| o.overflow_checks = false),
            Box::new(|o| {
                o.externs.push(source::ExternCrate {
                    name: "dep".into(),
                    root: "/dep/lib.rs".into(),
                    cfg: o.cfg.clone(),
                    cap_lints_allow: false,
                })
            }),
            Box::new(|o| o.library = true),
            Box::new(|o| o.test_filter.filters.push("test".into())),
            Box::new(|o| o.test_filter.skip.push("skip".into())),
            Box::new(|o| o.test_filter.exact = true),
            Box::new(|o| o.test_filter.ignored = true),
            Box::new(|o| o.test_filter.include_ignored = true),
            Box::new(|o| o.cap_lints_allow = true),
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut next = original.clone();
            change(&mut next);
            assert_ne!(option_identity(&next), Some(base), "field {index}");
        }
        let mut a = original.clone();
        let mut b = original.clone();
        a.test_filter.filters = vec!["ab".into(), "c".into()];
        b.test_filter.filters = vec!["a".into(), "bc".into()];
        assert_ne!(option_identity(&a), option_identity(&b));
        let f = Fixture::new();
        f.file("main.rs", "fn main() {}");
        let unit = f.prepare(&original);
        assert!(!unit.inputs.matches_current_source(&a));
        assert!(!unit.inputs.shareability_blockers().is_empty());
    }

    #[test]
    fn errors_unknown_readers_and_exhausted_recording_fail_closed() {
        let f = Fixture::new();
        let path = f.file("main.rs", "fn main() {}");
        let options = SourceOptions::default();
        let mut fs = source::FsReader;
        let mut reader = TrackingReader::new(&mut fs, &options);
        assert!(!reader.exists(&f.0.join("absent")).unwrap());
        assert!(reader.read(&f.0.join("absent"), 16).is_err());
        assert!(!reader.finish().matches_current_source(&options));
        let mut reader = TrackingReader::new(&mut fs, &options);
        for _ in 0..=MAX_OBSERVATIONS {
            assert!(reader.exists(&path).unwrap());
        }
        let receipt = reader.finish();
        assert_eq!(receipt.observation_count(), MAX_OBSERVATIONS);
        assert!(!receipt.matches_current_source(&options));
        let mut reader = TrackingReader::new(&mut fs, &options);
        reader.remaining = 1;
        assert!(reader.exists(&path).unwrap());
        assert!(!reader.finish().matches_current_source(&options));
        let mut large = options.clone();
        large.native_link.objects.push(vec![0; MAX_IDENTITY_BYTES]);
        assert!(option_identity(&large).is_none());
        large.native_link.objects.clear();
        large.test_filter.filters = vec![String::new(); MAX_OBSERVATIONS + 1];
        assert!(option_identity(&large).is_none());
        let text = source::prepare_text_unit("memory.rs", "fn main() {}", &options).unwrap();
        assert!(!text.inputs.matches_current_source(&options));
        let embedded = source::prepare_prelude_unit(&options, &ExpansionBudget::default()).unwrap();
        assert!(!embedded.inputs.matches_current_source(&options));
        assert!(
            embedded
                .inputs
                .shareability_blockers()
                .iter()
                .any(|b| b.contains("procedural macros"))
        );
    }

    #[test]
    fn graph_check_and_build_publish_per_file_receipts_at_one_and_eight_workers() {
        use crate::engine::Engine;
        let f = Fixture::new();
        f.file("main.rs", "mod child; fn main() { child::value(); }");
        f.file("child.rs", "pub fn value() {}");
        for workers in [1, 8] {
            for check_only in [true, false] {
                let options = SourceOptions {
                    prelude: false,
                    ..Default::default()
                };
                let snapshot = Arc::new(compiler::Snapshot {
                    input: compiler::Input::File(f.0.join("main.rs")),
                    source_options: options.clone(),
                    budget: compiler::ir::PlanBudget::default(),
                });
                let engine = Engine::new(compiler::CompilerProvider(snapshot));
                let check = engine.request(compiler::keys::CheckRoot.key());
                let build = (!check_only).then(|| engine.request(compiler::keys::BuildRoot.key()));
                engine.run(workers).unwrap();
                check.completion().unwrap();
                for key in [
                    compiler::keys::SourceFile::input(),
                    compiler::keys::SourceFile::input().child(0),
                ] {
                    let fact = engine.fact_of::<source::UnitResult>(&key.key(), 0).unwrap();
                    let unit = fact.as_ref().as_ref().unwrap();
                    assert!(unit.inputs.matches_current_source(&options));
                    assert!(!unit.inputs.shareability_blockers().is_empty());
                }
                if let Some(build) = build {
                    build.completion().unwrap();
                    let bytes = build.fact::<Vec<u8>>(0).unwrap();
                    let executable = f.0.join(format!("program-{workers}"));
                    std::fs::write(&executable, bytes.as_slice()).unwrap();
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    assert!(
                        std::process::Command::new(executable)
                            .status()
                            .unwrap()
                            .success()
                    );
                }
            }
        }
    }

    #[test]
    fn discovery_evidence_reaches_source_facts_and_manifest_changes_invalidate() {
        use crate::engine::Engine;
        let f = Fixture::new();
        f.file("main.rs", "fn main() {}");
        let manifest = "[package]\nname='tracked'\nversion='0.1.0'\nedition='2024'\n[[bin]]\nname='tracked'\npath='main.rs'\n[dependencies]\ndep={path='selected'}\n";
        f.file("Cargo.toml", manifest);
        for directory in ["one", "two"] {
            std::fs::create_dir_all(f.0.join(directory).join("src")).unwrap();
            f.file(
                &format!("{directory}/Cargo.toml"),
                "[package]\nname='dep'\nversion='0.1.0'\nedition='2024'\n",
            );
            f.file(&format!("{directory}/src/lib.rs"), "pub fn value() {}\n");
        }
        let selected = f.0.join("selected");
        std::os::unix::fs::symlink("one", &selected).unwrap();
        for workers in [1, 8] {
            for check_only in [true, false] {
                let workspace = crate::project::manifest::load(&f.0.join("Cargo.toml")).unwrap();
                assert!(workspace.inputs.matches_current());
                let options = SourceOptions {
                    prelude: false,
                    project_inputs: Some(workspace.inputs.clone()),
                    ..Default::default()
                };
                let snapshot = Arc::new(compiler::Snapshot {
                    input: compiler::Input::File(f.0.join("main.rs")),
                    source_options: options.clone(),
                    budget: compiler::ir::PlanBudget::default(),
                });
                let engine = Engine::new(compiler::CompilerProvider(snapshot));
                let check = engine.request(compiler::keys::CheckRoot.key());
                let build = (!check_only).then(|| engine.request(compiler::keys::BuildRoot.key()));
                engine.run(workers).unwrap();
                check.completion().unwrap();
                let fact = engine
                    .fact_of::<source::UnitResult>(&compiler::keys::SourceFile::input().key(), 0)
                    .unwrap();
                let unit = fact.as_ref().as_ref().unwrap();
                let project = unit
                    .inputs
                    .project_inputs()
                    .expect("missing project receipt in source fact");
                assert_eq!(project.identity(), workspace.inputs.identity());
                assert!(unit.inputs.matches_current_source(&options));
                std::fs::remove_file(&selected).unwrap();
                std::os::unix::fs::symlink("two", &selected).unwrap();
                assert!(!unit.inputs.matches_current_source(&options));
                std::fs::remove_file(&selected).unwrap();
                assert!(!unit.inputs.matches_current_source(&options));
                std::os::unix::fs::symlink("one", &selected).unwrap();
                assert!(unit.inputs.matches_current_source(&options));
                f.file("Cargo.toml", &manifest.replace("0.1.0", "0.1.1"));
                assert!(!unit.inputs.matches_current_source(&options));
                f.file("Cargo.toml", manifest);
                assert!(unit.inputs.matches_current_source(&options));
                if let Some(build) = build {
                    build.completion().unwrap();
                    assert!(!build.fact::<Vec<u8>>(0).unwrap().is_empty());
                }
            }
        }
    }
}
