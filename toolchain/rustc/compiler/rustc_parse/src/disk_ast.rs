//! Default-off, same-user disk snapshots of the narrowly admitted parser AST.
//!
//! This is not a wire format or an artifact-server cache. The explicitly selected
//! directory is private compiler scratch space. A checksum detects corruption, not
//! a malicious owner who can replace both the record and its checksum. Even that
//! owner must not be able to feed invalid UTF-8 to the native unchecked string
//! decoder: CheckedDecoder overrides that operation. Native AST decoding otherwise
//! uses the compiler's own schema, bounded by the envelope and admission limits.
//!
//! Positions are never relocated. The key includes the exact normalized source
//! bytes, original-source hash/normalization information, filename, edition, strip
//! mode and absolute start position. The encoder checks *every encoded span* for
//! root hygiene, no parent, and membership in that exact source (or DUMMY_SP).
//! The decoder checks the same source bounds on every span before construction.
//! Symbols are spelled out by MemEncoder and re-interned on decode.

use std::env;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use rustc_ast as ast;
use rustc_serialize::opaque::mem_encoder::MemEncoder;
use rustc_serialize::opaque::{MAGIC_END_BYTES, MemDecoder};
use rustc_serialize::{Decodable, Decoder, Encodable, Encoder};
use rustc_session::parse::ParseSess;
use rustc_span::def_id::{CrateNum, DefId, DefIndex};
use rustc_span::hygiene::ExpnId;
use rustc_span::{
    AttrId, BlobDecoder, BytePos, ByteSymbol, SourceFile, SourceFileHash, SourceFileHashAlgorithm,
    Span, SpanDecoder, SpanEncoder, Symbol, SyntaxContext,
};

use crate::resident_ast_eligibility::{Limits, inspect};

const MAGIC: &[u8; 16] = b"JRDISKAST0000001";
const SCHEMA: u32 = 2;
const HEADER: usize = 16 + 4 + 32 + 8 + 32;
const MAX_PAYLOAD: usize = 8 * 1024 * 1024;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_SEQUENCE: usize = 4096;
// A direct-mapped cache bounds persistent scratch to 32 records plus one staging
// record, each at most HEADER + MAX_PAYLOAD bytes, without a directory scan.
const SLOTS: usize = 32;
const TEMP: &str = ".justrust-disk-ast-v1.tmp";
const LOCK: &str = ".justrust-disk-ast-v1.lock";

