//! NAR, the archive nix keeps a store path's files in, and whose sha256 a narinfo signs: read
//! as it streams in, and written from a tree on disk to check that tree against that hash.
//!
//! A NAR is a series of strings, each its length as eight bytes, little-endian, then its bytes,
//! then zeros to a multiple of eight: `nix-archive-1`, then one node. A node is `(` `type`,
//! then `regular` (with `executable` `""` for a program) `contents` and the file's bytes, or
//! `symlink` `target` and the link, or `directory` and its entries, each `entry` `(` `name`
//! `<name>` `node` `<node>` `)`, sorted by name, and last `)`.

use std::{
    io::{self, Read, Write},
    path::Path,
};

use anyhow::{Context as _, bail};
use sha2::Digest as _;

use super::narinfo::Sha256;

const MAGIC: &str = "nix-archive-1";

/// The longest name of an entry and the longest link target taken: what a path on Linux may
/// be. A NAR that says more is not one blaze could lay out.
const NAME_MAX: u64 = 255;
const TARGET_MAX: u64 = 4096;

/// How deep directories are taken: more is a NAR made to exhaust the reader.
const DEPTH_MAX: usize = 256;

/// What a NAR holds, told as it is read. A path is the entry's names from the root joined by
/// `/`, and empty for the root itself.
pub(crate) trait Sink {
    fn directory(&mut self, path: &[u8]) -> anyhow::Result<()>;
    /// `contents` has the file's `size` bytes; what the sink leaves of them is skipped.
    fn file(
        &mut self,
        path: &[u8],
        executable: bool,
        size: u64,
        contents: &mut dyn Read,
    ) -> anyhow::Result<()>;
    fn symlink(&mut self, path: &[u8], target: &[u8]) -> anyhow::Result<()>;
}

/// Reads the NAR `from` to its end, telling `sink` what it holds. Nothing after the NAR is
/// read: the caller says whether anything is there.
pub(crate) fn read(from: impl Read, sink: &mut impl Sink) -> anyhow::Result<()> {
    let mut nar = Strings { from };
    nar.expect(MAGIC).context("it is not a NAR")?;
    node(&mut nar, sink, &mut Vec::new(), 0)
}

fn node(
    nar: &mut Strings<impl Read>,
    sink: &mut impl Sink,
    path: &mut Vec<u8>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth > DEPTH_MAX {
        bail!("its directories are nested more than {DEPTH_MAX} deep");
    }
    nar.expect("(")?;
    nar.expect("type")?;
    match &nar.word()?[..] {
        b"regular" => {
            let mut word = nar.word()?;
            let executable = word == b"executable";
            if executable {
                nar.expect("")?;
                word = nar.word()?;
            }
            if word != b"contents" {
                bail!("a file of it has `{}` where its contents go", word.escape_ascii());
            }
            let size = nar.number()?;
            let mut contents = (&mut nar.from).take(size);
            sink.file(path, executable, size, &mut contents)?;
            io::copy(&mut contents, &mut io::sink())?;
            if contents.limit() != 0 {
                bail!("it ends in the middle of a file");
            }
            nar.padding(size)?;
        }
        b"symlink" => {
            nar.expect("target")?;
            let target = nar.bytes(TARGET_MAX)?;
            if target.is_empty() || target.contains(&0) {
                bail!("a link of it has no target that is a path");
            }
            sink.symlink(path, &target)?;
        }
        b"directory" => {
            sink.directory(path)?;
            let mut last: Option<Vec<u8>> = None;
            loop {
                match &nar.word()?[..] {
                    b")" => return Ok(()),
                    b"entry" => {}
                    other => bail!("a directory of it has `{}` in it", other.escape_ascii()),
                }
                nar.expect("(")?;
                nar.expect("name")?;
                let name = nar.bytes(NAME_MAX)?;
                let named = !name.is_empty()
                    && name != b"."
                    && name != b".."
                    && !name.contains(&b'/')
                    && !name.contains(&0);
                if !named {
                    bail!("an entry of it is named `{}`, which is no name", name.escape_ascii());
                }
                if last.as_ref().is_some_and(|last| *last >= name) {
                    bail!("the entries of a directory of it are not sorted by name");
                }
                nar.expect("node")?;
                let len = path.len();
                if len != 0 {
                    path.push(b'/');
                }
                path.extend_from_slice(&name);
                node(nar, sink, path, depth + 1)?;
                path.truncate(len);
                nar.expect(")")?;
                last = Some(name);
            }
        }
        other => bail!("a node of it is of the type `{}`", other.escape_ascii()),
    }
    nar.expect(")")
}

