//! Private trusted-local machine-output namespace, never graph authority.
//!
//! The caller validates the current object and its graph prerequisites before
//! reuse, and frames its resolved instance symbol into a separate object key.
//! This helper only admits closed snapshots and native-only configurations.
use super::{ir, source, ssa};
use crate::persistent_cache::{KeyBuilder, MAX_ENTRY_BYTES};
use std::path::Path;

const DOMAIN: &[u8] = b"justrust/checked-native-build/x86_64-linux/v1";

enum EncodingError {
    Ineligible,
    Budget(String),
}
struct Encoding<'a> {
    key: KeyBuilder,
    work: usize,
    budget: &'a ir::PlanBudget,
}
impl Encoding<'_> {
    fn field(&mut self, bytes: &[u8]) -> Result<(), EncodingError> {
        self.work = self
            .work
            .checked_add(8)
            .and_then(|n| n.checked_add(bytes.len()))
            .filter(|n| *n <= MAX_ENTRY_BYTES)
            .ok_or(EncodingError::Ineligible)?;
        self.budget
            .charge_work(bytes.len().saturating_add(8))
            .map_err(EncodingError::Budget)?;
        self.key.field(bytes);
        Ok(())
    }
    fn count(&mut self, count: usize) -> Result<(), EncodingError> {
        // Every element has at least one framed field. Reject enormous vectors
        // before traversing even empty strings or cfg value sets.
        if count > MAX_ENTRY_BYTES / 8 {
            return Err(EncodingError::Ineligible);
        }
        self.field(&(count as u64).to_le_bytes())
    }
    fn cfg(&mut self, cfg: &source::CfgSet) -> Result<(), EncodingError> {
        self.field(&[u8::from(cfg.debug_assertions())])?;
        self.count(cfg.names.len())?;
        for name in &cfg.names {
            self.field(name.as_bytes())?;
        }
        self.count(cfg.values.len())?;
        for (name, values) in &cfg.values {
            self.field(name.as_bytes())?;
            self.count(values.len())?;
            for value in values {
                self.field(value.as_bytes())?;
            }
        }
        Ok(())
    }
}

