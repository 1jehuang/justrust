//! justrust: cross-crate early cutoff.
//!
//! When a library unit is dirty only because a dependency was rebuilt in this
//! build, rustc does not need to run again if every rebuilt dependency kept
//! the same *interface hash* as when this unit was last compiled.
//!
//! Contract with jr-rustc: under `-Zjr-iface-hash` it writes a sidecar
//! `<final .rmeta path>.jriface` (32 lowercase hex chars and a newline) next
//! to every `.rmeta` it emits. Two builds of a crate with the same interface
//! hash are interchangeable for dependents (body-only edits keep it), and
//! jr-rustc accepts a dependent rmeta that recorded an older crate hash of a
//! dependency whose interface hash matches. Stock rustc writes no sidecar, so
//! nothing is ever cut off and cargo behaves exactly as stock.
//!
//! Protocol:
//!
//! 1. While rustc runs for a unit, its own sidecar is removed
//!    ([`SidecarGuard`]) and only restored if rustc left the `.rmeta`
//!    untouched. A sidecar older than its `.rmeta` is ignored.
//! 2. After a non-linking lib unit (rlib/lib `build`, or `check`) compiles
//!    successfully, write `jr-dep-ifaces` into its fingerprint directory:
//!    `dep <iface> <rmeta path>` for every transitive library dependency and
//!    `out <mtime> <path>` for each own output. A dependency without a
//!    sidecar means no record.
//! 3. When the unit is next found dirty and the only reason is a stale
//!    dependency (`StaleDependency` / `StaleDepFingerprint`), its fingerprint
//!    hash, own sources and env are unchanged, every dependency without an
//!    `.rmeta` (build script runs, proc macros) is up to date, and the record
//!    matches its outputs, the decision is deferred to the job queue: once
//!    its dependencies are done, the recorded interface hashes are compared
//!    with the current sidecars. All equal: the unit is reported fresh, its
//!    outputs' (and own sidecar's) mtimes are bumped to now, contents
//!    untouched, and its fingerprint is written. Otherwise the record is
//!    deleted and rustc runs as usual.
//!
//! The `out` mtimes tie a record to the exact compile that wrote it, so a
//! rebuild by a cargo that does not know about records invalidates it.
//!
//! `JUSTRUST_CARGO_CUTOFF=0` disables recording and skipping.

use std::fs;
use std::path::{Path, PathBuf};

use cargo_util::paths;
use filetime::FileTime;
use tracing::debug;

/// File name of the record in the unit's fingerprint directory.
pub const RECORD_NAME: &str = "jr-dep-ifaces";

/// Whether cutoff is enabled (`JUSTRUST_CARGO_CUTOFF`, default on).
pub fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_CARGO_CUTOFF").as_deref(),
        Ok("0") | Ok("false") | Ok("off") | Ok("no")
    )
}

/// Path of the interface-hash sidecar of an `.rmeta` file.
pub fn sidecar_path(rmeta: &Path) -> PathBuf {
    let mut s = rmeta.as_os_str().to_owned();
    s.push(".jriface");
    PathBuf::from(s)
}

/// Reads the interface hash of a crate from its `.rmeta` sidecar. A sidecar
/// older than its `.rmeta` belongs to an earlier compile (a rustc that does
/// not write sidecars rebuilt the crate) and is ignored.
pub fn read_iface(rmeta: &Path) -> Option<String> {
    let side = sidecar_path(rmeta);
    if paths::mtime(&side).ok()? < paths::mtime(rmeta).ok()? {
        return None;
    }
    let text = fs::read_to_string(side).ok()?;
    let h = text.trim_end_matches('\n');
    (h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_string())
}

/// Guards a unit's own sidecar while rustc runs for it: the sidecar is
/// removed so that a rustc that does not write one (stock rustc) can never
/// leave a stale one next to a new `.rmeta`. If rustc did not rewrite the
/// `.rmeta` at all (incremental reuse, rust-lang/rust#114669), the old
/// sidecar still describes it and is put back.
pub struct SidecarGuard {
    rmeta: PathBuf,
    old: Option<(String, FileTime)>,
}

impl SidecarGuard {
    pub fn take(rmeta: &Path) -> SidecarGuard {
        let old = match (fs::read_to_string(sidecar_path(rmeta)), paths::mtime(rmeta)) {
            (Ok(text), Ok(m)) => Some((text, m)),
            _ => None,
        };
        let _ = fs::remove_file(sidecar_path(rmeta));
        SidecarGuard {
            rmeta: rmeta.to_path_buf(),
            old,
        }
    }

