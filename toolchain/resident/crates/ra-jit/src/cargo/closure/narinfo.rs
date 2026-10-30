//! What a nixpkgs binary cache says of a store path: its `.narinfo`, the hash of its NAR in
//! nix's own base32, and the Ed25519 signature that makes the rest worth believing.

use std::fmt;

use anyhow::{Context as _, bail};
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};

/// The key cache.nixos.org signs with, as nix's `trusted-public-keys` names it.
pub(crate) const CACHE_NIXOS_ORG: &str =
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=";

/// Where every store path of nixpkgs is, and so the only place one can be.
pub(crate) const STORE: &str = "/nix/store/";

/// nix's base32: its own alphabet, with no `e`, `o`, `t` or `u`, and the bytes taken from the
/// last one, so that the first digit written is of the hash's last bits.
const ALPHABET: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

pub(crate) fn base32(bytes: &[u8]) -> String {
    let len = (bytes.len() * 8).div_ceil(5);
    (0..len)
        .rev()
        .map(|n| {
            let (i, j) = (n * 5 / 8, n * 5 % 8);
            let high = bytes.get(i + 1).map_or(0, |&it| u16::from(it) << (8 - j));
            let digit = (u16::from(bytes[i]) >> j | high) & 0x1f;
            char::from(ALPHABET[digit as usize])
        })
        .collect()
}

/// The `N` bytes `text` writes in nix's base32, or `None` if it writes no `N` bytes.
pub(crate) fn from_base32<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != (N * 8).div_ceil(5) {
        return None;
    }
    let mut bytes = [0; N];
    for (n, c) in text.bytes().rev().enumerate() {
        let digit = ALPHABET.iter().position(|&it| it == c)? as u16;
        let (i, j) = (n * 5 / 8, n * 5 % 8);
        bytes[i] |= (digit << j) as u8;
        let carry = digit >> (8 - j);
        match bytes.get_mut(i + 1) {
            Some(next) => *next |= carry as u8,
            None if carry != 0 => return None,
            None => {}
        }
    }
    Some(bytes)
}

/// A sha256, the one hash a NAR or a file of a cache is named by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Sha256(pub(crate) [u8; 32]);

impl Sha256 {
    /// `sha256:` and the hash in nix's base32 or in hex, as a narinfo writes it.
    pub(crate) fn parse(text: &str) -> anyhow::Result<Sha256> {
        let hash =
            text.strip_prefix("sha256:").with_context(|| format!("`{text}` is no sha256"))?;
        let hex = || -> Option<[u8; 32]> {
            let pairs = hash.as_bytes().chunks(2);
            let bytes = pairs.map(|it| u8::from_str_radix(std::str::from_utf8(it).ok()?, 16).ok());
            bytes.collect::<Option<Vec<u8>>>()?.try_into().ok()
        };
        let bytes = match hash.len() {
            64 => hex(),
            _ => from_base32(hash),
        };
        bytes.map(Sha256).with_context(|| format!("`{text}` is no sha256"))
    }

    #[cfg(test)]
    pub(crate) fn of(bytes: &[u8]) -> Sha256 {
        use sha2::Digest as _;
        Sha256(sha2::Sha256::digest(bytes).into())
    }
}

impl fmt::Display for Sha256 {
    /// As nix writes it in a fingerprint: `sha256:` and its base32.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", base32(&self.0))
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// How the file of a NAR is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Compression {
    Xz,
    Zstd,
    None,
}

/// A `.narinfo`, with only what blaze reads of it.
#[derive(Debug)]
pub(crate) struct NarInfo {
    /// The whole path: `/nix/store/<hash>-<name>`.
    pub(crate) store_path: String,
    /// The file of the NAR, relative to the cache: `nar/<hash>.nar.zst`.
    pub(crate) url: String,
    pub(crate) compression: Compression,
    /// Of the file, compressed, when the cache says.
    pub(crate) file_hash: Option<Sha256>,
    pub(crate) file_size: Option<u64>,
    pub(crate) nar_hash: Sha256,
    pub(crate) nar_size: u64,
    /// The whole paths, in nix's order: sorted, as a set of store paths is.
    pub(crate) references: Vec<String>,
    /// Each `Sig:` line: the name of the key and the signature.
    sigs: Vec<(String, String)>,
}

