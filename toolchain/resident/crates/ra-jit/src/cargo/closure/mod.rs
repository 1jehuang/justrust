//! The nixpkgs store paths build scripts run with (docs/closure.md): a C toolchain, a small shell
//! userland and the system libraries crates link, each pinned in `pins.txt` by its store path
//! and the NarHash cache.nixos.org signs for it, at one nixpkgs revision.
//!
//! Nobody needs nix, and no path here runs it. CI takes the paths from cache.nixos.org itself
//! (`ra-jit closure fetch`: each narinfo's signature checked against the cache's key, each NAR
//! against its NarHash, the NAR read by blaze) into blaze's store, as ordinary entries that travel
//! through the remote cache. Users take them from there (`ra-jit closure materialize`), never
//! from cache.nixos.org, and lay each out in `~/.blaze/nix/<hash>-<name>`, checked again against
//! its NarHash, where a build script's namespace mounts it at `/nix/store/<hash>-<name>` so that
//! the paths nixpkgs wrote into its programs resolve.

mod fetch;
mod kept;
mod nar;
mod narinfo;

use std::{path::PathBuf, sync::Mutex, time::Instant};

use anyhow::{Context as _, bail};
use ra_jit_store::Store;

pub(crate) use narinfo::Sha256;

/// The nixpkgs revision and the roots of each host: written by `harness/closure_pins.py`.
const PINS: &str = include_str!("pins.txt");

/// The project of blaze's remote cache the closure is kept in, for every project to take from.
pub(crate) const PROJECT: &str = "nix-closure";

/// What the store is told the toolchain is when nothing else told it first. The closure's
/// entries are lasting ones, the same for every toolchain, so it only has to be something.
const KEPT_AS: &str = "nix closure";

/// How many store paths are laid out at once.
const LAYING_OUT: usize = 4;

/// A store path, with what its NAR comes to: what names its entries in blaze's store.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct StorePath {
    /// The whole path, `/nix/store/<hash>-<name>`.
    pub(crate) path: String,
    pub(crate) nar_hash: Sha256,
    pub(crate) nar_size: u64,
}

/// A store path pins.txt names, by the nixpkgs attribute it is the output of.
#[derive(Clone, Debug)]
pub(crate) struct Root {
    /// `openssl.dev`, `clang`.
    pub(crate) attribute: &'static str,
    pub(crate) path: StorePath,
    /// What its closure, itself included, comes to as NARs.
    pub(crate) closure_bytes: u64,
    pub(crate) closure_paths: u64,
}

/// The nixpkgs commit the roots are of.
pub(crate) fn nixpkgs() -> &'static str {
    PINS.lines()
        .find_map(|it| it.strip_prefix("nixpkgs "))
        .map(str::trim)
        .expect("pins.txt names nixpkgs: harness/closure_pins.py --write")
}

/// This machine as nixpkgs names a system, where pins.txt has roots for it.
pub(crate) fn host() -> Option<&'static str> {
    Some(match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-linux",
        ("aarch64", "linux") => "aarch64-linux",
        _ => return None,
    })
}

/// The roots pins.txt has for `host` (`aarch64-linux`), in its order; none for another.
pub(crate) fn roots(host: &str) -> Vec<Root> {
    read_roots(PINS, host).expect("pins.txt is as harness/closure_pins.py writes it")
}

fn read_roots(pins: &'static str, host: &str) -> anyhow::Result<Vec<Root>> {
    let mut roots = Vec::new();
    for line in pins.lines().filter(|it| !it.starts_with('#')) {
        let words: Vec<&'static str> = line.split_whitespace().collect();
        let [of, attribute, path, nar_hash, nar_size, closure_bytes, closure_paths] = words[..]
        else {
            continue;
        };
        if of != host {
            continue;
        }
        narinfo::base_name(path)?;
        let number = |it: &str| it.parse::<u64>().with_context(|| format!("`{line}`"));
        let path = StorePath {
            path: path.to_owned(),
            nar_hash: Sha256::parse(nar_hash)?,
            nar_size: number(nar_size)?,
        };
        let (closure_bytes, closure_paths) = (number(closure_bytes)?, number(closure_paths)?);
        roots.push(Root { attribute, path, closure_bytes, closure_paths });
    }
    Ok(roots)
}

/// Where blaze lays out the store paths it materializes: `~/.blaze/nix`.
pub(crate) fn dir() -> Option<PathBuf> {
    Some(super::toolchain::home()?.join("nix"))
}

