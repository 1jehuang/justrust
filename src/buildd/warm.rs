//! Keeping builds warm across the two changes no file watcher reports.
//!
//! - **The default toolchain changed** ([`crate::toolchain_policy`]: a ship
//!   or a rollback). Every unit of a workspace is then stale, and the first
//!   request paid for all of them (111 hosted runs in 7 days carried "the
//!   toolchain changed"). The daemon asks for the policy twice a minute
//!   while it remembers a request, and on a change queues the remembered
//!   requests that follow the policy as prebuilds: one at a time, only while
//!   no agent build is in flight, at the lowest priority, under the memory
//!   governor, and at most once per request.
//! - **The daemon was replaced** (a client update runs a daemon of its own
//!   build, an idle daemon exited). What to prebuild lived in memory only.
//!   It is now written to `~/.justrust/buildd/recent-<build>.json`, and a
//!   starting daemon takes over every such file. A daemon whose executable
//!   was replaced starts its successor at once instead of waiting for the
//!   next request to do so.
//!
//! The file only says what to prebuild. Keys, source roots and freshness
//! are computed again from what is on disk, so a wrong or forged file costs
//! at most a wasted guess. It holds request environments: mode 0600.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// A queued prebuild is dropped after this long without a quiet moment.
const MAX_WAIT: Duration = Duration::from_secs(900);
const TICK: Duration = Duration::from_secs(2);
const POLICY_EVERY: Duration = Duration::from_secs(30);
/// After a release, how long to assume its build is still about to start.
const START_GRACE: Duration = Duration::from_secs(5);
const MAX_FILE: u64 = 4 << 20;
/// Value of [`GUESS_ENV`] on these prebuilds: nobody is about to ask.
const GUESS_WARM: &str = "warm";

struct Due {
    key: String,
    since: Instant,
    /// A toolchain the request names: wait until it is installed here.
    needs: Option<String>,
}

#[derive(Default)]
pub(super) struct Warm {
    // Lock order: `Daemon::state` first, then these.
    due: Mutex<Vec<Due>>,
    ready: Mutex<Vec<String>>,
    running: Mutex<Option<(String, Instant)>>,
    /// Something other than an agent's request changed what is remembered.
    dirty: AtomicBool,
    wake: Mutex<Option<std::sync::mpsc::Sender<PathBuf>>>,
}

impl Warm {
    pub(super) fn new(wake: std::sync::mpsc::Sender<PathBuf>) -> Self {
        Warm {
            wake: Mutex::new(Some(wake)),
            ..Default::default()
        }
    }

    /// Keys the prebuild loop should start now, whatever their sources.
    pub(super) fn take_ready(&self) -> Vec<String> {
        std::mem::take(&mut *self.ready.lock().unwrap())
    }

    fn wake(&self) {
        if let Some(tx) = &*self.wake.lock().unwrap() {
            // No root contains the empty path: it only ends the wait.
            let _ = tx.send(PathBuf::new());
        }
    }
}

/// Niceness of a prebuild: a guess after an edit is below agent builds, a
/// prebuild nobody is about to ask for is below those too.
pub(super) fn niceness(req: &Request) -> i32 {
    let warm = req
        .env
        .iter()
        .any(|(k, v)| k == GUESS_ENV && v == GUESS_WARM);
    if warm { 19 } else { 10 }
}

/// Mark a prebuild released by [`Warm::take_ready`].
pub(super) fn mark(req: &mut Request) {
    for (k, v) in req.env.iter_mut() {
        if k == GUESS_ENV {
            *v = GUESS_WARM.into();
        }
    }
    if cfg!(target_os = "linux") && !req.env.iter().any(|(k, _)| k == crate::memgov::ENV) {
        req.env.push((crate::memgov::ENV.into(), "1".into()));
    }
}

// ------------------------------------------------------------- persistence

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    /// The policy toolchain when this was written.
    toolchain: Option<String>,
    entries: Vec<SavedRecent>,
}

