//! Experimental reviewed source pin for an attributed identity runtime probe.
//!
//! This is an integrity/admission envelope, not a compiler builtin. The immutable
//! reviewed source revision and byte hashes authorize only inert metadata. The
//! ordinary parser, body checker, ownership and backend remain semantic authority.
use super::{SourceError, SourceErrorKind, SourceOptions, SourceSpan};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const ITEM: &str = "core::convert::identity";
const REVISION: &str = "d907fba494adcabfdd642a18a7918b0c9847d35b";
const SOURCE_HASH: &str = "0fd346eff31bfa6cc7c79fc4cba8367e98c579ccd7072dd141ce0bea76b602ac";
const ITEM_HASH: &str = "c4187b524514fd5daf107703898a3489da0cdf7806eea13988f4f7a365c4d6a8";
const SELECTED: Range<usize> = 2258..3893;
const MAX_LOCK: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProbeRequest {
    pub lock_path: PathBuf,
    pub item: String,
}

/// Audit data only. These public fields confer no metadata or semantic capability.
#[derive(Clone, Debug)]
pub struct SourceProbeRecord {
    pub item: String,
    pub source_revision: String,
    pub source_sha256: String,
    pub item_sha256: String,
    pub span: SourceSpan,
    pub license: String,
    pub attribution: String,
    pub lock_path: PathBuf,
    inputs: super::source_inputs::SourceInputs,
}
impl SourceProbeRecord {
    /// Bounded observations of the lock and complete provider bytes consumed.
    /// Revalidation is not atomic and confers no semantic or cache authority.
    pub fn inputs(&self) -> &super::source_inputs::SourceInputs {
        &self.inputs
    }
}

/// Opaque and snapshot-bound. There is deliberately no public constructor.
pub(super) struct VerifiedIdentityContext {
    selected: Arc<str>,
}
impl VerifiedIdentityContext {
    pub(super) fn permits_metadata(
        &self,
        text: &str,
        attribute: Range<usize>,
        item: Range<usize>,
    ) -> bool {
        text == self.selected.as_ref()
            && item == (1589..1634)
            && attribute.start >= 1405
            && attribute.start < attribute.end
            && attribute.end <= 1589
            && text.is_char_boundary(attribute.start)
            && text.is_char_boundary(attribute.end)
    }
}
struct VerifiedProvider {
    inputs: super::source_inputs::SourceInputs,
    path: PathBuf,
    full_text: Arc<str>,
    context: VerifiedIdentityContext,
}

fn error(message: impl Into<String>) -> SourceError {
    super::Loader::error(
        SourceErrorKind::Unsupported,
        None,
        format!("source lock: {}", message.into()),
    )
}
fn read(
    reader: &mut dyn super::SourceReader,
    path: &Path,
    limit: usize,
) -> Result<Vec<u8>, SourceError> {
    reader.canonicalize(path).map_err(|e| {
        super::Loader::error(
            SourceErrorKind::Io,
            None,
            format!("{}: {e}", path.display()),
        )
    })?;
    if !reader.is_file(path).map_err(|e| {
        super::Loader::error(
            SourceErrorKind::Io,
            None,
            format!("{}: {e}", path.display()),
        )
    })? {
        return Err(error(format!("{} must be a regular file", path.display())));
    }
    let bytes = reader.read(path, limit + 1).map_err(|e| {
        super::Loader::error(
            SourceErrorKind::Io,
            None,
            format!("{}: {e}", path.display()),
        )
    })?;
    if bytes.len() > limit {
        return Err(super::Loader::error(
            SourceErrorKind::Limit,
            None,
            "source lock snapshot byte limit exceeded",
        ));
    }
    Ok(bytes)
}

/// Verify before preparing the privileged fragment, and never share its policy
/// with the independently prepared client or any of the client's child modules.
pub fn prepare_probe_file(
    client: &Path,
    options: &SourceOptions,
    request: &SourceProbeRequest,
) -> Result<super::PreparedSource, SourceError> {
    options.cfg.validate_native().map_err(error)?;
    let verified = verify(request, options)?;
    let lexical_budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let prepared = super::prepare_file_with_budget(client, options, &lexical_budget)?;
    attach_verified(prepared, options, request, verified, lexical_budget)
}

