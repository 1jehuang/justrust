use super::*;
use crate::remote_evict::{self as ev, Probe};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "jr-durable-gc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&base).unwrap();
        Self(base.canonicalize().unwrap())
    }
    fn root(&self, name: &str) -> PathBuf {
        let root = self.0.join(name);
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("target/debug/artifact"), [1u8; 4096]).unwrap();
        root
    }
    fn store(&self) -> Store {
        Store::at(&self.0.join("receipts"), "").unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct LowDisk;
impl Probe for LowDisk {
    fn disk(&self, _: &Path) -> Option<(u64, u64)> {
        Some((0, 1 << 30))
    }
    fn size(&self, path: &Path) -> u64 {
        ev::System.size(path)
    }
    fn in_use_by_other(&self, _: &Path) -> bool {
        false
    }
    fn now(&self) -> u64 {
        ev::System.now()
    }
}
fn age(store: &Store, root: &Path) {
    File::open(store.path(root))
        .unwrap()
        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(100))
        .unwrap();
}
fn collect(registry: &ev::Registry, mode: ev::Mode) -> (u64, Vec<String>) {
    let mut lines = Vec::new();
    let bytes = ev::collect(
        registry,
        Path::new("/"),
        mode,
        ev::DEFAULT_GRACE,
        &LowDisk,
        &mut |l| lines.push(l),
    );
    (bytes, lines)
}

#[test]
fn restart_retains_authority_and_deletes_only_registered_target() {
    let f = Fixture::new();
    let root = f.root("known");
    let unknown = f.root("unknown");
    let store = f.store();
    store.register(&root).unwrap();
    age(&store, &root);
    drop(store);
    let registry = ev::Registry::from_store(f.store()).unwrap();
    let (bytes, logs) = collect(&registry, ev::Mode::Dry);
    assert!(bytes >= 4096);
    assert!(root.join("target").exists());
    assert!(logs.iter().any(|l| l.contains("allocated_tree_bytes=")));
    let (bytes, logs) = collect(&registry, ev::Mode::On);
    assert!(bytes >= 4096);
    assert!(!root.join("target").exists());
    assert!(root.join("Cargo.toml").exists());
    assert!(unknown.join("target/debug/artifact").exists());
    assert!(
        logs.iter()
            .any(|l| l.contains("observed_free_delta_bytes="))
    );
}

#[test]
fn restart_retains_unclean_lease_until_successful_idle_completion() {
    let f = Fixture::new();
    let root = f.root("build");
    let mut first = ev::Registry::from_store(f.store()).unwrap();
    first.register(&root).unwrap();
    let lock = ev::build_lock(&root).unwrap();
    first.begin(&root).unwrap();
    age(&f.store(), &root);
    drop(first);
    let mut next = ev::Registry::from_store(f.store()).unwrap();
    assert_eq!(collect(&next, ev::Mode::On).0, 0);
    assert!(root.join("target").exists());
    drop(lock);
    assert_eq!(collect(&next, ev::Mode::On).0, 0); // crash uncertainty stays closed
    let lock = ev::build_lock(&root).unwrap();
    next.begin(&root).unwrap();
    next.end(&root, ev::System.now()).unwrap();
    drop(lock);
    next.finish_idle(&root).unwrap();
    assert_eq!(f.store().state(&root).unwrap().leases, 0);
    assert_eq!(collect(&next, ev::Mode::On).0, 0); // full two-hour grace
}

#[test]
fn concurrent_agents_cannot_clear_each_others_lease() {
    let f = Fixture::new();
    let root = f.root("parallel");
    let mut a = ev::Registry::from_store(f.store()).unwrap();
    a.register(&root).unwrap();
    let mut b = ev::Registry::from_store(f.store()).unwrap();
    let la = ev::build_lock(&root).unwrap();
    a.begin(&root).unwrap();
    let lb = ev::build_lock(&root).unwrap();
    b.begin(&root).unwrap();
    a.register(&root).unwrap();
    assert_eq!(a.roots[&root].leases, 1);
    a.end(&root, ev::System.now()).unwrap();
    drop(la);
    a.finish_idle(&root).unwrap();
    assert_eq!(f.store().state(&root).unwrap().leases, 1);
    b.end(&root, ev::System.now()).unwrap();
    drop(lb);
    b.finish_idle(&root).unwrap();
    assert_eq!(f.store().state(&root).unwrap().leases, 0);
}

#[test]
fn existing_gc_instance_observes_other_agents_recent_activity() {
    let f = Fixture::new();
    let root = f.root("recent");
    let store = f.store();
    store.register(&root).unwrap();
    age(&store, &root);
    let gc = ev::Registry::from_store(f.store()).unwrap();
    store.activity(&root).unwrap();
    let (_, logs) = collect(&gc, ev::Mode::On);
    assert!(root.join("target").exists());
    assert!(logs.iter().any(|l| l.contains("Recent")));
}