#[derive(Serialize, Deserialize)]
struct SavedRecent {
    req: Request,
    /// Unix time of the agent's last request for it.
    used_at: f64,
    budget: u8,
}

fn file_in(d: &Path, stem: &str) -> PathBuf {
    d.join(format!("recent-{stem}.json"))
}

fn snapshot(recent: &HashMap<String, Recent>, toolchain: Option<String>, now: f64) -> Saved {
    Saved {
        version: 1,
        toolchain,
        entries: recent
            .values()
            .map(|r| SavedRecent {
                req: r.req.clone(),
                used_at: now - r.used.elapsed().as_secs_f64(),
                budget: r.budget,
            })
            .collect(),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let r = o
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn save(dm: &Daemon, toolchain: Option<String>) {
    let saved = snapshot(
        &dm.state.lock().unwrap().recent,
        toolchain,
        crate::paths::now(),
    );
    let Ok(d) = dir() else { return };
    if let Ok(bytes) = serde_json::to_vec(&saved) {
        let _ = write_private(&file_in(&d, &stem()), &bytes);
    }
}

/// One remembered request from a file, validated against the disk.
struct Inherited {
    req: Request,
    age: Duration,
    used_at: f64,
    budget: u8,
    /// The policy toolchain its daemon last knew.
    toolchain: Option<String>,
}

/// Every still valid entry of every `recent-*.json` in `d`, newest first.
/// Files nobody refreshed within the maximum age are removed.
fn load(d: &Path, now: f64) -> Vec<Inherited> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(d) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("recent-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let stale = md
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|a| a >= RECENT_MAX_AGE);
        if stale {
            let _ = std::fs::remove_file(e.path());
            continue;
        }
        if md.len() > MAX_FILE {
            continue;
        }
        let Some(saved) = std::fs::read(e.path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
        else {
            continue;
        };
        if saved.version != 1 {
            continue;
        }
        for s in saved.entries {
            let age = now - s.used_at;
            if !(0.0..RECENT_MAX_AGE.as_secs_f64()).contains(&age)
                || s.req.prebuild
                || s.req.argv.is_empty()
                || !wants_prebuild(&s.req)
                || !Path::new(&s.req.cwd).is_absolute()
                || !Path::new(&s.req.cwd).is_dir()
            {
                continue;
            }
            out.push(Inherited {
                req: s.req,
                age: Duration::from_secs_f64(age),
                used_at: s.used_at,
                budget: s.budget.min(PREBUILDS_PER_REQUEST),
                toolchain: saved.toolchain.clone(),
            });
        }
    }
    out.sort_by_key(|r| r.age);
    out
}

/// A source file under `roots` is newer than `since` (unix seconds): it was
/// edited while no daemon watched. Skips what the watcher skips.
fn edited_since(roots: &[PathBuf], since: f64) -> bool {
    let newer = |md: &std::fs::Metadata| {
        md.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|t| t.as_secs_f64() > since)
    };
    let mut stack = roots.to_vec();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if n != "target" && !n.starts_with("target-") && !n.starts_with('.') {
                    stack.push(e.path());
                }
            } else if !n.starts_with('.') && e.metadata().is_ok_and(|m| newer(&m)) {
                return true;
            }
        }
    }
    false
}