/// Append the reviewed probe item to an already prepared client source. The
/// graph prepares the client per file and attaches the probe after assembly.
pub fn attach_probe(
    prepared: super::PreparedSource,
    options: &SourceOptions,
    request: &SourceProbeRequest,
) -> Result<super::PreparedSource, SourceError> {
    options.cfg.validate_native().map_err(error)?;
    let verified = verify(request, options)?;
    attach_verified(
        prepared,
        options,
        request,
        verified,
        crate::compiler::expansion_budget::ExpansionBudget::default(),
    )
}

fn attach_verified(
    mut prepared: super::PreparedSource,
    options: &SourceOptions,
    request: &SourceProbeRequest,
    verified: VerifiedProvider,
    lexical_budget: crate::compiler::expansion_budget::ExpansionBudget,
) -> Result<super::PreparedSource, SourceError> {
    // A verified addition must not re-authorize changes to previously sealed
    // embedded sources. Validate the old assembly before extending it.
    if let Some(transport) = &prepared.embedded_transport {
        transport.validate(&prepared)?;
    } else if !prepared.embedded_files.is_empty() {
        return Err(super::Loader::error(
            SourceErrorKind::Contract,
            None,
            "embedded source assembly transport is missing",
        ));
    }
    if verified.full_text.len() > super::MAX_BYTES - prepared.source_bytes
        || prepared.files >= super::MAX_FILES
    {
        return Err(super::Loader::error(
            SourceErrorKind::Limit,
            None,
            "combined probe/client source size or file limit exceeded",
        ));
    }
    let id = prepared
        .sources
        .add(verified.path, verified.full_text.clone());
    // The verified provider is a separate original-source attachment, not a
    // child of the client crate. Keep the per-file ownership map complete so
    // lexical sealing cannot conflate its provenance with the client.
    prepared.crate_roots.push(id);
    prepared.source_bytes += verified.full_text.len();
    prepared.files += 1;
    let mut loader = super::Loader::new(
        options,
        None,
        super::LOCAL_FILE,
        verified.full_text.clone(),
        false,
    );
    loader.lexical_budget = lexical_budget;
    let result = (|| {
        loader.validate_original(&verified.full_text, 0)?;
        let filtered = super::source_attrs::prepare_verified_identity(
            &verified.context.selected,
            &options.cfg,
            &verified.context,
        )
        .map_err(|e| {
            super::Loader::error(
                match e.kind {
                    super::source_attrs::AttributeErrorKind::Syntax => SourceErrorKind::Syntax,
                    super::source_attrs::AttributeErrorKind::Unsupported => {
                        SourceErrorKind::Unsupported
                    }
                    super::source_attrs::AttributeErrorKind::Limit => SourceErrorKind::Limit,
                },
                Some(super::Loader::span(
                    id,
                    SELECTED.start + e.range.start..SELECTED.start + e.range.end,
                )),
                e.message,
            )
        })?;
        if !filtered.inactive_items.is_empty() {
            return Err(error(
                "reviewed identity cannot contain disabled declarations",
            ));
        }
        loader.append(
            "\n",
            super::Loader::span(super::LOCAL_FILE, SELECTED.start..SELECTED.start),
            true,
        )?;
        for attribute in filtered.attributes {
            prepared.attributes.push(super::PreparedAttribute {
                span: super::Loader::span(
                    id,
                    SELECTED.start + attribute.range.start..SELECTED.start + attribute.range.end,
                ),
                name: attribute.name,
                effective: attribute.effective,
            });
        }
        loader.expand(&filtered.text, SELECTED.start, Path::new("."), 0)?;
        if !loader.children.is_empty() {
            return Err(error("reviewed identity cannot declare external modules"));
        }
        for segment in &loader.origins {
            super::append_segment(
                &mut prepared,
                &loader.text[segment.prepared.clone()],
                super::remap_span(segment.origin, id),
                segment.synthetic,
            )?;
        }
        prepared.probes.push(SourceProbeRecord {
            inputs: verified.inputs,
            item: ITEM.into(),
            source_revision: REVISION.into(),
            source_sha256: SOURCE_HASH.into(),
            item_sha256: ITEM_HASH.into(),
            span: super::Loader::span(id, SELECTED),
            license: "MIT OR Apache-2.0".into(),
            attribution: "The Rust Project Contributors".into(),
            lock_path: request.lock_path.clone(),
        });
        Ok(())
    })();
    result.map_err(|e: SourceError| {
        let primary = e.primary.map(|span| super::remap_span(span, id));
        super::SourceError { primary, ..e }.with_sources(&prepared.sources)
    })?;
    if prepared.embedded_transport.is_some() {
        prepared.embedded_transport =
            Some(super::source_embedded::AssemblyTransport::issue(&prepared)?);
    }
    Ok(prepared)
}

