//! Store paths taken from a nixpkgs binary cache into blaze's store: what CI does, with
//! `ra-jit closure fetch`, and nothing else does. Each narinfo is checked against the cache's
//! key, each NAR against the NarHash it signs, and what is kept is what the NAR held, in the
//! form [`super::kept`] describes. Users take the result from blaze's remote cache.

use std::{
    collections::BTreeMap,
    io::{self, BufReader, Read},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, bail};
use futures_util::{StreamExt as _, stream};
use ra_jit_store::Store;
use tokio::runtime::Runtime;

use super::{
    StorePath,
    kept::{self, Keeping},
    nar::{self, Hashing},
    narinfo::{Compression, NarInfo},
};
use crate::remote::{self, Https, Traffic};

/// nixpkgs' binary cache, which Hydra fills for every revision of nixos-unstable.
const CACHE: &str = "https://cache.nixos.org";

/// How many times a NAR is asked for before the fetch gives up.
const TRIES: u32 = 4;

/// How long a request waits for the cache to answer, or for the next of its bytes.
const PATIENCE: Duration = Duration::from_secs(60);

/// How many NARs are downloaded and read at once, and how many narinfos asked for.
const JOBS: usize = 8;
const ASKED: usize = 32;

/// What a fetch did.
#[derive(Debug, Default)]
pub(crate) struct Fetched {
    /// The store paths of the closure.
    pub(crate) paths: usize,
    /// Of them, those the store had.
    pub(crate) had: usize,
    /// What the NARs downloaded came to, as they were sent and unpacked.
    pub(crate) file_bytes: u64,
    pub(crate) nar_bytes: u64,
}

/// Where narinfos and NARs come from: the cache, or what `RA_JIT_NIX_CACHE` names, an address
/// or a directory laid out as a cache is (`<hash>.narinfo`, `nar/<file>`).
pub(crate) enum Source {
    Web { infos: Https, nars: Https },
    Dir(PathBuf),
}

impl Source {
    pub(crate) fn from_env() -> anyhow::Result<Source> {
        let given = std::env::var("RA_JIT_NIX_CACHE").ok().filter(|it| !it.is_empty());
        if let Some(dir) = given.as_ref().filter(|it| !it.contains("://")) {
            return Ok(Source::Dir(PathBuf::from(dir)));
        }
        let address = given.unwrap_or_else(|| CACHE.to_owned());
        let traffic = Arc::new(Traffic::new(ASKED));
        let infos = url::Url::parse(&address)?;
        let nars = url::Url::parse(&format!("{}/nar", address.trim_end_matches('/')))?;
        Ok(Source::Web {
            infos: Https::open(&infos, PATIENCE, traffic.clone())?,
            nars: Https::open(&nars, PATIENCE, traffic)?,
        })
    }

    fn name(&self) -> String {
        match self {
            Source::Web { infos, .. } => infos.place(),
            Source::Dir(dir) => dir.display().to_string(),
        }
    }

    async fn narinfo(&self, path: &str) -> anyhow::Result<NarInfo> {
        let base = super::narinfo::base_name(path)?;
        let file = format!("{}.narinfo", &base[..32]);
        let text = match self {
            Source::Web { infos, .. } => infos.read_small(&file, 1 << 20).await?.map(|it| it.0),
            Source::Dir(dir) => std::fs::read(dir.join(&file)).ok(),
        };
        let text = text.with_context(|| format!("{} has no narinfo of {path}", self.name()))?;
        let text = String::from_utf8(text).context("its narinfo is not text")?;
        NarInfo::parse(&text).with_context(|| format!("the narinfo of {path} cannot be read"))
    }

    async fn nar(&self, url: &str) -> anyhow::Result<Body> {
        let file = url
            .strip_prefix("nar/")
            .filter(|it| !it.is_empty() && !it.contains(['/', '\\']) && !it.starts_with('.'));
        let file = file.with_context(|| format!("`{url}` is not where a cache keeps a NAR"))?;
        let missing = || anyhow::anyhow!("{} has no {url}", self.name());
        Ok(match self {
            Source::Web { nars, .. } => {
                Body::Web(nars.read_from(file, 0).await?.ok_or_else(missing)?)
            }
            Source::Dir(dir) => {
                Body::File(std::fs::File::open(dir.join("nar").join(file)).map_err(|_| missing())?)
            }
        })
    }
}

enum Body {
    Web(hyper::body::Incoming),
    File(std::fs::File),
}

/// A body read as one stream, by a thread outside the runtime.
struct Reading<'a> {
    runtime: &'a Runtime,
    body: Body,
    piece: Vec<u8>,
    at: usize,
}

