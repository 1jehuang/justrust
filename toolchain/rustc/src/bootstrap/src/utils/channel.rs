//! Build configuration for Rust's release channels.
//!
//! Implements the stable/beta/nightly channel distinctions by setting various
//! flags like the `unstable_features`, calculating variables like `release` and
//! `package_vers`, and otherwise indicating to the compiler what it should
//! print out as part of its version information.

use std::fs;
use std::path::Path;

use super::exec::ExecutionContext;
use super::helpers;
use crate::core::session::Session;
use crate::utils::helpers::t;

#[derive(Clone, Default)]
pub enum GitInfo {
    /// This is not a git repository.
    #[default]
    Absent,
    /// This is a git repository.
    /// If the info should be used (`omit_git_hash` is false), this will be
    /// `Some`, otherwise it will be `None`.
    Present(Option<Info>),
    /// This is not a git repository, but the info can be fetched from the
    /// `git-commit-info` file.
    RecordedForTarball(Info),
    /// Owned source in the justrust repository or its source archive. Its
    /// content identity does not imply Git management or release-tarball defaults.
    OwnedSource(Info),
}

#[derive(Clone)]
pub struct Info {
    pub commit_date: String,
    pub sha: String,
    pub short_sha: String,
}

impl GitInfo {
    pub fn new(omit_git_hash: bool, dir: &Path, exec_ctx: impl AsRef<ExecutionContext>) -> GitInfo {
        // See if this even begins to look like a git dir
        if !dir.join(".git").exists() {
            // Owned sources live inside justrust, not a managed rust-lang/rust checkout.
            // Identify the actual source, including uncommitted edits and archives,
            // without enabling any Git operations on the containing repository.
            if let Ok(relative_root) = fs::read_to_string(dir.join("justrust-monorepo-root")) {
                let root = dir.join(relative_root.trim());
                if root.join("toolchain/sources.json").is_file()
                    && root.join("toolchain/rustc").canonicalize().ok() == dir.canonicalize().ok()
                {
                    return GitInfo::OwnedSource(owned_source_info(dir));
                }
            }
            match read_commit_info_file(dir) {
                Some(info) => return GitInfo::RecordedForTarball(info),
                None => return GitInfo::Absent,
            }
        }

        let mut git_command = helpers::git(Some(dir));
        git_command.arg("rev-parse");
        let output = git_command.allow_failure().run_capture(&exec_ctx);

        if output.is_failure() {
            return GitInfo::Absent;
        }

        // If we're ignoring the git info, we don't actually need to collect it, just make sure this
        // was a git repo in the first place.
        if omit_git_hash {
            return GitInfo::Present(None);
        }

        // Ok, let's scrape some info
        // We use the command's spawn API to execute these commands concurrently, which leads to performance improvements.
        let mut git_log_cmd = helpers::git(Some(dir));
        let ver_date = git_log_cmd
            .arg("log")
            .arg("-1")
            .arg("--date=short")
            .arg("--pretty=format:%cd")
            .run_in_dry_run()
            .start_capture_stdout(&exec_ctx);

        let mut git_hash_cmd = helpers::git(Some(dir));
        let ver_hash = git_hash_cmd
            .arg("rev-parse")
            .arg("HEAD")
            .run_in_dry_run()
            .start_capture_stdout(&exec_ctx);

        let mut git_short_hash_cmd = helpers::git(Some(dir));
        let short_ver_hash = git_short_hash_cmd
            .arg("rev-parse")
            .arg("--short=9")
            .arg("HEAD")
            .run_in_dry_run()
            .start_capture_stdout(&exec_ctx);

        GitInfo::Present(Some(Info {
            commit_date: ver_date.wait_for_output(&exec_ctx).stdout().trim().to_string(),
            sha: ver_hash.wait_for_output(&exec_ctx).stdout().trim().to_string(),
            short_sha: short_ver_hash.wait_for_output(&exec_ctx).stdout().trim().to_string(),
        }))
    }

