//! Private, SSH-principal scoped receipts. Only authenticated agent Synced
//! messages enroll roots. The collector enumerates receipts, never source trees.
#[cfg(test)]
#[path = "remote_evict_registry_tests.rs"]
mod tests;

use super::{RootState, Skip};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct Receipt {
    version: u32,
    scope: String,
    uid: u32,
    root: PathBuf,
    device: u64,
    inode: u64,
}

pub(super) struct Store {
    dir: PathBuf,
    scope: String,
    prefix: Option<PathBuf>,
    uid: u32,
}

impl Store {
    pub fn open(prefix: &str) -> io::Result<Self> {
        Self::at(
            &crate::remote::dir()
                .map_err(io::Error::other)?
                .join("evict-registry"),
            prefix,
        )
    }

    pub(super) fn at(base: &Path, prefix: &str) -> io::Result<Self> {
        let uid = unsafe { libc::geteuid() };
        let prefix = if prefix.is_empty() {
            None
        } else {
            let p = PathBuf::from(prefix);
            if !p.is_absolute() || p.canonicalize()? != p {
                return Err(io::Error::other(
                    "agent prefix must be canonical and absolute",
                ));
            }
            Some(p)
        };
        let scope = format!(
            "v1-{uid}-{:016x}",
            super::fxhash(
                prefix
                    .as_deref()
                    .unwrap_or(Path::new(""))
                    .as_os_str()
                    .as_encoded_bytes()
            )
        );
        let dir = base.join(&scope);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        let meta = std::fs::symlink_metadata(&dir)?;
        if !meta.is_dir()
            || meta.uid() != uid
            || meta.mode() & 0o077 != 0
            || dir.canonicalize()? != dir
        {
            return Err(io::Error::other(
                "registry must be a private, owned, canonical directory",
            ));
        }
        Ok(Self {
            dir,
            scope,
            prefix,
            uid,
        })
    }

    fn path(&self, root: &Path) -> PathBuf {
        self.dir.join(format!(
            "{:016x}.json",
            super::fxhash(root.as_os_str().as_encoded_bytes())
        ))
    }

    fn identity(&self, root: &Path) -> io::Result<std::fs::Metadata> {
        // This authorizes activity recording, not deletion. Sources may live
        // under .justrust/bench (or other GC-excluded trees); target_of checks
        // deletion eligibility separately, including immediately before GC.
        if !root.is_absolute()
            || root.canonicalize()? != root
            || self.prefix.as_ref().is_some_and(|p| !root.starts_with(p))
        {
            return Err(io::Error::other("root outside authorized source scope"));
        }
        let meta = std::fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.uid() != self.uid {
            return Err(io::Error::other("root ownership mismatch"));
        }
        Ok(meta)
    }

    fn read(&self, path: &Path) -> io::Result<(Receipt, File, u64)> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != self.uid
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
            || meta.len() > 16384
        {
            return Err(io::Error::other("untrusted activity receipt"));
        }
        let receipt: Receipt = serde_json::from_reader((&file).take(16385))?;
        if receipt.version != 1
            || receipt.scope != self.scope
            || receipt.uid != self.uid
            || self.path(&receipt.root) != path
        {
            return Err(io::Error::other("receipt principal/scope mismatch"));
        }
        let root = self.identity(&receipt.root)?;
        if root.dev() != receipt.device || root.ino() != receipt.inode {
            return Err(io::Error::other("registered root was replaced"));
        }
        let stamp = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        Ok((receipt, file, stamp))
    }

    fn read_root(&self, root: &Path) -> io::Result<(Receipt, File, u64)> {
        let result = self.read(&self.path(root))?;
        if result.0.root != root {
            return Err(io::Error::other("receipt root mismatch"));
        }
        Ok(result)
    }

    pub fn register(&self, root: &Path) -> io::Result<()> {
        let meta = self.identity(root)?;
        let path = self.path(root);
        if path.try_exists()? {
            // Do not overwrite an invalid receipt, even through a new Synced.
            return self.activity(root);
        }
        let receipt = Receipt {
            version: 1,
            scope: self.scope.clone(),
            uid: self.uid,
            root: root.to_path_buf(),
            device: meta.dev(),
            inode: meta.ino(),
        };
        let temp = self.dir.join(format!(
            ".{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos()
        ));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temp)?;
            f.write_all(&serde_json::to_vec(&receipt)?)?;
            f.sync_all()?;
            // Hard link publishes without replacing a concurrent registration.
            match std::fs::hard_link(&temp, &path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e),
            }
            std::fs::remove_file(&temp)?;
            File::open(&self.dir)?.sync_all()?;
            self.activity(root)
        })();
        let _ = std::fs::remove_file(temp);
        result
    }

    pub fn activity(&self, root: &Path) -> io::Result<()> {
        let (_, file, _) = self.read_root(root)?;
        // Metadata-only updates continue to work when the disk has no data blocks.
        file.set_modified(SystemTime::now())?;
        file.sync_all()
    }

    pub fn begin(&self, root: &Path) -> io::Result<()> {
        let (_, file, _) = self.read_root(root)?;
        // Persistent uncertainty marker. A crash or failed end update cannot
        // turn an old start timestamp into permission to delete a recent build.
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        file.sync_all()?;
        self.activity(root)
    }

    pub fn finish(&self, root: &Path) -> io::Result<()> {
        self.activity(root)?;
        let (_, file, _) = self.read_root(root)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.sync_all()
    }

    pub fn state(&self, root: &Path) -> io::Result<RootState> {
        let (_, file, last_build_end) = self.read_root(root)?;
        Ok(RootState {
            leases: u32::from(file.metadata()?.mode() & 0o100 != 0),
            last_build_end,
        })
    }

    pub fn snapshot(&self) -> io::Result<super::Snapshot> {
        let mut roots = BTreeMap::new();
        let mut skipped = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match self.read(&path) {
                Ok((r, _, _)) => {
                    roots.insert(r.root.clone(), self.state(&r.root)?);
                }
                Err(e) => skipped.push((path, Skip::Unsafe(e.to_string()))),
            }
        }
        Ok((roots, skipped))
    }
}
