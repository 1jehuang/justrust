#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_evict::{Mode, Probe, Registry, RootState, collect as pressure_collect};
    use std::time::SystemTime;

    struct P {
        now: u64,
        busy: Vec<PathBuf>,
        free: u64,
    }
    impl Probe for P {
        fn disk(&self, _: &Path) -> Option<(u64, u64)> {
            Some((self.free, 100 << 30))
        }
        fn size(&self, _: &Path) -> u64 {
            10
        }
        fn in_use_by_other(&self, p: &Path) -> bool {
            self.busy.iter().any(|b| b.starts_with(p))
        }
        fn now(&self) -> u64 {
            self.now
        }
    }
    fn fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, Registry, P) {
        // Orphan collection scans the registered root's parent. Keep it private
        // so this test cannot inspect or delete other tests' scratch targets.
        let root = crate::remote_evict::tests::scratch(tag).join("project");
        let build = root.join("target/debug/build");
        let old = build.join("some-package-0000000000000001");
        let newest = build.join("some-package-0000000000000002");
        std::fs::create_dir_all(old.join("out/nested")).unwrap();
        std::fs::write(old.join("out/nested/file"), "regenerable").unwrap();
        std::fs::create_dir_all(&newest).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Deliberately future-date the retained generation. ctime cannot be
        // backdated, so inject observation time rather than weakening checks.
        std::fs::File::open(&newest)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(now + 60))
            .unwrap();
        std::fs::write(root.join("Cargo.toml"), "keep").unwrap();
        let mut registry = Registry::default();
        registry.roots.insert(
            root.clone(),
            RootState {
                leases: 0,
                last_build_end: now + IDLE.as_secs(),
            },
        );
        (
            root,
            old,
            newest,
            registry,
            P {
                now: now + IDLE.as_secs() + 10,
                busy: vec![],
                free: 1 << 30,
            },
        )
    }

    #[test]
    fn pressure_reclaims_recent_root_outputs_but_retains_newest_and_sources() {
        // Parallel tests can fork while a dry-run flock is held and briefly
        // retain it after this test drops it. Keep all assertions, but exercise
        // this lock-lifetime sequence without unrelated inheriting children.
        if crate::remote_evict::lock_tests::isolated_lock_test(
            "remote_evict::outputs::tests::pressure_reclaims_recent_root_outputs_but_retains_newest_and_sources",
        ) {
            return;
        }
        let (root, old, newest, registry, p) = fixture("output-pressure");
        assert_eq!(
            pressure_collect(
                &registry,
                &root,
                Mode::Off,
                super::super::DEFAULT_GRACE,
                &p,
                &mut |_| {}
            ),
            0
        );
        assert!(old.exists());
        let high = P {
            now: p.now,
            busy: vec![],
            free: 80 << 30,
        };
        assert_eq!(
            pressure_collect(
                &registry,
                &root,
                Mode::On,
                super::super::DEFAULT_GRACE,
                &high,
                &mut |_| {}
            ),
            0
        );
        assert!(old.exists());
        let mut logs = Vec::new();
        assert_eq!(
            pressure_collect(
                &registry,
                &root,
                Mode::Dry,
                super::super::DEFAULT_GRACE,
                &p,
                &mut |s| logs.push(s)
            ),
            10
        );
        assert!(old.exists());
        assert!(
            logs.iter()
                .any(|s| s.contains("would delete stale build output"))
        );
        assert_eq!(
            pressure_collect(
                &registry,
                &root,
                Mode::On,
                super::super::DEFAULT_GRACE,
                &p,
                &mut |_| {}
            ),
            10
        );
        assert!(!old.exists());
        assert!(newest.exists() && root.join("Cargo.toml").exists());
        assert_eq!(
            super::super::below_floor(&p, &root),
            Some(1 << 30),
            "insufficient reclaim must still refuse admission"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn leases_consumers_age_and_build_lock_exclude_cleanup() {
        if crate::remote_evict::lock_tests::isolated_lock_test(
            "remote_evict::outputs::tests::leases_consumers_age_and_build_lock_exclude_cleanup",
        ) {
            return;
        }
        let (root, old, _, mut registry, mut p) = fixture("output-lock");
        registry.roots.get_mut(&root).unwrap().leases = 1;
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        registry.roots.get_mut(&root).unwrap().leases = 0;
        p.busy.push(old.join("out/nested/file"));
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        p.busy.clear();
        let later = p.now;
        p.now -= IDLE.as_secs();
        assert!(candidates(&root, &p).unwrap().is_empty());
        p.now = later;
        let lock = super::super::build_lock(&root).unwrap();
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        assert!(old.exists());
        drop(lock);
        assert_eq!(
            collect(&registry, &registry.roots, 0, false, &p, &mut |_| {}),
            0
        );
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            10
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn durable_live_lease_and_unregistered_roots_remain_protected() {
        let (root, old, _, _, p) = fixture("output-durable");
        let store = crate::remote_evict::durable::Store::at(&root.join("receipts"), "").unwrap();
        store.register(&root).unwrap();
        let registry = Registry::from_store(store).unwrap();
        let other = crate::remote_evict::durable::Store::at(&root.join("receipts"), "").unwrap();
        other.begin(&root).unwrap();
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        assert!(old.exists());
        let unregistered = Registry::default();
        assert_eq!(
            collect(&unregistered, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        assert!(old.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_symlink_and_recent_nested_outputs_are_never_candidates() {
        let (root, old, _, registry, p) = fixture("output-authority");
        let build = old.parent().unwrap();
        for name in [
            "package",
            "package-xyz",
            "-0000000000000003",
            "package-0123456789abcdef-extra",
        ] {
            std::fs::create_dir(build.join(name)).unwrap();
            assert!(package(name).is_none());
        }
        let outside = root.join("source");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), "source").unwrap();
        std::os::unix::fs::symlink(&outside, old.join("out/link")).unwrap();
        assert!(candidates(&root, &p).unwrap().is_empty());
        std::fs::remove_file(old.join("out/link")).unwrap();
        std::fs::File::open(old.join("out/nested/file"))
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(p.now))
            .unwrap();
        assert!(candidates(&root, &p).unwrap().is_empty());
        let debug = root.join("target/debug");
        std::fs::rename(&debug, root.join("real-debug")).unwrap();
        std::os::unix::fs::symlink(root.join("real-debug"), &debug).unwrap();
        assert!(build_dir(&root).is_err());
        assert_eq!(
            collect(&registry, &registry.roots, 100, false, &p, &mut |_| {}),
            0
        );
        assert!(outside.join("keep").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