fn verify(
    request: &SourceProbeRequest,
    options: &SourceOptions,
) -> Result<VerifiedProvider, SourceError> {
    let mut filesystem = super::FsReader;
    let mut reader = super::source_inputs::TrackingReader::new(&mut filesystem, options);
    if request.item != ITEM {
        return Err(error("unregistered reviewed probe item"));
    }
    let bytes = read(&mut reader, &request.lock_path, MAX_LOCK)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| error("lock is not UTF-8"))?;
    let mut root = Parser::parse(text)?.object()?;
    if take_number(&mut root, "version")? != 1 {
        return Err(error("unsupported lock version"));
    }
    let rows = root
        .remove("items")
        .ok_or_else(|| error("missing items"))?
        .array()?;
    if !root.is_empty() {
        return Err(error("unknown lock root field"));
    }
    if rows.is_empty() || rows.len() > 32 {
        return Err(error("lock item count out of bounds"));
    }
    let mut names = BTreeSet::new();
    let mut selected = None;
    for row in rows {
        let mut row = row.object()?;
        let name = take_string(&mut row, "name")?;
        let path = take_string(&mut row, "path")?;
        let source_hash = take_string(&mut row, "source_sha256")?;
        let start = usize::try_from(take_number(&mut row, "start")?)
            .map_err(|_| error("source range overflow"))?;
        let end = usize::try_from(take_number(&mut row, "end")?)
            .map_err(|_| error("source range overflow"))?;
        let item_hash = take_string(&mut row, "item_sha256")?;
        let license = take_string(&mut row, "license")?;
        let attribution = take_string(&mut row, "attribution")?;
        let revision = take_string(&mut row, "source_revision")?;
        if !row.is_empty() {
            return Err(error("unknown lock item field"));
        }
        if !names.insert(name.clone()) {
            return Err(error("duplicate source item name"));
        }
        let relative = Path::new(&path);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(error(
                "source path must be a relative lexical path without parent traversal",
            ));
        }
        if !valid_hash(&source_hash)
            || !valid_hash(&item_hash)
            || start >= end
            || end > super::MAX_BYTES
        {
            return Err(error("invalid digest or source range"));
        }
        if name == request.item {
            if source_hash != SOURCE_HASH
                || item_hash != ITEM_HASH
                || (start..end) != SELECTED
                || revision != REVISION
                || license != "MIT OR Apache-2.0"
                || attribution != "The Rust Project Contributors"
            {
                return Err(error("lock does not match the reviewed source pin"));
            }
            selected = Some(
                request
                    .lock_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(relative),
            );
        }
    }
    let path = selected.ok_or_else(|| error("requested item absent from lock"))?;
    let full = read(&mut reader, &path, super::MAX_BYTES)?;
    if sha256(&full) != SOURCE_HASH {
        return Err(error("provider source SHA-256 mismatch"));
    }
    let selected = full
        .get(SELECTED)
        .ok_or_else(|| error("selected source range out of bounds"))?;
    if sha256(selected) != ITEM_HASH {
        return Err(error("selected item SHA-256 mismatch"));
    }
    let selected = Arc::<str>::from(
        std::str::from_utf8(selected)
            .map_err(|_| error("selected source is not UTF-8 or splits a character"))?,
    );
    let full_text = Arc::<str>::from(
        std::str::from_utf8(&full).map_err(|_| error("provider source is not UTF-8"))?,
    );
    Ok(VerifiedProvider {
        inputs: reader.finish(),
        path,
        full_text,
        context: VerifiedIdentityContext { selected },
    })
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug)]
enum Json {
    String(String),
    Number(u64),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}
