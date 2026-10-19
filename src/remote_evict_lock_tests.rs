use super::*;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn private_scratch(tag: &str) -> PathBuf {
    let dir = super::tests::scratch(tag);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn foreign_principal_inode_is_rejected_before_migration() {
    let dir = private_scratch("foreign-lock-principal");
    let path = dir.join("root.lock");
    std::fs::write(&path, b"unchanged").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(
        validate_lock_inode(&meta, meta.uid().wrapping_add(1))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o664);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_0664_migration_preserves_inode_contents_and_live_shared_holder() {
    let dir = private_scratch("legacy-lock");
    let path = dir.join("root.lock");
    std::fs::write(&path, b"legacy sentinel").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    let legacy = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let before = legacy.metadata().unwrap();
    assert_eq!(unsafe { libc::flock(legacy.as_raw_fd(), libc::LOCK_SH) }, 0);
    let new = flock_at(&path, false, true).unwrap();
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(after.mode() & 0o7777, 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), b"legacy sentinel");
    drop(new);
    let error = flock_at(&path, true, false)
        .err()
        .expect("legacy holder still excludes GC");
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert!(error.to_string().contains(&path.display().to_string()));
    assert!(error.to_string().contains("os error"), "{error}");
    drop(legacy);
    drop(
        flock_at(&path, true, false)
            .expect("same inode becomes exclusively available only after legacy holder exits"),
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn simultaneous_legacy_migration_keeps_one_inode_and_existing_lease() {
    let dir = private_scratch("concurrent-lock-migration");
    let path = dir.join("root.lock");
    std::fs::write(&path, b"unchanged").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    let legacy = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let inode = legacy.metadata().unwrap().ino();
    assert_eq!(unsafe { libc::flock(legacy.as_raw_fd(), libc::LOCK_SH) }, 0);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(9));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let lease = flock_at(&path, false, true).unwrap();
                assert_eq!(lease.0.metadata().unwrap().ino(), inode);
                barrier.wait();
                barrier.wait();
                drop(lease);
            })
        })
        .collect();
    barrier.wait();
    barrier.wait();
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
    assert_eq!(
        flock_at(&path, true, false).err().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    drop(legacy);
    drop(flock_at(&path, true, false).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn migration_rejects_symlink_hardlink_and_unsafe_parent_without_chmod() {
    let dir = private_scratch("unsafe-lock-migration");
    let path = dir.join("original");
    std::fs::write(&path, b"unchanged").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
    let hard = dir.join("hard.lock");
    std::fs::hard_link(&path, &hard).unwrap();
    assert_eq!(
        flock_at(&hard, false, true).err().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o664);
    std::fs::remove_file(hard).unwrap();
    let alias = dir.join("alias.lock");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let error = flock_at(&alias, false, true).err().unwrap();
    assert!(error.to_string().contains(&alias.display().to_string()));
    assert!(
        error
            .to_string()
            .contains(&format!("os error {}", libc::ELOOP)),
        "{error}"
    );
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o664);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        flock_at(&path, false, true).err().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o664);
    std::fs::remove_dir_all(dir).unwrap();
}