/// Take over what earlier daemons remembered. Requests an agent already
/// made of this daemon win.
fn inherit(dm: &Daemon, policy: Option<&str>) {
    let Ok(d) = dir() else { return };
    let build = crate::remote_daemon::build_id();
    let mut taken = 0;
    let mut due = Vec::new();
    for mut i in load(&d, crate::paths::now()) {
        i.req.build = build.clone();
        let cwd = PathBuf::from(&i.req.cwd);
        let roots = source_roots(&cwd);
        let Some(used) = Instant::now().checked_sub(i.age) else {
            continue;
        };
        let key = request_key(&i.req);
        {
            let mut st = dm.state.lock().unwrap();
            if st.recent.contains_key(&key) {
                continue;
            }
            st.recent.insert(
                key.clone(),
                Recent {
                    req: i.req,
                    roots: roots.clone(),
                    used,
                    seen: MIN_REQUESTS_TO_PREBUILD,
                    budget: i.budget,
                },
            );
            prune_recent(&mut st.recent);
            if !st.recent.contains_key(&key) {
                continue;
            }
            taken += 1;
            // The toolchain changed while no daemon looked.
            if let (Some(old), Some(new)) = (i.toolchain.as_deref(), policy)
                && old != new
            {
                due.extend(retarget_one(&mut st, &key, old, new, &follows_policy));
                continue;
            }
        }
        watch_roots(dm, &roots);
        if i.budget > 0 && edited_since(&roots, i.used_at) {
            due.push(Due {
                key,
                since: Instant::now(),
                needs: None,
            });
        }
    }
    if taken > 0 {
        log(&format!(
            "took over {taken} remembered requests, {} to prebuild now",
            due.len()
        ));
    }
    // After a rename the roots are watched under the new key's entry.
    let roots: Vec<Vec<PathBuf>> = {
        let st = dm.state.lock().unwrap();
        st.recent.values().map(|r| r.roots.clone()).collect()
    };
    for r in roots {
        watch_roots(dm, &r);
    }
    dm.warm.due.lock().unwrap().extend(due);
}

// ---------------------------------------------------------------- toolchain

/// No project file or daemon variable pins the toolchain of builds in `cwd`.
fn follows_policy(cwd: &Path) -> bool {
    !crate::toolchain::project_pinned(cwd) && matches!(crate::toolchain::resolve(cwd), Ok(None))
}

/// The remembered request `key` after the policy toolchain went from `old`
/// to `new`, when it is affected. A request the hosted agent stamped with
/// the old toolchain is renamed to the new one, as the agent will send it
/// from now on. A request naming any other toolchain is left alone.
fn retarget_one(
    st: &mut State,
    key: &str,
    old: &str,
    new: &str,
    follows: &dyn Fn(&Path) -> bool,
) -> Option<Due> {
    let r = st.recent.get(key)?;
    let named: Vec<&(String, String)> = r
        .req
        .env
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "JUSTRUST_TOOLCHAIN" | "RUSTUP_TOOLCHAIN"))
        .collect();
    let (key, needs) = match named.as_slice() {
        [] if follows(Path::new(&r.req.cwd)) => (key.to_owned(), None),
        [(k, v)] if k == "JUSTRUST_TOOLCHAIN" && v == old => {
            let mut r = st.recent.remove(key)?;
            for (k, v) in r.req.env.iter_mut() {
                if k == "JUSTRUST_TOOLCHAIN" {
                    *v = new.to_owned();
                }
            }
            let nk = request_key(&r.req);
            st.recent.insert(nk.clone(), r);
            (nk, Some(new.to_owned()))
        }
        _ => return None,
    };
    // A finished prebuild by the old compiler is no answer any more.
    st.finished.remove(&key);
    if let Some(r) = st.recent.get_mut(&key) {
        // One prebuild per change, also when edits used the budget up.
        r.budget = r.budget.max(1);
    }
    Some(Due {
        key,
        since: Instant::now(),
        needs,
    })
}

fn retarget(st: &mut State, old: &str, new: &str, follows: &dyn Fn(&Path) -> bool) -> Vec<Due> {
    let keys: Vec<String> = st.recent.keys().cloned().collect();
    keys.iter()
        .filter_map(|k| retarget_one(st, k, old, new, follows))
        .collect()
}

/// The policy toolchain, when this justrust can drive it.
fn policy_toolchain() -> Option<String> {
    crate::toolchain_policy::known()
        .filter(|p| p.check().is_ok())
        .map(|p| p.toolchain)
}