/// The store of this process, opened as the daemon's would be if nothing opened it first.
fn store() -> anyhow::Result<&'static Store> {
    ra_jit_store::compiles_against(KEPT_AS);
    let store = Store::global_or(super::daemon::store().as_deref());
    store.context("there is no store to keep or take the closure in: BLAZE_CACHE turns it off")
}

/// Has blaze's store see what other processes put in it since this one opened it.
pub(crate) fn refresh() -> anyhow::Result<()> {
    store()?.refresh();
    Ok(())
}

/// Every store path `roots` refer to, themselves included, each once and sorted by path, from
/// blaze's store and the remote it takes what it lacks from: each with its NarHash, which is
/// what a key of what runs with them has to name.
pub(crate) fn closure_of(roots: &[StorePath]) -> anyhow::Result<Vec<StorePath>> {
    kept::closure_of(store()?, roots)
}

/// Lays out `path` from blaze's store, unless it is laid out, and returns where:
/// `~/.blaze/nix/<hash>-<name>`, read-only, its NAR the one its NarHash names. A path the
/// store lacks and no remote brings is an error that says CI fills the remote with it.
pub(crate) fn materialize(path: &StorePath) -> anyhow::Result<PathBuf> {
    let into = dir().context("blaze has no directory: neither BLAZE_HOME nor HOME is set")?;
    kept::materialize(store()?, path, &into)
}

const USAGE: &str = "usage: ra-jit closure fetch|materialize [--host <host>] [<root>...]
  fetch         takes the roots' closure from cache.nixos.org into blaze's store, each NAR
                checked against the NarHash cache.nixos.org signs: what CI does, before
                `ra-jit cache push <remote> nix-closure`
  materialize   lays the closure out in ~/.blaze/nix from blaze's store, taking what it
                lacks from RA_JIT_REMOTE, and prints each store path and its directory
  A root is an attribute pins.txt names (`openssl.dev`) or its store path; with none, every
  root of the host. The host is this machine's (x86_64-linux, aarch64-linux) unless given.";

/// `ra-jit closure fetch|materialize [--host <host>] [<root>...]`.
pub(crate) fn command(args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let mut args = args;
    let verb = args.next().context(USAGE)?;
    let mut host = None;
    let mut named = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = Some(args.next().context(USAGE)?),
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if arg.starts_with('-') => bail!("`{arg}` is not an option of closure\n{USAGE}"),
            _ => named.push(arg),
        }
    }
    let host = match host {
        Some(host) => host,
        None => host_or_none()?.to_owned(),
    };
    let pinned = roots(&host);
    if pinned.is_empty() {
        bail!("pins.txt has no roots for {host}: it has x86_64-linux and aarch64-linux");
    }
    let mut chosen = Vec::new();
    for name in &named {
        let root = pinned.iter().find(|it| it.attribute == name.as_str() || it.path.path == *name);
        let root = root.with_context(|| {
            let names: Vec<&str> = pinned.iter().map(|it| it.attribute).collect();
            format!("pins.txt names no root `{name}` for {host}: it has {}", names.join(", "))
        })?;
        chosen.push(root.clone());
    }
    if named.is_empty() {
        chosen = pinned;
    }
    match verb.as_str() {
        "fetch" => fetch_command(&host, &chosen),
        "materialize" => {
            materialize_command(&chosen.into_iter().map(|it| it.path).collect::<Vec<_>>())
        }
        _ => bail!(USAGE),
    }
}

fn host_or_none() -> anyhow::Result<&'static str> {
    host().context("pins.txt has closures for Linux hosts only: name one with --host")
}