    /// Call after rustc succeeded.
    pub fn finish(self) {
        let side = sidecar_path(&self.rmeta);
        if let Some((text, m)) = self.old {
            if !side.exists() && paths::mtime(&self.rmeta).ok() == Some(m) {
                let _ = paths::write(&side, text);
            }
        }
    }
}

fn mtime_str(path: &Path) -> Option<String> {
    let m = paths::mtime(path).ok()?;
    Some(format!("{}.{:09}", m.unix_seconds(), m.nanoseconds()))
}

/// Writes the record for a unit that just compiled successfully: the
/// interface hash of every (transitive) library dependency and the mtime of
/// every own output.
/// The output mtimes tie the record to this exact compilation, so a rebuild
/// by any other cargo (which does not know about the record) invalidates it.
/// Anything unreadable means no record (no cutoff next time).
pub fn write_record(record: &Path, rmeta_deps: &[PathBuf], outputs: &[PathBuf]) {
    if rmeta_deps.is_empty() {
        // Nothing to cut off against; keep the target dir as stock.
        remove_record(record);
        return;
    }
    let mut out = String::new();
    for dep in rmeta_deps {
        let Some(svh) = read_iface(dep) else {
            debug!("jr-cutoff: no interface hash for {dep:?}, not recording");
            remove_record(record);
            return;
        };
        out.push_str(&format!("dep {svh} {}\n", dep.to_string_lossy()));
    }
    for o in outputs {
        let Some(m) = mtime_str(o) else {
            remove_record(record);
            return;
        };
        out.push_str(&format!("out {m} {}\n", o.to_string_lossy()));
    }
    if let Err(e) = paths::write_atomic(record, out.as_bytes()) {
        debug!("jr-cutoff: failed to write {record:?}: {e}");
        remove_record(record);
    }
}

pub fn remove_record(record: &Path) {
    let _ = fs::remove_file(record);
}

struct Record {
    /// `(rmeta path, svh)`
    deps: Vec<(PathBuf, String)>,
    /// `(output path, mtime)`
    outs: Vec<(PathBuf, String)>,
}

fn read_record(record: &Path) -> Option<Record> {
    let text = fs::read_to_string(record).ok()?;
    let mut r = Record {
        deps: Vec::new(),
        outs: Vec::new(),
    };
    for line in text.lines() {
        let mut it = line.splitn(3, ' ');
        let (kind, val, path) = (it.next()?, it.next()?, it.next()?);
        match kind {
            "dep" => {
                if val.len() != 32 || !val.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                r.deps.push((PathBuf::from(path), val.to_string()));
            }
            "out" => r.outs.push((PathBuf::from(path), val.to_string())),
            _ => return None,
        }
    }
    Some(r)
}

/// Plan-time decision for a unit that may be cut off at execution time.
#[derive(Debug, Clone)]
pub struct CutoffPlan {
    pub record: PathBuf,
    /// Current `.rmeta` outputs of the (transitive) library dependencies.
    pub rmeta_deps: Vec<PathBuf>,
    /// Own outputs, whose mtimes are bumped when the unit is cut off.
    pub outputs: Vec<PathBuf>,
}

impl CutoffPlan {
    /// Whether a record exists that covers exactly these dependencies and
    /// whose own outputs are still the ones it was written for.
    pub fn has_matching_record(&self) -> bool {
        let Some(r) = read_record(&self.record) else {
            return false;
        };
        self.same_shape(&r) && self.outputs_match(&r)
    }

    fn same_shape(&self, r: &Record) -> bool {
        r.deps.len() == self.rmeta_deps.len()
            && self
                .rmeta_deps
                .iter()
                .all(|d| r.deps.iter().any(|(p, _)| p == d))
            && r.outs.len() == self.outputs.len()
            && self
                .outputs
                .iter()
                .all(|o| r.outs.iter().any(|(p, _)| p == o))
    }

    fn outputs_match(&self, r: &Record) -> bool {
        r.outs
            .iter()
            .all(|(p, m)| mtime_str(p).as_deref() == Some(m.as_str()))
    }

    /// Execution-time check, after all dependencies finished. Returns true if
    /// every dependency's current SVH equals the recorded one. On false the
    /// record is removed (rustc is about to run).
    pub fn probe(&self) -> bool {
        let ok = self.probe_inner();
        if !ok {
            remove_record(&self.record);
        }
        debug!("jr-cutoff: probe {:?} -> {ok}", self.record);
        ok
    }