fn directory() -> Option<PathBuf> {
    env::var_os("JUSTRUST_RUSTC_DISK_AST").filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// An explicit directory is required. No implicit home/cache directory is used.
/// This experimental implementation only supports the Linux native compiler.
pub fn enabled() -> bool {
    cfg!(target_os = "linux") && directory().is_some()
}

fn telemetry(event: &str) {
    let Some(path) = env::var_os("JUSTRUST_RUSTC_DISK_AST_STATS").filter(|v| !v.is_empty()) else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            if private_file(&file).is_ok() {
                // One small append, no diagnostics or build-result changes on IO
                // failure. The caller explicitly owns telemetry file lifecycle.
                let _ = file.write_all(
                    format!("{{\"event\":\"{event}\",\"pid\":{}}}\n", std::process::id())
                        .as_bytes(),
                );
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (path, event);
}

/// Independent of STATS: disabled timing does not read the clock, and telemetry
/// failures never affect cache acceptance or the compiler result.
pub(crate) struct Phases(Option<PathBuf>);

impl Phases {
    pub(crate) fn new() -> Self {
        Self(
            env::var_os("JUSTRUST_RUSTC_DISK_AST_PHASES")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        )
    }

    pub(crate) fn measure<T>(&self, phase: &str, bytes: usize, operation: impl FnOnce() -> T) -> T {
        let Some(path) = &self.0 else { return operation() };
        let start = std::time::Instant::now();
        let result = operation();
        let elapsed_ns = start.elapsed().as_nanos();
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .append(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
            {
                if private_file(&file).is_ok() {
                    let _ = file.write_all(format!(
                        "{{\"event\":\"phase\",\"pid\":{},\"phase\":\"{phase}\",\"elapsed_ns\":{elapsed_ns},\"bytes\":{bytes}}}\n",
                        std::process::id(),
                    ).as_bytes());
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (path, phase, bytes, elapsed_ns);
        result
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    SourceFileHash::new_in_memory(SourceFileHashAlgorithm::Blake3, bytes)
        .hash_bytes()
        .try_into()
        .expect("BLAKE3 has a 32-byte digest")
}

fn field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

/// The candidate builder supplies a digest of the complete native compiler and
/// library source/configuration, including dirty owned files. An upstream commit
/// alone is insufficient for an owned, dirty compiler. Missing exact build
/// identity deliberately disables reuse rather than hashing a large driver on
/// every fresh compiler process and contaminating the experiment's latency.
fn compiler_identity() -> Option<[u8; 32]> {
    static IDENTITY: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    *IDENTITY.get_or_init(|| {
        let source_hash = option_env!("JUSTRUST_NATIVE_COMPILER_SOURCE_HASH")?;
        if source_hash.len() != 64 || !source_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let version_hash = option_env!("CFG_VER_HASH")?;
        if version_hash.is_empty() {
            return None;
        }
        let mut bytes = Vec::new();
        field(&mut bytes, b"justrust native disk AST compiler/schema v1");
        field(&mut bytes, &SCHEMA.to_le_bytes());
        field(&mut bytes, source_hash.as_bytes());
        field(&mut bytes, version_hash.as_bytes());
        field(&mut bytes, option_env!("CFG_VERSION").unwrap_or("").as_bytes());
        field(&mut bytes, &digest(include_str!("disk_ast.rs").as_bytes()));
        Some(digest(&bytes))
    })
}

fn key(psess: &ParseSess, source: &SourceFile, strip: u8) -> Option<[u8; 32]> {
    let text = source.src.as_ref()?;
    if text.len() > MAX_SOURCE || strip > 2 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() + 256);
    field(&mut bytes, &compiler_identity()?);
    field(&mut bytes, format!("{:?}", psess.edition).as_bytes());
    field(&mut bytes, &[strip]);
    field(&mut bytes, &source.start_pos.0.to_le_bytes());
    field(&mut bytes, format!("{:?}", source.name).as_bytes());
    field(&mut bytes, source.src_hash.hash_bytes());
    field(&mut bytes, &source.unnormalized_source_len.to_le_bytes());
    field(&mut bytes, format!("{:?}", source.normalized_pos).as_bytes());
    field(&mut bytes, text.as_bytes());
    Some(digest(&bytes))
}

fn envelope(key: &[u8; 32], payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD || !payload.ends_with(MAGIC_END_BYTES) {
        return None;
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&SCHEMA.to_le_bytes());
    bytes.extend_from_slice(key);
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    // Authenticate the header as well as the payload. This is accidental-
    // corruption detection, not authentication against the cache's own owner.
    let checksum =
        SourceFileHash::new(SourceFileHashAlgorithm::Blake3, bytes.as_slice().chain(payload))
            .ok()?;
    bytes.extend_from_slice(checksum.hash_bytes());
    bytes.extend_from_slice(payload);
    Some(bytes)
}

/// Validate *all* record bounds, exact key, header, checksum and native footer
/// before constructing any decoder. Never treat a checksum as a trust boundary.
fn open_envelope<'a>(bytes: &'a [u8], expected: &[u8; 32]) -> Option<&'a [u8]> {
    if bytes.len() < HEADER || bytes.len() > HEADER + MAX_PAYLOAD {
        return None;
    }
    if bytes.get(..16)? != MAGIC || bytes.get(16..20)? != SCHEMA.to_le_bytes() {
        return None;
    }
    if bytes.get(20..52)? != expected {
        return None;
    }
    let len = usize::try_from(u64::from_le_bytes(bytes.get(52..60)?.try_into().ok()?)).ok()?;
    if len > MAX_PAYLOAD || HEADER.checked_add(len)? != bytes.len() {
        return None;
    }
    let payload = bytes.get(HEADER..)?;
    if !payload.ends_with(MAGIC_END_BYTES) {
        return None;
    }
    let checksum =
        SourceFileHash::new(SourceFileHashAlgorithm::Blake3, bytes.get(..60)?.chain(payload))
            .ok()?;
    if checksum.hash_bytes() != bytes.get(60..HEADER)? {
        return None;
    }
    Some(payload)
}

fn slot(key: &[u8; 32]) -> String {
    format!("justrust-disk-ast-v1-{:02}.ast", usize::from(key[0]) % SLOTS)
}

/// Load only after the caller's source/ParseSess purity checks and fresh lexing.
/// A miss, rejection, IO error or unsupported platform simply requests parsing.
pub fn load(psess: &ParseSess, source: &SourceFile, strip: u8) -> Option<ast::Crate> {
    if !enabled() {
        return None;
    }
    let phases = Phases::new();
    let result = (|| {
        let key = phases
            .measure("key", source.src.as_ref().map_or(0, |s| s.len()), || {
                key(psess, source, strip)
            })
            .ok_or(())?;
        let opened = phases
            .measure("open", 0, || {
                let dir = PrivateDir::open(
                    directory().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?,
                )?;
                let file = dir.open_file(&slot(&key), false);
                Ok::<_, io::Error>((dir, file))
            })
            .map_err(|_| ())?;
        let (_dir, file) = opened;
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        let file_len = file.metadata().map_err(|_| ())?.len();
        if file_len > (HEADER + MAX_PAYLOAD) as u64 {
            return Err(());
        }
        let mut bytes = Vec::new();
        phases
            .measure("read", file_len as usize, || {
                (&mut file).take((HEADER + MAX_PAYLOAD + 1) as u64).read_to_end(&mut bytes)
            })
            .map_err(|_| ())?;
        let payload =
            phases.measure("envelope", bytes.len(), || open_envelope(&bytes, &key)).ok_or(())?;
        let (krate, gallery) = phases
            .measure("decode", payload.len(), || {
                let len =
                    usize::try_from(u64::from_le_bytes(payload.get(..8)?.try_into().ok()?)).ok()?;
                let split = 8usize.checked_add(len)?;
                let gallery =
                    crate::disk_ast_gallery::Gallery::decode(payload.get(8..split)?, source)?;
                Some((decode(payload.get(split..)?, source)?, gallery))
            })
            .ok_or(())?;
        // Recheck the admitted subset after decoding. Every span was checked
        // directly by CheckedDecoder, without re-encoding the entire AST.
        // This does not make malicious, owner-authored ASTs a supported input.
        if phases.measure("admit", payload.len(), || inspect(&krate, Limits::default())).is_err() {
            return Err(());
        }
        phases.measure("gallery_replay", payload.len(), || gallery.apply(psess));
        Ok(Some(krate))
    })();
    match result {
        Ok(Some(krate)) => {
            telemetry("hit");
            Some(krate)
        }
        Ok(None) => {
            telemetry("miss");
            None
        }
        Err(()) => {
            telemetry("reject");
            None
        }
    }
}

/// Store only a successfully parsed, effect-free AST admitted by the caller.
/// Internal admission is defense in depth, not a replacement for those checks.
pub(crate) fn store(
    psess: &ParseSess,
    source: &SourceFile,
    strip: u8,
    krate: &ast::Crate,
    gallery: &crate::disk_ast_gallery::Gallery,
) {
    if !enabled() {
        return;
    }
    let phases = Phases::new();
    let stored = (|| {
        if inspect(krate, Limits::default()).is_err() {
            return None;
        }
        let key = key(psess, source, strip)?;
        let payload =
            phases.measure("store_encode", source.src.as_ref().map_or(0, |s| s.len()), || {
                let ast = encode(krate, source)?;
                let effects = gallery.encode();
                let mut payload = Vec::with_capacity(8 + effects.len() + ast.len());
                payload.extend_from_slice(&(effects.len() as u64).to_le_bytes());
                payload.extend_from_slice(&effects);
                payload.extend_from_slice(&ast);
                Some(payload)
            })?;
        let bytes = envelope(&key, &payload)?;
        let dir = PrivateDir::open(directory()?).ok()?;
        phases.measure("store_write", bytes.len(), || {
            let mut file = dir.open_file(TEMP, true).ok()?;
            file.set_len(0).ok()?;
            file.write_all(&bytes).ok()?;
            // Atomic publication prevents readers from seeing incomplete records.
            // Durability is not required: a lost record is an ordinary cache miss.
            dir.rename(TEMP, &slot(&key)).ok()
        })?;
        Some(())
    })();
    telemetry(if stored.is_some() { "store" } else { "reject" });
}

/// The encoder wrapper sees every native Span, including less obvious spans in
/// identifiers, literals and delimiters. A visitor over only expressions would
/// not be an adequate hygiene/parent proof.
struct SnapshotEncoder {
    inner: MemEncoder,
    start: u32,
    end: u32,
    valid: bool,
}

macro_rules! encode_scalars {
    ($($method:ident: $ty:ty),* $(,)?) => {
        $(fn $method(&mut self, value: $ty) { self.inner.$method(value); })*
    };
}

impl Encoder for SnapshotEncoder {
    fn emit_usize(&mut self, value: usize) {
        self.valid &= value <= MAX_SEQUENCE;
        self.inner.emit_usize(value);
    }
    encode_scalars! {
        emit_u128: u128, emit_u64: u64, emit_u32: u32, emit_u16: u16, emit_u8: u8,
        emit_isize: isize, emit_i128: i128, emit_i64: i64, emit_i32: i32, emit_i16: i16,
    }
    fn emit_raw_bytes(&mut self, bytes: &[u8]) {
        self.inner.emit_raw_bytes(bytes);
    }
}

impl SpanEncoder for SnapshotEncoder {
    fn encode_span(&mut self, span: Span) {
        let data = span.data();
        self.valid &= data.ctxt.is_root()
            && data.parent.is_none()
            && (span.is_dummy()
                || (data.lo.0 >= self.start && data.hi.0 <= self.end && data.lo <= data.hi));
        self.inner.encode_span(span);
    }
    fn encode_symbol(&mut self, symbol: Symbol) {
        self.emit_str(symbol.as_str());
    }
    fn encode_byte_symbol(&mut self, symbol: ByteSymbol) {
        self.emit_byte_str(symbol.as_byte_str());
    }
    fn encode_expn_id(&mut self, _: ExpnId) {
        self.valid = false;
    }
    fn encode_syntax_context(&mut self, _: SyntaxContext) {
        self.valid = false;
    }
    fn encode_crate_num(&mut self, _: CrateNum) {
        self.valid = false;
    }
    fn encode_def_index(&mut self, _: DefIndex) {
        self.valid = false;
    }
    fn encode_def_id(&mut self, _: DefId) {
        self.valid = false;
    }
}

fn encode(krate: &ast::Crate, source: &SourceFile) -> Option<Vec<u8>> {
    let (start, end) = source_bounds(source)?;
    let mut encoder = SnapshotEncoder { inner: MemEncoder::new(), start, end, valid: true };
    krate.encode(&mut encoder);
    if !encoder.valid || encoder.inner.position() > MAX_PAYLOAD - MAGIC_END_BYTES.len() {
        return None;
    }
    let mut bytes = encoder.inner.finish();
    // Unlike FileEncoder, MemEncoder::finish does not append the native footer.
    bytes.extend_from_slice(MAGIC_END_BYTES);
    Some(bytes)
}

/// Do not call MemDecoder::read_str: its sentinel is not a UTF-8 validator.
/// The native integer/raw-byte readers do bounds checks. This wrapper also
/// limits collection lengths, validates string bytes, and rejects unsupported
/// session-local IDs instead of inventing replacements.
struct CheckedDecoder<'a> {
    inner: MemDecoder<'a>,
    start: u32,
    end: u32,
}

fn invalid() -> ! {
    // resume_unwind bypasses the global panic hook: corrupt scratch must not
    // print diagnostics. Only our enclosing catch_unwind receives this payload.
    std::panic::resume_unwind(Box::new(()))
}

macro_rules! decode_scalars {
    ($($method:ident: $ty:ty),* $(,)?) => {
        $(fn $method(&mut self) -> $ty { self.inner.$method() })*
    };
}

impl Decoder for CheckedDecoder<'_> {
    fn read_usize(&mut self) -> usize {
        let value = self.inner.read_usize();
        if value > MAX_SEQUENCE {
            invalid();
        }
        value
    }
    decode_scalars! {
        read_u128: u128, read_u64: u64, read_u32: u32, read_u16: u16, read_u8: u8,
        read_isize: isize, read_i128: i128, read_i64: i64, read_i32: i32, read_i16: i16,
    }
    fn read_raw_bytes(&mut self, len: usize) -> &[u8] {
        if len > self.inner.remaining() {
            invalid();
        }
        self.inner.read_raw_bytes(len)
    }
    fn read_str(&mut self) -> &str {
        let len = self.read_usize();
        let bytes = self.read_raw_bytes(len.checked_add(1).unwrap_or_else(|| invalid()));
        if bytes[len] != 0xc1 {
            invalid();
        }
        std::str::from_utf8(&bytes[..len]).unwrap_or_else(|_| invalid())
    }
    fn read_byte_str(&mut self) -> &[u8] {
        let len = self.read_usize();
        let bytes = self.read_raw_bytes(len.checked_add(1).unwrap_or_else(|| invalid()));
        if bytes[len] != 0xc2 {
            invalid();
        }
        &bytes[..len]
    }
    fn peek_byte(&self) -> u8 {
        if self.inner.remaining() == 0 {
            invalid();
        }
        self.inner.peek_byte()
    }
    fn position(&self) -> usize {
        self.inner.position()
    }
}

impl BlobDecoder for CheckedDecoder<'_> {
    fn decode_symbol(&mut self) -> Symbol {
        Symbol::intern(self.read_str())
    }
    fn decode_byte_symbol(&mut self) -> ByteSymbol {
        ByteSymbol::intern(self.read_byte_str())
    }
    fn decode_def_index(&mut self) -> DefIndex {
        invalid()
    }
}

