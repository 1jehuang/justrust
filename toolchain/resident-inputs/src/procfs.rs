//! Linux runtime identities from the libraries actually mapped into this process.
//! Paths alone are not identity. Callers hash the returned regular files and
//! recapture this selection before committing and after compiler execution.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) fn runtime_paths() -> Result<BTreeMap<String, PathBuf>, String> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use std::os::unix::fs::MetadataExt;
        let mut text = String::new();
        File::open("/proc/self/maps")
            .map_err(|e| e.to_string())?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 1024 * 1024 {
            return Err("runtime map budget exceeded".into());
        }
        let mut paths = BTreeMap::new();
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 6 {
                continue;
            }
            let path = Path::new(fields[5]);
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !matches!(
                name,
                "libgcc_s.so.1"
                    | "libc.so.6"
                    | "libm.so.6"
                    | "libpthread.so.0"
                    | "libdl.so.2"
                    | "librt.so.1"
                    | "ld-linux-x86-64.so.2"
            ) {
                continue;
            }
            if fields.len() != 6 || !path.is_absolute() {
                return Err("ambiguous or deleted mapped runtime library".into());
            }
            let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
            let meta = fs::metadata(&canonical).map_err(|e| e.to_string())?;
            let inode = fields[4].parse::<u64>().map_err(|e| e.to_string())?;
            let (major, minor) = fields[3].split_once(':').ok_or("invalid mapped device")?;
            let major = u64::from_str_radix(major, 16).map_err(|e| e.to_string())?;
            let minor = u64::from_str_radix(minor, 16).map_err(|e| e.to_string())?;
            let dev = meta.dev();
            let actual_major = ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xfffff000);
            let actual_minor = (dev & 0xff) | ((dev >> 12) & 0xffffff00);
            if !meta.is_file()
                || meta.ino() != inode
                || actual_major != major
                || actual_minor != minor
            {
                return Err("mapped runtime library was replaced".into());
            }
            if let Some(previous) = paths.insert(name.to_owned(), canonical.clone())
                && previous != canonical
            {
                return Err("ambiguous mapped runtime library".into());
            }
        }
        Ok(paths)
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        Err("proc-macro runtime identity requires Linux x86_64".into())
    }
}

pub(crate) fn loader_configuration() -> Result<PathBuf, String> {
    // A fresh helper consults this cache at exec/dlopen time. It is an input,
    // not an authority to load arbitrary additional runtime dependencies.
    match fs::symlink_metadata("/etc/ld.so.preload") {
        Ok(meta) if !meta.is_file() || meta.len() != 0 => {
            return Err("global dynamic loader preloads unsupported".into());
        }
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.to_string()),
    }
    fs::canonicalize("/etc/ld.so.cache").map_err(|error| error.to_string())
}

pub(crate) fn validate_loader_paths(
    paths: &[PathBuf],
    needed: &str,
    selected: &Path,
) -> Result<(), String> {
    for dir in paths {
        let candidate = dir.join(needed);
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                if fs::canonicalize(candidate).map_err(|e| e.to_string())? != selected {
                    return Err("loader path shadows mapped runtime library".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