fn installed(id: &str) -> bool {
    crate::toolchain::Spec::parse(id)
        .ok()
        .and_then(|s| crate::toolchain::available_for_build(&s))
        .is_some()
}

/// Hand the next queued prebuild to the prebuild loop when nothing an agent
/// waits for is running and the previous one is done.
fn release(dm: &Daemon, installed: &dyn Fn(&str) -> bool) -> bool {
    let st = dm.state.lock().unwrap();
    let mut due = dm.warm.due.lock().unwrap();
    // An agent that asked since then is building it for real.
    due.retain(|d| {
        d.since.elapsed() < MAX_WAIT && st.recent.get(&d.key).is_some_and(|r| r.used <= d.since)
    });
    if due.is_empty() {
        return false;
    }
    let ready = dm.warm.ready.lock().unwrap().len();
    let mut running = dm.warm.running.lock().unwrap();
    if let Some((k, at)) = &*running
        && (ready > 0 || at.elapsed() < START_GRACE || st.builds.contains_key(k))
    {
        return false;
    }
    *running = None;
    if st.builds.values().any(|b| !*b.prebuild.lock().unwrap()) {
        return false;
    }
    let Some(i) = due
        .iter()
        .position(|d| d.needs.as_deref().is_none_or(installed))
    else {
        return false;
    };
    let d = due.remove(i);
    *running = Some((d.key.clone(), Instant::now()));
    dm.warm.ready.lock().unwrap().push(d.key);
    true
}

/// What is remembered and when it was last asked for, to notice a change.
fn remembered(dm: &Daemon) -> Vec<(String, Instant)> {
    let st = dm.state.lock().unwrap();
    let mut v: Vec<_> = st.recent.iter().map(|(k, r)| (k.clone(), r.used)).collect();
    v.sort();
    v
}