impl SpanDecoder for CheckedDecoder<'_> {
    fn decode_span(&mut self) -> Span {
        let lo = BytePos::decode(self);
        let hi = BytePos::decode(self);
        // MemEncoder stores only endpoints, so root hygiene and no parent are
        // restored below. Only (0, 0) is dummy, matching Span::is_dummy, and it
        // remains valid even when this exact source starts at a nonzero offset.
        if lo > hi || !((lo.0 == 0 && hi.0 == 0) || (lo.0 >= self.start && hi.0 <= self.end)) {
            invalid();
        }
        Span::new(lo, hi, SyntaxContext::root(), None)
    }
    fn decode_expn_id(&mut self) -> ExpnId {
        invalid()
    }
    fn decode_syntax_context(&mut self) -> SyntaxContext {
        invalid()
    }
    fn decode_crate_num(&mut self) -> CrateNum {
        invalid()
    }
    fn decode_def_id(&mut self) -> DefId {
        invalid()
    }
    fn decode_attr_id(&mut self) -> AttrId {
        invalid()
    }
}

fn source_bounds(source: &SourceFile) -> Option<(u32, u32)> {
    let start = source.start_pos.0;
    let end = start.checked_add(u32::try_from(source.src.as_ref()?.len()).ok()?)?;
    Some((start, end))
}