    fn probe_inner(&self) -> bool {
        let Some(r) = read_record(&self.record) else {
            debug!("jr-cutoff: no readable record");
            return false;
        };
        if !self.same_shape(&r) {
            debug!("jr-cutoff: record shape differs (deps or outputs changed)");
            return false;
        }
        if !self.outputs_match(&r) {
            debug!("jr-cutoff: own outputs changed since the record");
            return false;
        }
        for (p, h) in &r.deps {
            let now = read_iface(p);
            if now.as_deref() != Some(h.as_str()) {
                debug!("jr-cutoff: interface of {p:?} changed: {h} -> {now:?}");
                return false;
            }
        }
        true
    }

    /// Cut off: marks the outputs as produced now, so the next cargo run sees
    /// them as newer than the rebuilt dependencies, and re-records (same
    /// SVHs, new output mtimes).
    pub fn commit(&self) {
        // Take "now" from the filesystem like rustc invocations do
        // (`set_invocation_time`): `FileTime::now()` is a precise clock while
        // file mtimes use the kernel's coarse cached time, so a precise
        // timestamp can be newer than the invocation time of a dependent
        // that starts right after us, making it look stale on the next run.
        let now = self
            .record
            .parent()
            .and_then(|dir| paths::set_invocation_time(dir).ok())
            .unwrap_or_else(FileTime::now);
        for o in &self.outputs {
            paths::set_file_time_no_err(o, now);
            // Keep the own sidecar (contents untouched) not older than the
            // `.rmeta`, so dependents still trust it.
            let side = sidecar_path(o);
            if side.exists() {
                paths::set_file_time_no_err(&side, now);
            }
        }
        write_record(&self.record, &self.rmeta_deps, &self.outputs);
    }
}

/// Counts cut-off units in this process, for `JUSTRUST_CARGO_CUTOFF_LOG`.
pub fn log_skip(unit_desc: &str) {
    if let Some(path) = std::env::var_os("JUSTRUST_CARGO_CUTOFF_LOG") {
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "skip {unit_desc}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_rmeta(dir: &Path, name: &str, iface: u8) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"rust").unwrap();
        fs::write(
            sidecar_path(&p),
            format!("{}\n", format!("{iface:02x}").repeat(16)),
        )
        .unwrap();
        p
    }

    #[test]
    fn record_roundtrip_and_probe() {
        let dir = std::env::temp_dir().join(format!("jr-cutoff-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = fake_rmeta(&dir, "liba.rmeta", 1);
        let b = fake_rmeta(&dir, "libb.rmeta", 2);
        let out = dir.join("libme.rmeta");
        fs::write(&out, b"x").unwrap();
        let record = dir.join(RECORD_NAME);
        assert_eq!(read_iface(&a).unwrap(), "01".repeat(16));
        write_record(&record, &[a.clone(), b.clone()], &[out.clone()]);
        let plan = CutoffPlan {
            record: record.clone(),
            rmeta_deps: vec![b.clone(), a.clone()],
            outputs: vec![out.clone()],
        };
        assert!(plan.has_matching_record());
        assert!(plan.probe());
        plan.commit();
        assert!(plan.has_matching_record(), "commit re-records");
        // An output rewritten behind our back (stock cargo) invalidates it.
        filetime::set_file_mtime(&out, FileTime::from_unix_time(1, 0)).unwrap();
        assert!(!plan.has_matching_record());
        plan.commit();
        fake_rmeta(&dir, "libb.rmeta", 3);
        assert!(!plan.probe());
        assert!(!record.exists(), "failed probe removes the record");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_stale_sidecar() {
        let dir = std::env::temp_dir().join(format!("jr-cutoff-n-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("libx.rmeta");
        fs::write(&p, b"rust").unwrap();
        assert!(read_iface(&p).is_none());
        fs::write(sidecar_path(&p), "zz\n").unwrap();
        assert!(read_iface(&p).is_none(), "malformed");
        fs::write(sidecar_path(&p), format!("{}\n", "ab".repeat(16))).unwrap();
        assert!(read_iface(&p).is_some());
        // rmeta rewritten after the sidecar (by a rustc without sidecars)
        filetime::set_file_mtime(sidecar_path(&p), FileTime::from_unix_time(1, 0)).unwrap();
        assert!(read_iface(&p).is_none(), "stale");
        // guard: removed while rustc runs, restored when rmeta untouched
        fs::write(sidecar_path(&p), format!("{}\n", "ab".repeat(16))).unwrap();
        let g = SidecarGuard::take(&p);
        assert!(!sidecar_path(&p).exists());
        g.finish();
        assert!(read_iface(&p).is_some(), "restored");
        let g = SidecarGuard::take(&p);
        filetime::set_file_mtime(&p, FileTime::from_unix_time(2, 0)).unwrap();
        g.finish();
        assert!(!sidecar_path(&p).exists(), "rmeta rewritten, not restored");
        fs::remove_dir_all(&dir).unwrap();
    }
}
