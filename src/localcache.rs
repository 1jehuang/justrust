//! Content-addressed cache for the project's own (workspace and path) crates.
//!
//! The dependency cache (`depcache.rs`) only takes registry and git units.
//! Local crates are rebuilt whenever cargo sees a changed mtime or a new
//! target dir, even when the exact same sources were compiled minutes ago:
//! reverting an edit, switching a branch back, a second build slot, or a
//! fresh target dir of the same checkout. This cache keys local units the
//! same way depcache does (every argument with the profile dir normalized,
//! `--extern`s by crate metadata, then every dep-info source and tracked env
//! var by content) and restores their outputs by reflink.
//!
//! ```text
//! ~/.justrust/cache/localcache/
//!   m/<base-key>.jsonl     manifest lines, as in depcache
//!   o/<result-key>/        outputs + meta.json
//!   s/<result-key>.<pid>/  staged by the shim, committed by the finisher
//! ```
//!
//! Differences from depcache, and why:
//! - **No extra rustc arguments.** Dependencies are compiled with the target
//!   dir and cargo home remapped. A local unit is compiled exactly as cargo
//!   asked, so recording never changes its output, its incremental state, or
//!   cargo's fingerprints. Consequence: outputs carry the checkout's absolute
//!   path (debuginfo `comp_dir`, `CARGO_MANIFEST_DIR`), so the key includes
//!   the working dir and a checkout at another path (a new `git worktree`)
//!   does not hit. Outputs naming the target dir as data are stored in a
//!   separate opt-in tier (`JUSTRUST_LOCALCACHE_TARGET_BOUND=1`) keyed by
//!   the exact target and profile paths. They can
//!   hit on a revert in that same directory, never in another build slot.
//! - **`-C incremental` accepted.** A hit leaves the incremental dir as it
//!   was. rustc validates incremental state by content hashes, so the next
//!   real compile is correct, it only reuses less if the state is older.
//! - **Stored off the critical path.** A local miss is the agent's edit
//!   loop, and its test binary is hundreds of MB (Desktop UI: 285 MB): hashing
//!   and scanning it costs ~0.5 s. The shim only hashes the inputs and
//!   reflinks the outputs into `s/` (milliseconds), then a detached, niced
//!   `justrust __localcache-finish` hashes, checks, commits, and collects.
//! - **Off with hot patching** (`JUSTRUST_HOTPATCH=1`): the patcher tracks
//!   the binaries it linked.
//!
//! On by default, `JUSTRUST_LOCALCACHE=0` turns it off, cache dir
//! `JUSTRUST_LOCALCACHE_DIR`, size cap `JUSTRUST_LOCALCACHE_MAX` (default
//! 10G, LRU like depcache). Every error means "compile normally".
//!
//! Accepted risk, same as depcache: inputs a proc macro or build script read
//! without telling rustc (no dep-info line, no `env!`) are not keyed.

use crate::depcache::{self, Entry, H128, LocalKey, OutFile, Outputs, Plan};
use crate::paths;
use crate::sys::Detach;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FINISH_ARG: &str = "__localcache-finish";
const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const GC_INTERVAL_SECS: f64 = 600.0;

pub fn enabled() -> bool {
    if !depcache::enabled() || crate::hotpatch::enabled() {
        return false;
    }
    match std::env::var("JUSTRUST_LOCALCACHE").as_deref() {
        Ok("1") | Ok("on") | Ok("true") | Ok("yes") => true,
        Ok(_) => false,
        Err(_) => DEFAULT_ON,
    }
}

/// On: revert 10.6 -> 0.7 s and a fresh target dir 66 -> 6.3 s on the
/// Desktop snapshot, loop overhead within noise (FINDINGS 50).
const DEFAULT_ON: bool = true;

pub fn cache_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("JUSTRUST_LOCALCACHE_DIR") {
        return Some(PathBuf::from(d));
    }
    paths::home().ok().map(|h| h.join("cache/localcache"))
}

/// Key a local unit. `salt`: arguments the shim adds to the compile
/// (linker selection), which change the output but are not in `args`.
pub fn plan(
    real: &Path,
    args: &[OsString],
    env: &[(String, String)],
    salt: Vec<String>,
) -> Option<Plan> {
    let root = cache_dir()?;
    depcache::plan_with(
        real,
        args,
        env,
        Some(LocalKey {
            root,
            salt,
            cwd: None,
        }),
    )
}