/// The strings of a NAR, as they are read.
struct Strings<R> {
    from: R,
}

impl<R: Read> Strings<R> {
    fn number(&mut self) -> anyhow::Result<u64> {
        let mut bytes = [0; 8];
        self.from.read_exact(&mut bytes).context("it ends early")?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn padding(&mut self, len: u64) -> anyhow::Result<()> {
        let mut zeros = [0; 8];
        let pad = &mut zeros[..(len.wrapping_neg() % 8) as usize];
        self.from.read_exact(pad).context("it ends early")?;
        if pad.iter().any(|&it| it != 0) {
            bail!("it is padded with something other than zeros");
        }
        Ok(())
    }

    fn bytes(&mut self, most: u64) -> anyhow::Result<Vec<u8>> {
        let len = self.number()?;
        if len > most {
            bail!("it has a string of {len} bytes where at most {most} go");
        }
        let mut bytes = vec![0; len as usize];
        self.from.read_exact(&mut bytes).context("it ends early")?;
        self.padding(len)?;
        Ok(bytes)
    }

    /// One of the words of the format, which are all short.
    fn word(&mut self) -> anyhow::Result<Vec<u8>> {
        self.bytes(16)
    }

    fn expect(&mut self, word: &str) -> anyhow::Result<()> {
        let read = self.word()?;
        if read != word.as_bytes() {
            bail!("it has `{}` where `{word}` goes", read.escape_ascii());
        }
        Ok(())
    }
}

/// What bytes come to, sha256 and count, as they are read or written.
pub(crate) struct Hashing<T> {
    pub(crate) inner: T,
    hasher: sha2::Sha256,
    pub(crate) len: u64,
}

impl<T> Hashing<T> {
    pub(crate) fn new(inner: T) -> Hashing<T> {
        Hashing { inner, hasher: sha2::Sha256::new(), len: 0 }
    }

    pub(crate) fn hash(&self) -> Sha256 {
        Sha256(self.hasher.clone().finalize().into())
    }
}

impl<T: Read> Read for Hashing<T> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(into)?;
        self.hasher.update(&into[..read]);
        self.len += read as u64;
        Ok(read)
    }
}

impl<T: Write> Write for Hashing<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.len += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The sha256 and size of the NAR of the tree at `path`, as `nix-store --dump` would write it.
pub(crate) fn hash_of(path: &Path) -> io::Result<(Sha256, u64)> {
    let mut out = Hashing::new(io::sink());
    write(path, &mut io::BufWriter::with_capacity(1 << 20, &mut out))?;
    Ok((out.hash(), out.len))
}

/// Writes the NAR of the tree at `path`: a program is a file its owner may run, as nix has it.
pub(crate) fn write(path: &Path, to: &mut impl Write) -> io::Result<()> {
    string(to, MAGIC.as_bytes())?;
    write_node(path, to)?;
    to.flush()
}

fn write_node(path: &Path, to: &mut impl Write) -> io::Result<()> {
    use std::os::unix::{ffi::OsStrExt as _, fs::PermissionsExt as _};
    let meta = std::fs::symlink_metadata(path)?;
    for word in ["(", "type"] {
        string(to, word.as_bytes())?;
    }
    if meta.is_symlink() {
        for word in ["symlink", "target"] {
            string(to, word.as_bytes())?;
        }
        string(to, std::fs::read_link(path)?.as_os_str().as_bytes())?;
    } else if meta.is_dir() {
        string(to, b"directory")?;
        let mut names: Vec<_> = std::fs::read_dir(path)?
            .map(|it| it.map(|it| it.file_name()))
            .collect::<Result<_, _>>()?;
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        for name in names {
            for word in ["entry", "(", "name"] {
                string(to, word.as_bytes())?;
            }
            string(to, name.as_bytes())?;
            string(to, b"node")?;
            write_node(&path.join(&name), to)?;
            string(to, b")")?;
        }
    } else {
        string(to, b"regular")?;
        if meta.permissions().mode() & 0o100 != 0 {
            string(to, b"executable")?;
            string(to, b"")?;
        }
        string(to, b"contents")?;
        to.write_all(&meta.len().to_le_bytes())?;
        let copied = io::copy(&mut std::fs::File::open(path)?, to)?;
        if copied != meta.len() {
            return Err(io::Error::other(format!("{} changed as it was read", path.display())));
        }
        to.write_all(&[0; 8][..(copied.wrapping_neg() % 8) as usize])?;
    }
    string(to, b")")
}