impl NarInfo {
    pub(crate) fn parse(text: &str) -> anyhow::Result<NarInfo> {
        let mut fields = std::collections::HashMap::new();
        let mut sigs = Vec::new();
        for line in text.lines().filter(|it| !it.is_empty()) {
            let (name, value) = line.split_once(": ").with_context(|| format!("`{line}`"))?;
            if name == "Sig" {
                let (key, sig) = value.split_once(':').with_context(|| format!("`{line}`"))?;
                sigs.push((key.to_owned(), sig.to_owned()));
            } else if fields.insert(name, value).is_some() {
                bail!("it says {name} twice");
            }
        }
        let field = |name: &str| fields.get(name).copied().with_context(|| format!("no {name}"));
        let size = |name: &str| -> anyhow::Result<u64> {
            field(name)?.parse().with_context(|| format!("its {name} is no number"))
        };
        let store_path = field("StorePath")?.to_owned();
        let compression = match field("Compression")? {
            "xz" => Compression::Xz,
            "zstd" => Compression::Zstd,
            "none" => Compression::None,
            other => bail!("its NAR is compressed with {other}, which blaze does not read"),
        };
        let mut references: Vec<String> = match fields.get("References") {
            Some(it) => it.split_whitespace().map(|it| format!("{STORE}{it}")).collect(),
            None => Vec::new(),
        };
        references.sort();
        for path in references.iter().chain([&store_path]) {
            base_name(path)?;
        }
        Ok(NarInfo {
            store_path,
            url: field("URL")?.to_owned(),
            compression,
            file_hash: fields.get("FileHash").map(|it| Sha256::parse(it)).transpose()?,
            file_size: fields.get("FileSize").map(|_| size("FileSize")).transpose()?,
            nar_hash: Sha256::parse(field("NarHash")?)?,
            nar_size: size("NarSize")?,
            references,
            sigs,
        })
    }

    /// What the cache signs: nix's fingerprint of the path, its NAR and its references.
    pub(crate) fn fingerprint(&self) -> String {
        format!(
            "1;{};{};{};{}",
            self.store_path,
            self.nar_hash,
            self.nar_size,
            self.references.join(",")
        )
    }

    /// Whether one of its signatures is by `key` (`name:base64`) and holds for what it says.
    pub(crate) fn check(&self, key: &str) -> anyhow::Result<()> {
        let (name, public) = key.split_once(':').context("a key is `name:base64`")?;
        let public: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(public)
            .ok()
            .and_then(|it| it.try_into().ok())
            .context("a public key is 32 bytes of base64")?;
        let public = VerifyingKey::from_bytes(&public)?;
        let fingerprint = self.fingerprint();
        let holds = self.sigs.iter().filter(|(by, _)| by == name).any(|(_, sig)| {
            let sig = base64::engine::general_purpose::STANDARD.decode(sig).ok();
            let sig = sig.and_then(|it| <[u8; 64]>::try_from(it).ok());
            sig.is_some_and(|sig| {
                public.verify_strict(fingerprint.as_bytes(), &Signature::from_bytes(&sig)).is_ok()
            })
        });
        if !holds {
            bail!(
                "the narinfo of {} has no signature by {name} that holds for what it says",
                self.store_path
            );
        }
        Ok(())
    }
}

