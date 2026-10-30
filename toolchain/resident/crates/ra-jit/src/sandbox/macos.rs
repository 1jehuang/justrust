//! The sandbox on macOS. The command's own process stays outside: it makes the run's scratch
//! directory, forks the program's process, which applies a Seatbelt profile to itself
//! (`sandbox_init_with_parameters`, as Nix's and Bazel's sandboxes on macOS do) and runs the
//! program, waits for it, writes the record and removes the scratch directory.
//!
//! Seatbelt is what enforces, in the kernel and for every process the program starts: no
//! write but into the scratch directory, [`Spec::writable`] and the null devices, no network
//! but the loopback and Unix sockets in those directories (not the DNS resolver's, which a
//! network namespace has none of on Linux, nor any other daemon's), and no Mach service but
//! the few that only answer or log ([`MACH_SERVICES`]): `cfprefsd`, launchd and the rest
//! would write for the program outside. The program's process is a session of its own, with
//! no controlling terminal to push input into. Seatbelt can't make a write private or tell what was read,
//! so a library every process of the run loads (`DYLD_INSERT_LIBRARIES`,
//! `macos_interpose.c`) does that from inside the process: it replaces the C library's calls
//! that name a path, run a program or reach the network, keeps the writes under the overlays
//! in the scratch directory (a changed file is cloned there first, which on APFS copies no
//! data), answers `/tmp`, `/var/tmp` and the user's temporary directory from empty ones of the
//! run's own, and appends what it saw to a file of the run's, from which the record
//! ([`Recorder`]) is made when the run ends.
//!
//! macOS drops `DYLD_*` for the programs it protects (`/bin/sh`, everything in `/usr/bin`) and
//! for hardened ones; such a program still runs under Seatbelt, so it can't write into an
//! overlay or anywhere else outside the run, but what it reads is not seen. Each run of a
//! program is given a nonce the library says back when it loads, and a program that never said
//! it is an `X` line in the record: the record is incomplete.
//!
//! A view ([`Spec::view`]) is not made on macOS (decided, 2026-10-27): a spec with one is run
//! as though it had none, its binds unread, and the program sees the machine as Seatbelt
//! leaves it. Linux is where a build sees its declared closure alone.

use std::{
    collections::HashSet,
    ffi::{CString, OsString},
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
};

use super::{Reached, Recorder, Spec, tidy};

/// The variables the sandbox gives the program's processes for the library, which takes them
/// out of what the program sees as it loads.
const OURS: [&str; 3] = ["DYLD_INSERT_LIBRARIES", "__RA_JIT_SANDBOX", "__RA_JIT_SANDBOX_EXEC"];

/// The library every process of a run loads, built from `macos_interpose.c` by `build.rs`.
const LIBRARY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ra_jit_sandbox.dylib"));

unsafe extern "C" {
    // libsystem_sandbox, part of libSystem: what `sandbox-exec` does, without a process of
    // its own to run.
    fn sandbox_init_with_parameters(
        profile: *const libc::c_char,
        flags: u64,
        parameters: *const *const libc::c_char,
        errorbuf: *mut *mut libc::c_char,
    ) -> libc::c_int;
}

