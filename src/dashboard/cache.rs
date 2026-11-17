//! Private, bounded, best-effort display snapshots. Never authoritative state.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs::{self, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_AGE: f64 = 24.0 * 60.0 * 60.0;
const VERSION: u32 = 1;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
pub(super) struct Cached<T> {
    pub at: f64,
    pub data: T,
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    version: u32,
    scope: String,
    #[serde(flatten)]
    cached: Cached<T>,
}

pub(super) fn scope() -> Option<String> {
    scope_for(&crate::remote_hosted::creds().ok()?)
}

/// Bind a fetched response to exactly the credentials that fetched it.
pub(super) fn scope_for(credentials: &crate::remote_hosted::Creds) -> Option<String> {
    credentials.dashboard_cache_scope()
}

fn now() -> Option<f64> {
    Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs_f64(),
    )
}

fn valid_time(at: f64) -> bool {
    now().is_some_and(|now| at.is_finite() && at <= now && now - at <= MAX_AGE)
}

fn private(meta: &Metadata, directory: bool) -> bool {
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and does not access Rust memory.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return false;
        }
        if !directory && meta.nlink() != 1 {
            return false;
        }
    }
    true
}

fn directory(home: &Path, create: bool) -> Option<PathBuf> {
    // Do not follow a replaced home or dashboard directory.
    let home_meta = fs::symlink_metadata(home).ok()?;
    if home_meta.file_type().is_symlink() || !home_meta.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if home_meta.uid() != unsafe { libc::geteuid() } || home_meta.mode() & 0o022 != 0 {
            return None;
        }
    }
    let dir = home.join("dashboard");
    if create {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return None,
        }
    }
    private(&fs::symlink_metadata(&dir).ok()?, true).then_some(dir)
}

fn path(home: &Path, scope: &str, name: &str, create: bool) -> Option<PathBuf> {
    if scope.len() != 64
        || !scope.bytes().all(|b| b.is_ascii_hexdigit())
        || name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(directory(home, create)?.join(format!("{scope}-{name}.json")))
}

pub(super) fn load<T: DeserializeOwned>(scope: &str, name: &str) -> Option<Cached<T>> {
    load_at(&crate::paths::home().ok()?, scope, name)
}