/// `<hash>-<name>` of the path `/nix/store/<hash>-<name>`, checked to be one: a 32-digit base32
/// hash, then a name of the characters nix allows, which never makes it leave a directory.
pub(crate) fn base_name(path: &str) -> anyhow::Result<&str> {
    let not = || format!("`{path}` is not a store path");
    let base = path.strip_prefix(STORE).with_context(not)?;
    let (hash, name) = base.split_at_checked(32).with_context(not)?;
    let allowed = |c: u8| c.is_ascii_alphanumeric() || b"+-._?=".contains(&c);
    let ok = hash.bytes().all(|it| ALPHABET.contains(&it))
        && name.strip_prefix('-').is_some_and(|it| !it.is_empty() && !it.starts_with('.'))
        && name.bytes().all(allowed);
    if !ok {
        bail!(not());
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NARINFO: &str = include_str!("testdata/pkg-config-wrapper.narinfo");

    #[test]
    fn base32_is_nixs() {
        // As `nix hash convert --to nix32` writes them.
        let cases: [(&[u8], &str); 2] = [
            (b"abc", "1b8m03r63zqhnjf7l5wnldhh7c134ap5vpj0850ymkq1iyzicy5s"),
            (b"", "0mdqa9w1p6cmli6976v4wi0sw9r4p5prkj7lzfd1877wk11c9c73"),
        ];
        for (bytes, written) in cases {
            let hash = Sha256::of(bytes);
            assert_eq!(base32(&hash.0), written);
            assert_eq!(from_base32::<32>(written), Some(hash.0));
        }
        // sha1("abc"), 20 bytes: the last digit takes only some bits.
        let sha1: [u8; 20] = [
            0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
            0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
        ];
        assert_eq!(base32(&sha1), "kpcd173cq987hw957sx6m0868wv3x6d9");
        assert_eq!(from_base32::<20>("kpcd173cq987hw957sx6m0868wv3x6d9"), Some(sha1));
        let mut bytes = [0u8; 32];
        for at in 0..32 {
            bytes[at] = (at as u8).wrapping_mul(37) ^ 0x5a;
            assert_eq!(from_base32::<32>(&base32(&bytes)), Some(bytes));
        }
        // Digits outside the alphabet, the wrong length, and bits past the last byte.
        assert_eq!(from_base32::<32>("e".repeat(52).as_str()), None);
        assert_eq!(from_base32::<32>("0".repeat(51).as_str()), None);
        assert_eq!(from_base32::<32>(&format!("z{}", "0".repeat(51))), None);
        let hex = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(Sha256::parse(hex).unwrap(), Sha256::of(b"abc"));
    }

    #[test]
    fn a_narinfo_of_cache_nixos_org_holds() {
        let info = NarInfo::parse(NARINFO).unwrap();
        assert_eq!(
            info.store_path,
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-pkg-config-wrapper-0.29.2"
        );
        assert_eq!(info.compression, Compression::Zstd);
        assert_eq!(info.nar_size, 11888);
        assert_eq!(info.references.len(), 5);
        assert!(info.references.contains(&info.store_path), "it refers to itself");
        info.check(CACHE_NIXOS_ORG).unwrap();
    }

    #[test]
    fn a_changed_narinfo_is_refused() {
        let changed = [
            NARINFO.replace("NarSize: 11888", "NarSize: 11889"),
            NARINFO.replace(
                "NarHash: sha256:195bj6pmhb43j91391j1xnnl7mfzcvigkz824zinxvszmq20ahzz",
                "NarHash: sha256:095bj6pmhb43j91391j1xnnl7mfzcvigkz824zinxvszmq20ahzz",
            ),
            NARINFO.replace(" 89igyzp72y4ky0qyj4x5r52z84w203ki-strip.sh", ""),
            NARINFO.replace("StorePath: /nix/store/ilg26", "StorePath: /nix/store/ilg27"),
            NARINFO.replace("Sig: cache.nixos.org-1:kkDLn7", "Sig: cache.nixos.org-1:kkDLn8"),
            NARINFO.replace("Sig: cache.nixos.org-1:", "Sig: cache.nixos.org-2:"),
        ];
        for text in changed {
            assert_ne!(text, NARINFO);
            let info = NarInfo::parse(&text).unwrap();
            let said = info.check(CACHE_NIXOS_ORG).unwrap_err().to_string();
            assert!(said.contains("no signature by cache.nixos.org-1"), "{said}");
        }
        // Another key signed nothing of it.
        let other = "other-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=";
        assert!(NarInfo::parse(NARINFO).unwrap().check(other).is_err());
    }

    #[test]
    fn store_paths_are_checked() {
        let good = "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-pkg-config-wrapper-0.29.2";
        assert_eq!(base_name(good).unwrap(), &good[STORE.len()..]);
        for bad in [
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s",
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-",
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-a/b",
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-..",
            "/nix/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9e-a",
            "/gnu/store/ilg26l78wi5v6nhdn4ffxba1f4yc1h9s-a",
        ] {
            assert!(base_name(bad).is_err(), "{bad}");
        }
    }
}