/// The Mach services a program of the sandbox may look up; every other lookup is refused. A
/// service runs outside the sandbox, so one that writes for its callers (`cfprefsd`'s
/// preferences, launchd's jobs, LaunchServices' and the background task manager's login items,
/// the keychain) would be a way out: these are the ones that only answer, or only log. Found by
/// running the C toolchain (`cc`, `ld`), `xcrun metal` and `metallib`, a Metal program that
/// compiles a library at run time and an FSEvents stream under a profile that refused every
/// lookup, and adding what each needed (Nix's sandbox on macOS allows the first and the
/// `trustd` ones).
const MACH_SERVICES: &[&str] = &[
    // getpwuid, getgrgid and group membership (Nix: "Allow getpwuid").
    "(global-name \"com.apple.system.opendirectoryd.libinfo\")",
    "(global-name \"com.apple.system.opendirectoryd.membership\")",
    // libSystem's notifications (time zone and network changes).
    "(global-name \"com.apple.system.notification_center\")",
    // The unified log: `os_log`, `syslog`, what a crash or a failed assertion reports.
    "(global-name \"com.apple.logd\")",
    "(global-name \"com.apple.diagnosticd\")",
    "(global-name \"com.apple.system.logger\")",
    // Watching files (the `notify` crate on macOS).
    "(global-name \"com.apple.FSEvents\")",
    // Checking a certificate against the system's roots (a TLS test against a server on
    // loopback); Nix allows these for the derivations that reach the network.
    "(global-name \"com.apple.trustd\")",
    "(global-name \"com.apple.trustd.agent\")",
    // Metal's compiler, which a program that makes a library from source at run time reaches.
    "(xpc-service-name \"com.apple.MTLCompilerService\")",
];

