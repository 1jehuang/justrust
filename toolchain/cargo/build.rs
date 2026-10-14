use flate2::{Compression, GzBuilder};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    commit_info();
    compress_man();
    windows_manifest();
    #[expect(
        clippy::disallowed_methods,
        reason = "not `cargo`, not needing to load from config"
    )]
    let target = std::env::var("TARGET").unwrap();
    println!("cargo:rustc-env=RUST_HOST_TARGET={target}");
}

fn compress_man() {
    #[expect(
        clippy::disallowed_methods,
        reason = "not `cargo`, not needing to load from config"
    )]
    let out_path = Path::new(&std::env::var("OUT_DIR").unwrap()).join("man.tgz");
    let dst = fs::File::create(out_path).unwrap();
    let encoder = GzBuilder::new()
        .filename("man.tar")
        .write(dst, Compression::best());
    let mut ar = tar::Builder::new(encoder);
    ar.mode(tar::HeaderMode::Deterministic);

    let mut add_files = |dir, extension| {
        let mut files = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            if path.extension() != Some(extension) {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            ar.append_path_with_name(&path, path.file_name().unwrap())
                .unwrap();
        }
    };

    add_files(Path::new("etc/man"), OsStr::new("1"));
    add_files(Path::new("doc/man/generated_txt"), OsStr::new("txt"));
    let encoder = ar.into_inner().unwrap();
    encoder.finish().unwrap();
}

struct CommitInfo {
    hash: String,
    short_hash: String,
    date: String,
}

fn commit_info_from_git() -> Option<CommitInfo> {
    if !Path::new(".git").exists() {
        return None;
    }

    let output = match Command::new("git")
        .arg("log")
        .arg("-1")
        .arg("--date=short")
        .arg("--format=%H %h %cd")
        .arg("--abbrev=9")
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return None,
    };

    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut parts = stdout.split_whitespace().map(|s| s.to_string());

    Some(CommitInfo {
        hash: parts.next()?,
        short_hash: parts.next()?,
        date: parts.next()?,
    })
}

// The rustc source tarball is meant to contain all the source code to build an exact copy of the
// toolchain, but it doesn't include the git repository itself. It wouldn't thus be possible to
// populate the version information with the commit hash and the commit date.
//
// To work around this, the rustc build process obtains the git information when creating the
// source tarball and writes it to the `git-commit-info` file. The build process actually creates
// at least *two* of those files, one for Rust as a whole (in the root of the tarball) and one
// specifically for Cargo (in src/tools/cargo). This function loads that file.
//
// The file is a newline-separated list of full commit hash, short commit hash, and commit date.
fn commit_info_from_rustc_source_tarball() -> Option<CommitInfo> {
    let path = Path::new("git-commit-info");
    if !path.exists() {
        return None;
    }

    // Dependency tracking is a nice to have for this (git doesn't do it), so if the path is not
    // valid UTF-8 just avoid doing it rather than erroring out.
    if let Some(utf8) = path.to_str() {
        println!("cargo:rerun-if-changed={utf8}");
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let mut parts = content.split('\n').map(|s| s.to_string());
    Some(CommitInfo {
        hash: parts.next()?,
        short_hash: parts.next()?,
        date: parts.next()?,
    })
}

fn commit_info() {
    // Var set by bootstrap whenever omit-git-hash is enabled in rust-lang/rust's config.toml.
    println!("cargo:rerun-if-env-changed=CFG_OMIT_GIT_HASH");
    #[expect(
        clippy::disallowed_methods,
        reason = "not `cargo`, not needing to load from config"
    )]
    if std::env::var_os("CFG_OMIT_GIT_HASH").is_some() {
        return;
    }

    let Some(git) = commit_info_from_justrust_base()
        .or_else(commit_info_from_git)
        .or_else(commit_info_from_rustc_source_tarball)
    else {
        return;
    };

    println!("cargo:rustc-env=CARGO_COMMIT_HASH={}", git.hash);
    println!("cargo:rustc-env=CARGO_COMMIT_SHORT_HASH={}", git.short_hash);
    println!("cargo:rustc-env=CARGO_COMMIT_DATE={}", git.date);
    // justrust fork: identify the build without changing the upstream
    // commit it reports (tools compare that commit with rustc's).
    // This source fingerprint changes for dirty edits too and works on hosted
    // source mirrors without Git metadata. Public Cargo identity remains upstream.
    println!(
        "cargo:rustc-env=JR_CARGO_FORK_COMMIT={}",
        source_fingerprint(Path::new("."))
    );
}

pub(crate) fn source_fingerprint(root: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    for relative in [
        "src",
        "crates",
        "credential",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "windows.manifest.xml",
        "JUSTRUST_BASE",
        ".cargo/config.toml",
        "etc/man",
        "doc/man/generated_txt",
    ] {
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        hash_source_path(root, &path, &mut hasher);
    }
    hasher.finalize().to_hex()[..40].to_owned()
}

fn hash_source_path(root: &Path, path: &Path, hasher: &mut blake3::Hasher) {
    use std::io::Read;
    let metadata = fs::symlink_metadata(path).expect("owned Cargo source must be readable");
    let relative = path
        .strip_prefix(root)
        .unwrap()
        .as_os_str()
        .as_encoded_bytes();
    hasher.update(&relative.len().to_le_bytes());
    hasher.update(relative);
    if metadata.is_symlink() {
        hasher.update(b"symlink");
        hasher.update(fs::read_link(path).unwrap().as_os_str().as_encoded_bytes());
    } else if metadata.is_dir() {
        hasher.update(b"directory");
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            let name = entry.file_name().unwrap();
            if name == ".git" || (entry.is_dir() && (name == "target" || name == "build")) {
                continue;
            }
            hash_source_path(root, &entry, hasher);
        }
    } else if metadata.is_file() {
        hasher.update(b"file");
        hasher.update(&metadata.len().to_le_bytes());
        let mut file = fs::File::open(path).unwrap();
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
    }
}

/// justrust fork: `JUSTRUST_BASE` names the upstream cargo commit this fork
/// is based on (full hash, short hash, date), so `cargo -V` reports the
/// cargo the pinned toolchain ships rather than the fork's own commit.
fn commit_info_from_justrust_base() -> Option<CommitInfo> {
    println!("cargo:rerun-if-changed=JUSTRUST_BASE");
    let content = std::fs::read_to_string("JUSTRUST_BASE").ok()?;
    let mut parts = content.lines().map(|s| s.trim().to_string());
    Some(CommitInfo {
        hash: parts.next()?,
        short_hash: parts.next()?,
        date: parts.next()?,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "not `cargo`, not needing to load from config"
)]
fn windows_manifest() {
    use std::env;
    let target_os = env::var("CARGO_CFG_TARGET_OS");
    let target_env = env::var("CARGO_CFG_TARGET_ENV");
    if Ok("windows") == target_os.as_deref() && Ok("msvc") == target_env.as_deref() {
        static WINDOWS_MANIFEST_FILE: &str = "windows.manifest.xml";

        let mut manifest = env::current_dir().unwrap();
        manifest.push(WINDOWS_MANIFEST_FILE);

        println!("cargo:rerun-if-changed={WINDOWS_MANIFEST_FILE}");
        // Embed the Windows application manifest file.
        println!("cargo:rustc-link-arg-bin=cargo=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=cargo=/MANIFESTINPUT:{}",
            manifest.to_str().unwrap()
        );
        // Turn linker warnings into errors.
        println!("cargo:rustc-link-arg-bin=cargo=/WX");
    }
}