impl Read for Reading<'_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let web = match &mut self.body {
            Body::File(file) => return file.read(into),
            Body::Web(web) => web,
        };
        while self.at == self.piece.len() {
            // The timer needs the runtime when it is made, so it is made inside `block_on`.
            let next = self
                .runtime
                .block_on(async { tokio::time::timeout(PATIENCE, remote::https_next(web)).await });
            match next {
                Err(_) => return Err(io::Error::other(format!("no bytes for {PATIENCE:?}"))),
                Ok(Err(e)) => return Err(io::Error::other(e)),
                Ok(Ok(None)) => return Ok(0),
                Ok(Ok(Some(piece))) => (self.piece, self.at) = (piece, 0),
            }
        }
        let n = into.len().min(self.piece.len() - self.at);
        into[..n].copy_from_slice(&self.piece[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// A store path of the closure: what it is, what it refers to (with what their NARs are, where
/// the store's manifest says), and the narinfo to download it by when the store lacks it.
struct Resolved {
    path: StorePath,
    references: Vec<(String, Option<StorePath>)>,
    info: Option<NarInfo>,
}

/// Takes the closure of `roots` into `store`: what the store has is kept, the rest downloaded
/// from `source`, each narinfo taken only if signed by one of `keys`.
pub(crate) fn fetch(
    store: &Store,
    roots: &[StorePath],
    source: &Source,
    keys: &[&str],
) -> anyhow::Result<Fetched> {
    let runtime = remote::runtime()?;
    let mut known: BTreeMap<String, Resolved> = BTreeMap::new();
    let mut level: Vec<(String, Option<StorePath>)> =
        roots.iter().map(|it| (it.path.clone(), Some(it.clone()))).collect();
    while !level.is_empty() {
        let resolved = runtime.block_on(
            stream::iter(
                level
                    .iter()
                    .map(|(path, expected)| resolve(store, source, keys, path, expected.as_ref())),
            )
            .buffer_unordered(ASKED)
            .collect::<Vec<_>>(),
        );
        let mut next: BTreeMap<String, Option<StorePath>> = BTreeMap::new();
        for resolved in resolved {
            let resolved = resolved?;
            for (reference, expected) in &resolved.references {
                let wanted = next.entry(reference.clone()).or_default();
                if wanted.is_none() {
                    wanted.clone_from(expected);
                }
            }
            known.insert(resolved.path.path.clone(), resolved);
        }
        next.retain(|path, _| !known.contains_key(path));
        level = next.into_iter().collect();
    }
    let mut fetched = Fetched { paths: known.len(), ..Fetched::default() };
    let todo: Vec<&Resolved> = known.values().filter(|it| it.info.is_some()).collect();
    fetched.had = known.len() - todo.len();
    let next = Mutex::new(todo.into_iter());
    let added = Mutex::new(Vec::new());
    let counted = Mutex::new((0, 0));
    let failed = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..JOBS {
            scope.spawn(|| {
                loop {
                    if failed.lock().is_ok_and(|it| it.is_some()) {
                        return;
                    }
                    let Some(resolved) = next.lock().ok().and_then(|mut it| it.next()) else {
                        return;
                    };
                    match download(&runtime, source, store, resolved, &known) {
                        Ok((file, nar, keys)) => {
                            if let Ok(mut counted) = counted.lock() {
                                (counted.0, counted.1) = (counted.0 + file, counted.1 + nar);
                            }
                            added.lock().unwrap_or_else(|it| it.into_inner()).extend(keys);
                        }
                        Err(e) => {
                            let e = e.context(format!("fetching {}", resolved.path.path));
                            if let Ok(mut failed) = failed.lock() {
                                failed.get_or_insert(e);
                            }
                            return;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap_or_else(|it| it.into_inner()) {
        return Err(e);
    }
    (fetched.file_bytes, fetched.nar_bytes) = counted.into_inner().unwrap_or_default();
    store.seal();
    let added = added.into_inner().unwrap_or_default();
    let lost = added.iter().filter(|it| !store.has_lasting(it)).count();
    if lost != 0 {
        bail!(
            "blaze's store did not keep {lost} of the {} entries the fetch added: what it said \
             above is why",
            added.len()
        );
    }
    Ok(fetched)
}

/// What `path` is and refers to: from the store's manifest of it when the store has one, else
/// from its narinfo, which must be signed and say the NAR `expected` names, where pins.txt or
/// the manifest of what refers to it does.
async fn resolve(
    store: &Store,
    source: &Source,
    keys: &[&str],
    path: &str,
    expected: Option<&StorePath>,
) -> anyhow::Result<Resolved> {
    let had = |path: &StorePath| {
        let manifest = kept::manifest(store, path).ok()?;
        let references =
            manifest.references.into_iter().map(|it| (it.path.clone(), Some(it))).collect();
        Some(Resolved { path: path.clone(), references, info: None })
    };
    if let Some(had) = expected.and_then(had) {
        return Ok(had);
    }
    let info = source.narinfo(path).await?;
    let checked: Vec<anyhow::Result<()>> = keys.iter().map(|key| info.check(key)).collect();
    if !checked.iter().any(Result::is_ok) {
        let refused = checked.into_iter().find_map(Result::err);
        return Err(refused.unwrap_or_else(|| anyhow::anyhow!("no key to check {path} by")));
    }
    if info.store_path != path {
        bail!("{}'s narinfo of {path} is of {}", source.name(), info.store_path);
    }
    let of = StorePath { path: path.to_owned(), nar_hash: info.nar_hash, nar_size: info.nar_size };
    if let Some(expected) = expected
        && *expected != of
    {
        bail!(
            "{}'s narinfo of {path} says its NAR is {} bytes of {}, where {} of {} was expected",
            source.name(),
            of.nar_size,
            of.nar_hash,
            expected.nar_size,
            expected.nar_hash
        );
    }
    if let Some(had) = had(&of) {
        return Ok(had);
    }
    let references = info.references.iter().map(|it| (it.clone(), None)).collect();
    Ok(Resolved { path: of, references, info: Some(info) })
}

/// Downloads the NAR of `resolved` and keeps what it holds, trying again where that fails.
/// The result is the bytes downloaded and unpacked, and the keys added.
fn download(
    runtime: &Runtime,
    source: &Source,
    store: &Store,
    resolved: &Resolved,
    known: &BTreeMap<String, Resolved>,
) -> anyhow::Result<(u64, u64, Vec<[u8; 32]>)> {
    let info = resolved.info.as_ref().context("nothing to download")?;
    let references = info.references.iter().map(|it| match known.get(it) {
        Some(it) => Ok(it.path.clone()),
        None => bail!("{} refers to {it}, which the fetch did not resolve", info.store_path),
    });
    let references = references.collect::<anyhow::Result<Vec<_>>>()?;
    let mut tries = 0;
    loop {
        match download_once(runtime, source, store, info) {
            Ok((file, nar, keeping)) => {
                return Ok((file, nar, keeping.finish(resolved.path.clone(), references)));
            }
            Err(e) if tries + 1 == TRIES => return Err(e),
            Err(e) => {
                eprintln!("closure: {}: {e:#}; trying again", info.store_path);
                tries += 1;
                std::thread::sleep(Duration::from_secs(tries.into()));
            }
        }
    }
}

fn download_once<'a>(
    runtime: &Runtime,
    source: &Source,
    store: &'a Store,
    info: &NarInfo,
) -> anyhow::Result<(u64, u64, Keeping<'a>)> {
    let body = runtime.block_on(source.nar(&info.url))?;
    let mut file = Hashing::new(Reading { runtime, body, piece: Vec::new(), at: 0 });
    let mut keeping = Keeping::new(store);
    let unpacked = {
        let decompressed: Box<dyn Read + '_> = match info.compression {
            Compression::Xz => Box::new(lzma_rust2::XzReader::new(
                BufReader::with_capacity(1 << 20, &mut file),
                false,
            )),
            Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(&mut file)?),
            Compression::None => Box::new(&mut file),
        };
        let mut nar = Hashing::new(decompressed);
        nar::read(BufReader::with_capacity(1 << 20, &mut nar), &mut keeping)
            .with_context(|| format!("the NAR of {}", info.store_path))?;
        io::copy(&mut nar, &mut io::sink())?;
        if (nar.hash(), nar.len) != (info.nar_hash, info.nar_size) {
            bail!(
                "its NAR is {} bytes of {}, and its narinfo signs {} of {}",
                nar.len,
                nar.hash(),
                info.nar_size,
                info.nar_hash
            );
        }
        nar.len
    };
    io::copy(&mut file, &mut io::sink())?;
    let checks = [
        info.file_hash.is_none_or(|it| it == file.hash()),
        info.file_size.is_none_or(|it| it == file.len),
    ];
    if checks.contains(&false) {
        bail!("its file {} is not the one its narinfo names", info.url);
    }
    Ok((file.len, unpacked, keeping))
}