/// The profile: everything allowed but writing outside the run, the network beyond the
/// loopback, a Unix socket outside the run's own and writable directories, and every Mach
/// service but [`MACH_SERVICES`]. Paths come in as parameters (`WRITE0`, `WRITE1`, ...), which
/// need no quoting. No terminal is written by name: what the program prints goes to the
/// descriptors it was given, which no rule checks again.
fn profile(writes: usize) -> String {
    let dirs: String = (0..writes).map(|at| format!(" (subpath (param \"WRITE{at}\"))")).collect();
    let mut text = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    text.push_str(&format!("(allow file-write*{dirs}\n"));
    text.push_str(
        "  (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/random\")
  (literal \"/dev/urandom\") (literal \"/dev/dtracehelper\") (literal \"/dev/stdout\")
  (literal \"/dev/stderr\") (regex #\"^/dev/fd/[0-9]+$\"))
(deny network*)
(allow network-bind (local ip \"localhost:*\"))
(allow network-inbound (local ip \"localhost:*\"))
(allow network-outbound (remote ip \"localhost:*\"))
",
    );
    // A socket's form takes one path: a rule each (two `subpath`s in one keep the first).
    for at in 0..writes {
        let dir = format!("(subpath (param \"WRITE{at}\"))");
        text.push_str(&format!("(allow network-bind network-inbound (local unix-socket {dir}))\n"));
        text.push_str(&format!("(allow network-outbound (remote unix-socket {dir}))\n"));
    }
    text.push_str("(deny mach-lookup)\n(allow mach-lookup");
    for service in MACH_SERVICES {
        text.push_str("\n  ");
        text.push_str(service);
    }
    text.push_str(")\n");
    text
}

fn os(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn c(bytes: &[u8]) -> CString {
    CString::new(bytes).unwrap_or_default()
}

/// The ways a directory is spelled: as given, and as the file system resolves it (`/tmp` is
/// `/private/tmp`), which is what a program that resolved it names.
fn spellings(dir: &Path) -> Vec<String> {
    let mut out = vec![tidy(dir)];
    if let Ok(real) = std::fs::canonicalize(dir) {
        out.push(tidy(&real));
    }
    for it in out.clone() {
        if let Some(rest) = it.strip_prefix("/private").filter(|rest| rest.starts_with('/')) {
            out.push(rest.to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The directory in `parent` that stands in for `dir` (an overlay's upper, a temporary
/// directory's empty one), made: named as the file system names `dir`. A directory's own name
/// is what `getattrlist` says of it, from which `realpath` builds the path, so one named
/// otherwise (`up/0`) turned `/w/flags/target` into `/w/0/target`, which is not there.
fn stand_in(parent: &Path, dir: &Path) -> Result<PathBuf, String> {
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
    let made = real.file_name().map_or_else(|| parent.to_owned(), |name| parent.join(name));
    std::fs::create_dir_all(&made).map_err(|error| format!("{}: {error}", made.display()))?;
    Ok(made)
}

/// The user's temporary directory, as macOS gives it to each user (`/var/folders/...`).
fn user_temp() -> Option<PathBuf> {
    let mut buffer = [0u8; 1024];
    let len = unsafe {
        libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buffer.as_mut_ptr().cast(), buffer.len())
    };
    if len == 0 || len > buffer.len() {
        return None;
    }
    let text = &buffer[..len - 1];
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(text)))
}

/// Where the library is, written there from this executable the first time: a path named by
/// its contents in the user's cache directory, so each build of ra-jit has its own and an
/// installed one needs nothing besides itself.
fn library() -> Result<PathBuf, String> {
    let caches = std::env::var_os("XDG_CACHE_HOME").filter(|it| !it.is_empty()).map(PathBuf::from);
    let caches = caches
        .or_else(|| std::env::home_dir().map(|it| it.join(".cache")))
        .ok_or("no XDG_CACHE_HOME or home directory to keep the sandbox's library in")?;
    let dir = caches.join("ra-jit/sandbox").join(env!("RA_JIT_SANDBOX_LIBRARY"));
    let file = dir.join("libra_jit_sandbox.dylib");
    // The name is that of the contents it should have; what is there is still compared, so a
    // file left half written, or changed since, is written again.
    if std::fs::read(&file).is_ok_and(|it| it == LIBRARY) {
        return Ok(file);
    }
    std::fs::create_dir_all(&dir).map_err(|it| format!("{}: {it}", dir.display()))?;
    let writing = dir.join(format!("writing-{}", std::process::id()));
    std::fs::write(&writing, LIBRARY)
        .and_then(|()| std::fs::rename(&writing, &file))
        .map_err(|it| format!("{}: {it}", file.display()))?;
    Ok(file)
}

/// The run's own directory: the overlays' upper directories, the empty temporary
/// directories, the listings, the shared state and the raw record.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(overlays: usize) -> Result<Scratch, String> {
        let base = std::env::temp_dir();
        let base = std::fs::canonicalize(&base).unwrap_or(base);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |it| it.subsec_nanos());
        let dir = base.join(format!("ra-jit-sandbox-{}-{nanos:x}", std::process::id()));
        let scratch = Scratch { dir };
        let mut made = vec!["list".to_owned(), "tmp/tmp".to_owned()];
        made.extend(["tmp/var".to_owned(), "tmp/user".to_owned()]);
        made.extend((0..overlays).map(|at| format!("up/{at}")));
        for it in made {
            let it = scratch.dir.join(it);
            std::fs::create_dir_all(&it).map_err(|error| format!("{}: {error}", it.display()))?;
        }
        let state = scratch.dir.join("state");
        std::fs::write(&state, [0u8; 4096])
            .map_err(|error| format!("{}: {error}", state.display()))?;
        Ok(scratch)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::fs::remove_dir_all(&self.dir).is_ok() {
            return;
        }
        // The program may have left a directory it could not be removed from.
        fn writable(dir: &Path) {
            use std::os::unix::fs::PermissionsExt as _;
            _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.file_type().is_ok_and(|it| it.is_dir()) {
                    writable(&entry.path());
                }
            }
        }
        writable(&self.dir);
        _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs `program` sandboxed as `spec` says, or with `None` makes the sandbox and runs nothing
/// in it. What it ends with: its status, or 128 and the signal that ended it.
pub(super) fn run(spec: &Spec, program: Option<&[OsString]>) -> Result<i32, String> {
    let library = library()?;
    let scratch = Scratch::new(spec.overlays.len())?;
    let dir = scratch.dir.to_string_lossy().into_owned();
    let recording = spec.record.is_some() && program.is_some();

    // What the library is told.
    let mut config = String::new();
    for (at, overlay) in spec.overlays.iter().enumerate() {
        std::fs::create_dir_all(overlay)
            .map_err(|error| format!("{}: {error}", overlay.display()))?;
        let upper = stand_in(&scratch.dir.join("up").join(at.to_string()), overlay)?;
        for it in spellings(overlay) {
            config.push_str(&format!("O\t{at}\t{it}\t{}\n", upper.display()));
        }
    }
    let user = user_temp();
    let env_temp = spec.env.iter().find(|(name, _)| name == "TMPDIR");
    let env_temp = env_temp.map(|(_, it)| PathBuf::from(it));
    let users = scratch.dir.join("tmp/user");
    let user_in = match user.as_ref().or(env_temp.as_ref()) {
        Some(it) => stand_in(&users, it)?,
        None => users,
    };
    let mut temps = Vec::new();
    for (temp, to) in [("/private/tmp", "tmp"), ("/private/var/tmp", "var")] {
        let temp = PathBuf::from(temp);
        let empty = stand_in(&scratch.dir.join("tmp").join(to), &temp)?;
        temps.push((temp, empty));
    }
    temps.extend(user.map(|it| (it, user_in.clone())));
    temps.extend(env_temp.map(|it| (it, user_in.clone())));
    for (temp, to) in &temps {
        for it in spellings(temp) {
            config.push_str(&format!("B\t{it}\t{}\n", to.display()));
        }
    }
    // A directory passed through or overlaid below a temporary one is reached through the
    // directories above it, which are answered from the empty one: they are made there, as
    // Linux makes the points it mounts it on. Without them `realpath` (Rust's `canonicalize`),
    // which looks at each directory of a path in turn, says the path is not there: a
    // proc-macro server canonicalizes the library it loads, and every macro of a workspace
    // below `/tmp` failed; a cargo a test runs canonicalizes the workspace once rustc has
    // compiled a crate of it, and failed "could not parse/generate dep info".
    for passed in spec.writable.iter().chain(&spec.readable).chain(&spec.overlays) {
        for (temp, to) in &temps {
            let below = spellings(passed).into_iter().find_map(|passed| {
                spellings(temp).into_iter().find_map(|temp| {
                    Path::new(&passed).strip_prefix(&temp).ok().map(Path::to_owned)
                })
            });
            let Some(below) = below.filter(|it| !it.as_os_str().is_empty()) else { continue };
            let made = to.join(below);
            std::fs::create_dir_all(&made)
                .map_err(|error| format!("{}: {error}", made.display()))?;
        }
    }
    let mut writes = vec![dir.clone()];
    for writable in &spec.writable {
        std::fs::create_dir_all(writable)
            .map_err(|error| format!("{}: {error}", writable.display()))?;
        for it in spellings(writable) {
            config.push_str(&format!("W\t{it}\n"));
            writes.push(it);
        }
    }
    // A readable directory is passed through as it is, not answered from an empty one: the
    // library takes it as it takes a writable one, and Seatbelt still refuses its writes.
    for readable in &spec.readable {
        for it in spellings(readable) {
            config.push_str(&format!("W\t{it}\n"));
        }
    }
    if recording {
        config.push_str("L\n");
        if spec.record_reads {
            config.push_str("R\n");
        }
    }
    let config_path = scratch.dir.join("config");
    std::fs::write(&config_path, config)
        .map_err(|error| format!("{}: {error}", config_path.display()))?;

    // Everything the child uses is made before the fork.
    let profile = c(profile(writes.len()).as_bytes());
    let params: Vec<CString> = writes
        .iter()
        .enumerate()
        .flat_map(|(at, it)| [c(format!("WRITE{at}").as_bytes()), c(it.as_bytes())])
        .collect();
    let mut param_ptrs: Vec<*const libc::c_char> = params.iter().map(|it| it.as_ptr()).collect();
    param_ptrs.push(std::ptr::null());
    let argv: Vec<CString> =
        program.unwrap_or_default().iter().map(|it| c(it.as_bytes())).collect();
    let mut argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|it| it.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());
    let nonce = format!("{}.top", std::process::id());
    // The program's environment is the spec's, its `TMPDIR` the run's empty one.
    let mut env: Vec<CString> = spec
        .env
        .iter()
        .filter(|(name, _)| name != "TMPDIR" && !OURS.contains(&name.as_str()))
        .map(|(name, value)| c(format!("{name}={value}").as_bytes()))
        .collect();
    // The program's own `DYLD_INSERT_LIBRARIES` loads after the library, which takes it away
    // again before the program starts.
    let mut insert = library.as_os_str().to_owned();
    if let Some((_, theirs)) = spec.env.iter().find(|(name, _)| name == "DYLD_INSERT_LIBRARIES") {
        insert.push(":");
        insert.push(theirs);
    }
    let mut insert_var = OsString::from("DYLD_INSERT_LIBRARIES=");
    insert_var.push(insert);
    env.push(c(insert_var.as_bytes()));
    env.push(c(format!("TMPDIR={}/", user_in.display()).as_bytes()));
    env.push(c(format!("__RA_JIT_SANDBOX={dir}").as_bytes()));
    env.push(c(format!("__RA_JIT_SANDBOX_EXEC={nonce}").as_bytes()));
    let mut env_ptrs: Vec<*const libc::c_char> = env.iter().map(|it| it.as_ptr()).collect();
    env_ptrs.push(std::ptr::null());
    let cwd = c(spec.cwd.as_os_str().as_bytes());
    let raw = scratch.dir.join("record");
    if recording {
        let program = program.unwrap_or_default().first().map(|it| it.to_string_lossy());
        let line = format!("E\t{nonce}\t\t{}\n", program.unwrap_or_default());
        std::fs::write(&raw, line).map_err(|error| format!("{}: {error}", raw.display()))?;
    }

    // Before the fork, so that no signal ends this process before it passes them on.
    super::pass_signals_to(0);
    // SAFETY: the process is one thread (the command runs before anything else); the child
    // makes system calls, applies the profile and runs the program or exits.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(os("fork"));
    }
    if child == 0 {
        // A session of its own, with no controlling terminal: nothing of the sandbox can push
        // input into the caller's (`TIOCSTI` is refused but to the terminal's own session).
        unsafe { libc::setsid() };
        let mut error: *mut libc::c_char = std::ptr::null_mut();
        // SAFETY: the profile and the parameters are strings ending in NUL, the parameters a
        // list of names and values ending in a null pointer.
        let applied = unsafe {
            sandbox_init_with_parameters(profile.as_ptr(), 0, param_ptrs.as_ptr(), &mut error)
        };
        if applied != 0 {
            let why = match error.is_null() {
                true => "it said no more".into(),
                // SAFETY: the error is a string sandbox_init_with_parameters made.
                false => unsafe { std::ffi::CStr::from_ptr(error) }.to_string_lossy(),
            };
            eprintln!("blaze: the sandbox could not be made: Seatbelt refused the profile: {why}");
            unsafe { libc::_exit(super::COULD_NOT) };
        }
        if program.is_none() {
            unsafe { libc::_exit(0) };
        }
        if unsafe { libc::chdir(cwd.as_ptr()) } < 0 {
            let error = os(&format!("the working directory {}", spec.cwd.display()));
            eprintln!("blaze: the sandbox could not be made: {error}");
            unsafe { libc::_exit(super::COULD_NOT) };
        }
        unsafe { libc::execve(argv_ptrs[0], argv_ptrs.as_ptr(), env_ptrs.as_ptr()) };
        let error = std::io::Error::last_os_error();
        eprintln!("blaze: could not run {}: {error}", argv[0].to_string_lossy());
        unsafe { libc::_exit(127) };
    }
    super::pass_signals_to(child);
    let mut status = 0;
    while unsafe { libc::waitpid(child, &mut status, 0) } < 0 {
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(os("waitpid"));
        }
    }
    // Its number may be another's now.
    super::pass_signals_to(0);
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    };
    if let Some(file) = spec.record.as_ref().filter(|_| recording) {
        // The paths the library saw are as the program spelled them, so the overlays and what
        // is no input are known by each of their spellings.
        let spelled = |dirs: &[PathBuf]| -> Vec<PathBuf> {
            dirs.iter().flat_map(|it| spellings(it)).map(PathBuf::from).collect()
        };
        let mut known = spec.clone();
        known.overlays = spelled(&spec.overlays);
        known.not_inputs = spelled(&spec.not_inputs);
        let own: Vec<String> = ["/dev"]
            .into_iter()
            .map(str::to_owned)
            .chain(temps.iter().flat_map(|(it, _)| spellings(it)))
            .collect();
        let own: Vec<&str> = own.iter().map(String::as_str).collect();
        let mut recorder = Recorder::new(&known, &own);
        merge(&String::from_utf8_lossy(&std::fs::read(&raw).unwrap_or_default()), &mut recorder);
        recorder.write(file);
    }
    drop(scratch);
    Ok(code)
}

/// Gives `recorder` what the processes of a run appended (`macos_interpose.c` says what each
/// line is): its `R`, `W`, `N` and `X` lines, and an `X` line for each program that ran
/// without the library.
fn merge(raw: &str, recorder: &mut Recorder) {
    let mut ran = Vec::new();
    let mut loaded = HashSet::new();
    for line in raw.lines() {
        let mut fields = line.splitn(4, '\t');
        match (fields.next(), fields.next(), fields.next(), fields.next()) {
            (Some("E"), Some(nonce), Some(test), Some(path)) => ran.push((nonce, test, path)),
            (Some("H" | "F"), Some(nonce), None, None) => _ = loaded.insert(nonce),
            (Some(letter), Some(test), Some(what), None) => {
                if let Some(reached) = Reached::of(letter) {
                    recorder.reached(reached, test.to_owned(), what);
                }
            }
            _ => {}
        }
    }
    for (nonce, test, path) in ran {
        if !loaded.contains(nonce) {
            let why = format!(
                "{path} ran without the recorder (macOS keeps DYLD_INSERT_LIBRARIES from a \
                 program it protects), so what it read is not known"
            );
            recorder.reached(Reached::Incomplete, test.to_owned(), &why);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Recorder, Spec};

    #[test]
    fn merge_keeps_each_line_once_and_names_the_programs_not_seen() {
        let raw = "E\t1.top\t\t/w/target/debug/deps/t-1\n\
                   H\t1.top\n\
                   R\ta::b\t/w/data.txt\n\
                   R\ta::b\t/w/data.txt\n\
                   R\t\t/dev/null\n\
                   R\t\t/w/target/debug/deps/t-1\n\
                   W\ta::b\t/Users/u/.config/x\n\
                   N\ta::c\t1.1.1.1:443\n\
                   E\t5.ab.0\ta::c\t/bin/sh\n\
                   E\t5.ab.1\ta::c\t/w/target/debug/helper\n\
                   H\t5.ab.1\n\
                   E\t5.ab.2\ta::c\t/missing\n\
                   F\t5.ab.2\n\
                   X\ta::d\ta path with a tab or a line break in it\n\
                   R\ta::b\t/w/last";
        let spec = Spec {
            overlays: vec!["/w".into()],
            not_inputs: vec!["/w/target/debug/deps/t-1".into()],
            record_reads: true,
            ..Spec::default()
        };
        let mut recorder = Recorder::new(&spec, &["/dev"]);
        super::merge(raw, &mut recorder);
        let dir = std::env::temp_dir().join(format!("ra-jit-macos-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("record");
        recorder.write(&file);
        let text = std::fs::read_to_string(&file).unwrap();
        _ = std::fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6, "{text}");
        assert_eq!(lines[0], "R\ta::b\t/w/data.txt");
        assert_eq!(lines[1], "R\ta::b\t/w/last");
        assert_eq!(lines[2], "W\ta::b\t/Users/u/.config/x");
        assert_eq!(lines[3], "N\ta::c\t1.1.1.1:443");
        assert!(lines[4].starts_with("X\ta::c\t/bin/sh ran without the recorder"), "{text}");
        assert_eq!(lines[5], "X\ta::d\ta path with a tab or a line break in it");
    }
}
