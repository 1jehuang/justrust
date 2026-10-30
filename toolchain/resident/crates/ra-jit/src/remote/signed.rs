//! Who a list on the remote is from.
//!
//! Everything a machine takes from a remote is named in a list by the hash of its bytes, so
//! the lists are the one thing a remote could change without being caught by a hash. A
//! machine that pushes signs its lists with an Ed25519 key, and a machine that names the
//! keys it trusts takes no list that none of them signed. A machine that names none takes
//! no list at all, unless `RA_JIT_REMOTE_UNSIGNED=1` says that whoever writes to the remote
//! may say what this machine runs.
//!
//! A signature does not say that a list is the newest one. Every signed list has a turn, a
//! number inside what is signed that only goes up, and a machine remembers the highest turn
//! it took of each list: an older list, which may name what its signer has since taken out,
//! is not taken again. See [`Turns`].

use std::{
    fs::File,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use ra_jit_store::write_file_whole;
use sha2::{Digest as _, Sha256};

use crate::content::hex;

/// The word the last line of a signed list starts with.
const SIGNED: &str = "signed";

/// The keys of this machine: `RA_JIT_REMOTE_SIGN` names the file of the key it signs with,
/// `RA_JIT_REMOTE_TRUST` lists the public keys it takes lists from, with commas between.
pub(crate) struct Trust {
    signer: Option<SigningKey>,
    trusted: Vec<VerifyingKey>,
    /// Whether a machine that names no keys takes lists: `RA_JIT_REMOTE_UNSIGNED=1`.
    unsigned: bool,
}

/// A signed list that a machine with no key was about to write back.
#[derive(Debug)]
pub(crate) struct NotRewritten(String);

impl std::fmt::Display for NotRewritten {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotRewritten {}

/// A list as a machine took it.
pub(crate) struct Opened<'a> {
    pub(crate) lines: &'a str,
    /// The turn it was signed at, when a signature was what it was taken for.
    pub(crate) turn: Option<u64>,
}

impl Trust {
    pub(crate) fn from_env() -> anyhow::Result<Trust> {
        let signer = match std::env::var_os("RA_JIT_REMOTE_SIGN") {
            Some(file) => Some(read_key(Path::new(&file))?),
            None => None,
        };
        let trusted = std::env::var("RA_JIT_REMOTE_TRUST").unwrap_or_default();
        let trusted = trusted.split(',').filter(|it| !it.is_empty()).map(|key| {
            let bytes = unhex::<32>(key).context("BLAZE_REMOTE_TRUST holds what is not a key")?;
            VerifyingKey::from_bytes(&bytes).context("BLAZE_REMOTE_TRUST holds what is not a key")
        });
        let unsigned = std::env::var_os("RA_JIT_REMOTE_UNSIGNED").is_some_and(|it| it == "1");
        Ok(Trust { signer, trusted: trusted.collect::<anyhow::Result<_>>()?, unsigned })
    }

    /// What takes any list and signs none, whatever the environment says: for a remote that
    /// is as trusted as the program itself, its own release.
    pub(crate) fn of_release() -> Trust {
        Trust { signer: None, trusted: Vec::new(), unsigned: true }
    }

    /// Whether lists are checked at all.
    pub(crate) fn checks(&self) -> bool {
        !self.trusted.is_empty()
    }

    /// Why this machine may not write lists, if it may not: other machines that trust as
    /// this one does would take nothing from an unsigned list.
    pub(crate) fn cannot_sign(&self) -> Option<&'static str> {
        (self.checks() && self.signer.is_none())
            .then_some("BLAZE_REMOTE_TRUST is set and BLAZE_REMOTE_SIGN is not")
    }

    /// Why this machine may not write the list in `text` back, if it may not: the list is
    /// signed and the machine has no key, so what it wrote would be a list with no
    /// signature where the machines that trust the signer look for one.
    pub(crate) fn cannot_rewrite(&self, name: &str, text: &str) -> Result<(), NotRewritten> {
        if self.signer.is_some() || split(text).1.is_empty() {
            return Ok(());
        }
        Err(NotRewritten(format!(
            "`{name}` is signed, and this machine has no key in BLAZE_REMOTE_SIGN: it is left \
             as it is"
        )))
    }

    /// `body` as the object `name` holds it: with the turn and the signature of this
    /// machine's key as its last line, when it has one. The turn is one past `after`, the
    /// highest this machine knows the list to have had, and no less than the time: a list
    /// started again from nothing, or signed by a key that never saw the last one, still
    /// comes after what the machines that read it remember.
    pub(crate) fn seal(&self, name: &str, body: &str, after: u64) -> (String, Option<u64>) {
        let Some(signer) = &self.signer else { return (body.to_owned(), None) };
        let turn = after.saturating_add(1).max(seconds_now());
        (sealed_at(signer, name, body, turn), Some(turn))
    }

    /// The lines of the list in `text`, the object `name`, for a machine that reads it.
    /// With keys to trust, a list that none of them signed for this name is an error, and
    /// with none every list is, unless the machine was told to take them. A machine that
    /// signs takes only what it or a key it trusts signed, whatever it was told: what it
    /// takes it will send again under its own name.
    pub(crate) fn open<'a>(&self, name: &str, text: &'a str) -> anyhow::Result<Opened<'a>> {
        if self.checks() || self.signer.is_some() {
            return self.verify(name, text);
        }
        if !self.unsigned {
            bail!(
                "`{name}` is not taken: BLAZE_REMOTE_TRUST names no key, and without one whoever \
                 writes to the remote says what is run here. Name the keys of the machines that \
                 push, or set BLAZE_REMOTE_UNSIGNED=1 to take what is there"
            );
        }
        Ok(Opened { lines: split(text).0, turn: None })
    }

    /// The lines of the list, for a machine that adds to it and writes it back. One that
    /// signs puts its name to every line, so it takes none it would not have trusted. One
    /// that does not sign takes nothing from the list by adding to it.
    pub(crate) fn open_to_extend<'a>(
        &self,
        name: &str,
        text: &'a str,
    ) -> anyhow::Result<Opened<'a>> {
        if self.signer.is_some() || self.checks() {
            self.verify(name, text)
        } else {
            Ok(Opened { lines: split(text).0, turn: None })
        }
    }

    /// The lines of a list that a trusted key, or this machine's own, signed for `name`.
    fn verify<'a>(&self, name: &str, text: &'a str) -> anyhow::Result<Opened<'a>> {
        let (body, signature) = split(text);
        let fields: Vec<&str> = signature.split(' ').collect();
        let signed = match fields[..] {
            [SIGNED, key, turn, signature] => {
                unhex::<32>(key).zip(turn_of(turn)).zip(unhex::<64>(signature))
            }
            _ => None,
        };
        let Some(((key, turn), signature)) = signed else {
            bail!("`{name}` is not signed");
        };
        let own = self.signer.as_ref().map(|it| it.verifying_key());
        let Some(key) = self.trusted.iter().chain(&own).find(|it| *it.as_bytes() == key) else {
            bail!("`{name}` is signed by {}, which is not a key this machine trusts", hex(&key));
        };
        key.verify_strict(&signed_bytes(name, turn, body), &Signature::from_bytes(&signature))
            .ok()
            .with_context(|| format!("`{name}` is not what {} signed", hex(key.as_bytes())))?;
        Ok(Opened { lines: body, turn: Some(turn) })
    }
}