fn load_at<T: DeserializeOwned>(home: &Path, scope: &str, name: &str) -> Option<Cached<T>> {
    let path = path(home, scope, name, false)?;
    let meta = fs::symlink_metadata(&path).ok()?;
    if !private(&meta, false) || meta.len() > MAX_BYTES {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !private(&meta, false) || meta.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let value: Envelope<T> = serde_json::from_slice(&bytes).ok()?;
    (value.version == VERSION && value.scope == scope && valid_time(value.cached.at))
        .then_some(value.cached)
}

pub(super) fn save<T: Serialize>(scope: &str, name: &str, at: f64, data: &T) -> Option<()> {
    save_at(&crate::paths::home().ok()?, scope, name, at, data)
}

fn save_at<T: Serialize>(home: &Path, scope: &str, name: &str, at: f64, data: &T) -> Option<()> {
    if !valid_time(at) {
        return None;
    }
    let target = path(home, scope, name, true)?;
    if let Ok(meta) = fs::symlink_metadata(&target)
        && !private(&meta, false)
    {
        return None;
    }
    // Serialize through the same bound as reads, rather than allocating an
    // arbitrarily large response before discovering that it cannot be cached.
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_BYTES as usize - self.0.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "display cache exceeds bound",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(
        &mut bytes,
        &Envelope {
            version: VERSION,
            scope: scope.to_owned(),
            cached: Cached { at, data },
        },
    )
    .ok()?;
    let bytes = bytes.0;
    let temp = target.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&temp).ok()?;
    let result = (|| {
        file.write_all(&bytes).ok()?;
        drop(file);
        fs::rename(&temp, &target).ok()?;
        Some(())
    })();
    if result.is_none() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestHome(PathBuf);
    impl TestHome {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (TestHome, String) {
        let path = std::env::temp_dir().join(format!(
            "justrust-dashboard-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        (TestHome(path), "a".repeat(64))
    }

    #[test]
    fn account_and_endpoint_namespaces() {
        use crate::remote_hosted::Creds;
        let a = Creds::new("https://api.example".into(), "secret-a".into()).unwrap();
        let b = Creds::new("https://api.example".into(), "secret-b".into()).unwrap();
        let mut c = a.clone();
        c.set_build_base("https://build.example".into()).unwrap();
        let d = Creds::new("https://other.example".into(), "secret-a".into()).unwrap();
        assert_ne!(a.dashboard_cache_scope(), b.dashboard_cache_scope());
        assert_ne!(a.dashboard_cache_scope(), c.dashboard_cache_scope());
        assert_ne!(a.dashboard_cache_scope(), d.dashboard_cache_scope());
        assert_eq!(
            Creds::anonymous("https://api.example".into())
                .unwrap()
                .dashboard_cache_scope(),
            None
        );
        let (home, _) = fixture();
        let scope = a.dashboard_cache_scope().unwrap();
        save_at(home.path(), &scope, "personal", now().unwrap(), &vec![1]).unwrap();
        let file = path(home.path(), &scope, "personal", false).unwrap();
        assert!(!fs::read_to_string(file).unwrap().contains("secret-a"));
        assert!(
            load_at::<Vec<i32>>(home.path(), &b.dashboard_cache_scope().unwrap(), "personal")
                .is_none()
        );
    }

    #[test]
    fn bounded_versioned_and_fresh() {
        let (home, scope) = fixture();
        let at = now().unwrap();
        save_at(home.path(), &scope, "personal", at, &vec![1, 2]).unwrap();
        assert_eq!(
            load_at::<Vec<i32>>(home.path(), &scope, "personal")
                .unwrap()
                .data,
            vec![1, 2]
        );
        let file = path(home.path(), &scope, "personal", false).unwrap();
        for bytes in [
            b"bad".to_vec(),
            vec![b' '; MAX_BYTES as usize + 1],
            serde_json::to_vec(&serde_json::json!({"version":2,"scope":scope,"at":at,"data":[]}))
                .unwrap(),
            serde_json::to_vec(
                &serde_json::json!({"version":1,"scope":scope,"at":at+3600.0,"data":[]}),
            )
            .unwrap(),
            serde_json::to_vec(
                &serde_json::json!({"version":1,"scope":scope,"at":at-MAX_AGE-1.0,"data":[]}),
            )
            .unwrap(),
        ] {
            fs::write(&file, bytes).unwrap();
            assert!(load_at::<Vec<i32>>(home.path(), &scope, "personal").is_none());
        }
        assert!(load_at::<i32>(home.path(), "../bad", "personal").is_none());
        assert!(save_at(home.path(), &scope, "../escape", at, &0).is_none());
        assert!(save_at(home.path(), &scope, "personal", f64::NAN, &0).is_none());
    }

    #[test]
    fn replacement_is_complete_and_scope_is_checked() {
        let (home, scope) = fixture();
        let at = now().unwrap();
        save_at(home.path(), &scope, "personal", at, &vec![1, 2]).unwrap();
        save_at(home.path(), &scope, "personal", at, &vec![3]).unwrap();
        assert!(
            save_at(
                home.path(),
                &scope,
                "personal",
                at,
                &"x".repeat(MAX_BYTES as usize)
            )
            .is_none()
        );
        assert_eq!(
            load_at::<Vec<i32>>(home.path(), &scope, "personal")
                .unwrap()
                .data,
            vec![3]
        );
        let file = path(home.path(), &scope, "personal", false).unwrap();
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!({
                "version": VERSION, "scope": "b".repeat(64), "at": at, "data": [9]
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(load_at::<Vec<i32>>(home.path(), &scope, "personal").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn private_files_and_symlink_rejection() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        let (home, scope) = fixture();
        save_at(home.path(), &scope, "personal", now().unwrap(), &1).unwrap();
        let file = path(home.path(), &scope, "personal", false).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(file.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_at::<i32>(home.path(), &scope, "personal").is_none());
        assert!(save_at(home.path(), &scope, "personal", now().unwrap(), &2).is_none());
        fs::remove_file(&file).unwrap();
        symlink(home.path().join("victim"), &file).unwrap();
        assert!(load_at::<i32>(home.path(), &scope, "personal").is_none());
        assert!(save_at(home.path(), &scope, "personal", now().unwrap(), &2).is_none());
        fs::remove_file(&file).unwrap();
        fs::set_permissions(file.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(save_at(home.path(), &scope, "personal", now().unwrap(), &2).is_none());
        assert!(load_at::<i32>(home.path(), &scope, "personal").is_none());
        fs::set_permissions(file.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir(file.parent().unwrap()).unwrap();
        symlink(home.path(), home.path().join("dashboard")).unwrap();
        assert!(save_at(home.path(), &scope, "personal", now().unwrap(), &2).is_none());
    }
}
