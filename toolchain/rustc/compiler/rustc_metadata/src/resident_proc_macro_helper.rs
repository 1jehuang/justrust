//! Fresh-exec proc-macro endpoint. The driver passes a dedicated inherited
//! socket, never stdin/stdout. Libraries intentionally live until process exit,
//! sharing macro statics across expansions in this compiler request only.
//!
//! This endpoint is currently non-reentrant: an Invoke while awaiting a callback
//! Reply is a fatal protocol error in `helper_pump`, not a macro panic or a reason
//! to replay an expansion. The compiler must not route nested invocations over
//! this connection until the transport has a reentrant callback pump.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rustc_data_structures::resident_macro_transport::{self as transport, Outcome};
use rustc_proc_macro::bridge::client::Client;

pub const MAX_LIBRARIES: usize = 256;
pub const MAX_MACROS: usize = 65_536;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const LOAD: u8 = 1;
const EXPAND: u8 = 2;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Wire format: tag 1, u32 path length, UTF-8 path, u32 symbol length,
/// UTF-8 symbol. All integers in this protocol are little endian.
pub fn encode_load(path: &Path, symbol: &str) -> io::Result<Vec<u8>> {
    let path = path.to_str().ok_or_else(|| invalid("proc-macro path is not UTF-8"))?;
    validate_text(path, symbol)?;
    let len = 9usize
        .checked_add(path.len())
        .and_then(|len| len.checked_add(symbol.len()))
        .filter(|&len| len <= MAX_FRAME_BYTES)
        .ok_or_else(|| invalid("proc-macro Load exceeds frame limit"))?;
    let mut bytes = Vec::with_capacity(len);
    bytes.push(LOAD);
    bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(&(symbol.len() as u32).to_le_bytes());
    bytes.extend_from_slice(symbol.as_bytes());
    Ok(bytes)
}

/// Wire format: tag 2, u32 library index, u32 client index, boolean byte,
/// followed by the unchanged ABI bridge input bytes.
pub fn encode_expand(
    library: u32,
    client: u32,
    show_panics: bool,
    input: &[u8],
) -> io::Result<Vec<u8>> {
    if library as usize >= MAX_LIBRARIES || client as usize >= MAX_MACROS {
        return Err(invalid("proc-macro index exceeds limit"));
    }
    if input.len() > MAX_FRAME_BYTES - 10 {
        return Err(invalid("proc-macro Expand exceeds frame limit"));
    }
    let mut bytes = Vec::with_capacity(10 + input.len());
    bytes.push(EXPAND);
    bytes.extend_from_slice(&library.to_le_bytes());
    bytes.extend_from_slice(&client.to_le_bytes());
    bytes.push(u8::from(show_panics));
    bytes.extend_from_slice(input);
    Ok(bytes)
}

/// Load Complete is exactly two u32 values: library index and client count.
pub fn decode_loaded(bytes: &[u8]) -> io::Result<(u32, u32)> {
    let mut input = bytes;
    let library = take_u32(&mut input)?;
    let count = take_u32(&mut input)?;
    if !input.is_empty() || library as usize >= MAX_LIBRARIES || count as usize > MAX_MACROS {
        return Err(invalid("invalid proc-macro Load response"));
    }
    Ok((library, count))
}

fn take_u32(input: &mut &[u8]) -> io::Result<u32> {
    let bytes = input.get(..4).ok_or_else(|| invalid("truncated proc-macro command"))?;
    let value = u32::from_le_bytes(bytes.try_into().unwrap());
    *input = &input[4..];
    Ok(value)
}

fn take_text<'a>(input: &mut &'a [u8]) -> io::Result<&'a str> {
    let len = take_u32(input)? as usize;
    let bytes = input.get(..len).ok_or_else(|| invalid("truncated proc-macro string"))?;
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("invalid proc-macro UTF-8"))?;
    *input = &input[len..];
    Ok(text)
}

fn validate_text(path: &str, symbol: &str) -> io::Result<()> {
    if path.is_empty() || symbol.is_empty() || path.contains('\0') || symbol.contains('\0') {
        return Err(invalid("empty or NUL-containing proc-macro path or symbol"));
    }
    Ok(())
}

struct Library {
    path: PathBuf,
    symbol: String,
    clients: &'static [Client],
}

