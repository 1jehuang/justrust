// std::io, as ordinary Rust source. This initial slice is the in-memory
// buffered-read surface: `Result`, an opaque `Error`, `ErrorKind`, `Read` and
// `BufRead` for byte slices, and `BufReader` over a `Read`. There is no file,
// socket or stdin I/O here; those need their own syscall-backed sources.

/// An I/O error. In-memory readers never fail, so only the kind is retained.
pub struct Error {
    kind: ErrorKind,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEof,
    InvalidInput,
    Other,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error { kind }
    }
}

impl super::fmt::Debug for Error {
    fn fmt(&self, f: &mut super::fmt::Formatter) -> super::fmt::Result {
        f.write_str(match self.kind {
            ErrorKind::UnexpectedEof => "Kind(UnexpectedEof)",
            ErrorKind::InvalidInput => "Kind(InvalidInput)",
            ErrorKind::Other => "Kind(Other)",
        })
    }
}

pub type Result<T> = super::result::Result<T, Error>;

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
}

pub trait BufRead {
    fn fill_buf(&mut self) -> Result<&[u8]>;
    fn consume(&mut self, amt: usize);
}

impl Read for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let source: &[u8] = *self;
        let amount = if buf.len() < source.len() { buf.len() } else { source.len() };
        let mut i = 0usize;
        while i < amount {
            buf[i] = source[i];
            i += 1usize;
        }
        *self = &source[amount..];
        super::result::Result::Ok(amount)
    }
}

impl BufRead for &[u8] {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        super::result::Result::Ok(*self)
    }
    fn consume(&mut self, amt: usize) {
        let source: &[u8] = *self;
        *self = &source[amt..];
    }
}

// std forwards both traits through `&mut R`, so `f(&mut reader)` works.
impl<R: Read> Read for &mut R {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
}

impl<B: BufRead> BufRead for &mut B {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        (**self).fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

/// Buffers a `Read` in 8 KiB blocks, like std's default `BufReader`.
pub struct BufReader<R> {
    inner: R,
    buf: super::vec::Vec<u8>,
    pos: usize,
    filled: usize,
}

impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> BufReader<R> {
        BufReader::with_capacity(8192usize, inner)
    }
    pub fn with_capacity(capacity: usize, inner: R) -> BufReader<R> {
        let mut buf = super::vec::Vec::with_capacity(capacity);
        let mut i = 0usize;
        while i < capacity {
            buf.push(0u8);
            i += 1usize;
        }
        BufReader { inner, buf, pos: 0usize, filled: 0usize }
    }
}

impl<R: Read> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.filled {
            let read = self.inner.read(&mut self.buf[..])?;
            self.filled = read;
            self.pos = 0usize;
        }
        super::result::Result::Ok(&self.buf[self.pos..self.filled])
    }
    fn consume(&mut self, amt: usize) {
        let next = self.pos + amt;
        self.pos = if next < self.filled { next } else { self.filled };
    }
}