    pub fn info(&self) -> Option<&Info> {
        match self {
            GitInfo::Absent => None,
            GitInfo::Present(info) => info.as_ref(),
            GitInfo::RecordedForTarball(info) | GitInfo::OwnedSource(info) => Some(info),
        }
    }

    pub fn sha(&self) -> Option<&str> {
        self.info().map(|s| &s.sha[..])
    }

    pub fn sha_short(&self) -> Option<&str> {
        self.info().map(|s| &s.short_sha[..])
    }

    pub fn commit_date(&self) -> Option<&str> {
        self.info().map(|s| &s.commit_date[..])
    }

    pub fn version(&self, sess: &Session, num: &str) -> String {
        let mut version = sess.release(num);
        if let Some(inner) = self.info() {
            version.push_str(" (");
            version.push_str(&inner.short_sha);
            version.push(' ');
            version.push_str(&inner.commit_date);
            version.push(')');
        }
        version
    }

    /// Returns whether this directory has a `.git` directory which should be managed by bootstrap.
    pub fn is_managed_git_subrepository(&self) -> bool {
        match self {
            GitInfo::Absent | GitInfo::RecordedForTarball(_) | GitInfo::OwnedSource(_) => false,
            GitInfo::Present(_) => true,
        }
    }

    /// Returns whether this is being built from a tarball.
    pub fn is_from_tarball(&self) -> bool {
        match self {
            GitInfo::Absent | GitInfo::Present(_) | GitInfo::OwnedSource(_) => false,
            GitInfo::RecordedForTarball(_) => true,
        }
    }
}

const OWNED_SOURCE_DIRS: &[&str] = &[
    "compiler",
    "library",
    "src/bootstrap",
    "src/build_helper",
    "src/shim_utils",
    "src/rustc-std-workspace",
];

const OWNED_SOURCE_FILES: &[&str] =
    &["src/version", "src/ci/channel", "Cargo.toml", "Cargo.lock", "justrust.bootstrap.toml"];

/// The upstream import and CI artifact commit are provenance, not the identity
/// of our current compiler. Include owned input bytes so dirty edits and source
/// archives get the same identity as committed builds of identical source.
fn owned_source_info(root: &Path) -> Info {
    use sha2::{Digest, Sha256};

    use crate::utils::build_stamp::hash_source_tree;

    let mut hasher = Sha256::new();
    hasher.update(b"justrust-rustc-owned-source-v1\0");
    for relative in OWNED_SOURCE_DIRS {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let dir = root.join(relative);
        if dir.is_dir() {
            hash_source_tree(root, &dir, &mut hasher);
        } else {
            hasher.update(b"missing\0");
        }
    }
    for relative in OWNED_SOURCE_FILES {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let file = root.join(relative);
        match fs::read(&file) {
            Ok(contents) => {
                hasher.update(b"file\0");
                hasher.update((contents.len() as u64).to_le_bytes());
                hasher.update(contents);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing\0");
            }
            Err(error) => panic!("failed to hash compiler input {}: {error}", file.display()),
        }
    }

    let sha = format!("{:x}", hasher.finalize())[..40].to_owned();
    Info {
        short_sha: sha[..9].to_owned(),
        sha,
        commit_date: read_commit_info_file(root)
            .map(|info| info.commit_date)
            .unwrap_or_else(|| "unknown".into()),
    }
}

/// Read the commit information from the `git-commit-info` file given the
/// project root.
pub fn read_commit_info_file(root: &Path) -> Option<Info> {
    if let Ok(contents) = fs::read_to_string(root.join("git-commit-info")) {
        let mut lines = contents.lines();
        let sha = lines.next();
        let short_sha = lines.next();
        let commit_date = lines.next();
        let info = match (commit_date, sha, short_sha) {
            (Some(commit_date), Some(sha), Some(short_sha)) => Info {
                commit_date: commit_date.to_owned(),
                sha: sha.to_owned(),
                short_sha: short_sha.to_owned(),
            },
            _ => panic!("the `git-commit-info` file is malformed"),
        };
        Some(info)
    } else {
        None
    }
}