/// The daemon's housekeeping thread.
pub(super) fn run(dm: &Arc<Daemon>) {
    let mut policy = policy_toolchain();
    inherit(dm, policy.as_deref());
    let mut asked = Instant::now();
    let mut saved = Vec::new();
    loop {
        if release(dm, &installed) {
            dm.warm.wake();
        }
        std::thread::sleep(TICK);
        if dm.draining.load(Ordering::SeqCst) {
            return;
        }
        let remembers = !dm.state.lock().unwrap().recent.is_empty();
        if remembers && asked.elapsed() >= POLICY_EVERY {
            asked = Instant::now();
            let now = policy_toolchain();
            if let (Some(old), Some(new)) = (&policy, &now)
                && old != new
            {
                let due = retarget(&mut dm.state.lock().unwrap(), old, new, &follows_policy);
                log(&format!(
                    "default toolchain changed from {old} to {new}: {} remembered requests to prebuild",
                    due.len()
                ));
                dm.warm.due.lock().unwrap().extend(due);
                dm.warm.dirty.store(true, Ordering::Relaxed);
            }
            if now.is_some() {
                policy = now;
            }
        }
        let now_remembered = remembered(dm);
        if dm.warm.dirty.swap(false, Ordering::Relaxed) || now_remembered != saved {
            save(dm, policy.clone());
            saved = now_remembered;
        }
        // A client update replaced the executable: the next request would
        // start the new daemon, and until then nobody prebuilds. Start it
        // now, it takes over what was just saved.
        if remembers && dm.exe_replaced() && live_exe().is_ok_and(|p| p.is_file()) {
            save(dm, policy.clone());
            // The socket goes first: the successor's is its own.
            drain_and_exit(dm, "binary replaced on disk, starting its successor");
            if let Err(e) = spawn() {
                log(&format!("successor build daemon: {e:#}"));
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jr-warm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn req(cwd: &Path, env: &[(&str, &str)], argv: &[&str]) -> Request {
        Request {
            build: "sha256-old".into(),
            cwd: cwd.to_string_lossy().into_owned(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            prebuild: false,
        }
    }

    fn remember(st: &mut State, r: Request) -> String {
        let key = request_key(&r);
        st.recent.insert(
            key.clone(),
            Recent {
                req: r,
                roots: Vec::new(),
                used: Instant::now(),
                seen: MIN_REQUESTS_TO_PREBUILD,
                budget: 0,
            },
        );
        key
    }

    #[test]
    fn a_new_daemon_takes_over_what_another_remembered() {
        let d = tmp("load");
        let ws = d.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let now = 1_000_000.0;
        let entry = |cwd: &Path, used_at: f64, env: &[(&str, &str)]| SavedRecent {
            req: req(cwd, env, &["check"]),
            used_at,
            budget: 200,
        };
        let saved = Saved {
            version: 1,
            toolchain: Some("jr-aaaaaaa".into()),
            entries: vec![
                entry(&ws, now - 60.0, &[]),
                // Too old, from the future, deleted, relative, opted out.
                entry(&ws, now - RECENT_MAX_AGE.as_secs_f64() - 1.0, &[]),
                entry(&ws, now + 60.0, &[]),
                entry(&d.join("gone"), now - 60.0, &[]),
                entry(Path::new("ws"), now - 60.0, &[]),
                entry(&ws, now - 60.0, &[("JUSTRUST_BUILDD_PREBUILD", "0")]),
            ],
        };
        let path = file_in(&d, "0123456789abcdef");
        write_private(&path, &serde_json::to_vec(&saved).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "it holds request environments");
        }
        // Fail open on anything unreadable.
        std::fs::write(file_in(&d, "garbage"), b"{not json").unwrap();
        std::fs::write(file_in(&d, "future"), br#"{"version":2,"entries":[]}"#).unwrap();
        let got = load(&d, now);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].req.cwd, ws.to_string_lossy());
        assert_eq!(got[0].budget, PREBUILDS_PER_REQUEST);
        assert_eq!(got[0].toolchain.as_deref(), Some("jr-aaaaaaa"));
        assert_eq!(got[0].age, Duration::from_secs(60));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn what_is_remembered_round_trips_through_the_file() {
        let mut st = State::default();
        let key = remember(
            &mut st,
            req(Path::new("/w"), &[("A", "1")], &["test", "--lib"]),
        );
        let saved = snapshot(&st.recent, None, 500.0);
        let back: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(request_key(&back.entries[0].req), key);
        assert!((back.entries[0].used_at - 500.0).abs() < 5.0);
    }

    #[test]
    fn an_edit_nobody_watched_is_noticed() {
        let d = tmp("edited");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("target")).unwrap();
        std::fs::write(d.join("target/out"), b"x").unwrap();
        let future = crate::paths::now() + 3600.0;
        std::fs::write(d.join("src/lib.rs"), b"x").unwrap();
        assert!(edited_since(std::slice::from_ref(&d), 0.0));
        assert!(!edited_since(std::slice::from_ref(&d), future));
        // Build output is not source.
        std::fs::remove_file(d.join("src/lib.rs")).unwrap();
        assert!(!edited_since(std::slice::from_ref(&d), 0.0));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_toolchain_change_queues_only_requests_that_follow_the_policy() {
        let (old, new) = ("jr-aaaaaaa", "jr-bbbbbbb");
        let mut st = State::default();
        // As the hosted agent sends it, as a laptop sends it, pinned by the
        // caller, pinned by the project.
        let agent = remember(
            &mut st,
            req(Path::new("/a"), &[("JUSTRUST_TOOLCHAIN", old)], &["check"]),
        );
        let plain = remember(&mut st, req(Path::new("/b"), &[], &["check"]));
        let other = remember(
            &mut st,
            req(
                Path::new("/c"),
                &[("JUSTRUST_TOOLCHAIN", "nightly")],
                &["check"],
            ),
        );
        let pinned = remember(&mut st, req(Path::new("/pinned"), &[], &["check"]));
        let follows = |cwd: &Path| cwd != Path::new("/pinned");
        let due = retarget(&mut st, old, new, &follows);
        let renamed = request_key(&req(
            Path::new("/a"),
            &[("JUSTRUST_TOOLCHAIN", new)],
            &["check"],
        ));
        let mut keys: Vec<(&str, Option<&str>)> = due
            .iter()
            .map(|d| (d.key.as_str(), d.needs.as_deref()))
            .collect();
        keys.sort();
        let mut want = vec![(renamed.as_str(), Some(new)), (plain.as_str(), None)];
        want.sort();
        assert_eq!(keys, want);
        // The agent's next request has the new key: it attaches.
        assert!(!st.recent.contains_key(&agent));
        assert_eq!(st.recent[&renamed].budget, 1);
        assert_eq!(st.recent[&plain].budget, 1);
        assert_eq!(st.recent[&other].budget, 0);
        assert_eq!(st.recent[&pinned].budget, 0);
    }

    fn daemon() -> Daemon {
        Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: AtomicBool::new(false),
            warm: Default::default(),
        }
    }

    fn build(prebuild: bool) -> Arc<Build> {
        Arc::new(Build {
            stamp: None,
            compile_only: Mutex::default(),
            units_key: Mutex::default(),
            session: String::new(),
            cwd: String::new(),
            run_id: None,
            started: Instant::now(),
            finished: Mutex::default(),
            pid: 0,
            prebuild: Mutex::new(prebuild),
            stopped: Default::default(),
            claim: Mutex::default(),
            log: Mutex::default(),
            cv: Condvar::new(),
        })
    }

    #[test]
    fn queued_prebuilds_wait_for_agent_builds_and_for_each_other() {
        let dm = daemon();
        let mut keys = Vec::new();
        for (i, cmd) in ["check", "clippy", "build"].iter().enumerate() {
            let mut st = dm.state.lock().unwrap();
            let k = remember(&mut st, req(Path::new("/w"), &[], &[cmd]));
            dm.warm.due.lock().unwrap().push(Due {
                key: k.clone(),
                since: Instant::now(),
                // The first names a compiler that is not installed yet.
                needs: (i == 0).then(|| "jr-bbbbbbb".to_owned()),
            });
            keys.push(k);
        }
        let none = |_: &str| false;
        // An agent waits for a build: nothing starts.
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("agent".into(), build(false));
        assert!(!release(&dm, &none));
        // An edit's prebuild does not hold them back.
        dm.state.lock().unwrap().builds.clear();
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("guess".into(), build(true));
        assert!(release(&dm, &none));
        assert_eq!(dm.warm.take_ready(), [keys[1].clone()]);
        // One at a time: not while the first is about to start or running.
        assert!(!release(&dm, &none));
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert(keys[1].clone(), build(true));
        dm.warm.running.lock().unwrap().as_mut().unwrap().1 = Instant::now() - START_GRACE;
        assert!(!release(&dm, &none));
        dm.state.lock().unwrap().builds.remove(&keys[1]);
        // The agent asked for the third itself meanwhile.
        dm.state
            .lock()
            .unwrap()
            .recent
            .get_mut(&keys[2])
            .unwrap()
            .used = Instant::now();
        assert!(!release(&dm, &none));
        assert_eq!(dm.warm.due.lock().unwrap().len(), 1);
        // The compiler arrived.
        assert!(release(&dm, &|id| id == "jr-bbbbbbb"));
        assert_eq!(dm.warm.take_ready(), [keys[0].clone()]);
    }

    #[test]
    fn warm_prebuilds_run_below_guesses_and_under_the_memory_governor() {
        let mut r = req(Path::new("/w"), &[(GUESS_ENV, "1")], &["check"]);
        assert_eq!(niceness(&r), 10);
        mark(&mut r);
        assert_eq!(niceness(&r), 19);
        assert_eq!(
            r.env
                .iter()
                .any(|(k, v)| k == crate::memgov::ENV && v == "1"),
            cfg!(target_os = "linux")
        );
    }
}
