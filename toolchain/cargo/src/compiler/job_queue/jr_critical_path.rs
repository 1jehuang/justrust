use crate::util::GlobalContext;
use crate::util::data_structures::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;

const MAX_BYTES: usize = 64 * 1024;
const MAX_PACKAGES: usize = 1024;
const DEFAULT_COST: usize = 100;

pub(super) struct Costs {
    packages: HashMap<String, usize>,
}

impl Costs {
    pub(super) fn from_env(gctx: &GlobalContext) -> Option<Self> {
        if gctx.get_env("JUSTRUST_CARGO_CRITICAL_PATH").ok() != Some("1") {
            return None;
        }
        let packages = gctx
            .get_env_os("JUSTRUST_CARGO_CRITICAL_PATH_COSTS")
            .and_then(|path| read_costs(Path::new(path)))
            .unwrap_or_default();
        Some(Self { packages })
    }

    pub(super) fn cost(&self, package: &str) -> usize {
        self.packages.get(package).copied().unwrap_or(DEFAULT_COST)
    }
}

fn read_costs(path: &Path) -> Option<HashMap<String, usize>> {
    if !fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    parse_costs(&bytes)
}

fn parse_costs(bytes: &[u8]) -> Option<HashMap<String, usize>> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let packages: HashMap<String, usize> = serde_json::from_slice(bytes).ok()?;
    if packages.len() > MAX_PACKAGES || packages.values().any(|&cost| cost == 0) {
        return None;
    }
    Some(packages)
}

#[cfg(test)]
mod tests {
    use super::{Costs, DEFAULT_COST, MAX_BYTES, MAX_PACKAGES, parse_costs, read_costs};
    use crate::util::data_structures::HashMap;
    use std::fs;

    #[test]
    fn package_hints_and_missing_package_fallback() {
        let costs = Costs {
            packages: parse_costs(br#"{"short":1,"long":1200}"#).unwrap(),
        };
        assert_eq!(costs.cost("short"), 1);
        assert_eq!(costs.cost("long"), 1200);
        assert_eq!(costs.cost("unknown"), DEFAULT_COST);
    }

    #[test]
    fn invalid_tables_fail_open() {
        for bytes in [
            &b""[..],
            &b"not json"[..],
            &b"[]"[..],
            &b"null"[..],
            &br#"{"package":0}"#[..],
            &br#"{"package":-1}"#[..],
            &br#"{"package":1.5}"#[..],
            &br#"{"package":"10"}"#[..],
            &br#"{"package":18446744073709551616}"#[..],
        ] {
            let parsed = parse_costs(bytes);
            assert!(parsed.is_none());
            let costs = Costs {
                packages: parsed.unwrap_or_default(),
            };
            assert_eq!(costs.cost("package"), DEFAULT_COST);
        }
    }

    #[test]
    fn table_size_and_package_count_are_bounded() {
        let mut bytes = b"{}".to_vec();
        bytes.resize(MAX_BYTES, b' ');
        assert!(parse_costs(&bytes).is_some());
        bytes.push(b' ');
        assert!(parse_costs(&bytes).is_none());

        let mut packages: HashMap<String, usize> = (0..MAX_PACKAGES)
            .map(|i| (format!("package-{i}"), 1))
            .collect();
        assert!(parse_costs(&serde_json::to_vec(&packages).unwrap()).is_some());
        packages.insert("one-too-many".into(), 1);
        assert!(parse_costs(&serde_json::to_vec(&packages).unwrap()).is_none());
    }

    #[test]
    fn only_bounded_regular_files_are_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("costs.json");
        assert!(read_costs(&path).is_none());
        assert!(read_costs(temp.path()).is_none());

        fs::write(&path, br#"{"package":42}"#).unwrap();
        assert_eq!(read_costs(&path).unwrap()["package"], 42);
        fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(read_costs(&path).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_sockets_are_not_hint_files() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("costs.json");
        fs::write(&path, br#"{"package":42}"#).unwrap();
        let link = temp.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(read_costs(&link).is_none());

        let socket = temp.path().join("socket.json");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(read_costs(&socket).is_none());
    }
}