impl Json {
    fn object(self) -> Result<BTreeMap<String, Json>, SourceError> {
        if let Self::Object(v) = self {
            Ok(v)
        } else {
            Err(error("expected JSON object"))
        }
    }
    fn array(self) -> Result<Vec<Json>, SourceError> {
        if let Self::Array(v) = self {
            Ok(v)
        } else {
            Err(error("expected JSON array"))
        }
    }
}
fn take_string(row: &mut BTreeMap<String, Json>, name: &str) -> Result<String, SourceError> {
    match row.remove(name) {
        Some(Json::String(v)) => Ok(v),
        _ => Err(error(format!("missing or invalid string field {name}"))),
    }
}
fn take_number(row: &mut BTreeMap<String, Json>, name: &str) -> Result<u64, SourceError> {
    match row.remove(name) {
        Some(Json::Number(v)) => Ok(v),
        _ => Err(error(format!("missing or invalid numeric field {name}"))),
    }
}
struct Parser<'a> {
    text: &'a str,
    pos: usize,
    nodes: usize,
}
impl<'a> Parser<'a> {
    fn parse(text: &'a str) -> Result<Json, SourceError> {
        if text.len() > MAX_LOCK {
            return Err(error("lock byte limit exceeded"));
        }
        let mut p = Self {
            text,
            pos: 0,
            nodes: 0,
        };
        let value = p.value(0)?;
        p.space();
        if p.pos != text.len() {
            return Err(error("trailing JSON data"));
        }
        Ok(value)
    }
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.pos)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.pos += 1;
        }
    }
    fn eat(&mut self, b: u8) -> bool {
        self.space();
        if self.text.as_bytes().get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json, SourceError> {
        self.nodes += 1;
        if depth > 8 || self.nodes > 2048 {
            return Err(error("lock JSON nesting/work limit exceeded"));
        }
        self.space();
        match self.text.as_bytes().get(self.pos).copied() {
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'{') => {
                self.pos += 1;
                let mut values = BTreeMap::new();
                if self.eat(b'}') {
                    return Ok(Json::Object(values));
                }
                loop {
                    let key = self.string()?;
                    if !self.eat(b':') {
                        return Err(error("expected JSON colon"));
                    }
                    let value = self.value(depth + 1)?;
                    if values.insert(key, value).is_some() {
                        return Err(error("duplicate JSON key"));
                    }
                    if self.eat(b'}') {
                        break;
                    }
                    if !self.eat(b',') {
                        return Err(error("expected JSON object separator"));
                    }
                }
                Ok(Json::Object(values))
            }
            Some(b'[') => {
                self.pos += 1;
                let mut values = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    if self.eat(b']') {
                        break;
                    }
                    if !self.eat(b',') {
                        return Err(error("expected JSON array separator"));
                    }
                }
                Ok(Json::Array(values))
            }
            Some(b'0'..=b'9') => {
                let start = self.pos;
                while self
                    .text
                    .as_bytes()
                    .get(self.pos)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.pos += 1;
                }
                let s = &self.text[start..self.pos];
                if s.len() > 1 && s.starts_with('0') {
                    return Err(error("leading zero in JSON number"));
                }
                Ok(Json::Number(
                    s.parse().map_err(|_| error("JSON integer overflow"))?,
                ))
            }
            _ => Err(error("unsupported or malformed lock JSON value")),
        }
    }
    fn hex4(&mut self) -> Result<u32, SourceError> {
        let end = self
            .pos
            .checked_add(4)
            .ok_or_else(|| error("JSON escape overflow"))?;
        let s = self
            .text
            .get(self.pos..end)
            .ok_or_else(|| error("invalid Unicode escape"))?;
        if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error("invalid Unicode escape"));
        }
        let value = u32::from_str_radix(s, 16).map_err(|_| error("invalid Unicode escape"))?;
        self.pos = end;
        Ok(value)
    }
    fn string(&mut self) -> Result<String, SourceError> {
        self.space();
        if !self.eat(b'"') {
            return Err(error("expected JSON string"));
        }
        let mut out = String::new();
        loop {
            let c = self.text[self.pos..]
                .chars()
                .next()
                .ok_or_else(|| error("unterminated JSON string"))?;
            self.pos += c.len_utf8();
            match c {
                '"' => break,
                '\\' => {
                    let escaped = self
                        .text
                        .as_bytes()
                        .get(self.pos)
                        .copied()
                        .ok_or_else(|| error("unterminated JSON escape"))?;
                    self.pos += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut value = self.hex4()?;
                            if (0xd800..=0xdbff).contains(&value) {
                                if !self.text[self.pos..].starts_with("\\u") {
                                    return Err(error("missing low surrogate"));
                                }
                                self.pos += 2;
                                let low = self.hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(error("invalid low surrogate"));
                                }
                                value = 0x10000 + ((value - 0xd800) << 10) + (low - 0xdc00);
                            }
                            out.push(
                                char::from_u32(value)
                                    .ok_or_else(|| error("invalid Unicode scalar"))?,
                            );
                        }
                        _ => return Err(error("invalid JSON escape")),
                    }
                }
                c if c < ' ' => return Err(error("unescaped JSON control character")),
                c => out.push(c),
            }
            if out.len() > 16 * 1024 {
                return Err(error("lock JSON string byte limit exceeded"));
            }
        }
        Ok(out)
    }
}

