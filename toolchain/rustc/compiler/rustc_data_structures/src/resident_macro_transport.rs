//! Bounded, synchronous transport for the native proc-macro helper.
//!
//! This module does not launch processes or depend on sockets. A connected
//! `UnixStream`, for example, can be passed directly to either pump. An exchange
//! owns its stream: after a terminal frame it must be discarded, not reused.
//! This deliberately prevents trailing lifecycle frames from becoming a second
//! request. No probing read follows completion, since generic `Read` may block.
//!
//! Once sending Invoke has been attempted, errors MUST NOT trigger replay. The
//! peer may have executed arbitrary macro or callback side effects even when a
//! write, flush, read, or response fails. Establishing a new helper is safe only
//! for a new request, never as an implicit retry of this request.

use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Counts wire bytes in both directions, including every frame header.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024 * 1024;
const MAGIC: [u8; 8] = *b"JRPMRPC\0";
const VERSION: u16 = 1;
const HEADER_BYTES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Invoke(Vec<u8>),
    Callback(Vec<u8>),
    Reply(Vec<u8>),
    Complete(Vec<u8>),
    Failure(Vec<u8>),
    Shutdown,
}

impl Frame {
    fn parts(&self) -> (u8, &[u8]) {
        match self {
            Self::Invoke(bytes) => (1, bytes),
            Self::Callback(bytes) => (2, bytes),
            Self::Reply(bytes) => (3, bytes),
            Self::Complete(bytes) => (4, bytes),
            Self::Failure(bytes) => (5, bytes),
            Self::Shutdown => (6, &[]),
        }
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Writes one versioned frame and flushes it. Partial writes poison the stream.
pub fn write_frame<W: Write + ?Sized>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let (kind, payload) = frame.parts();
    if payload.len() > MAX_FRAME_BYTES {
        return Err(invalid("proc-macro frame exceeds size limit"));
    }
    let mut header = [0; HEADER_BYTES];
    header[..8].copy_from_slice(&MAGIC);
    header[8..10].copy_from_slice(&VERSION.to_le_bytes());
    header[10] = kind;
    header[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    writer.write_all(&header)?;
    writer.write_all(payload)?;
    writer.flush()
}

/// Reads exactly one frame. EOF, including clean EOF between frames, is an error.
pub fn read_frame<R: Read + ?Sized>(reader: &mut R) -> io::Result<Frame> {
    read_bounded_frame(reader, MAX_FRAME_BYTES + HEADER_BYTES)
}

fn read_bounded_frame<R: Read + ?Sized>(reader: &mut R, remaining: usize) -> io::Result<Frame> {
    if remaining < HEADER_BYTES {
        return Err(invalid("proc-macro request exceeds size limit"));
    }
    let mut header = [0; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    if header[..8] != MAGIC
        || u16::from_le_bytes([header[8], header[9]]) != VERSION
        || header[11] != 0
    {
        return Err(invalid("invalid proc-macro frame magic, version, or reserved bits"));
    }
    let kind = header[10];
    let len = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if !(1..=6).contains(&kind) || (kind == 6 && len != 0) {
        return Err(invalid("invalid proc-macro frame kind or shutdown length"));
    }
    if len > MAX_FRAME_BYTES || len > remaining - HEADER_BYTES {
        return Err(invalid("proc-macro frame or request exceeds size limit"));
    }
    let mut payload = Vec::new();
    payload.try_reserve_exact(len).map_err(|_| invalid("cannot allocate proc-macro frame"))?;
    payload.resize(len, 0);
    reader.read_exact(&mut payload)?;
    Ok(match kind {
        1 => Frame::Invoke(payload),
        2 => Frame::Callback(payload),
        3 => Frame::Reply(payload),
        4 => Frame::Complete(payload),
        5 => Frame::Failure(payload),
        6 => Frame::Shutdown,
        _ => unreachable!(),
    })
}

struct Exchange<S> {
    stream: S,
    remaining: usize,
    terminal: bool,
}

impl<S: Read + Write> Exchange<S> {
    fn new(stream: S) -> Self {
        Self { stream, remaining: MAX_REQUEST_BYTES, terminal: false }
    }

    fn send(&mut self, frame: Frame) -> io::Result<()> {
        if self.terminal {
            return Err(invalid("frame after terminal proc-macro message"));
        }
        let len = frame.parts().1.len() + HEADER_BYTES;
        if len > self.remaining {
            self.terminal = true;
            return Err(invalid("proc-macro request exceeds size limit"));
        }
        // Poison before IO: an error must never permit retrying a partial frame.
        self.terminal = true;
        write_frame(&mut self.stream, &frame)?;
        self.remaining -= len;
        self.terminal = matches!(frame, Frame::Complete(_) | Frame::Failure(_) | Frame::Shutdown);
        Ok(())
    }

    fn receive(&mut self) -> io::Result<Frame> {
        if self.terminal {
            return Err(invalid("frame after terminal proc-macro message"));
        }
        self.terminal = true;
        let frame = read_bounded_frame(&mut self.stream, self.remaining)?;
        self.remaining -= frame.parts().1.len() + HEADER_BYTES;
        self.terminal = matches!(frame, Frame::Complete(_) | Frame::Failure(_) | Frame::Shutdown);
        Ok(frame)
    }
}

/// A helper's terminal result. Failure bytes are opaque, not necessarily UTF-8.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete(Vec<u8>),
    Failure(Vec<u8>),
    Shutdown,
}

/// Compiler-side pump. Dispatches any number of callbacks before one terminal
/// result. Callback errors/panics close this exchange without replaying Invoke.
/// The caller must discard the underlying connection on every return path.
pub fn server_pump<S, C>(stream: S, invocation: Vec<u8>, mut callback: C) -> io::Result<Outcome>
where
    S: Read + Write,
    C: FnMut(Vec<u8>) -> io::Result<Vec<u8>>,
{
    let mut exchange = Exchange::new(stream);
    exchange.send(Frame::Invoke(invocation))?;
    loop {
        match exchange.receive()? {
            Frame::Callback(payload) => {
                let reply = catch_unwind(AssertUnwindSafe(|| callback(payload)))
                    .map_err(|_| io::Error::other("proc-macro callback panicked"))??;
                exchange.send(Frame::Reply(reply))?;
            }
            Frame::Complete(payload) => return Ok(Outcome::Complete(payload)),
            Frame::Failure(payload) => return Ok(Outcome::Failure(payload)),
            _ => return Err(invalid("unexpected frame while awaiting macro result")),
        }
    }
}

/// Helper-side pump for exactly one invocation, or an idle Shutdown. Operation
/// errors and panics become Failure frames. A callback protocol/IO error poisons
/// the exchange, so no subsequent callback or completion can be emitted.
pub fn helper_pump<S, H>(stream: S, handler: H) -> io::Result<Outcome>
where
    S: Read + Write,
    H: FnOnce(Vec<u8>, &mut dyn FnMut(Vec<u8>) -> io::Result<Vec<u8>>) -> io::Result<Vec<u8>>,
{
    let mut exchange = Exchange::new(stream);
    let invocation = match exchange.receive()? {
        Frame::Invoke(payload) => payload,
        Frame::Shutdown => return Ok(Outcome::Shutdown),
        _ => return Err(invalid("expected Invoke or idle Shutdown")),
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        handler(invocation, &mut |payload| {
            exchange.send(Frame::Callback(payload))?;
            match exchange.receive()? {
                Frame::Reply(reply) => Ok(reply),
                _ => {
                    exchange.terminal = true;
                    Err(invalid("expected callback Reply"))
                }
            }
        })
    }));
    let outcome = match result {
        Ok(Ok(payload)) => Outcome::Complete(payload),
        Ok(Err(error)) => Outcome::Failure(error.to_string().into_bytes()),
        Err(_) => Outcome::Failure(b"proc-macro helper panicked".to_vec()),
    };
    let frame = match &outcome {
        Outcome::Complete(payload) => Frame::Complete(payload.clone()),
        Outcome::Failure(payload) => Frame::Failure(payload.clone()),
        Outcome::Shutdown => unreachable!(),
    };
    exchange.send(frame)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encoded(frame: Frame) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        bytes
    }

    #[test]
    fn round_trip_all_kinds() {
        for frame in [
            Frame::Invoke(vec![0, 255]),
            Frame::Callback(vec![1]),
            Frame::Reply(vec![]),
            Frame::Complete(vec![2]),
            Frame::Failure(vec![255]),
            Frame::Shutdown,
        ] {
            assert_eq!(read_frame(&mut Cursor::new(encoded(frame.clone()))).unwrap(), frame);
        }
    }

    #[test]
    fn malformed_headers_and_lengths() {
        for index in [0, 8, 9, 10, 11] {
            let mut bytes = encoded(Frame::Shutdown);
            bytes[index] = 255;
            assert_eq!(
                read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        for (kind, len) in [(6, 1), (1, MAX_FRAME_BYTES as u32 + 1), (1, u32::MAX)] {
            let mut bytes = encoded(Frame::Shutdown);
            bytes[10] = kind;
            bytes[12..16].copy_from_slice(&len.to_le_bytes());
            assert_eq!(
                read_frame(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn every_truncation_is_disconnect() {
        let bytes = encoded(Frame::Invoke(vec![1, 2, 3]));
        for end in 0..bytes.len() {
            assert_eq!(
                read_frame(&mut Cursor::new(&bytes[..end])).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
    }

    #[test]
    fn budget_is_bidirectional_and_checked_before_allocation() {
        let mut exchange = Exchange::new(Cursor::new(encoded(Frame::Reply(vec![1, 2]))));
        exchange.remaining = HEADER_BYTES + 1;
        assert!(exchange.receive().is_err());
        assert!(exchange.receive().is_err());
        let mut exchange = Exchange::new(Cursor::new(Vec::new()));
        exchange.remaining = HEADER_BYTES + 3;
        exchange.send(Frame::Callback(vec![1, 2])).unwrap();
        assert_eq!(exchange.remaining, 1);
        assert!(exchange.send(Frame::Reply(vec![1, 2])).is_err());
        assert!(exchange.send(Frame::Reply(vec![])).is_err());
    }

    #[test]
    fn terminal_frames_reject_trailing_lifecycle_messages() {
        for terminal in [Frame::Complete(vec![]), Frame::Failure(vec![]), Frame::Shutdown] {
            let mut bytes = encoded(terminal.clone());
            bytes.extend(encoded(Frame::Invoke(vec![])));
            let mut exchange = Exchange::new(Cursor::new(bytes));
            assert_eq!(exchange.receive().unwrap(), terminal);
            assert!(exchange.receive().is_err());
            assert!(exchange.send(Frame::Reply(vec![])).is_err());
            let mut exchange = Exchange::new(Cursor::new(Vec::new()));
            exchange.send(terminal).unwrap();
            assert!(exchange.send(Frame::Invoke(vec![])).is_err());
        }
    }

    #[test]
    fn helper_rejects_non_invocation() {
        for frame in [
            Frame::Reply(vec![]),
            Frame::Callback(vec![]),
            Frame::Complete(vec![]),
            Frame::Failure(vec![]),
        ] {
            assert!(
                helper_pump(Cursor::new(encoded(frame)), |_, _| panic!("must not run")).is_err()
            );
        }
        assert_eq!(
            helper_pump(Cursor::new(encoded(Frame::Shutdown)), |_, _| panic!("must not run"))
                .unwrap(),
            Outcome::Shutdown
        );
    }

    struct Script {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Script {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.input.read(bytes)
        }
    }

    impl Write for Script {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rejects_invalid_reply_and_never_completes_after_callback_error() {
        for frame in [
            Frame::Invoke(vec![]),
            Frame::Callback(vec![]),
            Frame::Complete(vec![]),
            Frame::Failure(vec![]),
            Frame::Shutdown,
        ] {
            let mut input = encoded(Frame::Invoke(vec![]));
            input.extend(encoded(frame));
            let mut script = Script { input: Cursor::new(input), output: Vec::new() };
            assert!(
                helper_pump(&mut script, |_, callback| {
                    // Even a handler that swallows the error cannot complete.
                    assert!(callback(vec![1]).is_err());
                    Ok(vec![])
                })
                .is_err()
            );
            let mut output = Cursor::new(script.output);
            assert_eq!(read_frame(&mut output).unwrap(), Frame::Callback(vec![1]));
            assert!(read_frame(&mut output).is_err());
        }
    }

    #[test]
    fn server_rejects_unexpected_lifecycle_and_catches_callback_panic() {
        for frame in
            [Frame::Invoke(vec![]), Frame::Reply(vec![]), Frame::Shutdown, Frame::Callback(vec![])]
        {
            let mut script = Script { input: Cursor::new(encoded(frame)), output: Vec::new() };
            assert!(server_pump(&mut script, vec![9], |_| panic!("callback panic")).is_err());
            let mut output = Cursor::new(script.output);
            assert_eq!(read_frame(&mut output).unwrap(), Frame::Invoke(vec![9]));
            assert!(read_frame(&mut output).is_err());
        }
    }

    #[test]
    fn ordinary_handler_error_is_terminal_failure() {
        let mut script =
            Script { input: Cursor::new(encoded(Frame::Invoke(vec![]))), output: Vec::new() };
        assert_eq!(
            helper_pump(&mut script, |_, _| Err(io::Error::other("failed"))).unwrap(),
            Outcome::Failure(b"failed".to_vec())
        );
        assert_eq!(
            read_frame(&mut Cursor::new(script.output)).unwrap(),
            Frame::Failure(b"failed".to_vec())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_repeated_callbacks_and_completion() {
        use std::os::unix::net::UnixStream;
        let (server, helper) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            helper_pump(helper, |input, callback| {
                assert_eq!(input, b"invoke");
                for n in 0..8 {
                    assert_eq!(callback(vec![n])?, vec![n, 42]);
                }
                Ok(b"done".to_vec())
            })
            .unwrap()
        });
        let mut calls = 0;
        assert_eq!(
            server_pump(server, b"invoke".to_vec(), |mut input| {
                calls += 1;
                input.push(42);
                Ok(input)
            })
            .unwrap(),
            Outcome::Complete(b"done".to_vec())
        );
        assert_eq!(calls, 8);
        assert_eq!(worker.join().unwrap(), Outcome::Complete(b"done".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn unix_panics_are_failure_and_disconnect_is_not_replayed() {
        use std::os::unix::net::UnixStream;
        let (server, helper) = UnixStream::pair().unwrap();
        let worker =
            std::thread::spawn(move || helper_pump(helper, |_, _| panic!("macro panic")).unwrap());
        assert!(matches!(
            server_pump(server, vec![], |_| unreachable!()).unwrap(),
            Outcome::Failure(_)
        ));
        assert!(matches!(worker.join().unwrap(), Outcome::Failure(_)));
        let (server, mut helper) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            assert_eq!(read_frame(&mut helper).unwrap(), Frame::Invoke(vec![7]));
        });
        assert!(server_pump(server, vec![7], |_| unreachable!()).is_err());
        worker.join().unwrap();
    }
}