/// `body` with the line that says `signer` signed it for `name` at `turn`.
fn sealed_at(signer: &SigningKey, name: &str, body: &str, turn: u64) -> String {
    let signature = signer.sign(&signed_bytes(name, turn, body));
    let (key, signature) = (hex(signer.verifying_key().as_bytes()), hex(&signature.to_bytes()));
    format!("{body}{SIGNED} {key} {turn} {signature}\n")
}

/// A turn as it is written: a number in the one way there is to write it.
fn turn_of(text: &str) -> Option<u64> {
    let turn: u64 = text.parse().ok()?;
    (turn.to_string() == text).then_some(turn)
}

fn seconds_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |it| it.as_secs())
}

/// The highest turn this machine took of each list of one remote, kept where what the
/// lists name is kept: a file a list, in a directory a remote, under `turns`.
///
/// The turn is of the list and not of a key, so a list signed with a new key at a lower
/// turn is as old as any other. A machine that remembers nothing takes what it is given:
/// the first list it sees may be an old one.
#[derive(Clone)]
pub(crate) struct Turns {
    dir: PathBuf,
}

/// The memory of turns could not be read or written. It is not taken for no memory: what
/// keeps an old list out is not something damage takes away quietly.
#[derive(Debug)]
pub(crate) struct Unremembered(String);

impl std::fmt::Display for Unremembered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unremembered {}