#[test]
fn fresh_receipt_recheck_catches_activity_after_planning() {
    let f = Fixture::new();
    let root = f.root("race");
    let store = f.store();
    store.register(&root).unwrap();
    age(&store, &root);
    let gc = ev::Registry::from_store(f.store()).unwrap();
    struct Racing<'a>(&'a Store, &'a Path);
    impl Probe for Racing<'_> {
        fn disk(&self, p: &Path) -> Option<(u64, u64)> {
            LowDisk.disk(p)
        }
        fn size(&self, p: &Path) -> u64 {
            self.0.activity(self.1).unwrap();
            ev::System.size(p)
        }
        fn in_use_by_other(&self, _: &Path) -> bool {
            false
        }
        fn now(&self) -> u64 {
            ev::System.now()
        }
    }
    let mut logs = Vec::new();
    assert_eq!(
        ev::collect(
            &gc,
            &f.0,
            ev::Mode::On,
            ev::DEFAULT_GRACE,
            &Racing(&store, &root),
            &mut |l| logs.push(l)
        ),
        0
    );
    assert!(root.join("target").exists());
    assert!(
        logs.iter()
            .any(|l| l.contains("fresh lease/recent/process"))
    );
}

#[test]
fn shared_flock_survives_registry_restart() {
    let f = Fixture::new();
    let root = f.root("locked");
    let store = f.store();
    store.register(&root).unwrap();
    age(&store, &root);
    let lock = ev::build_lock(&root).unwrap();
    let gc = ev::Registry::from_store(f.store()).unwrap();
    let (_, logs) = collect(&gc, ev::Mode::On);
    assert!(root.join("target").exists());
    assert!(logs.iter().any(|l| l.contains("build holds lock")));
    drop(lock);
    assert!(collect(&gc, ev::Mode::On).0 >= 4096);
}

#[test]
fn foreign_scope_paths_permissions_and_replaced_roots_fail_closed() {
    let f = Fixture::new();
    let root = f.root("foreign");
    let scope = f.0.join("tenant");
    std::fs::create_dir(&scope).unwrap();
    let scoped = Store::at(&f.0.join("scoped"), scope.to_str().unwrap()).unwrap();
    assert!(scoped.register(&root).is_err());
    let store = f.store();
    store.register(&root).unwrap();
    age(&store, &root);
    let path = store.path(&root);
    let mut receipt: Receipt = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    receipt.scope = "another-principal".into();
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(store.state(&root).is_err());
    assert_eq!(
        collect(&ev::Registry::from_store(f.store()).unwrap(), ev::Mode::On).0,
        0
    );
    receipt.scope = store.scope.clone();
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.state(&root).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(&root, f.0.join("old-root")).unwrap();
    f.root("foreign");
    assert!(store.state(&root).is_err());
    assert!(root.join("target").exists());
}

#[test]
fn symlink_receipts_and_private_cache_roots_are_never_authority() {
    let f = Fixture::new();
    let root = f.root("project");
    let store = f.store();
    store.register(&root).unwrap();
    let path = store.path(&root);
    let saved = f.0.join("saved");
    std::fs::rename(&path, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &path).unwrap();
    assert!(store.state(&root).is_err());
    assert!(
        collect(&ev::Registry::from_store(f.store()).unwrap(), ev::Mode::On)
            .1
            .iter()
            .any(|l| l.contains("Unsafe"))
    );
    let cache = f.root(".justrust");
    assert!(store.register(&cache).is_err());
}

#[test]
fn authenticated_monorepo_root_does_not_require_a_root_manifest() {
    let f = Fixture::new();
    let root = f.root("monorepo");
    std::fs::remove_file(root.join("Cargo.toml")).unwrap();
    let mut registry = ev::Registry::from_store(f.store()).unwrap();
    registry.register(&root).unwrap();
    let lock = ev::build_lock(&root).unwrap();
    registry.begin(&root).unwrap();
    registry.end(&root, ev::System.now()).unwrap();
    drop(lock);
    registry.finish_idle(&root).unwrap();
    assert_eq!(f.store().state(&root).unwrap().leases, 0);
}

#[test]
fn enrollment_failure_cannot_bypass_future_build_protection() {
    let f = Fixture::new();
    let root = f.root(".justrust");
    let mut registry = ev::Registry::from_store(f.store()).unwrap();
    assert!(registry.register(&root).is_err());
    assert_eq!(registry.root_of(&root), Some(root.clone()));
    assert!(registry.begin(&root).is_err());
    assert_eq!(collect(&registry, ev::Mode::On).0, 0);
}

#[test]
fn real_live_process_cwd_is_excluded() {
    let f = Fixture::new();
    let root = f.root("live");
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(&root)
        .spawn()
        .unwrap();
    let busy = ev::System.in_use_by_other(&root);
    let _ = child.kill();
    child.wait().unwrap();
    assert!(busy);
}