fn load(libraries: &mut Vec<Library>, path: &str, symbol: &str) -> io::Result<Vec<u8>> {
    validate_text(path, symbol)?;
    // Resolve before calling the loader, whose canonicalization otherwise unwraps.
    let path = std::fs::canonicalize(path)?;
    let index = if let Some(index) =
        libraries.iter().position(|library| library.path == path && library.symbol == symbol)
    {
        index
    } else {
        if libraries.len() >= MAX_LIBRARIES {
            return Err(invalid("proc-macro library limit exceeded"));
        }
        // SAFETY: these are trusted compiler-selected proc-macro dylibs and their
        // generated declaration symbols, just as in dlsym_proc_macros. No pointer
        // or Client received over IPC is ever interpreted. The loader leaks the
        // library, keeping this slice valid until the fresh helper exits.
        let pointer = unsafe { crate::load_symbol_from_dylib::<*const &[Client]>(&path, symbol) }
            .map_err(|error| match error {
            crate::DylibError::DlOpen(path, message) => {
                io::Error::other(format!("cannot load proc-macro {path}: {message}"))
            }
            crate::DylibError::DlSym(path, message) => {
                io::Error::other(format!("cannot resolve proc-macro in {path}: {message}"))
            }
        })?;
        if pointer.is_null() {
            return Err(invalid("null proc-macro declaration symbol"));
        }
        // SAFETY: the generated symbol has the same trusted ABI as above.
        let clients = unsafe { *pointer };
        if clients.len() > MAX_MACROS {
            return Err(invalid("proc-macro client count exceeds limit"));
        }
        let index = libraries.len();
        libraries.push(Library { path, symbol: symbol.to_owned(), clients });
        index
    };
    let mut response = Vec::with_capacity(8);
    response.extend_from_slice(&(index as u32).to_le_bytes());
    response.extend_from_slice(&(libraries[index].clients.len() as u32).to_le_bytes());
    Ok(response)
}

/// Runs only inside the fresh helper, with resident mode disabled. Shutdown or
/// any Failure ends the session. No operation is retried after an Invoke.
pub fn run(stream: impl Read + Write) -> io::Result<()> {
    if rustc_session::resident::is_process() {
        return Err(io::Error::other("proc-macro helper must be a fresh non-resident process"));
    }
    let mut libraries = Vec::new();
    let mut session = transport::Session::new(stream);
    loop {
        // The ABI callback returns bytes rather than Result. Keep the first IO
        // failure outside that ABI, so an encoded client-side decode panic cannot
        // obscure it or turn an interrupted expansion into a successful reply.
        let mut callback_error = None;
        let result = session.respond(|payload, callback| {
            if payload.len() > MAX_FRAME_BYTES {
                return Err(invalid("proc-macro command exceeds frame limit"));
            }
            let (&tag, mut input) =
                payload.split_first().ok_or_else(|| invalid("empty proc-macro command"))?;
            match tag {
                LOAD => {
                    let path = take_text(&mut input)?;
                    let symbol = take_text(&mut input)?;
                    if !input.is_empty() {
                        return Err(invalid("trailing bytes in proc-macro Load"));
                    }
                    load(&mut libraries, path, symbol)
                }
                EXPAND => {
                    let library = take_u32(&mut input)? as usize;
                    let client = take_u32(&mut input)? as usize;
                    let (&show, input) = input
                        .split_first()
                        .ok_or_else(|| invalid("missing proc-macro panic flag"))?;
                    if show > 1 {
                        return Err(invalid("invalid proc-macro panic flag"));
                    }
                    let client = libraries
                        .get(library)
                        .and_then(|library| library.clients.get(client))
                        .ok_or_else(|| invalid("unknown proc-macro library or client index"))?;
                    let output = client.run_external_bytes(
                        input.to_vec(),
                        |request| {
                            if callback_error.is_some() {
                                return Vec::new();
                            }
                            match callback(request) {
                                Ok(reply) => reply,
                                Err(error) => {
                                    callback_error = Some(error);
                                    Vec::new()
                                }
                            }
                        },
                        show != 0,
                    );
                    if callback_error.is_some() {
                        return Err(io::Error::other("proc-macro callback transport failed"));
                    }
                    Ok(output)
                }
                _ => Err(invalid("unknown proc-macro command")),
            }
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
        match result? {
            Outcome::Complete(_) => {}
            Outcome::Shutdown => return Ok(()),
            Outcome::Failure(message) => {
                return Err(io::Error::other(String::from_utf8_lossy(&message).into_owned()));
            }
        }
    }
}