/// Write the commit information to the `git-commit-info` file given the project
/// root.
pub fn write_commit_info_file(root: &Path, info: &Info) {
    let commit_info = format!("{}\n{}\n{}\n", info.sha, info.short_sha, info.commit_date);
    t!(fs::write(root.join("git-commit-info"), commit_info));
}

/// Write the commit hash to the `git-commit-hash` file given the project root.
pub fn write_commit_hash_file(root: &Path, sha: &str) {
    t!(fs::write(root.join("git-commit-hash"), sha));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{GitInfo, Info, OWNED_SOURCE_DIRS, OWNED_SOURCE_FILES, fs, write_commit_info_file};
    use crate::utils::exec::ExecutionContext;
    use crate::utils::tests::git::git_test;

    fn archived_info() -> Info {
        Info {
            sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            short_sha: "aaaaaaaaa".into(),
            commit_date: "2026-10-01".into(),
        }
    }

    fn vendored_sources(root: &std::path::Path) -> PathBuf {
        let sources = root.join("toolchain/rustc");
        fs::create_dir_all(&sources).unwrap();
        fs::write(root.join("toolchain/sources.json"), "{}\n").unwrap();
        fs::write(sources.join("justrust-monorepo-root"), "../..\n").unwrap();
        write_commit_info_file(&sources, &archived_info());
        for relative in OWNED_SOURCE_DIRS {
            let dir = sources.join(relative);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("source.rs"), "owned source\n").unwrap();
        }
        for relative in OWNED_SOURCE_FILES {
            let file = sources.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "owned input\n").unwrap();
        }
        let ci_llvm = sources.join("src/bootstrap/ci-llvm");
        fs::create_dir_all(&ci_llvm).unwrap();
        write_commit_info_file(
            &ci_llvm,
            &Info {
                sha: "1d9e68013afd163e05032b507400ff74d007ed72".into(),
                short_sha: "1d9e68013".into(),
                commit_date: "2026-10-03".into(),
            },
        );
        sources
    }

    fn info(sources: &std::path::Path) -> GitInfo {
        GitInfo::new(false, sources, &ExecutionContext::new(0, true))
    }

    fn assert_archived(info: &GitInfo) {
        assert!(!info.is_managed_git_subrepository());
        assert!(info.is_from_tarball());
        assert_eq!(info.sha(), Some(archived_info().sha.as_str()));
        assert_eq!(info.sha_short(), Some("aaaaaaaaa"));
        assert_eq!(info.commit_date(), Some("2026-10-01"));
    }

    #[test]
    fn vendored_identity_tracks_dirty_source_without_managing_git() {
        git_test(|git| {
            let sources = vendored_sources(git.get_path());
            git.commit();
            let first = info(&sources);
            assert!(!sources.join(".git").exists());
            assert!(!first.is_managed_git_subrepository());
            assert!(!first.is_from_tarball());
            assert!(matches!(first, GitInfo::OwnedSource(_)));
            let sha = first.sha().unwrap();
            assert_eq!(sha.len(), 40);
            assert!(sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(first.sha_short(), Some(&sha[..9]));
            assert_eq!(first.commit_date(), Some("2026-10-01"));

            let ci_pin = sources.join("src/bootstrap/ci-llvm/git-commit-info");
            let original_ci_pin = fs::read(&ci_pin).unwrap();
            git.write("toolchain/rustc/compiler/source.rs", "owned compiler edit");
            let second = info(&sources);
            assert_ne!(first.sha(), second.sha());
            assert!(!second.is_managed_git_subrepository());
            assert!(!second.is_from_tarball());
            git.commit();
            assert_eq!(second.sha(), info(&sources).sha());
            git.write("README.md", "unrelated client change");
            git.commit();
            assert_eq!(second.sha(), info(&sources).sha());
            // Imported provenance must not change along with build identity.
            assert_eq!(super::read_commit_info_file(&sources).unwrap().sha, archived_info().sha);
            assert_eq!(fs::read(ci_pin).unwrap(), original_ci_pin);
        });
    }

    #[test]
    fn vendored_archive_without_git_uses_recorded_identity() {
        let archive = tempfile::TempDir::new().unwrap();
        let sources = vendored_sources(archive.path());
        fs::remove_file(sources.join("justrust-monorepo-root")).unwrap();
        assert_archived(&info(&sources));
    }

    #[test]
    fn vendored_git_and_archive_with_same_source_have_same_identity() {
        git_test(|git| {
            let working_sources = vendored_sources(git.get_path());
            git.commit();
            let archive = tempfile::TempDir::new().unwrap();
            let archived_sources = vendored_sources(archive.path());
            let archived_identity = info(&archived_sources);
            assert!(matches!(archived_identity, GitInfo::OwnedSource(_)));
            assert!(!archived_identity.is_managed_git_subrepository());
            assert!(!archived_identity.is_from_tarball());
            assert_eq!(info(&working_sources).sha(), archived_identity.sha());
            fs::write(archived_sources.join("library/source.rs"), "dirty library edit\n").unwrap();
            assert_ne!(info(&working_sources).sha(), info(&archived_sources).sha());
        });
    }

    #[test]
    fn vendored_identity_covers_all_owned_inputs() {
        let archive = tempfile::TempDir::new().unwrap();
        let sources = vendored_sources(archive.path());
        let initial = info(&sources);
        let inputs = OWNED_SOURCE_DIRS
            .iter()
            .map(|relative| sources.join(relative).join("source.rs"))
            .chain(OWNED_SOURCE_FILES.iter().map(|relative| sources.join(relative)));
        for file in inputs {
            let original = fs::read(&file).unwrap();
            fs::write(&file, "changed source input\n").unwrap();
            assert_ne!(
                initial.sha(),
                info(&sources).sha(),
                "input {} was not hashed",
                file.display()
            );
            fs::write(&file, original).unwrap();
            assert_eq!(initial.sha(), info(&sources).sha());
        }
    }

    #[test]
    fn vendored_identity_ignores_generated_outputs() {
        let archive = tempfile::TempDir::new().unwrap();
        let sources = vendored_sources(archive.path());
        let initial = info(&sources);
        for relative in [
            "compiler/build",
            "library/target",
            "src/bootstrap/dist",
            "src/bootstrap/obj",
            "src/bootstrap/.git",
            "src/bootstrap/__pycache__",
        ] {
            let dir = sources.join(relative);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("generated"), "generated output\n").unwrap();
        }
        assert_eq!(initial.sha(), info(&sources).sha());
    }

    #[test]
    fn vendored_identity_does_not_discover_unmarked_ancestor_git() {
        git_test(|git| {
            let sources = vendored_sources(git.get_path());
            fs::remove_file(sources.join("justrust-monorepo-root")).unwrap();
            git.commit();
            assert_archived(&info(&sources));
        });
    }

    #[test]
    fn vendored_identity_requires_monorepo_manifest() {
        git_test(|git| {
            let sources = vendored_sources(git.get_path());
            fs::remove_file(git.get_path().join("toolchain/sources.json")).unwrap();
            git.commit();
            assert_archived(&info(&sources));
        });
    }

    #[test]
    fn vendored_identity_rejects_marker_in_other_source_directory() {
        git_test(|git| {
            vendored_sources(git.get_path());
            let other = git.get_path().join("toolchain/unrelated");
            fs::create_dir_all(&other).unwrap();
            fs::write(other.join("justrust-monorepo-root"), "../..\n").unwrap();
            write_commit_info_file(&other, &archived_info());
            git.commit();
            assert_archived(&info(&other));
        });
    }

    #[test]
    fn vendored_archive_without_metadata_is_absent() {
        let archive = tempfile::TempDir::new().unwrap();
        let sources = vendored_sources(archive.path());
        fs::remove_file(sources.join("git-commit-info")).unwrap();
        fs::remove_file(sources.join("justrust-monorepo-root")).unwrap();
        let identity = info(&sources);
        assert!(matches!(identity, GitInfo::Absent));
        assert!(!identity.is_managed_git_subrepository());
        assert!(!identity.is_from_tarball());
    }
}