fn string(to: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    to.write_all(&(bytes.len() as u64).to_le_bytes())?;
    to.write_all(bytes)?;
    to.write_all(&[0; 8][..(bytes.len() as u64).wrapping_neg() as usize % 8])
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        os::unix::{ffi::OsStrExt as _, fs::PermissionsExt as _},
        path::PathBuf,
    };

    use super::*;
    use crate::cargo::closure::narinfo::NarInfo;

    /// A NAR laid out on disk, as nix lays one out in its store.
    pub(crate) struct Unpack(pub(crate) PathBuf);

    impl Unpack {
        fn at(&self, path: &[u8]) -> PathBuf {
            match path {
                [] => self.0.clone(),
                path => self.0.join(std::ffi::OsStr::from_bytes(path)),
            }
        }
    }

    impl Sink for Unpack {
        fn directory(&mut self, path: &[u8]) -> anyhow::Result<()> {
            Ok(std::fs::create_dir(self.at(path))?)
        }

        fn file(
            &mut self,
            path: &[u8],
            executable: bool,
            _: u64,
            contents: &mut dyn Read,
        ) -> anyhow::Result<()> {
            let at = self.at(path);
            io::copy(contents, &mut std::fs::File::create(&at)?)?;
            let mode = if executable { 0o755 } else { 0o644 };
            Ok(std::fs::set_permissions(at, std::fs::Permissions::from_mode(mode))?)
        }

        fn symlink(&mut self, path: &[u8], target: &[u8]) -> anyhow::Result<()> {
            Ok(std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), self.at(path))?)
        }
    }

    /// The NAR of the real store path the test narinfo is of.
    pub(crate) fn real() -> (NarInfo, Vec<u8>) {
        let info = NarInfo::parse(include_str!("testdata/pkg-config-wrapper.narinfo")).unwrap();
        let zst = include_bytes!("testdata/pkg-config-wrapper.nar.zst");
        (info, zstd::decode_all(&zst[..]).unwrap())
    }

    #[test]
    fn a_nar_laid_out_and_written_again_is_the_one_signed() {
        let (info, nar) = real();
        assert_eq!((Sha256::of(&nar), nar.len() as u64), (info.nar_hash, info.nar_size));
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("out");
        read(&nar[..], &mut Unpack(root.clone())).unwrap();
        let program = std::fs::metadata(root.join("bin/pkg-config")).unwrap();
        assert_ne!(program.permissions().mode() & 0o100, 0, "a program is one");
        assert_eq!(hash_of(&root).unwrap(), (info.nar_hash, info.nar_size));
        let mut written = Vec::new();
        write(&root, &mut written).unwrap();
        assert!(written == nar, "the same bytes");
    }

    #[test]
    fn a_nar_that_is_not_one_is_refused() {
        let (_, nar) = real();
        let refused = |nar: &[u8]| {
            let work = tempfile::tempdir().unwrap();
            format!("{:#}", read(nar, &mut Unpack(work.path().join("out"))).unwrap_err())
        };
        assert!(refused(&nar[..nar.len() - 8]).contains("ends early"));
        assert!(refused(&nar[8..]).contains("not a NAR") || refused(&nar[8..]).contains("string"));
        let mut padded = nar.clone();
        // The magic is 13 bytes: its padding is the three after them.
        padded[8 + 13] = 1;
        assert!(refused(&padded).contains("padded"));
        let mut escaping = Vec::new();
        for word in [MAGIC, "(", "type", "directory", "entry", "(", "name", "..", "node"] {
            string(&mut escaping, word.as_bytes()).unwrap();
        }
        assert!(refused(&escaping).contains("no name"));
    }
}