pub(super) fn identity(
    path: &Path,
    options: &source::SourceOptions,
    budget: &ir::PlanBudget,
) -> Result<Option<[u8; 32]>, String> {
    // Exhaustive by design: every new option requires an explicit cache audit.
    let source::SourceOptions {
        local_cache,
        compiler_image,
        project_inputs,
        source_snapshot,
        edition,
        native_link,
        cfg,
        prelude,
        opt,
        overflow_checks,
        panic_unwind,
        externs,
        library,
        test_filter,
        cap_lints_allow,
    } = options;
    let super::c_abi::elf::LinkOptions {
        emit_object,
        objects,
        libraries,
        // The capsule is consumed by external linking, not machine-object
        // generation. It is eagerly captured even for native-only builds.
        // No linker output or capsule authority is cached at this boundary.
        toolchain: _,
    } = native_link;
    let (Some(cache), Some(compiler), Some(snapshot)) =
        (local_cache, compiler_image, source_snapshot)
    else {
        return Ok(None);
    };
    if cache.computed_export_enabled()
        || project_inputs.is_some()
        || *edition != source::Edition::E2024
        || *emit_object
        || !objects.is_empty()
        || !libraries.is_empty()
    {
        return Ok(None);
    }
    let encode = || -> Result<[u8; 32], EncodingError> {
        budget
            .charge_work(DOMAIN.len() + 32)
            .map_err(EncodingError::Budget)?;
        let mut key = Encoding {
            key: KeyBuilder::new(DOMAIN, compiler.implementation_digest()),
            work: DOMAIN.len() + 32,
            budget,
        };
        key.field(snapshot.digest())?;
        key.field(path.as_os_str().as_encoded_bytes())?;
        key.field(b"2024")?;
        key.cfg(cfg)?;
        key.field(&[u8::from(*prelude)])?;
        key.field(&[match opt {
            ssa::OptMode::Off => 0,
            ssa::OptMode::Adaptive => 1,
            ssa::OptMode::Final => 2,
        }])?;
        key.field(&[u8::from(*overflow_checks), u8::from(*panic_unwind)])?;
        key.count(externs.len())?;
        for external in externs {
            let source::ExternCrate {
                name,
                root,
                cfg,
                cap_lints_allow,
            } = external;
            key.field(name.as_bytes())?;
            key.field(root.as_os_str().as_encoded_bytes())?;
            key.cfg(cfg)?;
            key.field(&[u8::from(*cap_lints_allow)])?;
        }
        key.field(&[u8::from(*library)])?;
        let source::TestFilter {
            filters,
            skip,
            exact,
            ignored,
            include_ignored,
        } = test_filter;
        for strings in [filters, skip] {
            key.count(strings.len())?;
            for string in strings {
                key.field(string.as_bytes())?;
            }
        }
        key.field(&[
            u8::from(*exact),
            u8::from(*ignored),
            u8::from(*include_ignored),
        ])?;
        key.field(&[u8::from(*cap_lints_allow)])?;
        Ok(key.key.finish())
    };
    match encode() {
        Ok(key) => Ok(Some(key)),
        Err(EncodingError::Ineligible) => Ok(None),
        Err(EncodingError::Budget(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistent_cache::LocalCache;
    use std::path::PathBuf;

    fn snapshot(extra: Option<&[u8]>) -> std::sync::Arc<source::SourceSnapshot> {
        let mut files = vec![(PathBuf::from("main.rs"), b"fn main() {}".to_vec())];
        if let Some(bytes) = extra {
            files.push(("optional.rs".into(), bytes.to_vec()));
        }
        source::SourceSnapshot::new("/private/build-cache-test".into(), files).unwrap()
    }
    fn options() -> source::SourceOptions {
        source::SourceOptions {
            local_cache: Some(LocalCache::new(
                std::env::temp_dir().join("jr-build-cache-unit-unused"),
            )),
            compiler_image: Some(super::super::procfs::CompilerImage::current().unwrap()),
            source_snapshot: Some(snapshot(None)),
            ..Default::default()
        }
    }
    fn key(options: &source::SourceOptions) -> Option<[u8; 32]> {
        identity(
            Path::new("/private/build-cache-test/main.rs"),
            options,
            &ir::PlanBudget::default(),
        )
        .unwrap()
    }
    #[test]
    fn identity_covers_semantic_options_and_order() {
        let mut base = options();
        base.externs = vec![source::ExternCrate {
            name: "dep".into(),
            root: "/private/build-cache-test/dep.rs".into(),
            cfg: source::CfgSet::default(),
            cap_lints_allow: false,
        }];
        let original = key(&base).unwrap();
        for mutation in 0..21 {
            let mut changed = base.clone();
            match mutation {
                0 => changed.cfg.set_debug_assertions(false),
                1 => {
                    changed.cfg.names.insert("custom".into());
                }
                2 => {
                    changed
                        .cfg
                        .values
                        .insert("custom".into(), ["value".into()].into());
                }
                3 => changed.prelude = !changed.prelude,
                4 => changed.opt = ssa::OptMode::Adaptive,
                5 => changed.opt = ssa::OptMode::Final,
                6 => changed.overflow_checks = !changed.overflow_checks,
                7 => changed.panic_unwind = !changed.panic_unwind,
                8 => changed.externs[0].name.push('x'),
                9 => changed.externs[0].root.push("x"),
                10 => {
                    changed.externs[0].cfg.names.insert("custom".into());
                }
                11 => changed.externs[0].cap_lints_allow = true,
                12 => changed.library = !changed.library,
                13 => changed.test_filter.filters.push("abc".into()),
                14 => changed.test_filter.skip.push("abc".into()),
                15 => changed.test_filter.exact = true,
                16 => changed.test_filter.ignored = true,
                17 => changed.test_filter.include_ignored = true,
                18 => changed.cap_lints_allow = true,
                19 => changed.externs.clear(),
                20 => changed.source_snapshot = Some(snapshot(Some(b"added"))),
                _ => unreachable!(),
            }
            assert_ne!(Some(original), key(&changed), "mutation {mutation}");
        }
        assert_ne!(
            Some(original),
            identity(Path::new("other.rs"), &base, &ir::PlanBudget::default()).unwrap()
        );
        let mut changed = base.clone();
        changed.test_filter.filters = vec!["a".into(), "b".into()];
        let ordered = key(&changed);
        changed.test_filter.filters.reverse();
        assert_ne!(ordered, key(&changed));
        changed.externs.push(source::ExternCrate {
            name: "other".into(),
            ..changed.externs[0].clone()
        });
        let ordered = key(&changed);
        changed.externs.reverse();
        assert_ne!(ordered, key(&changed));
    }
    #[test]
    fn snapshot_absence_content_and_missing_files_are_distinct() {
        let mut options = options();
        let absent = key(&options).unwrap();
        options.source_snapshot = Some(snapshot(Some(b"one")));
        let present = key(&options).unwrap();
        options.source_snapshot = Some(snapshot(Some(b"two")));
        assert_ne!(present, key(&options).unwrap());
        assert_ne!(absent, present);
        options.source_snapshot = None;
        assert_eq!(key(&options), None);
    }
    #[test]
    fn computed_export_is_ineligible_and_key_work_is_metered() {
        let mut options = options();
        let directory = std::env::temp_dir().join(format!(
            "jr-build-cache-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        options.local_cache = Some(
            options
                .local_cache
                .take()
                .unwrap()
                .with_computed_export(directory.clone())
                .unwrap(),
        );
        assert_eq!(key(&options), None);
        std::fs::remove_dir(directory).unwrap();
        options.local_cache = Some(LocalCache::new("/unused".into()));
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            work: 1,
            ..Default::default()
        });
        assert!(identity(Path::new("main.rs"), &options, &budget).is_err());
        let budget = ir::PlanBudget::default();
        let mut encoding = Encoding {
            key: KeyBuilder::new(DOMAIN, &[0; 32]),
            work: MAX_ENTRY_BYTES,
            budget: &budget,
        };
        assert!(matches!(
            encoding.field(b""),
            Err(EncodingError::Ineligible)
        ));
        assert!(matches!(
            encoding.count(MAX_ENTRY_BYTES),
            Err(EncodingError::Ineligible)
        ));
    }

    #[test]
    fn framed_lists_and_cfg_fields_cannot_alias() {
        let base = options();
        let mut left = base.clone();
        let mut right = base.clone();
        left.test_filter.filters = vec!["ab".into(), "c".into()];
        right.test_filter.filters = vec!["a".into(), "bc".into()];
        assert_ne!(key(&left), key(&right));
        right = base.clone();
        right.test_filter.skip = left.test_filter.filters.clone();
        assert_ne!(key(&left), key(&right));
        left = base.clone();
        right = base;
        left.cfg.values.insert("ab".into(), ["c".into()].into());
        right.cfg.values.insert("a".into(), ["bc".into()].into());
        assert_ne!(key(&left), key(&right));
    }

    #[test]
    fn set_insertion_order_and_cache_location_do_not_change_identity() {
        let mut left = options();
        let mut right = left.clone();
        for name in ["alpha", "beta"] {
            left.cfg.names.insert(name.into());
        }
        for name in ["beta", "alpha"] {
            right.cfg.names.insert(name.into());
        }
        left.cfg
            .values
            .insert("feature".into(), ["first".into(), "second".into()].into());
        right
            .cfg
            .values
            .insert("feature".into(), ["second".into(), "first".into()].into());
        right.local_cache = Some(LocalCache::new("/different-unused-cache".into()));
        assert_eq!(key(&left), key(&right));
        assert!(key(&left).is_some());
    }

    #[test]
    fn encoding_counter_overflow_and_huge_counts_fail_closed() {
        let budget = ir::PlanBudget::default();
        let mut encoding = Encoding {
            key: KeyBuilder::new(DOMAIN, &[0; 32]),
            work: usize::MAX,
            budget: &budget,
        };
        assert!(matches!(
            encoding.field(b"x"),
            Err(EncodingError::Ineligible)
        ));
        assert!(matches!(
            encoding.count(usize::MAX),
            Err(EncodingError::Ineligible)
        ));
    }

    #[test]
    fn uncontrolled_inputs_are_ineligible() {
        let base = options();
        for mutation in 0..9 {
            let mut changed = base.clone();
            match mutation {
                0 => {
                    changed.project_inputs =
                        Some(crate::project::inputs::DiscoveryInputs::untracked())
                }
                1 => changed.native_link.emit_object = true,
                2 => changed.native_link.objects.push(vec![1]),
                3 => changed.native_link.libraries.push("m".into()),
                4 => changed.compiler_image = None,
                5 => changed.local_cache = None,
                6 => changed.edition = source::Edition::E2015,
                7 => changed.edition = source::Edition::E2018,
                8 => changed.edition = source::Edition::E2021,
                _ => unreachable!(),
            }
            assert_eq!(key(&changed), None, "mutation {mutation}");
        }
    }
}