fn decode(payload: &[u8], source: &SourceFile) -> Option<ast::Crate> {
    let (start, end) = source_bounds(source)?;
    let inner = MemDecoder::new(payload, 0).ok()?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut decoder = CheckedDecoder { inner, start, end };
        let krate = ast::Crate::decode(&mut decoder);
        if decoder.inner.remaining() != 0 {
            invalid();
        }
        krate
    }))
    .ok()
}

#[cfg(target_os = "linux")]
fn private_file(file: &File) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions and does not mutate Rust memory.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "nonprivate AST cache file"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct PrivateDir {
    file: File,
    // Keep the nonblocking exclusive flock until every operation finishes. All
    // opens and rename use the pinned directory FD, not a raceable pathname.
    _lock: File,
}

#[cfg(target_os = "linux")]
impl PrivateDir {
    fn open(path: PathBuf) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "nonprivate AST cache dir",
            ));
        }
        let lock = Self::open_at(&file, LOCK, true)?;
        // SAFETY: the FD is owned and remains live, and flock has no Rust-memory
        // side effects. Contention rejects the cache rather than blocking parse.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file, _lock: lock })
    }

    fn open_at(dir: &File, name: &str, create: bool) -> io::Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = std::ffi::CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid cache name"))?;
        let flags = libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if create { libc::O_RDWR | libc::O_CREAT } else { libc::O_RDONLY };
        // SAFETY: valid live directory FD, NUL-terminated basename, and supplied
        // mode for O_CREAT. The returned FD is adopted exactly once on success.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned FD.
        let file = unsafe { File::from_raw_fd(fd) };
        private_file(&file)?;
        Ok(file)
    }

    fn open_file(&self, name: &str, create: bool) -> io::Result<File> {
        Self::open_at(&self.file, name, create)
    }

    fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let from = std::ffi::CString::new(from).unwrap();
        let to = std::ffi::CString::new(to).unwrap();
        // SAFETY: both names are internal NUL-terminated basenames and the same
        // pinned directory FD remains live for the whole atomic rename.
        if unsafe {
            libc::renameat(self.file.as_raw_fd(), from.as_ptr(), self.file.as_raw_fd(), to.as_ptr())
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
struct PrivateDir;

#[cfg(not(target_os = "linux"))]
impl PrivateDir {
    fn open(_: PathBuf) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "disk AST prototype requires Linux"))
    }
    fn open_file(&self, _: &str, _: bool) -> io::Result<File> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Vec<u8> {
        let mut payload = b"native bytes".to_vec();
        payload.extend_from_slice(MAGIC_END_BYTES);
        payload
    }

    #[test]
    fn envelope_roundtrip_and_exact_key() {
        let payload = payload();
        let bytes = envelope(&[7; 32], &payload).unwrap();
        assert_eq!(open_envelope(&bytes, &[7; 32]), Some(payload.as_slice()));
        assert!(open_envelope(&bytes, &[8; 32]).is_none());
    }

    #[test]
    fn every_truncation_and_single_byte_corruption_is_rejected_before_decode() {
        let bytes = envelope(&[7; 32], &payload()).unwrap();
        for len in 0..bytes.len() {
            assert!(open_envelope(&bytes[..len], &[7; 32]).is_none(), "truncated at {len}");
        }
        for index in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[index] ^= 1;
            assert!(open_envelope(&damaged, &[7; 32]).is_none(), "damaged at {index}");
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(open_envelope(&trailing, &[7; 32]).is_none());
    }

    #[test]
    fn envelope_rejects_oversize_and_missing_native_footer() {
        assert!(envelope(&[7; 32], b"missing footer").is_none());
        let mut too_big = vec![0; MAX_PAYLOAD + 1];
        too_big.extend_from_slice(MAGIC_END_BYTES);
        assert!(envelope(&[7; 32], &too_big).is_none());
        let mut bytes = envelope(&[7; 32], &payload()).unwrap();
        bytes[52..60].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(open_envelope(&bytes, &[7; 32]).is_none());
        assert!(open_envelope(&vec![0; HEADER + MAX_PAYLOAD + 1], &[7; 32]).is_none());
    }

    #[test]
    fn checked_decoder_rejects_non_utf8_even_with_native_string_sentinel() {
        let mut encoder = MemEncoder::new();
        encoder.emit_usize(1);
        encoder.emit_raw_bytes(&[0xff, 0xc1]);
        let mut bytes = encoder.finish();
        bytes.extend_from_slice(MAGIC_END_BYTES);
        let inner = MemDecoder::new(&bytes, 0).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut decoder = CheckedDecoder { inner, start: 100, end: 200 };
                let _ = decoder.read_str();
            }))
            .is_err()
        );
    }

    // Encode endpoints directly so even reversed spans reach CheckedDecoder
    // without Span::new normalizing them first.
    fn decode_test_span(lo: u32, hi: u32) -> Result<Span, Box<dyn std::any::Any + Send>> {
        let mut encoder = MemEncoder::new();
        BytePos(lo).encode(&mut encoder);
        BytePos(hi).encode(&mut encoder);
        let mut bytes = encoder.finish();
        bytes.extend_from_slice(MAGIC_END_BYTES);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut decoder =
                CheckedDecoder { inner: MemDecoder::new(&bytes, 0).unwrap(), start: 100, end: 200 };
            let span = decoder.decode_span();
            assert_eq!(decoder.inner.remaining(), 0);
            span
        }))
    }

    #[test]
    fn checked_decoder_rejects_every_out_of_source_span_shape() {
        rustc_span::create_default_session_globals_then(|| {
            for (lo, hi) in [
                (99, 100),
                (100, 201),
                (0, 100),
                (0, 1),
                (201, 201),
                (150, 149),
                (u32::MAX, u32::MAX),
            ] {
                assert!(decode_test_span(lo, hi).is_err(), "accepted {lo}..{hi}");
            }
        });
    }

    #[test]
    fn checked_decoder_accepts_dummy_and_exact_source_boundaries() {
        rustc_span::create_default_session_globals_then(|| {
            let dummy = decode_test_span(0, 0).expect("dummy outside source remains valid");
            assert!(dummy.is_dummy());
            assert!(dummy.ctxt().is_root());
            assert!(dummy.parent().is_none());
            for (lo, hi) in [(100, 100), (100, 200), (200, 200)] {
                let span = decode_test_span(lo, hi).expect("exact source bounds");
                assert_eq!((span.lo().0, span.hi().0), (lo, hi));
            }
        });
    }

    #[test]
    fn native_ast_codec_roundtrips_spans_and_reinterns_symbols_across_sessions() {
        let text = "pub fn disk_snapshot_value(x: u32) -> u32 { (x ^ 7) & 255 }";
        let (bytes, original_start) = rustc_span::create_default_session_globals_then(|| {
            let psess = ParseSess::new();
            let source = psess.source_map().new_source_file(
                rustc_span::FileName::anon_source_code("disk-codec"),
                text.to_owned(),
            );
            let mut parser = crate::new_parser_from_source_file(
                &psess,
                source.clone(),
                crate::lexer::StripTokens::Nothing,
            )
            .unwrap_or_else(|_| panic!("valid lexing"));
            let krate = parser.parse_crate_mod().unwrap_or_else(|_| panic!("valid AST"));
            assert!(inspect(&krate, Limits::default()).is_ok());
            (encode(&krate, &source).expect("admitted native AST codec"), source.start_pos)
        });
        rustc_span::create_default_session_globals_then(|| {
            // Perturb interning order. Serialized symbol indices would be wrong.
            // Deliberately perturb dynamic interner order before restoring text symbols.
            #[allow(rustc::symbol_intern_string_literal)]
            Symbol::intern("different_first_symbol");
            let psess = ParseSess::new();
            let source = psess.source_map().new_source_file(
                rustc_span::FileName::anon_source_code("disk-codec"),
                text.to_owned(),
            );
            assert_eq!(source.start_pos, original_start);
            let krate = decode(&bytes, &source).expect("native AST roundtrip");
            assert_eq!(krate.items[0].kind.ident().unwrap().name.as_str(), "disk_snapshot_value");
            assert_eq!(encode(&krate, &source).unwrap(), bytes);
            assert!(inspect(&krate, Limits::default()).is_ok());
            let shifted_source = psess.source_map().new_source_file(
                rustc_span::FileName::anon_source_code("different-source-offset"),
                text.to_owned(),
            );
            assert_ne!(shifted_source.start_pos, source.start_pos);
            assert!(decode(&bytes, &shifted_source).is_none());
        });
    }

    #[test]
    fn encoder_rejects_parent_spans_and_out_of_source_positions() {
        rustc_span::create_default_session_globals_then(|| {
            let mut encoder =
                SnapshotEncoder { inner: MemEncoder::new(), start: 100, end: 200, valid: true };
            encoder.encode_span(Span::new(BytePos(100), BytePos(200), SyntaxContext::root(), None));
            assert!(encoder.valid);
            encoder.encode_span(Span::new(BytePos(99), BytePos(200), SyntaxContext::root(), None));
            assert!(!encoder.valid);
            encoder.valid = true;
            encoder.encode_span(Span::new(
                BytePos(100),
                BytePos(200),
                SyntaxContext::root(),
                Some(rustc_span::def_id::LocalDefId { local_def_index: DefIndex::from_u32(1) }),
            ));
            assert!(!encoder.valid);
        });
    }
}
