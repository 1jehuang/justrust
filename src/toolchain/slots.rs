//! Opt-in, two-slot compiler lifecycle. Never changes default selection policy.
use super::{Spec, Toolchain};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand)]
pub enum Command {
    /// Register an installed/downloadable compiler as known-good.
    Init { current: String },
    /// Stage a candidate without changing global compiler policy.
    Begin { next: String },
    /// Assert successful validation of the active candidate.
    Validate { active: String },
    /// Explicitly discard an unused failed candidate, retaining the known-good compiler.
    Discard { failed: String },
    /// Print durable manager state as JSON.
    Status,
    /// Delete only a retired managed compiler, once all usage has drained.
    Cleanup,
}

fn arguments(command: Command) -> Vec<String> {
    match command {
        Command::Init { current } => vec!["init".into(), current],
        Command::Begin { next } => vec!["begin".into(), next],
        Command::Validate { active } => vec!["validate".into(), active],
        Command::Discard { failed } => vec!["discard".into(), failed],
        Command::Status => vec!["status".into()],
        Command::Cleanup => vec!["cleanup".into()],
    }
}

/// Existing authenticated tenant transport and immutable exact-client upload.
/// Never depends on a mutable fleet binary, PATH, or an additional service.
pub fn remote(command: Command) -> Result<()> {
    if !matches!(
        crate::remote_backend::load(),
        Some(crate::remote_backend::Backend::Hosted)
    ) {
        bail!("compiler slots require the hosted backend");
    }
    let target = crate::remote_hosted::ensure_ready()?;
    let executable = crate::remote_build::ensure_session_executable(&target)?;
    let args = arguments(command);
    let script = format!(
        "exec {} toolchain slots {}",
        crate::remote_build::shell_quote(&executable.path),
        args.iter()
            .map(|a| crate::remote_build::shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let status = target.command().arg(script).status()?;
    if !status.success() {
        bail!("hosted compiler slot command failed: {status}");
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Slot {
    id: String,
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    active: Slot,
    last_good: String,
    previous: Option<Slot>,
    #[serde(default)]
    discarded: Option<String>,
}

fn id(value: &str) -> Result<Spec> {
    let spec = Spec::parse(value)?;
    if !spec.is_jr() || spec.id() != value {
        bail!("slots require an explicit jr-<7..40 hex digits> ID");
    }
    Ok(spec)
}

fn read(root: &Path) -> Result<Option<State>> {
    let bytes = match std::fs::read(root.join(".slots.json")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let state: State =
        serde_json::from_slice(&bytes).context("invalid slots state; refusing mutation")?;
    if state.version != 1 {
        bail!("unsupported slots state version");
    }
    for slot in std::iter::once(&state.active).chain(state.previous.iter()) {
        id(&slot.id)?;
        let managed = root.join("managed");
        if slot.path != managed.join(&slot.id) && slot.path != managed.join("staged").join(&slot.id)
        {
            bail!("managed path is outside its exact toolchain namespace");
        }
    }
    if state
        .previous
        .as_ref()
        .is_some_and(|p| p.id == state.active.id)
        || (state.last_good != state.active.id
            && !state
                .previous
                .as_ref()
                .is_some_and(|p| p.id == state.last_good))
    {
        bail!("inconsistent slots state");
    }
    if let Some(discarded) = &state.discarded {
        id(discarded)?;
        if state.previous.is_some()
            || state.last_good != state.active.id
            || discarded == &state.active.id
        {
            bail!("inconsistent discarded candidate receipt");
        }
    }
    Ok(Some(state))
}

fn write(root: &Path, state: &State) -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = root.join(format!(".slots.json.tmp-{}-{nonce}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&serde_json::to_vec_pretty(state)?)?;
    file.sync_all()?;
    std::fs::rename(path, root.join(".slots.json"))?;
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    Ok(())
}

fn open_lock(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

fn inheritable(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        // SAFETY: fd is a live, owned descriptor. Only its descriptor flags change.
        let result = unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 {
                -1
            } else {
                libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC)
            }
        };
        if result < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

/// Selection is serialized with lifecycle changes, before any compiler lookup.
pub fn selection() -> Result<Option<File>> {
    selection_at(&super::root()?)
}

fn selection_at(root: &Path) -> Result<Option<File>> {
    let open = || -> Result<File> {
        std::fs::create_dir_all(root)?;
        let file = open_lock(&root.join(".slots.lock"))?;
        file.lock_shared()?;
        read(root)?;
        // Probes inherit admission if their recording parent is killed.
        inheritable(&file)?;
        Ok(file)
    };
    match open() {
        Ok(file) => Ok(Some(file)),
        // An unmanaged command must not fail merely because recording storage
        // is unavailable. Existing managed state remains fail-closed.
        Err(_) if !root.join(".slots.json").exists() => Ok(None),
        Err(error) => Err(error),
    }
}

fn usage_path(root: &Path, name: &str) -> PathBuf {
    root.join(format!(".usage-{name}.lock"))
}

/// An inherited shared descriptor keeps compiler usage admitted until all holders exit.
pub struct Lease {
    _file: File,
}

impl Lease {
    pub fn inherit(&self, command: &mut std::process::Command) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            let fd = self._file.as_raw_fd();
            // SAFETY: this child-only hook performs only async-signal-safe fcntl.
            unsafe {
                command.pre_exec(move || {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(not(unix))]
        let _ = command;
    }
}

/// Caller holds selection() until this lease has been acquired.
pub fn lease(tc: Option<&Toolchain>) -> Result<Option<Lease>> {
    let Some(tc) = tc else { return Ok(None) };
    let root = super::root()?;
    lease_at(&root, tc)
}

fn lease_at(root: &Path, tc: &Toolchain) -> Result<Option<Lease>> {
    // Legacy clients and explicit project installations never select this
    // namespace, so their unleased processes cannot race managed deletion.
    if !tc.dir.starts_with(root.join("managed")) {
        return Ok(None);
    }
    let spec = Spec::parse(&tc.id)?;
    let file = open_lock(&usage_path(root, &spec.id()))?;
    file.lock_shared()?;
    reject_retired(root, &tc.id)?;
    if !tc.bin("rustc").is_file() || !tc.cargo().is_file() {
        bail!(
            "selected compiler {} disappeared before usage admission",
            tc.id
        );
    }
    Ok(Some(Lease { _file: file }))
}

pub(crate) fn reject_retired(root: &Path, name: &str) -> Result<()> {
    if let Some(state) = read(root)?
        && state.last_good == state.active.id
        && state.previous.as_ref().is_some_and(|p| p.id == name)
    {
        bail!(
            "compiler {name} is retired by the slot manager; select {}",
            state.active.id
        );
    }
    Ok(())
}

pub(super) fn for_build(spec: &Spec) -> Option<Toolchain> {
    let root = super::root().ok()?;
    lookup(&root, spec).ok().flatten()
}

fn lookup(root: &Path, spec: &Spec) -> Result<Option<Toolchain>> {
    let Some(state) = read(root)? else {
        return Ok(None);
    };
    let registered =
        state.active.id == spec.id() || state.previous.as_ref().is_some_and(|p| p.id == spec.id());
    if !registered || reject_retired(root, &spec.id()).is_err() {
        return Ok(None);
    }
    super::installed_in(spec, &root.join("managed"))
}

fn install(name: &str) -> Result<Slot> {
    let root = super::root()?.join("managed");
    std::fs::create_dir_all(&root)?;
    let base = super::bundle::base()?;
    let tc = super::install_locked(
        &id(name)?,
        super::DEFAULT_COMPONENTS,
        &mut std::io::stderr(),
        &root,
        base.as_deref(),
    )?;
    Ok(Slot {
        id: tc.id,
        path: tc.dir,
    })
}

fn response(state: Option<&State>, status: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "status": status,
        "active": state.map(|s| &s.active.id),
        "last_good": state.map(|s| &s.last_good),
        "validated": state.is_some_and(|s| s.last_good == s.active.id),
        "previous": state.and_then(|s| s.previous.as_ref()).map(|s| &s.id),
        "discarded": state.and_then(|s| s.discarded.as_ref()),
    })
}

// Caller holds both lifecycle and installer locks. False means safely retained.
fn delete_unused(root: &Path, slot: &Slot) -> Result<bool> {
    #[cfg(not(unix))]
    {
        let _ = (root, slot);
        bail!("cleanup requires inheritable usage leases on this platform");
    }
    #[cfg(unix)]
    {
        let usage = open_lock(&usage_path(root, &slot.id))?;
        if usage.try_lock().is_err() {
            return Ok(false);
        }
        if slot.path.exists() {
            let meta = std::fs::symlink_metadata(&slot.path)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                bail!("managed prefix is not a real directory");
            }
            // A symlink in staged/ must never redirect deletion into an unrelated tree.
            if std::fs::canonicalize(&slot.path)?
                != std::fs::canonicalize(root)?.join(slot.path.strip_prefix(root)?)
            {
                bail!("managed prefix traverses a symlink");
            }
            let info: super::Installed =
                serde_json::from_slice(&std::fs::read(slot.path.join(super::MARKER))?)?;
            if info.id != slot.id {
                bail!("managed compiler identity changed");
            }
            std::fs::remove_dir_all(&slot.path)?;
            #[cfg(unix)]
            File::open(slot.path.parent().context("missing parent")?)?.sync_all()?;
        }
        Ok(true)
    }
}

fn check_owned_prefixes(root: &Path, state: Option<&State>, command: &Command) -> Result<()> {
    // A crash between installation and state publication may leave one complete
    // unregistered candidate. Only an exact retry may adopt it, never silently
    // install a third compiler or remove an unknown directory.
    if matches!(command, Command::Status) {
        return Ok(());
    }
    let mut allowed = Vec::new();
    if let Some(state) = state {
        allowed.push(state.active.id.as_str());
        if let Some(previous) = &state.previous {
            allowed.push(previous.id.as_str());
        }
    }
    match command {
        Command::Init { current } if state.is_none() => allowed.push(current),
        Command::Begin { next } if state.is_some_and(|s| s.previous.is_none()) => {
            allowed.push(next)
        }
        _ => {}
    }
    let managed = root.join("managed");
    for directory in [&managed, &managed.join("staged")] {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("jr-") && !allowed.contains(&name.as_ref()) {
                bail!(
                    "unregistered managed compiler {name}; inspect interrupted release before installing another"
                );
            }
        }
    }
    Ok(())
}

pub fn run(command: Command) -> Result<()> {
    let root = super::root()?;
    run_at(&root, command, &mut install)
}

fn begin_allowed(root: &Path, state: &State, next: &str) -> Result<()> {
    id(next)?;
    if state.active.id == next {
        bail!("candidate is already active");
    }
    if state.previous.is_some() {
        bail!(
            "two slots occupied: validate the current candidate and cleanup before beginning another, or discard FAILED_ID after drain; last-good retained"
        );
    }
    reject_retired(root, next)
}

fn validate_allowed(state: &State, active: &str) -> Result<()> {
    id(active)?;
    if active != state.active.id {
        bail!("validation must name active compiler {}", state.active.id);
    }
    Ok(())
}

/// Download, unpack and verify the compilers a lifecycle change needs before
/// it excludes builds.
///
/// Every build on this machine waits at the lifecycle gate while it is held
/// exclusively. Installing a candidate takes seconds to minutes, so that
/// happens here, holding the gate shared like builds do: they keep selecting
/// and leasing compilers meanwhile. The installer lock still excludes other
/// installers, and deletion needs the exclusive gate. Installation publishes
/// a complete prefix with one rename, so a build never sees part of one.
fn prepare(
    root: &Path,
    command: &Command,
    install: &mut dyn FnMut(&str) -> Result<Slot>,
) -> Result<Vec<Slot>> {
    if !matches!(command, Command::Begin { .. } | Command::Validate { .. }) {
        return Ok(Vec::new());
    }
    let gate = open_lock(&root.join(".slots.lock"))?;
    gate.lock_shared()?;
    let _install = super::lock_exclusive(&root.join(".install.lock"))?;
    let state = read(root)?;
    check_owned_prefixes(root, state.as_ref(), command)?;
    let state = state.as_ref().context("initialize slots first")?;
    let names = match command {
        Command::Begin { next } => {
            begin_allowed(root, state, next)?;
            // Reverify CURRENT before NEXT. Neither activates defaults.
            vec![state.active.id.as_str(), next.as_str()]
        }
        Command::Validate { active } => {
            validate_allowed(state, active)?;
            vec![active.as_str()]
        }
        _ => Vec::new(),
    };
    names.into_iter().map(install).collect()
}

/// A compiler `prepare` installed is still complete under the exclusive gate.
fn still_installed(slot: &Slot) -> bool {
    std::fs::read(slot.path.join(super::MARKER))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<super::Installed>(&bytes).ok())
        .is_some_and(|info| info.id == slot.id)
}

fn run_at(
    root: &Path,
    command: Command,
    install: &mut dyn FnMut(&str) -> Result<Slot>,
) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let mut prepared = prepare(root, &command, install)?;
    let _lifecycle = super::lock_exclusive(&root.join(".slots.lock"))?;
    inheritable(&_lifecycle)?;
    let _install = super::lock_exclusive(&root.join(".install.lock"))?;
    // Another lifecycle change may have run between preparing and this gate:
    // every check repeats here, and a prepared compiler is reused only while
    // it is still installed.
    let mut install = |name: &str| -> Result<Slot> {
        match prepared.iter().position(|slot| slot.id == name) {
            Some(found) if still_installed(&prepared[found]) => Ok(prepared.swap_remove(found)),
            _ => install(name),
        }
    };
    let mut state = read(root)?;
    check_owned_prefixes(root, state.as_ref(), &command)?;
    let mut outcome = "ok";
    match command {
        Command::Status => {
            println!("{}", response(state.as_ref(), "ok"));
            return Ok(());
        }
        Command::Init { current } => {
            if state.is_some() {
                bail!("slots already initialized");
            }
            let active = install(&current)?;
            state = Some(State {
                version: 1,
                last_good: active.id.clone(),
                active,
                previous: None,
                discarded: None,
            });
        }
        Command::Begin { next } => {
            let s = state.as_mut().context("initialize slots first")?;
            begin_allowed(root, s, &next)?;
            // Reverify CURRENT and install NEXT with the native installer. Neither activates defaults.
            install(&s.active.id)?;
            let candidate = install(&next)?;
            s.previous = Some(std::mem::replace(&mut s.active, candidate));
            s.discarded = None;
        }
        Command::Validate { active } => {
            let s = state.as_mut().context("initialize slots first")?;
            validate_allowed(s, &active)?;
            install(&active)?;
            s.last_good = active;
        }
        Command::Discard { failed } => {
            id(&failed)?;
            let s = state.as_mut().context("initialize slots first")?;
            if s.active.id != failed || s.last_good == s.active.id || s.previous.is_none() {
                bail!(
                    "discard must name the unvalidated active candidate; known-good cannot be discarded"
                );
            }
            if !delete_unused(root, &s.active)? {
                println!("{}", response(state.as_ref(), "busy"));
                return Ok(());
            }
            s.active = s.previous.take().context("missing last-good")?;
            s.discarded = Some(failed);
            outcome = "discarded";
        }
        Command::Cleanup => {
            let s = state.as_mut().context("initialize slots first")?;
            if s.last_good != s.active.id {
                bail!("candidate is not validated; retaining last-good");
            }
            outcome = "nothing";
            if let Some(previous) = &s.previous {
                if !delete_unused(root, previous)? {
                    println!("{}", response(state.as_ref(), "busy"));
                    return Ok(());
                }
                s.previous = None;
                outcome = "cleaned";
            }
        }
    }
    write(root, state.as_ref().context("missing slots state")?)?;
    println!("{}", response(state.as_ref(), outcome));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "justrust-slots-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn run(&self, command: Command) -> Result<()> {
            run_at(&self.0, command, &mut |name| {
                id(name)?;
                let path = self.0.join("managed/staged").join(name);
                std::fs::create_dir_all(&path)?;
                let info = super::super::Installed {
                    id: name.into(),
                    ..Default::default()
                };
                std::fs::write(path.join(super::super::MARKER), serde_json::to_vec(&info)?)?;
                Ok(Slot {
                    id: name.into(),
                    path,
                })
            })
        }
        fn init(&self) {
            self.run(Command::Init {
                current: "jr-aaaaaaa".into(),
            })
            .unwrap();
        }
        fn begin(&self) {
            self.run(Command::Begin {
                next: "jr-bbbbbbb".into(),
            })
            .unwrap();
        }
        #[cfg(not(unix))]
        fn assert_deletion_refused(&self, command: Command) {
            let receipt = self.0.join(".slots.json");
            let before = std::fs::read(&receipt).unwrap();
            let error = self.run(command).unwrap_err();
            assert_eq!(
                error.to_string(),
                "cleanup requires inheritable usage leases on this platform"
            );
            assert_eq!(std::fs::read(receipt).unwrap(), before);
            for id in ["jr-aaaaaaa", "jr-bbbbbbb"] {
                assert!(
                    self.0
                        .join("managed/staged")
                        .join(id)
                        .join(super::super::MARKER)
                        .is_file()
                );
            }
        }
        #[cfg(unix)]
        fn cleanup_until_drained(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                self.run(Command::Cleanup).unwrap();
                if read(&self.0).unwrap().unwrap().previous.is_none() {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "compiler leases did not drain"
                );
                // Parallel process-spawning tests can briefly inherit a descriptor
                // between fork and exec, even when it is close-on-exec.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        fn validate(&self) {
            self.run(Command::Validate {
                active: "jr-bbbbbbb".into(),
            })
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn explicit_ids_include_legacy_short_but_not_aliases_or_paths() {
        for good in [
            "jr-abcdef0",
            "jr-0123456789ab",
            "jr-0123456789012345678901234567890123456789",
        ] {
            assert!(id(good).is_ok());
        }
        for bad in [
            "nightly",
            "1.90.0",
            "jr-abcd",
            "jr-abcdefg",
            "jr-../abcdef",
            " jr-abcdef0",
        ] {
            assert!(id(bad).is_err());
        }
    }

    #[test]
    fn failed_candidates_never_displace_last_good_or_allow_third_slot() {
        let f = Fixture::new();
        f.init();
        f.begin();
        let before = std::fs::read(f.0.join(".slots.json")).unwrap();
        for next in ["jr-ccccccc", "jr-ddddddd"] {
            assert!(f.run(Command::Begin { next: next.into() }).is_err());
        }
        assert!(f.run(Command::Cleanup).is_err());
        assert!(
            f.run(Command::Validate {
                active: "jr-aaaaaaa".into()
            })
            .is_err()
        );
        assert_eq!(before, std::fs::read(f.0.join(".slots.json")).unwrap());
        assert_eq!(read(&f.0).unwrap().unwrap().last_good, "jr-aaaaaaa");
        assert!(reject_retired(&f.0, "jr-aaaaaaa").is_ok());
    }

    #[test]
    fn cleanup_requires_validation_and_drain_and_only_deletes_registered_prefix() {
        let f = Fixture::new();
        f.init();
        f.begin();
        let unrelated = f.0.join("jr-ccccccc");
        std::fs::create_dir(&unrelated).unwrap();
        let lease = open_lock(&usage_path(&f.0, "jr-aaaaaaa")).unwrap();
        lease.lock_shared().unwrap();
        f.validate();
        assert!(reject_retired(&f.0, "jr-aaaaaaa").is_err());
        #[cfg(not(unix))]
        {
            f.assert_deletion_refused(Command::Cleanup);
            drop(lease);
            f.assert_deletion_refused(Command::Cleanup);
            assert!(unrelated.is_dir());
        }
        #[cfg(unix)]
        {
            f.run(Command::Cleanup).unwrap();
            assert!(f.0.join("managed/staged/jr-aaaaaaa").exists());
            drop(lease);
            f.cleanup_until_drained();
            assert!(!f.0.join("managed/staged/jr-aaaaaaa").exists());
            assert!(f.0.join("managed/staged/jr-bbbbbbb").exists());
            assert!(unrelated.exists());
            assert!(reject_retired(&f.0, "jr-aaaaaaa").is_ok());
            f.run(Command::Cleanup).unwrap();
            f.run(Command::Begin {
                next: "jr-ddddddd".into(),
            })
            .unwrap();
            assert_eq!(read(&f.0).unwrap().unwrap().last_good, "jr-bbbbbbb");
        }
    }

    #[test]
    fn state_corruption_and_foreign_paths_fail_closed() {
        let f = Fixture::new();
        f.init();
        let mut s = read(&f.0).unwrap().unwrap();
        s.active.path = f.0.join("foreign");
        write(&f.0, &s).unwrap();
        assert!(read(&f.0).is_err());
        assert!(f.run(Command::Cleanup).is_err());
        std::fs::write(f.0.join(".slots.json"), b"broken").unwrap();
        assert!(
            f.run(Command::Init {
                current: "jr-bbbbbbb".into()
            })
            .is_err()
        );
    }

    #[test]
    fn failed_candidate_discard_is_explicit_drained_and_preserves_last_good() {
        let f = Fixture::new();
        f.init();
        f.begin();
        let held = open_lock(&usage_path(&f.0, "jr-bbbbbbb")).unwrap();
        held.lock_shared().unwrap();
        #[cfg(not(unix))]
        {
            let discard = || Command::Discard {
                failed: "jr-bbbbbbb".into(),
            };
            f.assert_deletion_refused(discard());
            drop(held);
            f.assert_deletion_refused(discard());
        }
        #[cfg(unix)]
        {
            f.run(Command::Discard {
                failed: "jr-bbbbbbb".into(),
            })
            .unwrap();
            assert_eq!(read(&f.0).unwrap().unwrap().active.id, "jr-bbbbbbb");
            assert!(
                f.run(Command::Discard {
                    failed: "jr-aaaaaaa".into()
                })
                .is_err()
            );
            drop(held);
            // A child another test is forking can hold the use lock until it
            // execs, and a discard leaves a toolchain that is still in use.
            assert!(crate::test_fork::eventually(|| {
                f.run(Command::Discard {
                    failed: "jr-bbbbbbb".into(),
                })
                .unwrap();
                !f.0.join("managed/staged/jr-bbbbbbb").exists()
            }));
            assert!(f.0.join("managed/staged/jr-aaaaaaa").exists());
            let state = read(&f.0).unwrap().unwrap();
            assert_eq!(state.discarded.as_deref(), Some("jr-bbbbbbb"));
            assert_eq!(response(Some(&state), "ok")["discarded"], "jr-bbbbbbb");
            f.run(Command::Begin {
                next: "jr-ccccccc".into(),
            })
            .unwrap();
            assert_eq!(read(&f.0).unwrap().unwrap().last_good, "jr-aaaaaaa");
            assert!(read(&f.0).unwrap().unwrap().discarded.is_none());
        }
    }

    #[test]
    fn legacy_and_project_installations_are_never_managed_deletion_targets() {
        let f = Fixture::new();
        f.init();
        f.begin();
        let legacy = Toolchain {
            id: "jr-aaaaaaa".into(),
            dir: f.0.join("staged/jr-aaaaaaa"),
            nightly: true,
        };
        std::fs::create_dir_all(&legacy.dir).unwrap();
        std::fs::write(legacy.dir.join("keep"), b"unleased legacy compiler").unwrap();
        assert!(lease_at(&f.0, &legacy).unwrap().is_none());
        f.validate();
        #[cfg(not(unix))]
        f.assert_deletion_refused(Command::Cleanup);
        #[cfg(unix)]
        {
            f.run(Command::Cleanup).unwrap();
            assert!(!f.0.join("managed/staged/jr-aaaaaaa").exists());
        }
        assert!(legacy.dir.join("keep").exists());
        // A legacy prefix can never be adopted by modifying the manager receipt.
        let mut state = read(&f.0).unwrap().unwrap();
        state.active.path = legacy.dir;
        write(&f.0, &state).unwrap();
        assert!(read(&f.0).is_err());
    }

    #[test]
    fn orphaned_installed_candidate_requires_exact_retry_not_a_third_slot() {
        let f = Fixture::new();
        f.init();
        std::fs::create_dir_all(f.0.join("managed/staged/jr-bbbbbbb")).unwrap();
        assert!(
            f.run(Command::Begin {
                next: "jr-ccccccc".into()
            })
            .is_err()
        );
        f.begin();
        assert_eq!(read(&f.0).unwrap().unwrap().active.id, "jr-bbbbbbb");
    }

    #[test]
    fn unmanaged_admission_stays_fail_open_when_storage_is_unavailable() {
        let f = Fixture::new();
        let file = f.0.join("not-a-directory");
        std::fs::write(&file, b"fixture").unwrap();
        assert!(selection_at(&file).unwrap().is_none());
        f.init();
        std::fs::write(f.0.join(".slots.json"), b"broken").unwrap();
        assert!(selection_at(&f.0).is_err());
    }

    #[test]
    fn public_response_has_flat_versioned_contract() {
        let f = Fixture::new();
        assert_eq!(response(None, "ok")["active"], serde_json::Value::Null);
        f.init();
        f.begin();
        let state = read(&f.0).unwrap().unwrap();
        let json = response(Some(&state), "busy");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["active"], "jr-bbbbbbb");
        assert_eq!(json["last_good"], "jr-aaaaaaa");
        assert_eq!(json["validated"], false);
        assert_eq!(json["status"], "busy");
    }

    #[test]
    fn builds_keep_selecting_compilers_while_a_candidate_installs() {
        let f = Fixture::new();
        f.init();
        let mut installs = Vec::new();
        let mut installer = |name: &str| -> Result<Slot> {
            // What a build does first: `selection_at`, then its usage lease.
            let gate = open_lock(&f.0.join(".slots.lock"))?;
            gate.try_lock_shared()
                .map_err(|_| anyhow::anyhow!("installing {name} excluded builds"))?;
            installs.push(name.to_owned());
            let path = f.0.join("managed/staged").join(name);
            std::fs::create_dir_all(&path)?;
            let info = super::super::Installed {
                id: name.into(),
                ..Default::default()
            };
            std::fs::write(path.join(super::super::MARKER), serde_json::to_vec(&info)?)?;
            Ok(Slot {
                id: name.into(),
                path,
            })
        };
        run_at(
            &f.0,
            Command::Begin {
                next: "jr-bbbbbbb".into(),
            },
            &mut installer,
        )
        .unwrap();
        run_at(
            &f.0,
            Command::Validate {
                active: "jr-bbbbbbb".into(),
            },
            &mut installer,
        )
        .unwrap();
        // Each compiler is installed or reverified once, never again under
        // the exclusive gate.
        assert_eq!(installs, ["jr-aaaaaaa", "jr-bbbbbbb", "jr-bbbbbbb"]);
        let state = read(&f.0).unwrap().unwrap();
        assert_eq!(state.active.id, "jr-bbbbbbb");
        assert_eq!(state.last_good, "jr-bbbbbbb");
        assert_eq!(state.previous.unwrap().id, "jr-aaaaaaa");
    }

    #[test]
    fn prepared_compiler_that_vanished_is_installed_again_under_the_gate() {
        let f = Fixture::new();
        f.init();
        let slot = Slot {
            id: "jr-aaaaaaa".into(),
            path: f.0.join("managed/staged/jr-aaaaaaa"),
        };
        assert!(still_installed(&slot));
        std::fs::write(
            slot.path.join(super::super::MARKER),
            b"{\"id\":\"jr-ccccccc\"}",
        )
        .unwrap();
        assert!(!still_installed(&slot));
        std::fs::remove_dir_all(&slot.path).unwrap();
        assert!(!still_installed(&slot));
        // Rejected requests install nothing before the gate either.
        let mut calls = 0;
        let rejected = run_at(
            &f.0,
            Command::Validate {
                active: "jr-bbbbbbb".into(),
            },
            &mut |_| {
                calls += 1;
                bail!("must not install")
            },
        );
        assert!(
            rejected
                .unwrap_err()
                .to_string()
                .contains("must name active compiler")
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn installer_failure_leaves_last_good_and_durable_state_unchanged() {
        let f = Fixture::new();
        f.init();
        let before = std::fs::read(f.0.join(".slots.json")).unwrap();
        let result = run_at(
            &f.0,
            Command::Begin {
                next: "jr-bbbbbbb".into(),
            },
            &mut |name| {
                if name == "jr-bbbbbbb" {
                    bail!("synthetic native install failure");
                }
                Ok(Slot {
                    id: name.into(),
                    path: f.0.join("managed/staged").join(name),
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(before, std::fs::read(f.0.join(".slots.json")).unwrap());
    }

    #[test]
    fn lease_checks_existence_and_retirement_under_lifecycle_gate() {
        let f = Fixture::new();
        f.init();
        f.begin();
        let tc = Toolchain {
            id: "jr-aaaaaaa".into(),
            dir: f.0.join("managed/staged/jr-aaaaaaa"),
            nightly: true,
        };
        assert!(lease_at(&f.0, &tc).is_err());
        std::fs::create_dir(tc.dir.join("bin")).unwrap();
        for bin in ["rustc", "cargo"] {
            std::fs::write(tc.bin(bin), b"fixture").unwrap();
        }
        let selection = open_lock(&f.0.join(".slots.lock")).unwrap();
        selection.lock_shared().unwrap();
        let lifecycle = open_lock(&f.0.join(".slots.lock")).unwrap();
        assert!(lifecycle.try_lock().is_err());
        let lease = lease_at(&f.0, &tc).unwrap();
        drop(selection);
        f.validate();
        #[cfg(not(unix))]
        f.assert_deletion_refused(Command::Cleanup);
        #[cfg(unix)]
        f.run(Command::Cleanup).unwrap();
        assert!(tc.dir.exists());
        assert!(lease_at(&f.0, &tc).is_err());
        drop(lease);
        #[cfg(not(unix))]
        f.assert_deletion_refused(Command::Cleanup);
        #[cfg(unix)]
        {
            f.cleanup_until_drained();
            assert!(!tc.dir.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_symlinked_managed_prefix() {
        let f = Fixture::new();
        f.init();
        f.begin();
        f.validate();
        let old = f.0.join("managed/staged/jr-aaaaaaa");
        let foreign = f.0.join("foreign");
        std::fs::rename(&old, &foreign).unwrap();
        std::os::unix::fs::symlink(&foreign, &old).unwrap();
        assert!(f.run(Command::Cleanup).is_err());
        assert!(foreign.exists());
    }

    #[cfg(unix)]
    #[test]
    fn inherited_lease_survives_parent_descriptor_close() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command as Process, Stdio};
        let f = Fixture::new();
        let path = usage_path(&f.0, "jr-aaaaaaa");
        let file = open_lock(&path).unwrap();
        file.lock_shared().unwrap();
        let lease = Lease { _file: file };
        // The child blocks on stdin after proving exec completed, with no sleep race.
        let mut cmd = Process::new("sh");
        cmd.args(["-c", "echo ready; read line"]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        lease.inherit(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n");
        drop(lease);
        let exclusive = open_lock(&path).unwrap();
        assert!(exclusive.try_lock().is_err());
        drop(child.stdin.take());
        child.wait().unwrap();
        exclusive.try_lock().unwrap();
    }
}