/// Prefer portable artifacts, then artifacts valid only at this exact target
/// and profile path. Keep both manifest and result namespaces distinct from
/// the portable cache so older clients cannot restore a target-bound entry.
pub fn restore(plan: &Plan, err: &mut dyn Write) -> Option<depcache::Hit> {
    restore_with(plan, err, target_bound_enabled())
}

fn target_bound_enabled() -> bool {
    matches!(
        std::env::var("JUSTRUST_LOCALCACHE_TARGET_BOUND").as_deref(),
        Ok("1" | "on" | "true" | "yes")
    )
}

fn restore_with(plan: &Plan, err: &mut dyn Write, target_bound: bool) -> Option<depcache::Hit> {
    plan.restore(err).or_else(|| {
        target_bound
            .then(|| target_bound_plan(plan).restore(err))
            .flatten()
    })
}

fn target_bound_key(key: &str, norm: &depcache::Norm) -> String {
    let mut h = H128::new();
    h.field(b"local-target-bound-v1");
    h.field(key.as_bytes());
    h.field(norm.target.as_bytes());
    h.field(norm.profile.as_bytes());
    h.hex()
}

fn target_bound_plan(plan: &Plan) -> Plan {
    let mut scoped = plan.clone();
    scoped.base = target_bound_key(&plan.base, &plan.norm);
    scoped
}

/// What the shim hands the finisher.
#[derive(Serialize, Deserialize)]
struct Staged {
    entry: Entry,
    /// Staged file names; `.d` files are already normalized.
    files: Vec<String>,
    stderr: Vec<String>,
    wall: f64,
    /// This build's target dir, and input files under it (allowed to be
    /// named in outputs, see `depcache::target_refs_are_inputs`).
    target: String,
    target_inputs: Vec<String>,
    manifest: PathBuf,
    /// Exact-target fallback. Absent in stages made by older binaries, which
    /// must still refuse nonportable output instead of publishing it globally.
    #[serde(default)]
    target_bound: Option<(PathBuf, String)>,
}

/// After a successful compile: key the inputs, reflink the outputs into a
/// staging dir, and leave the rest to a background finisher. Returns the
/// result key.
pub fn stage(plan: &Plan, start: f64, wall: f64, stderr: &[Vec<u8>]) -> Option<String> {
    stage_with(plan, start, wall, stderr, false, target_bound_enabled())
}