impl Turns {
    /// The memory kept in `kept_in` of the remote `remote` names.
    pub(crate) fn of(kept_in: &Path, remote: &str) -> Turns {
        let remote = hex(&Sha256::digest(format!("ra-jit turns 1\n{remote}")));
        Turns { dir: kept_in.join("turns").join(remote) }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(hex(&Sha256::digest(name)))
    }

    /// The highest turn taken of the list `name`, if any was.
    pub(crate) fn remembered(&self, name: &str) -> Result<Option<u64>, Unremembered> {
        let file = self.file(name);
        let unread = |why: &dyn std::fmt::Display| {
            Unremembered(format!(
                "{}, where the turn of `{name}` is remembered, {why}: no list is taken in its \
                 place. `blaze cache forget REMOTE` removes what is remembered of a remote",
                file.display()
            ))
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => match text.strip_suffix('\n').and_then(turn_of) {
                Some(turn) => Ok(Some(turn)),
                None => Err(unread(&"does not hold a turn")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(unread(&format_args!("cannot be read ({e})"))),
        }
    }

    /// Takes the list `name` at `turn`: an error when a later one was taken before, and
    /// remembered when it is the latest. Two processes that take lists at once do so one
    /// after the other.
    pub(crate) fn take(&self, name: &str, turn: u64) -> anyhow::Result<()> {
        let unwritten = |e: std::io::Error| {
            let dir = self.dir.display();
            Unremembered(format!("the turn of `{name}` cannot be remembered in {dir} ({e})"))
        };
        private_dir(&self.dir).map_err(unwritten)?;
        let lock = File::create(self.dir.join("lock")).map_err(unwritten)?;
        lock.lock().map_err(unwritten)?;
        match self.remembered(name)? {
            Some(taken) if turn < taken => bail!(
                "`{name}` is turn {turn}, and this machine has taken turn {taken} of it: an \
                 older list is not taken"
            ),
            Some(taken) if turn == taken => Ok(()),
            _ => Ok(write_file_whole(&self.file(name), format!("{turn}\n").as_bytes())
                .map_err(unwritten)?),
        }
    }

    /// Forgets every turn of this remote, and says whether there was any.
    pub(crate) fn forget(&self) -> std::io::Result<bool> {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Makes a directory that only this user reaches, where the system has a way to say so at
/// the making.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// The lines of a list and the line of its signature, which is empty when it has none.
fn split(text: &str) -> (&str, &str) {
    match text.trim_end_matches('\n').rsplit_once('\n') {
        Some((body, last)) if last.starts_with(SIGNED) => (&text[..body.len() + 1], last),
        _ if text.starts_with(SIGNED) => ("", text.trim_end_matches('\n')),
        _ => (text, ""),
    }
}

/// `ra-jit cache keygen FILE`: writes a new key to sign with and prints the public one.
pub(crate) fn keygen(file: &Path) -> anyhow::Result<()> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("no random bytes: {e}"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut to = options.open(file).with_context(|| format!("writing {}", file.display()))?;
    std::io::Write::write_all(&mut to, format!("{}\n", hex(&seed)).as_bytes())?;
    println!("{}", hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()));
    Ok(())
}

fn read_key(file: &Path) -> anyhow::Result<SigningKey> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("reading the key in {}", file.display()))?;
    let seed = unhex::<32>(text.trim())
        .with_context(|| format!("{} does not hold a key", file.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// What a signature is over: the name too, so that a list signed for one project is not
/// one for another, and the turn, so that nobody without the key makes an old list new.
fn signed_bytes(name: &str, turn: u64, body: &str) -> Vec<u8> {
    format!("ra-jit list 2\n{name}\n{turn}\n{body}").into_bytes()
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let mut bytes = [0; N];
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
impl Trust {
    /// A machine whose keys are made of the bytes given.
    pub(crate) fn of(signer: Option<u8>, trusted: &[u8], unsigned: bool) -> Trust {
        let key = |seed: u8| SigningKey::from_bytes(&[seed; 32]);
        Trust {
            signer: signer.map(key),
            trusted: trusted.iter().map(|&it| key(it).verifying_key()).collect(),
            unsigned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::tests::{changed, scratch};

    fn trusting(signer: Option<u8>, trusted: &[u8]) -> Trust {
        Trust::of(signer, trusted, false)
    }

    fn sealed(signer: u8, name: &str, body: &str) -> String {
        trusting(Some(signer), &[]).seal(name, body, 0).0
    }

    fn lines<'a>(opened: anyhow::Result<Opened<'a>>) -> &'a str {
        opened.unwrap().lines
    }

    #[test]
    fn a_list_is_taken_from_a_trusted_key_only() {
        let sealed = sealed(1, "manifests/a", "one\ntwo\n");
        assert_eq!(lines(trusting(None, &[1]).open("manifests/a", &sealed)), "one\ntwo\n");
        assert!(trusting(None, &[2]).open("manifests/a", &sealed).is_err());
        assert!(trusting(Some(2), &[]).open_to_extend("manifests/a", &sealed).is_err());
        assert!(trusting(Some(1), &[]).open_to_extend("manifests/a", &sealed).is_ok());
    }

    #[test]
    fn a_list_nobody_signed_is_taken_only_when_asked_for() {
        let signed = sealed(1, "manifests/a", "one\n");
        for text in ["one\n", &signed] {
            let refused = trusting(None, &[]).open("manifests/a", text).err().unwrap().to_string();
            assert!(refused.contains("BLAZE_REMOTE_UNSIGNED=1"), "{refused}");
            let taken = Trust::of(None, &[], true).open("manifests/a", text).unwrap();
            assert!(taken.lines == "one\n" && taken.turn.is_none());
            // Adding to a list is not taking what it names.
            assert_eq!(lines(trusting(None, &[]).open_to_extend("manifests/a", text)), "one\n");
        }
        // Being told to take unsigned lists does not undo naming keys, or having one.
        assert!(Trust::of(None, &[1], true).open("manifests/a", "one\n").is_err());
        assert!(Trust::of(Some(1), &[], true).open("manifests/a", "one\n").is_err());
        assert_eq!(lines(Trust::of(Some(1), &[], false).open("manifests/a", &signed)), "one\n");
        assert!(Trust::of(Some(2), &[], true).open("manifests/a", &signed).is_err());
    }

    #[test]
    fn a_list_that_was_changed_or_moved_is_refused() {
        let sealed = sealed(1, "manifests/a", "one\ntwo\n");
        let trust = trusting(None, &[1]);
        assert!(trust.open("manifests/b", &sealed).is_err());
        assert!(trust.open("manifests/a", &sealed.replace("two", "six")).is_err());
        assert!(trust.open("manifests/a", &sealed.replacen("one\n", "", 1)).is_err());
        assert!(trust.open("manifests/a", "one\ntwo\n").is_err());
        assert!(trust.open("manifests/a", "").is_err());
    }

    #[test]
    fn an_empty_list_is_signed_as_any_other() {
        let sealed = sealed(1, "manifests/a", "");
        assert_eq!(lines(trusting(None, &[1]).open("manifests/a", &sealed)), "");
    }

    #[test]
    fn the_turn_is_signed_with_the_list() {
        let signer = SigningKey::from_bytes(&[1; 32]);
        let trust = trusting(None, &[1]);
        let at = |turn| sealed_at(&signer, "manifests/a", "one\n", turn);
        assert_eq!(trust.open("manifests/a", &at(7)).unwrap().turn, Some(7));
        // The same list said to be of another turn is not what was signed.
        let later = at(7).replace(" 7 ", " 8 ");
        let refused = trust.open("manifests/a", &later).err().unwrap().to_string();
        assert!(refused.contains("is not what"), "{refused}");
        // A turn is written in one way.
        for written in ["07", "+7", "7.0", "", "-1", "18446744073709551616"] {
            let odd = at(7).replace(" 7 ", &format!(" {written} "));
            let refused = trust.open("manifests/a", &odd).err().unwrap().to_string();
            assert!(refused.contains("is not signed"), "{written}: {refused}");
        }
        assert_eq!(trust.open("manifests/a", &at(u64::MAX)).unwrap().turn, Some(u64::MAX));
    }

    #[test]
    fn a_list_signed_without_a_turn_is_not_signed() {
        let signer = SigningKey::from_bytes(&[1; 32]);
        let (name, body) = ("manifests/a", "one\n");
        let key = hex(signer.verifying_key().as_bytes());
        let old = signer.sign(format!("ra-jit list 1\n{name}\n{body}").as_bytes());
        let trust = trusting(None, &[1]);
        let without = format!("{body}signed {key} {}\n", hex(&old.to_bytes()));
        let refused = trust.open(name, &without).err().unwrap().to_string();
        assert!(refused.contains("is not signed"), "{refused}");
        // Nor does giving it a turn make the old signature one of a list with a turn.
        let with = format!("{body}signed {key} 7 {}\n", hex(&old.to_bytes()));
        assert!(trust.open(name, &with).is_err());
    }

    #[test]
    fn a_turn_is_past_the_last_and_not_behind_the_clock() {
        let trust = trusting(Some(1), &[]);
        let now = seconds_now();
        let (_, first) = trust.seal("manifests/a", "", 0);
        assert!(first.unwrap() >= now, "{first:?} is behind {now}");
        // A signer whose clock is behind the last list still comes after it.
        let ahead = now + (1 << 40);
        assert_eq!(trust.seal("manifests/a", "", ahead).1, Some(ahead + 1));
        assert_eq!(trust.seal("manifests/a", "", u64::MAX).1, Some(u64::MAX));
        assert_eq!(trusting(None, &[]).seal("manifests/a", "one\n", 5), ("one\n".to_owned(), None));
    }

    #[test]
    fn an_older_list_is_not_taken_again() {
        let kept_in = scratch("turns");
        let turns = Turns::of(&kept_in, "s3://bucket/");
        assert_eq!(turns.remembered("manifests/a").unwrap(), None);
        turns.take("manifests/a", 5).unwrap();
        turns.take("manifests/a", 7).unwrap();
        turns.take("manifests/a", 7).unwrap();
        let refused = turns.take("manifests/a", 5).unwrap_err().to_string();
        assert!(refused.contains("turn 5") && refused.contains("turn 7"), "{refused}");
        assert_eq!(turns.remembered("manifests/a").unwrap(), Some(7));
        // Another list, and the same list of another remote, have turns of their own.
        turns.take("manifests/b", 1).unwrap();
        Turns::of(&kept_in, "s3://other/").take("manifests/a", 1).unwrap();
        // What is remembered is remembered by the next process.
        assert!(Turns::of(&kept_in, "s3://bucket/").take("manifests/a", 6).is_err());
        _ = std::fs::remove_dir_all(&kept_in);
    }

    #[test]
    fn a_memory_that_is_damaged_takes_no_list_until_it_is_forgotten() {
        let kept_in = scratch("damaged-turns");
        let turns = Turns::of(&kept_in, "remote");
        turns.take("manifests/a", 5).unwrap();
        for damage in ["", "5", "five\n", "05\n", "5\n5\n", "\u{0}"] {
            std::fs::write(turns.file("manifests/a"), damage).unwrap();
            for turn in [0, 5, 9] {
                let refused = turns.take("manifests/a", turn).unwrap_err();
                assert!(refused.is::<Unremembered>(), "{damage:?}: {refused}");
                assert!(refused.to_string().contains("blaze cache forget"), "{refused}");
            }
            assert_eq!(std::fs::read_to_string(turns.file("manifests/a")).unwrap(), damage);
        }
        assert!(turns.forget().unwrap());
        assert!(!turns.forget().unwrap());
        turns.take("manifests/a", 1).unwrap();
        assert_eq!(turns.remembered("manifests/a").unwrap(), Some(1));
        _ = std::fs::remove_dir_all(&kept_in);
    }

    #[test]
    fn two_that_take_at_once_remember_the_later() {
        let kept_in = scratch("raced-turns");
        let turns = Turns::of(&kept_in, "remote");
        std::thread::scope(|scope| {
            for thread in 0..8u64 {
                let turns = &turns;
                scope.spawn(move || {
                    for turn in 0..50 {
                        _ = turns.take("manifests/a", turn * 8 + thread);
                    }
                });
            }
        });
        assert_eq!(turns.remembered("manifests/a").unwrap(), Some(49 * 8 + 7));
        _ = std::fs::remove_dir_all(&kept_in);
    }

    #[test]
    fn no_bytes_are_taken_for_a_list_that_was_signed() {
        let signer = SigningKey::from_bytes(&[1; 32]);
        let body = format!("{} 4096 {} 2\n", "ab".repeat(32), "0f".repeat(32));
        let seed = sealed_at(&signer, "manifests/a", &body, 1_792_000_000);
        let trust = trusting(None, &[1]);
        let (mut taken, mut refused) = (0, 0);
        for bytes in changed(seed.as_bytes(), 20_000) {
            let Ok(text) = String::from_utf8(bytes) else { continue };
            match trust.open("manifests/a", &text) {
                // Whatever is taken is the list that was signed, at the turn it was.
                Ok(opened) => {
                    assert_eq!((opened.lines, opened.turn), (&*body, Some(1_792_000_000)));
                    taken += 1;
                }
                Err(_) => refused += 1,
            }
            _ = split(&text);
        }
        assert!(taken >= 1 && refused > 10_000, "{taken} taken, {refused} refused");
    }
}