/// SHA-256 for source integrity, independent of filesystem paths and host hash seeds.
fn sha256(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64) * 8;
    let blocks = (input.len() + 9).div_ceil(64);
    for block in 0..blocks {
        let mut bytes = [0u8; 64];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            let pos = block * 64 + offset;
            *byte = if pos < input.len() {
                input[pos]
            } else if pos == input.len() {
                0x80
            } else if pos >= blocks * 64 - 8 {
                bit_len.to_be_bytes()[pos - (blocks * 64 - 8)]
            } else {
                0
            };
        }
        let mut w = [0u32; 64];
        for (i, word) in w[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    use std::fmt::Write;
    let mut output = String::with_capacity(64);
    for word in state {
        write!(&mut output, "{word:08x}").unwrap();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "jr-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("probes/identity")).unwrap();
            std::fs::write(
                root.join("lock.json"),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/libraries/source-lock.json"
                )),
            )
            .unwrap();
            std::fs::write(
                root.join("probes/identity/convert.rs"),
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/libraries/probes/identity/convert.rs"
                )),
            )
            .unwrap();
            std::fs::write(root.join("client.rs"), "fn main(){}").unwrap();
            Self { root }
        }
        fn request(&self) -> SourceProbeRequest {
            SourceProbeRequest {
                lock_path: self.root.join("lock.json"),
                item: ITEM.into(),
            }
        }
        fn prepare(&self) -> Result<super::super::PreparedSource, SourceError> {
            prepare_probe_file(
                &self.root.join("client.rs"),
                &SourceOptions::default(),
                &self.request(),
            )
        }
        fn rewrite_lock(&self, from: &str, to: &str) {
            let path = self.root.join("lock.json");
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(from));
            std::fs::write(path, text.replace(from, to)).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn probe_receipt_tracks_lock_bytes_provider_bytes_and_canonical_targets() {
        let fixture = Fixture::new();
        let options = SourceOptions::default();
        let prepared = fixture.prepare().unwrap();
        let inputs = prepared.probes[0].inputs();
        assert_eq!(inputs.observation_count(), 6);
        assert!(inputs.matches_current_source(&options));
        let lock = fixture.root.join("lock.json");
        let original_lock = std::fs::read(&lock).unwrap();
        let mut whitespace = original_lock.clone();
        whitespace.push(b'\n');
        std::fs::write(&lock, &whitespace).unwrap();
        assert!(!inputs.matches_current_source(&options));
        assert!(fixture.prepare().is_ok());
        std::fs::write(&lock, &original_lock).unwrap();
        assert!(inputs.matches_current_source(&options));
        let provider = fixture.root.join("probes/identity/convert.rs");
        let original_provider = std::fs::read(&provider).unwrap();
        let mut altered = original_provider.clone();
        altered[0] ^= 1;
        std::fs::write(&provider, altered).unwrap();
        assert!(!inputs.matches_current_source(&options));
        assert!(fixture.prepare().is_err());
        std::fs::write(&provider, &original_provider).unwrap();
        assert!(inputs.matches_current_source(&options));
        for path in [&lock, &provider] {
            let alternate = path.with_extension("alternate");
            std::fs::rename(path, &alternate).unwrap();
            assert!(!inputs.matches_current_source(&options));
            assert!(fixture.prepare().is_err());
            // Identical bytes at a different canonical identity still invalidate.
            std::os::unix::fs::symlink(&alternate, path).unwrap();
            assert!(!inputs.matches_current_source(&options));
            assert!(fixture.prepare().is_ok());
            std::fs::remove_file(path).unwrap();
            std::fs::rename(alternate, path).unwrap();
            assert!(inputs.matches_current_source(&options));
        }
        std::fs::write(&lock, vec![b' '; MAX_LOCK + 1]).unwrap();
        assert_eq!(fixture.prepare().unwrap_err().kind, SourceErrorKind::Limit);
        assert!(!inputs.matches_current_source(&options));
        std::fs::write(&lock, &original_lock).unwrap();
        assert!(inputs.matches_current_source(&options));
        let mut changed = options.clone();
        changed.overflow_checks = !changed.overflow_checks;
        assert!(!inputs.matches_current_source(&changed));
    }

    #[test]
    fn probe_receipts_reach_graph_facts_and_missing_inputs_never_publish_success() {
        use crate::{compiler, engine::Engine};
        let fixture = Fixture::new();
        std::fs::write(
            fixture.root.join("client.rs"),
            "fn main(){if identity::<i32>(7)==7{println!(\"probe tracked\");}}",
        )
        .unwrap();
        let options = SourceOptions::default();
        for workers in [1, 8] {
            for check_only in [true, false] {
                let snapshot = Arc::new(compiler::Snapshot {
                    input: compiler::Input::ProbeFile(
                        fixture.root.join("client.rs"),
                        fixture.request(),
                    ),
                    source_options: options.clone(),
                    budget: compiler::ir::PlanBudget::default(),
                });
                let engine = Engine::new(compiler::CompilerProvider(snapshot));
                let check = engine.request(compiler::keys::CheckRoot.key());
                let build = (!check_only).then(|| engine.request(compiler::keys::BuildRoot.key()));
                engine.run(workers).unwrap();
                check.completion().unwrap();
                let prepared = engine
                    .fact_of::<super::super::PreparedSource>(
                        &compiler::keys::ParseDebtRoot.key(),
                        compiler::PREPARED_SOURCE_OUTPUT,
                    )
                    .unwrap();
                let inputs = prepared.probes[0].inputs();
                assert_eq!(inputs.observation_count(), 6);
                assert!(inputs.matches_current_source(&options));
                let lock = fixture.root.join("lock.json");
                let bytes = std::fs::read(&lock).unwrap();
                std::fs::remove_file(&lock).unwrap();
                assert!(!inputs.matches_current_source(&options));
                std::fs::write(&lock, bytes).unwrap();
                assert!(inputs.matches_current_source(&options));
                if let Some(build) = build {
                    build.completion().unwrap();
                    let executable = fixture.root.join(format!("program-{workers}"));
                    std::fs::write(&executable, build.fact::<Vec<u8>>(0).unwrap().as_slice())
                        .unwrap();
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    let output = std::process::Command::new(executable).output().unwrap();
                    assert!(output.status.success());
                    assert_eq!(output.stdout, b"probe tracked\n");
                }
            }
            for path in [
                fixture.root.join("lock.json"),
                fixture.root.join("probes/identity/convert.rs"),
            ] {
                let bytes = std::fs::read(&path).unwrap();
                std::fs::remove_file(&path).unwrap();
                let snapshot = Arc::new(compiler::Snapshot {
                    input: compiler::Input::ProbeFile(
                        fixture.root.join("client.rs"),
                        fixture.request(),
                    ),
                    source_options: options.clone(),
                    budget: compiler::ir::PlanBudget::default(),
                });
                let engine = Engine::new(compiler::CompilerProvider(snapshot));
                let check = engine.request(compiler::keys::CheckRoot.key());
                let build = engine.request(compiler::keys::BuildRoot.key());
                // A completed scheduler run can contain failed graph nodes.
                engine.run(workers).unwrap();
                assert!(check.completion().is_err());
                assert!(build.completion().is_err());
                assert!(
                    engine
                        .fact_of::<super::super::PreparedSource>(
                            &compiler::keys::ParseDebtRoot.key(),
                            compiler::PREPARED_SOURCE_OUTPUT
                        )
                        .is_none()
                );
                assert!(build.fact::<Vec<u8>>(0).is_err());
                std::fs::write(&path, bytes).unwrap();
            }
        }
    }

    #[test]
    fn verified_probe_extends_sealed_prelude_without_reauthorizing_mutations() {
        use crate::compiler::source::{
            AssemblyStep, PreparedLexInput, SourceAssembler, prepare_child_unit,
            prepare_prelude_unit, prepare_text_unit,
        };
        let fixture = Fixture::new();
        let options = SourceOptions::default();
        let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
        let mut assembler = SourceAssembler::new(&options);
        let mut step = assembler
            .provide(Arc::new(prepare_text_unit(
                "client.rs",
                "fn main(){}",
                &options,
            )))
            .unwrap();
        let prepared = loop {
            match step {
                AssemblyStep::Done(source) => break *source,
                AssemblyStep::Need(need) => {
                    let unit = match need.parent.as_deref() {
                        None => prepare_prelude_unit(&options, &budget),
                        Some(parent) => prepare_child_unit(parent, need.index, &options, &budget),
                    };
                    step = assembler.provide(Arc::new(unit)).unwrap();
                }
            }
        };
        assert!(!prepared.embedded_files.is_empty());
        let extended = attach_probe(prepared.clone(), &options, &fixture.request()).unwrap();
        extended
            .embedded_transport
            .as_ref()
            .unwrap()
            .validate(&extended)
            .unwrap();
        let projection = PreparedLexInput::prepared(
            &extended,
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let provider = extended.probes.last().unwrap().span.file;
        let file = projection.snapshot().file(provider.0 as usize).unwrap();
        assert_eq!(projection.snapshot().source_crate_root(file).unwrap(), file);
        assert_eq!(extended.embedded_files.len(), prepared.embedded_files.len());
        crate::compiler::frontend::parse_prepared(&extended).unwrap();
        for mutation in 0..3 {
            let mut forged = prepared.clone();
            match mutation {
                0 => forged.text.push(' '),
                1 => forged.crate_roots[0] = *forged.crate_roots.last().unwrap(),
                2 => forged.embedded_transport = None,
                _ => unreachable!(),
            }
            let failure = attach_probe(forged, &options, &fixture.request()).unwrap_err();
            assert_eq!(
                failure.kind,
                SourceErrorKind::Contract,
                "{mutation}: {failure}"
            );
        }
    }

    #[test]
    fn verified_identity_retains_full_original_source_and_exact_selected_provenance() {
        let fixture = Fixture::new();
        let prepared = fixture.prepare().unwrap();
        assert_eq!(prepared.files, 2);
        assert_eq!(prepared.source_bytes, "fn main(){}".len() + 33122);
        assert_eq!(prepared.probes.len(), 1);
        let record = &prepared.probes[0];
        assert_eq!((record.span.lo, record.span.hi), (2258, 3893));
        assert_eq!(record.source_sha256, SOURCE_HASH);
        assert_eq!(record.item_sha256, ITEM_HASH);
        assert_eq!(record.source_revision, REVISION);
        assert_eq!(record.item, ITEM);
        assert_eq!(record.license, "MIT OR Apache-2.0");
        assert_eq!(record.attribution, "The Rust Project Contributors");
        let source = &prepared.sources.files[record.span.file.0 as usize];
        assert_eq!(
            source.lexical_path,
            fixture.root.join("probes/identity/convert.rs")
        );
        assert_eq!(source.contents.len(), 33122);
        assert_eq!(sha256(source.contents.as_bytes()), SOURCE_HASH);
        assert_eq!(sha256(&source.contents.as_bytes()[2258..3893]), ITEM_HASH);
        let start = prepared.text.find("pub const fn identity").unwrap();
        let span = prepared.map_range(start..start + 3).unwrap();
        assert_eq!(span.file, record.span.file);
        assert_eq!(span.lo, 3847);
        assert_eq!(prepared.sources.location(span).unwrap().1, 105);
        for name in ["stable", "rustc_const_stable", "rustc_diagnostic_item"] {
            assert!(
                prepared
                    .attributes
                    .iter()
                    .any(|a| a.name == name && a.span.file == record.span.file)
            );
        }
    }

    #[test]
    fn verified_context_cannot_be_reused_for_client_or_modified_snapshot_or_owner() {
        let fixture = Fixture::new();
        let verified = verify(&fixture.request(), &SourceOptions::default()).unwrap();
        let selected = verified.context.selected.as_ref();
        assert!(
            verified
                .context
                .permits_metadata(selected, 1407..1450, 1589..1634)
        );
        assert!(
            !verified
                .context
                .permits_metadata(selected, 1407..1450, 0..1634)
        );
        let changed = selected.replacen("    x\n}", "    1\n}", 1);
        assert!(
            !verified
                .context
                .permits_metadata(&changed, 1407..1450, 1589..1634)
        );
        assert!(
            super::super::source_attrs::prepare_verified_identity(
                &changed,
                &SourceOptions::default().cfg,
                &verified.context
            )
            .is_err()
        );
        assert!(
            super::super::source_attrs::prepare(selected, &SourceOptions::default().cfg).is_err()
        );
        std::fs::write(
            fixture.root.join("client.rs"),
            "#[stable(feature=\"convert_id\",since=\"1.33.0\")] fn main(){}",
        )
        .unwrap();
        let error = fixture.prepare().unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported);
        assert_eq!(error.location.unwrap().path, fixture.root.join("client.rs"));
    }

    #[test]
    fn lock_rejects_unknown_fields_duplicate_names_and_reviewed_pin_changes() {
        for (from, to) in [
            ("\"version\": 1", "\"version\": 2"),
            ("\"version\": 1", "\"version\": 1, \"unknown\": 1"),
            ("\"start\": 2258", "\"start\": 2259"),
            ("\"end\": 3893", "\"end\": 3894"),
            ("probes/identity/convert.rs", "../convert.rs"),
            ("probes/identity/convert.rs", "/absolute/convert.rs"),
            (REVISION, "0000000000000000000000000000000000000000"),
            (ITEM_HASH, SOURCE_HASH),
            (SOURCE_HASH, ITEM_HASH),
        ] {
            let fixture = Fixture::new();
            fixture.rewrite_lock(from, to);
            assert!(fixture.prepare().is_err(), "accepted {from}->{to}");
        }
        let fixture = Fixture::new();
        let text = std::fs::read_to_string(fixture.root.join("lock.json")).unwrap();
        let row = &text[text.find("    {").unwrap()..text.rfind("    }").unwrap() + 5];
        std::fs::write(
            fixture.root.join("lock.json"),
            format!("{{\"version\":1,\"items\":[{row},{row}]}}"),
        )
        .unwrap();
        assert!(fixture.prepare().unwrap_err().message.contains("duplicate"));
    }

    #[test]
    fn changed_source_cannot_be_authorized_by_recomputed_lock_digest() {
        let fixture = Fixture::new();
        let path = fixture.root.join("probes/identity/convert.rs");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.replace_range(3847..3850, "bad");
        std::fs::write(path, &text).unwrap();
        assert!(fixture.prepare().unwrap_err().message.contains("SHA-256"));
        fixture.rewrite_lock(SOURCE_HASH, &sha256(text.as_bytes()));
        assert!(
            fixture
                .prepare()
                .unwrap_err()
                .message
                .contains("reviewed source pin")
        );
    }

    #[test]
    fn combined_client_and_full_provider_source_budget_is_enforced() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.root.join("client.rs"),
            " ".repeat(super::super::MAX_BYTES - 33122 + 1),
        )
        .unwrap();
        assert_eq!(fixture.prepare().unwrap_err().kind, SourceErrorKind::Limit);
    }

    #[test]
    fn sha256_standard_vectors() {
        for (input, expected) in [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ] {
            assert_eq!(sha256(input.as_bytes()), expected);
        }
        assert_eq!(
            sha256(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
    #[test]
    fn lock_json_rejects_ambiguous_or_unbounded_forms() {
        for text in [
            "{\"version\":1,\"version\":1}",
            "{\"x\":01}",
            "{\"x\":1,}",
            "[1,]",
            "{\"x\":\"\\ud800\"}",
            "{\"x\":\"\\u+041\"}",
            "{\"x\":\"\\u-001\"}",
            "{}{}",
        ] {
            assert!(Parser::parse(text).is_err(), "{text}");
        }
        assert!(Parser::parse(&"[".repeat(10)).is_err());
        assert!(Parser::parse(&" ".repeat(MAX_LOCK + 1)).is_err());
        assert!(Parser::parse("{\"x\":\"\\ud83d\\ude42\"}").is_ok());
    }
}