/// Fetches the closure of `roots`, and checks that what came is the closure pins.txt says each
/// root has: a cache that answers for another revision is found here.
fn fetch_command(host: &str, roots: &[Root]) -> anyhow::Result<()> {
    let start = Instant::now();
    let source = fetch::Source::from_env()?;
    let store = store()?;
    let paths: Vec<StorePath> = roots.iter().map(|it| it.path.clone()).collect();
    let fetched = fetch::fetch(store, &paths, &source, &[narinfo::CACHE_NIXOS_ORG])?;
    for root in roots {
        let closure = kept::closure_of(store, std::slice::from_ref(&root.path))?;
        let bytes: u64 = closure.iter().map(|it| it.nar_size).sum();
        if (bytes, closure.len() as u64) != (root.closure_bytes, root.closure_paths) {
            bail!(
                "the closure of {} is {} store paths of {bytes} bytes, and pins.txt says {} of \
                 {}: harness/closure_pins.py --write writes what the cache has",
                root.attribute,
                closure.len(),
                root.closure_paths,
                root.closure_bytes
            );
        }
    }
    eprintln!(
        "closure: {} store paths of {host} at nixpkgs {}, {} of them in the store already: {} \
         downloaded, {} unpacked, in {:.1?}",
        fetched.paths,
        &nixpkgs()[..12],
        fetched.had,
        megabytes(fetched.file_bytes),
        megabytes(fetched.nar_bytes),
        start.elapsed()
    );
    Ok(())
}