/// `stage`; `sync` commits in this process instead of a finisher (tests).
fn stage_with(
    plan: &Plan,
    start: f64,
    wall: f64,
    stderr: &[Vec<u8>],
    sync: bool,
    target_bound: bool,
) -> Option<String> {
    let dep_info = plan
        .out_dir
        .join(format!("{}{}.d", plan.crate_name, plan.extra));
    let d = std::fs::read_to_string(&dep_info).ok()?;
    let (inputs, env_names) = depcache::parse_dep_info(&d);
    let mut entry = Entry {
        inputs: Vec::new(),
        env: Vec::new(),
        out: String::new(),
    };
    let mut h = H128::new();
    h.field(plan.base.as_bytes());
    for p in inputs {
        let abs = plan.cwd.join(&p);
        let hash = depcache::file_hash(&abs, false)?;
        let np = plan.norm.norm(abs.to_str()?);
        h.field(np.as_bytes());
        h.field(hash.as_bytes());
        entry.inputs.push((np, hash));
    }
    for k in env_names {
        let v = std::env::var(&k).ok().map(|v| plan.norm.norm(&v));
        h.field(k.as_bytes());
        h.field(v.as_deref().unwrap_or("\0unset").as_bytes());
        entry.env.push((k, v));
    }
    entry.out = h.hex();
    let root = &plan.root;
    let final_dir = root.join("o").join(&entry.out);
    let manifest = plan.manifest();
    if final_dir.join("meta.json").is_file() {
        // Same result already stored (two slots, or a rebuild after a
        // revert with the cache off): make sure the line is there.
        depcache::touch(&final_dir.join("meta.json"));
        let line = serde_json::to_string(&entry).ok()?;
        append_once(&manifest, &line)?;
        return Some(entry.out);
    }
    let scoped = target_bound.then(|| {
        (
            target_bound_plan(plan).manifest(),
            target_bound_key(&entry.out, &plan.norm),
        )
    });
    if let Some((manifest, out)) = &scoped
        && root.join("o").join(out).join("meta.json").is_file()
    {
        let mut scoped_entry = entry.clone();
        scoped_entry.out = out.clone();
        depcache::touch(&root.join("o").join(out).join("meta.json"));
        append_once(manifest, &serde_json::to_string(&scoped_entry).ok()?)?;
        return Some(scoped_entry.out);
    }
    if volatile(&entry) {
        return None;
    }

    let mut stderr_lines = Vec::new();
    let mut artifacts = Vec::new();
    for l in stderr {
        let s = String::from_utf8_lossy(l);
        let s = s.trim_end_matches(['\n', '\r']);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
            && v.get("$message_type").and_then(|m| m.as_str()) == Some("artifact")
            && let Some(a) = v.get("artifact").and_then(|a| a.as_str())
        {
            artifacts.push(plan.cwd.join(a));
        }
        stderr_lines.push(plan.norm.norm(s));
    }
    let mut files = Vec::new();
    for e in std::fs::read_dir(&plan.out_dir).ok()?.flatten() {
        let name = e.file_name().to_str()?.to_owned();
        if name.starts_with('.') || !name.contains(&plan.extra) {
            continue;
        }
        let m = e.metadata().ok()?;
        let mtime = crate::sys::mtime_f64(&m);
        if m.is_file() && mtime + 1.0 >= start {
            files.push(name);
        }
    }
    files.sort();
    for a in &artifacts {
        let name = a.file_name()?.to_str()?;
        if a.parent() != Some(plan.out_dir.as_path()) || !files.iter().any(|f| f == name) {
            return None;
        }
    }

    let dir = root
        .join("s")
        .join(format!("{}.{}", entry.out, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let ok = files.iter().all(|name| {
        let src = plan.out_dir.join(name);
        if name.ends_with(".d") {
            std::fs::read_to_string(&src)
                .ok()
                .and_then(|t| std::fs::write(dir.join(name), plan.norm.norm(&t)).ok())
                .is_some()
        } else {
            // A hard link, not a reflink: btrfs flushes a file's dirty
            // pages before cloning it, ~170 ms for the fresh 285 MB Desktop
            // test binary. The finisher copies it out and checks it did not
            // change meanwhile.
            std::fs::hard_link(&src, dir.join(name)).is_ok()
                && std::fs::metadata(&src)
                    .ok()
                    .and_then(|m| std::fs::write(dir.join(format!("{name}.stamp")), stamp(&m)).ok())
                    .is_some()
        }
    });
    let target_inputs = entry
        .inputs
        .iter()
        .map(|(p, _)| plan.norm.denorm(p))
        .filter(|p| p.starts_with(&plan.norm.target))
        .collect();
    let staged = Staged {
        entry,
        files,
        stderr: stderr_lines,
        wall,
        target: plan.norm.target.clone(),
        target_inputs,
        manifest,
        target_bound: scoped,
    };
    let out = staged.entry.out.clone();
    if !ok
        || serde_json::to_vec(&staged)
            .ok()
            .and_then(|b| std::fs::write(dir.join("stage.json"), b).ok())
            .is_none()
        || !(if sync {
            finish(&dir).is_some()
        } else {
            spawn_finisher(&dir)
        })
    {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    Some(out)
}

/// A tracked env var contains a fresh build-time stamp, so this result is
/// unlikely to be reused. Other env changes must remain cacheable: even
/// several distinct values can be a finite set of stable build inputs.
fn volatile(entry: &Entry) -> bool {
    entry.env.iter().any(|(_, v)| {
        v.as_deref()
            .is_some_and(|v| is_fresh_stamp(v, paths::now()))
    })
}

/// A Unix time in seconds or milliseconds from the last ten minutes: a
/// build script stamped the build time into the crate (`env!`), so this
/// result is never seen again. A stable value (`SOURCE_DATE_EPOCH`, a build
/// script that ran long ago) is older and still stored.
fn is_fresh_stamp(v: &str, now: f64) -> bool {
    let Ok(n) = v.trim().parse::<u64>() else {
        return false;
    };
    let secs = if n > 100_000_000_000 {
        n as f64 / 1000.0
    } else {
        n as f64
    };
    (now - secs).abs() < 600.0
}

fn spawn_finisher(dir: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // Null stdio: cargo reads the shim's stderr until EOF, so the finisher
    // must not hold it.
    std::process::Command::new(exe)
        .arg(FINISH_ARG)
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .detached()
        .spawn()
        .is_ok()
}

fn append_once(manifest: &Path, line: &str) -> Option<()> {
    std::fs::create_dir_all(manifest.parent()?).ok()?;
    let known = std::fs::read_to_string(manifest).unwrap_or_default();
    if !known.lines().any(|l| l == line) {
        paths::append_line(manifest, line).ok()?;
    }
    Some(())
}

/// `justrust __localcache-finish <staged dir>`: background commit.
pub fn finish_main(dir: PathBuf) -> ! {
    crate::sys::lower_priority();
    // IOPRIO_WHO_PROCESS, class idle.
    // SAFETY: plain syscall on the current process.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
    let t0 = paths::now();
    let outcome = finish(&dir);
    let ok = outcome.is_some();
    if !ok {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if let Some(root) = dir.parent().and_then(Path::parent) {
        let cpu = crate::sys::self_cpu();
        let line = serde_json::json!({
            "t": t0, "secs": paths::now() - t0, "cpu": cpu, "ok": ok,
            "target_bound": outcome,
        });
        let _ = paths::append_line(&root.join("finish.jsonl"), &line.to_string());
        maybe_gc(root);
    }
    std::process::exit(0);
}

/// Hash and check the staged outputs, then publish them.
fn finish(dir: &Path) -> Option<bool> {
    let mut staged: Staged =
        serde_json::from_slice(&std::fs::read(dir.join("stage.json")).ok()?).ok()?;
    let mut outs = Outputs {
        files: Vec::new(),
        stderr: staged.stderr,
        wall: staged.wall,
        glibc: None,
    };
    let mut target_bound = false;
    for name in &staged.files {
        let p = dir.join(name);
        let text = name.ends_with(".d");
        if !text && !unlink_copy(&p)? {
            return None;
        }
        if !text {
            let bytes = std::fs::read(&p).ok()?;
            if !depcache::target_refs_are_inputs(&bytes, &staged.target, &staged.target_inputs) {
                // Do not weaken the portable guard. Runtime paths such as
                // CARGO_BIN_EXE remain valid only in their original target.
                staged.target_bound.as_ref()?;
                target_bound = true;
            }
            if !name.ends_with(".rlib") && !name.ends_with(".rmeta") {
                let need = depcache::needed_glibc(&p)?;
                if need > (0, 0) {
                    outs.glibc = Some(outs.glibc.map_or(need, |g| g.max(need)));
                }
            }
        }
        outs.files.push(OutFile {
            name: name.clone(),
            hash: depcache::file_hash(&p, false)?,
            text,
        });
    }
    if target_bound {
        let (manifest, out) = staged.target_bound.take()?;
        staged.manifest = manifest;
        staged.entry.out = out;
    }
    let _ = std::fs::remove_file(dir.join("stage.json"));
    std::fs::write(dir.join("meta.json"), serde_json::to_vec(&outs).ok()?).ok()?;
    let root = dir.parent()?.parent()?;
    std::fs::create_dir_all(root.join("o")).ok()?;
    let final_dir = root.join("o").join(&staged.entry.out);
    if std::fs::rename(dir, &final_dir).is_err() {
        // Another process committed the same result first.
        let _ = std::fs::remove_dir_all(dir);
        if !final_dir.join("meta.json").is_file() {
            return None;
        }
    }
    append_once(
        &staged.manifest,
        &serde_json::to_string(&staged.entry).ok()?,
    )?;
    Some(target_bound)
}

fn stamp(m: &std::fs::Metadata) -> String {
    let (s, ns) = crate::sys::mtime(m);
    format!("{}:{}:{s}:{ns}", crate::sys::file_id(m).1, m.len())
}

/// Replace a staged hard link with a private copy. `Some(false)` when the
/// build dir's file changed after staging (rewritten in place by a later
/// build): its content no longer matches the key.
fn unlink_copy(p: &Path) -> Option<bool> {
    let stamp_file = PathBuf::from(format!("{}.stamp", p.display()));
    let want = std::fs::read_to_string(&stamp_file).ok()?;
    let _ = std::fs::remove_file(&stamp_file);
    if stamp(&std::fs::metadata(p).ok()?) != want {
        return Some(false);
    }
    let tmp = PathBuf::from(format!("{}.copy", p.display()));
    if !depcache::reflink_or_copy(p, &tmp) {
        return None;
    }
    // Checked again after the copy: any write in between moves the mtime.
    if stamp(&std::fs::metadata(p).ok()?) != want {
        let _ = std::fs::remove_file(&tmp);
        return Some(false);
    }
    std::fs::rename(&tmp, p).ok()?;
    Some(true)
}

fn max_bytes() -> u64 {
    std::env::var("JUSTRUST_LOCALCACHE_MAX")
        .ok()
        .and_then(|v| crate::depcache_gc::parse_size(&v))
        .unwrap_or(DEFAULT_MAX_BYTES)
}

/// LRU down to the cap, at most every ten minutes, from the finisher.
fn maybe_gc(root: &Path) {
    let stamp = root.join("gc.stamp");
    if let Ok(m) = std::fs::metadata(&stamp) {
        let age = paths::now() - crate::sys::mtime_f64(&m);
        if age < GC_INTERVAL_SECS {
            return;
        }
    }
    if std::fs::write(&stamp, b"").is_err() {
        return;
    }
    // Stale stages from crashed shims or finishers.
    for e in std::fs::read_dir(root.join("s"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(m) = e.metadata()
            && paths::now() - crate::sys::mtime_f64(&m) > 3600.0
        {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let max = max_bytes();
    if max > 0 {
        crate::depcache_gc::prune(root, max, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_time_stamps_are_volatile() {
        let now = 1_791_789_578.0;
        assert!(is_fresh_stamp("1791789570", now));
        assert!(is_fresh_stamp("1791789570123", now));
        assert!(
            !is_fresh_stamp("1791700000", now),
            "SOURCE_DATE_EPOCH a day ago"
        );
        assert!(!is_fresh_stamp("0.5.1", now));
        assert!(!is_fresh_stamp("42", now));
    }

    #[test]
    fn stable_env_variants_remain_cacheable() {
        // Three or more values do not establish that an input is volatile.
        // Admission must not depend on which variants were stored first.
        let entries: Vec<Entry> = [Some("red"), Some("blue"), Some("green"), None]
            .into_iter()
            .map(|value| Entry {
                inputs: vec![("src/lib.rs".into(), "same-source-hash".into())],
                env: vec![("FLAVOR".into(), value.map(str::to_owned))],
                out: String::new(),
            })
            .collect();
        for entry in entries.iter().chain(entries.iter().rev()) {
            assert!(!volatile(entry), "stable env variant: {:?}", entry.env);
        }
    }

    #[test]
    fn fresh_seconds_and_milliseconds_env_stamps_skip_admission() {
        let now = paths::now();
        for value in [
            format!("{}", now as u64),
            format!("{}", (now * 1000.0) as u64),
        ] {
            let entry = Entry {
                inputs: vec![("src/lib.rs".into(), "same-source-hash".into())],
                env: vec![("BUILD_ID".into(), Some(value))],
                out: String::new(),
            };
            assert!(volatile(&entry), "fresh build stamp: {:?}", entry.env);
        }
        let fixed_epoch = Entry {
            inputs: vec![("src/lib.rs".into(), "same-source-hash".into())],
            env: vec![("SOURCE_DATE_EPOCH".into(), Some("1791700000".into()))],
            out: String::new(),
        };
        assert!(
            !volatile(&fixed_epoch),
            "stable old epoch remains cacheable"
        );
    }

    #[test]
    fn legacy_stages_preserve_the_portable_guard() {
        let base =
            std::env::temp_dir().join(format!("justrust-localcache-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for (name, portable, scoped) in [
            ("portable", true, false),
            ("refused", false, false),
            ("scoped", false, true),
        ] {
            let root = base.join(name);
            let dir = root.join("s/stage");
            std::fs::create_dir_all(&dir).unwrap();
            let target = root.join("target").display().to_string();
            let bytes = if portable {
                "portable metadata".to_owned()
            } else {
                format!("runtime data: {target}/debug/current")
            };
            let file = dir.join("unit.rmeta");
            std::fs::write(&file, bytes).unwrap();
            std::fs::write(
                dir.join("unit.rmeta.stamp"),
                stamp(&std::fs::metadata(&file).unwrap()),
            )
            .unwrap();
            let manifest = root.join("m/portable.jsonl");
            let scoped_manifest = root.join("m/scoped.jsonl");
            let staged = Staged {
                entry: Entry {
                    inputs: vec![],
                    env: vec![],
                    out: "portable".into(),
                },
                files: vec!["unit.rmeta".into()],
                stderr: vec![],
                wall: 1.0,
                target,
                target_inputs: vec![],
                manifest: manifest.clone(),
                target_bound: scoped.then(|| (scoped_manifest.clone(), "scoped".into())),
            };
            let mut value = serde_json::to_value(&staged).unwrap();
            if !scoped {
                value.as_object_mut().unwrap().remove("target_bound");
            }
            std::fs::write(dir.join("stage.json"), value.to_string()).unwrap();
            assert_eq!(finish(&dir), (portable || scoped).then_some(scoped));
            assert_eq!(manifest.exists(), portable);
            assert_eq!(root.join("o/portable/meta.json").exists(), portable);
            assert_eq!(scoped_manifest.exists(), scoped);
            assert_eq!(root.join("o/scoped/meta.json").exists(), scoped);
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn target_bound_outputs_reuse_only_the_exact_target_and_profile() {
        if std::env::var_os("JR_TARGET_BOUND_TEST_CHILD").is_none() {
            // Other tests fork while cache restoration has an executable open
            // for writing. Their pre-exec children can temporarily inherit the
            // fd and cause ETXTBSY. Isolate this fixture, without retrying or
            // hiding any restore or executable errors.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "localcache::tests::target_bound_outputs_reuse_only_the_exact_target_and_profile",
                    "--test-threads=1",
                    "--nocapture",
                ])
                .env("JR_TARGET_BOUND_TEST_CHILD", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let base = std::env::temp_dir().join(format!(
            "justrust-localcache-target-bound-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        let runtime_data = ws.join("t1/debug/current.txt");
        let source = format!(
            "fn main() {{ print!(\"{{}}\", std::fs::read_to_string({:?}).unwrap()); }}",
            runtime_data.to_str().unwrap()
        );
        std::fs::write(ws.join("src/main.rs"), &source).unwrap();
        let rustc = paths::find_on_path("rustc", Path::new("/nonexistent")).unwrap();
        let build = |target: &str, profile: &str| {
            let out = ws.join(target).join(profile).join("deps");
            std::fs::create_dir_all(&out).unwrap();
            let args: Vec<OsString> = [
                "--crate-name",
                "bound",
                "--edition=2021",
                "src/main.rs",
                "--crate-type",
                "bin",
                "--emit=dep-info,link",
                "-C",
                "extra-filename=-0123456789abcdef",
                "--out-dir",
                out.to_str().unwrap(),
            ]
            .into_iter()
            .map(OsString::from)
            .collect();
            let plan = depcache::plan_with(
                &rustc,
                &args,
                &[],
                Some(LocalKey {
                    root: base.join("cache"),
                    salt: vec![],
                    cwd: Some(ws.clone()),
                }),
            )
            .unwrap();
            (plan, args)
        };
        let (plan, args) = build("t1", "debug");
        std::fs::write(&runtime_data, "first").unwrap();
        assert!(
            std::process::Command::new(&rustc)
                .args(&args)
                .current_dir(&ws)
                .status()
                .unwrap()
                .success()
        );
        assert!(stage_with(&plan, paths::now() - 2.0, 1.5, &[], true, true).is_some());
        assert!(
            !plan.manifest().exists(),
            "target-bound output must never be advertised as portable"
        );
        assert!(plan.restore(&mut Vec::new()).is_none());
        let binary = plan.out_dir.join(format!(
            "bound-0123456789abcdef{}",
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::remove_file(&binary).unwrap();
        assert!(restore_with(&plan, &mut Vec::new(), false).is_none());
        let hit = restore_with(&plan, &mut Vec::new(), true).expect("same-target restore");
        assert_eq!(hit.saved_secs, 1.5);
        assert_eq!(
            std::process::Command::new(&binary).output().unwrap().stdout,
            b"first"
        );
        // A runtime path must continue resolving the current file, not freeze
        // its old contents into the key or replay a different target's path.
        std::fs::write(&runtime_data, "second").unwrap();
        assert_eq!(
            std::process::Command::new(&binary).output().unwrap().stdout,
            b"second"
        );
        let (other_target, _) = build("t2", "debug");
        let (other_profile, _) = build("t1", "release");
        for other in [&other_target, &other_profile] {
            assert_eq!(
                plan.base, other.base,
                "portable base is deliberately shared"
            );
            assert!(restore_with(other, &mut Vec::new(), true).is_none());
        }
        std::fs::write(ws.join("src/main.rs"), "fn main() {}").unwrap();
        assert!(restore_with(&plan, &mut Vec::new(), true).is_none());
        std::fs::write(ws.join("src/main.rs"), &source).unwrap();
        assert!(restore_with(&plan, &mut Vec::new(), true).is_some());
        std::fs::remove_dir_all(plan.root.join("o").join(&hit.out)).unwrap();
        assert!(
            restore_with(&plan, &mut Vec::new(), true).is_none(),
            "eviction fails open"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// A local unit stored by one target dir is restored into another one,
    /// with its dep-info rewritten and stderr replayed, and misses once a
    /// source changes.
    #[test]
    fn local_unit_round_trip_across_target_dirs() {
        let base = std::env::temp_dir().join(format!("justrust-localcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/lib.rs"), "pub fn f() {}").unwrap();
        let cache = base.join("cache");
        let rustc = paths::find_on_path("rustc", Path::new("/nonexistent")).unwrap();
        let build = |target: &str| -> (Plan, Vec<OsString>) {
            let out = ws.join(target).join("debug/deps");
            std::fs::create_dir_all(&out).unwrap();
            let args: Vec<OsString> = [
                "--crate-name",
                "w",
                "--edition=2021",
                "src/lib.rs",
                "--crate-type",
                "lib",
                "--emit=dep-info,metadata,link",
                "-C",
                "extra-filename=-0123456789abcdef",
                "-C",
                &format!(
                    "incremental={}",
                    ws.join(target).join("debug/incremental").display()
                ),
                "--out-dir",
                out.to_str().unwrap(),
            ]
            .iter()
            .map(OsString::from)
            .collect();
            let key = LocalKey {
                root: cache.clone(),
                salt: vec![],
                cwd: Some(ws.clone()),
            };
            let p = depcache::plan_with(&rustc, &args, &[], Some(key)).unwrap();
            (p, args)
        };
        let (p1, args) = build("t1");
        let mut err = Vec::new();
        assert!(p1.restore(&mut err).is_none());
        let start = paths::now() - 0.5;
        let st = std::process::Command::new(&rustc)
            .args(&args)
            .current_dir(&ws)
            .status()
            .unwrap();
        assert!(st.success());
        assert!(stage_with(&p1, start, 1.5, &[b"warning: x\n".to_vec()], true, false).is_some());

        let (p2, _) = build("t2");
        assert_eq!(
            p1.base, p2.base,
            "base key must not depend on the target dir"
        );
        let hit = restore(&p2, &mut err).expect("hit in a second target dir");
        assert_eq!(hit.saved_secs, 1.5);
        assert_eq!(String::from_utf8_lossy(&err), "warning: x\n");
        let d = std::fs::read_to_string(ws.join("t2/debug/deps/w-0123456789abcdef.d")).unwrap();
        // Paths as cargo wrote them: `/` inside the target dir on every
        // platform, the native separator in the workspace prefix.
        let norm = d.replace('\\', "/");
        let ws_norm = ws.display().to_string().replace('\\', "/");
        assert!(norm.contains(&format!("{ws_norm}/t2/debug/deps")), "{d}");
        assert!(!norm.contains("/t1/"), "{d}");
        assert!(
            ws.join("t2/debug/deps/libw-0123456789abcdef.rlib")
                .is_file()
        );

        std::fs::write(ws.join("src/lib.rs"), "pub fn g() {}").unwrap();
        let (p3, _) = build("t3");
        assert!(
            p3.restore(&mut Vec::new()).is_none(),
            "changed source must miss"
        );
        // Revert: the first result is valid again.
        std::fs::write(ws.join("src/lib.rs"), "pub fn f() {}").unwrap();
        assert!(p3.restore(&mut Vec::new()).is_some());

        let _ = std::fs::remove_dir_all(&base);
    }
}