fn materialize_command(roots: &[StorePath]) -> anyhow::Result<()> {
    let start = Instant::now();
    if let Ok(spec) = std::env::var("RA_JIT_REMOTE")
        && !spec.is_empty()
        && spec != "off"
    {
        let dir = super::daemon::store().context("BLAZE_CACHE turns the store off")?;
        crate::remote::Shared::lazily(&dir, &spec, PROJECT)?.pull()?;
    }
    let closure = closure_of(roots)?;
    let next = Mutex::new(closure.iter());
    let laid_out = Mutex::new(Vec::new());
    let failed = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..LAYING_OUT {
            scope.spawn(|| {
                while let Some(path) = next.lock().ok().and_then(|mut it| it.next()) {
                    if failed.lock().is_ok_and(|it| it.is_some()) {
                        return;
                    }
                    match materialize(path) {
                        Ok(dir) => {
                            laid_out.lock().unwrap_or_else(|it| it.into_inner()).push((path, dir))
                        }
                        Err(e) => {
                            if let Ok(mut failed) = failed.lock() {
                                failed.get_or_insert(e);
                            }
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap_or_else(|it| it.into_inner()) {
        return Err(e);
    }
    let mut laid_out = laid_out.into_inner().unwrap_or_default();
    laid_out.sort();
    for (path, dir) in &laid_out {
        println!("{} {}", path.path, dir.display());
    }
    let bytes: u64 = closure.iter().map(|it| it.nar_size).sum();
    eprintln!(
        "closure: {} store paths, {}, in place in {:.1?}",
        closure.len(),
        megabytes(bytes),
        start.elapsed()
    );
    Ok(())
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, os::unix::fs::PermissionsExt as _, path::Path};

    use base64::Engine as _;
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    #[test]
    fn pins_have_every_root_of_both_hosts() {
        assert_eq!(nixpkgs().len(), 40);
        let (x86, arm) = (roots("x86_64-linux"), roots("aarch64-linux"));
        assert!(roots("x86_64-darwin").is_empty());
        let names = |roots: &[Root]| roots.iter().map(|it| it.attribute).collect::<Vec<_>>();
        assert_eq!(names(&x86), names(&arm));
        for wanted in
            ["clang", "lld", "glibc.dev", "pkg-config", "bash", "openssl.dev", "protobuf.out"]
        {
            assert!(names(&x86).contains(&wanted), "{wanted}");
        }
        for root in x86.iter().chain(&arm) {
            assert!(root.closure_bytes >= root.path.nar_size && root.closure_paths >= 1);
        }
    }

    /// A store path to fetch: its tree, what it refers to, and its NAR as a cache serves it.
    struct Made {
        path: StorePath,
        references: Vec<String>,
        nar: Vec<u8>,
    }

    fn store_path(name: &str) -> String {
        let hash = narinfo::base32(&Sha256::of(name.as_bytes()).0[..20]);
        format!("{}{hash}-{name}", narinfo::STORE)
    }

    fn made(tree: &Path, path: String, references: Vec<String>) -> Made {
        let mut nar = Vec::new();
        nar::write(tree, &mut nar).unwrap();
        let of = StorePath { path, nar_hash: Sha256::of(&nar), nar_size: nar.len() as u64 };
        Made { path: of, references, nar }
    }

    /// Writes `made` into `cache` as a binary cache has it, signed by `key`, compressed with
    /// zstd or xz.
    fn serve(cache: &Path, made: &Made, key: &SigningKey, xz: bool) {
        let (file, compression) = match xz {
            true => {
                let mut out = lzma_rust2::XzWriter::new(Vec::new(), Default::default()).unwrap();
                out.write_all(&made.nar).unwrap();
                (out.finish().unwrap(), "xz")
            }
            false => (zstd::encode_all(&made.nar[..], 3).unwrap(), "zstd"),
        };
        let name = format!(
            "{}.nar.{}",
            narinfo::base32(&Sha256::of(&file).0),
            if xz { "xz" } else { "zst" }
        );
        std::fs::create_dir_all(cache.join("nar")).unwrap();
        std::fs::write(cache.join("nar").join(&name), &file).unwrap();
        let mut references = made.references.clone();
        references.sort();
        let fingerprint = format!(
            "1;{};{};{};{}",
            made.path.path,
            made.path.nar_hash,
            made.path.nar_size,
            references.join(",")
        );
        let sig = base64::engine::general_purpose::STANDARD
            .encode(key.sign(fingerprint.as_bytes()).to_bytes());
        let bases: Vec<&str> = references.iter().map(|it| &it[narinfo::STORE.len()..]).collect();
        let info = format!(
            "StorePath: {}\nURL: nar/{name}\nCompression: {compression}\nFileHash: {}\nFileSize: {}\nNarHash: {}\nNarSize: {}\nReferences: {}\nSig: test-1:{sig}\n",
            made.path.path,
            Sha256::of(&file),
            file.len(),
            made.path.nar_hash,
            made.path.nar_size,
            bases.join(" "),
        );
        let base = &made.path.path[narinfo::STORE.len()..];
        std::fs::write(cache.join(format!("{}.narinfo", &base[..32])), info).unwrap();
    }

    fn public(key: &SigningKey) -> String {
        let public =
            base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());
        format!("test-1:{public}")
    }

    /// Two store paths, `app` referring to itself and to `lib`, with a program, a file of
    /// several pieces, an empty file and links among them.
    fn closure(work: &Path) -> (Made, Made) {
        let (lib, app) = (store_path("lib-1.0"), store_path("app-1.0"));
        let tree = work.join("lib-tree");
        std::fs::create_dir_all(tree.join("lib")).unwrap();
        let large: Vec<u8> =
            (0..kept::PIECE_MAX * 2 + 12345).map(|it| (it * 7 % 251) as u8).collect();
        std::fs::write(tree.join("lib/libbig.so"), &large).unwrap();
        std::fs::write(tree.join("lib/empty"), "").unwrap();
        let lib = made(&tree, lib, Vec::new());
        let tree = work.join("app-tree");
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        std::fs::write(tree.join("bin/app"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(tree.join("bin/app"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::os::unix::fs::symlink(format!("{}/lib", lib.path.path), tree.join("lib")).unwrap();
        std::os::unix::fs::symlink("app", tree.join("bin/app-again")).unwrap();
        let app = made(&tree, app.clone(), vec![app, lib.path.path.clone()]);
        (app, lib)
    }

    fn store_at(dir: &Path) -> Store {
        ra_jit_store::compiles_against(KEPT_AS);
        Store::open_at(dir).unwrap()
    }

    #[test]
    fn fetched_pushed_pulled_and_laid_out() {
        let work = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[7; 32]);
        let (app, lib) = closure(work.path());
        let cache = work.path().join("cache");
        serve(&cache, &app, &key, false);
        serve(&cache, &lib, &key, true);
        let source = fetch::Source::Dir(cache.clone());
        let filled = store_at(&work.path().join("filled"));
        let roots = [app.path.clone()];
        let other = SigningKey::from_bytes(&[8; 32]);
        let said = fetch::fetch(&filled, &roots, &source, &[public(&other).as_str()]).unwrap_err();
        assert!(format!("{said:#}").contains("no signature by test-1"), "{said:#}");
        let fetched = fetch::fetch(&filled, &roots, &source, &[public(&key).as_str()]).unwrap();
        assert_eq!((fetched.paths, fetched.had), (2, 0));
        assert_eq!(fetched.nar_bytes, app.path.nar_size + lib.path.nar_size);
        // What the store has is not downloaded again: the cache can be gone.
        std::fs::remove_dir_all(&cache).unwrap();
        let again = fetch::fetch(&filled, &roots, &source, &[public(&key).as_str()]).unwrap();
        assert_eq!((again.paths, again.had, again.file_bytes), (2, 2, 0));

        let (remote, _remote_dir) = crate::remote::tests::unsigned("closure", 1 << 30);
        let pushed = crate::remote::tests::wait(crate::remote::push(
            &work.path().join("filled"),
            &remote,
            PROJECT,
        ));
        pushed.unwrap();
        let taking = work.path().join("taking");
        let empty = store_at(&taking);
        let laid = work.path().join("nix");
        let missing = kept::materialize(&empty, &app.path, &laid).unwrap_err();
        assert!(missing.is::<kept::Missing>(), "{missing:#}");
        assert!(missing.to_string().contains("CI fills"), "{missing}");
        crate::remote::tests::wait(crate::remote::pull(&taking, &remote, PROJECT)).unwrap();
        empty.refresh();
        let closure = kept::closure_of(&empty, &roots).unwrap();
        let mut both = vec![app.path.clone(), lib.path.clone()];
        both.sort();
        assert_eq!(closure, both);
        let dir = kept::materialize(&empty, &app.path, &laid).unwrap();
        assert_eq!(dir, laid.join(&app.path.path[narinfo::STORE.len()..]));
        assert_eq!(nar::hash_of(&dir).unwrap(), (app.path.nar_hash, app.path.nar_size));
        assert_eq!(std::fs::read_to_string(dir.join("bin/app")).unwrap(), "#!/bin/sh\necho hi\n");
        let mode =
            |path: &Path| std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&dir.join("bin/app"))), (0o555, 0o555));
        assert_eq!(
            std::fs::read_link(dir.join("lib")).unwrap(),
            Path::new(&format!("{}/lib", lib.path.path))
        );
        let lib_dir = kept::materialize(&empty, &lib.path, &laid).unwrap();
        assert_eq!(mode(&lib_dir.join("lib/libbig.so")), 0o444);
        assert_eq!(
            std::fs::metadata(lib_dir.join("lib/libbig.so")).unwrap().len(),
            (kept::PIECE_MAX * 2 + 12345) as u64
        );
        // Laid out, it is not laid out again.
        assert_eq!(kept::materialize(&empty, &app.path, &laid).unwrap(), dir);
        let left: Vec<String> = std::fs::read_dir(&laid)
            .unwrap()
            .map(|it| it.unwrap().file_name().into_string().unwrap())
            .filter(|it| it.starts_with('.') && !it.ends_with(".lock"))
            .collect();
        assert!(left.is_empty(), "nothing of the laying out is left beside it: {left:?}");
        kept::remove(&laid).unwrap();
    }

    #[test]
    fn a_damaged_entry_is_not_put_in_place() {
        let work = tempfile::tempdir().unwrap();
        let tree = work.path().join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("a"), "right").unwrap();
        let made = made(&tree, store_path("damaged-1"), Vec::new());
        let store = store_at(&work.path().join("store"));
        let mut keeping = kept::Keeping::new(&store);
        let mut wrong = Vec::new();
        std::fs::write(tree.join("a"), "wrong").unwrap();
        nar::write(&tree, &mut wrong).unwrap();
        nar::read(&wrong[..], &mut keeping).unwrap();
        keeping.finish(made.path.clone(), Vec::new());
        store.seal();
        let laid = work.path().join("nix");
        let said = kept::materialize(&store, &made.path, &laid).unwrap_err().to_string();
        assert!(said.contains("is not the one cache.nixos.org signed"), "{said}");
        assert!(!laid.join(&made.path.path[narinfo::STORE.len()..]).exists());
        assert!(
            !laid.join(format!(".{}.unpacking", &made.path.path[narinfo::STORE.len()..])).exists()
        );
    }

    #[test]
    fn the_real_nar_is_kept_and_laid_out_as_signed() {
        let (info, nar) = nar::tests::real();
        let work = tempfile::tempdir().unwrap();
        let store = store_at(&work.path().join("store"));
        let mut keeping = kept::Keeping::new(&store);
        nar::read(&nar[..], &mut keeping).unwrap();
        let path = StorePath {
            path: info.store_path.clone(),
            nar_hash: info.nar_hash,
            nar_size: info.nar_size,
        };
        keeping.finish(path.clone(), Vec::new());
        store.seal();
        let dir = kept::materialize(&store, &path, &work.path().join("nix")).unwrap();
        assert_eq!(nar::hash_of(&dir).unwrap(), (info.nar_hash, info.nar_size));
        kept::remove(&work.path().join("nix")).unwrap();
    }
}
