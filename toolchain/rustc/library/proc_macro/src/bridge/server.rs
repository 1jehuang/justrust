//! Server-side traits.

use std::cell::Cell;
use std::hash::Hash;
use std::ops::{Bound, Range};
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::{panic, thread};

use crate::bridge::{
    ApiTags, BridgeConfig, Buffer, Decode, Diagnostic, Encode, ExpnGlobals, Literal, Mark, Marked,
    PanicMessage, TokenTree, client, handle,
};

pub(super) struct HandleStore<S: Server> {
    token_stream: handle::OwnedStore<MarkedTokenStream<S>>,
    span: handle::InternedStore<MarkedSpan<S>>,
}

impl<S: Server> HandleStore<S> {
    fn new() -> Self {
        static TOKEN_STREAM: AtomicU32 = AtomicU32::new(1);
        static SPAN: AtomicU32 = AtomicU32::new(1);

        HandleStore {
            token_stream: handle::OwnedStore::new(&TOKEN_STREAM),
            span: handle::InternedStore::new(&SPAN),
        }
    }
}

pub(super) type MarkedTokenStream<S> = Marked<<S as Server>::TokenStream, client::TokenStream>;
pub(super) type MarkedSpan<S> = Marked<<S as Server>::Span, client::Span>;
pub(super) type MarkedSymbol<S> = Marked<<S as Server>::Symbol, client::Symbol>;

impl<S: Server> Encode<HandleStore<S>> for MarkedTokenStream<S> {
    fn encode(self, w: &mut Buffer, s: &mut HandleStore<S>) {
        s.token_stream.alloc(self).encode(w, s);
    }
}

impl<S: Server> Decode<'_, '_, HandleStore<S>> for MarkedTokenStream<S> {
    fn decode(r: &mut &[u8], s: &mut HandleStore<S>) -> Self {
        s.token_stream.take(handle::Handle::decode(r, &mut ()))
    }
}

impl<'s, S: Server> Decode<'_, 's, HandleStore<S>> for &'s MarkedTokenStream<S> {
    fn decode(r: &mut &[u8], s: &'s mut HandleStore<S>) -> Self {
        &s.token_stream[handle::Handle::decode(r, &mut ())]
    }
}

impl<S: Server> Encode<HandleStore<S>> for MarkedSpan<S> {
    fn encode(self, w: &mut Buffer, s: &mut HandleStore<S>) {
        s.span.alloc(self).encode(w, s);
    }
}

impl<S: Server> Decode<'_, '_, HandleStore<S>> for MarkedSpan<S> {
    fn decode(r: &mut &[u8], s: &mut HandleStore<S>) -> Self {
        s.span.copy(handle::Handle::decode(r, &mut ()))
    }
}

macro_rules! define_server {
    (
        $(fn $method:ident($($arg:ident: $arg_ty:ty),* $(,)?) $(-> $ret_ty:ty)?;)*
    ) => {
        pub trait Server {
            type TokenStream: 'static + Clone + Default;
            type Span: 'static + Copy + Eq + Hash;
            type Symbol: 'static;

            fn globals(&mut self) -> ExpnGlobals<Self::Span>;

            /// Intern a symbol received from RPC
            fn intern_symbol(ident: &str) -> Self::Symbol;

            /// Recover the string value of a symbol, and invoke a callback with it.
            fn with_symbol_string(symbol: &Self::Symbol, f: impl FnOnce(&str));

            $(fn $method(&mut self, $($arg: $arg_ty),*) $(-> $ret_ty)?;)*
        }
    }
}
with_api!(define_server, Self::TokenStream, Self::Span, Self::Symbol);

// FIXME(eddyb) `pub` only for `ExecutionStrategy` below.
pub struct Dispatcher<S: Server> {
    handle_store: HandleStore<S>,
    server: S,
}

macro_rules! define_dispatcher {
    (
        $(fn $method:ident($($arg:ident: $arg_ty:ty),* $(,)?) $(-> $ret_ty:ty)?;)*
    ) => {
        impl<S: Server> Dispatcher<S> {
            fn dispatch(&mut self, mut buf: Buffer) -> Buffer {
                let Dispatcher { handle_store, server } = self;

                let mut reader = &buf[..];
                match ApiTags::decode(&mut reader, &mut ()) {
                    $(ApiTags::$method => {
                        let mut call_method = || {
                            $(let $arg = <$arg_ty>::decode(&mut reader, handle_store).unmark();)*
                            let r = server.$method($($arg),*);
                            $(let r: $ret_ty = Mark::mark(r);)?
                            r
                        };
                        // HACK(eddyb) don't use `panic::catch_unwind` in a panic.
                        // If client and server happen to use the same `std`,
                        // `catch_unwind` asserts that the panic counter was 0,
                        // even when the closure passed to it didn't panic.
                        let r = if thread::panicking() {
                            Ok(call_method())
                        } else {
                            panic::catch_unwind(panic::AssertUnwindSafe(call_method))
                                .map_err(PanicMessage::from)
                        };

                        buf.clear();
                        r.encode(&mut buf, handle_store);
                    })*
                }
                buf
            }
        }
    }
}
with_api!(define_dispatcher, MarkedTokenStream<S>, MarkedSpan<S>, MarkedSymbol<S>);

impl<S: Server> Dispatcher<S> {
    /// Dispatch one owned wire request without sharing allocator-owned buffers
    /// with another process. Handles remain owned by this expansion's server.
    pub fn dispatch_bytes(&mut self, request: Vec<u8>) -> Vec<u8> {
        self.dispatch(Buffer::from(request)).to_vec()
    }
}

/// Execute a one-input expansion through an external byte transport. This does
/// not change the dylib `Client` ABI or permit executing a client function pointer
/// in another process. The caller must independently identify and load that
/// client in its fresh helper process.
pub fn run1_external<S: Server>(
    server: S,
    input: S::TokenStream,
    execute: impl FnOnce(&mut Dispatcher<S>, Vec<u8>) -> Result<Vec<u8>, PanicMessage>,
) -> Result<S::TokenStream, PanicMessage> {
    run_server_external(server, <MarkedTokenStream<S>>::mark(input), execute)
        .map(|s| <Option<MarkedTokenStream<S>>>::unmark(s).unwrap_or_default())
}

/// Two-input counterpart used by attribute macros.
pub fn run2_external<S: Server>(
    server: S,
    input: S::TokenStream,
    input2: S::TokenStream,
    execute: impl FnOnce(&mut Dispatcher<S>, Vec<u8>) -> Result<Vec<u8>, PanicMessage>,
) -> Result<S::TokenStream, PanicMessage> {
    run_server_external(
        server,
        (<MarkedTokenStream<S>>::mark(input), <MarkedTokenStream<S>>::mark(input2)),
        execute,
    )
    .map(|s| <Option<MarkedTokenStream<S>>>::unmark(s).unwrap_or_default())
}

fn run_server_external<
    S: Server,
    I: Encode<HandleStore<S>>,
    O: for<'a, 's> Decode<'a, 's, HandleStore<S>>,
>(
    server: S,
    input: I,
    execute: impl FnOnce(&mut Dispatcher<S>, Vec<u8>) -> Result<Vec<u8>, PanicMessage>,
) -> Result<O, PanicMessage> {
    let mut dispatcher = Dispatcher { handle_store: HandleStore::new(), server };
    let globals = dispatcher.server.globals();
    let mut buf = Buffer::new();
    (<ExpnGlobals<MarkedSpan<S>> as Mark>::mark(globals), input)
        .encode(&mut buf, &mut dispatcher.handle_store);
    let output = execute(&mut dispatcher, buf.to_vec())?;
    Result::decode(&mut &output[..], &mut dispatcher.handle_store)
}

impl client::Client {
    /// Run a helper-local client against a byte callback transport. The input,
    /// output and callbacks are copied into buffers owned by this process.
    pub fn run_external_bytes(
        &self,
        input: Vec<u8>,
        mut dispatch: impl FnMut(Vec<u8>) -> Vec<u8>,
        force_show_panics: bool,
    ) -> Vec<u8> {
        let mut callback = |request: Buffer| Buffer::from(dispatch(request.to_vec()));
        (self.run)(BridgeConfig {
            input: Buffer::from(input),
            dispatch: (&mut callback).into(),
            force_show_panics,
        })
        .to_vec()
    }
}

#[cfg(test)]
mod external_tests {
    use super::*;

    struct TestServer;

    macro_rules! test_dispatch {
        (ts_drop, $stream:ident) => { drop($stream) };
        (ts_clone, $stream:ident) => { $stream.clone() };
        (ts_is_empty, $stream:ident) => { $stream.is_empty() };
        ($method:ident $(, $arg:ident)*) => {{
            $(let _ = $arg;)*
            panic!("unexpected bridge method {}", stringify!($method))
        }};
    }

    macro_rules! test_server {
        ($(fn $method:ident($($arg:ident: $arg_ty:ty),* $(,)?) $(-> $ret_ty:ty)?;)*) => {
            impl Server for TestServer {
                type TokenStream = Vec<u32>;
                type Span = u32;
                type Symbol = String;

                fn globals(&mut self) -> ExpnGlobals<u32> {
                    ExpnGlobals { def_site: 1, call_site: 2, mixed_site: 3 }
                }

                fn intern_symbol(ident: &str) -> String { ident.to_owned() }

                fn with_symbol_string(symbol: &String, f: impl FnOnce(&str)) { f(symbol) }

                $(fn $method(&mut self, $($arg: $arg_ty),*) $(-> $ret_ty)? {
                    test_dispatch!($method $(, $arg)*)
                })*
            }
        };
    }
    with_api!(test_server, Self::TokenStream, Self::Span, Self::Symbol);

    #[test]
    fn external_bytes_preserve_handles_and_callbacks() {
        let client = client::Client::expand1(|input: crate::TokenStream| input.clone());
        let expected = vec![7, 11, 19];
        let result = run1_external(TestServer, expected.clone(), |dispatcher, input| {
            Ok(client.run_external_bytes(
                input,
                |request| dispatcher.dispatch_bytes(request),
                false,
            ))
        });
        assert_eq!(result.unwrap_or_else(|_| panic!("external expansion failed")), expected);
    }

    #[test]
    fn external_attribute_bytes_keep_inputs_separate() {
        let client = client::Client::expand2(|_attr, input: crate::TokenStream| input);
        let result = run2_external(TestServer, vec![2], vec![5, 8], |dispatcher, input| {
            Ok(client.run_external_bytes(
                input,
                |request| dispatcher.dispatch_bytes(request),
                false,
            ))
        });
        assert_eq!(result.unwrap_or_else(|_| panic!("external expansion failed")), vec![5, 8]);
    }

    #[test]
    fn external_transport_failure_does_not_run_client() {
        let result = run1_external(TestServer, vec![3], |_, _| {
            Err(PanicMessage::StaticStr("helper disconnected after request commit"))
        });
        match result {
            Err(message) => {
                assert_eq!(message.as_str(), Some("helper disconnected after request commit"))
            }
            Ok(_) => panic!("disconnected helper unexpectedly succeeded"),
        }
    }
}

// This trait is currently only implemented and used once, inside of this crate.
// We keep it public to allow implementing more complex execution strategies in
// the future, such as wasm proc-macros.
pub trait ExecutionStrategy {
    fn run_bridge_and_client(
        &self,
        dispatcher: &mut Dispatcher<impl Server>,
        input: Buffer,
        run_client: extern "C" fn(BridgeConfig<'_>) -> Buffer,
        force_show_panics: bool,
    ) -> Buffer;
}

thread_local! {
    /// While running a proc-macro with the same-thread executor, this flag will
    /// be set, forcing nested proc-macro invocations (e.g. due to
    /// `TokenStream::expand_expr`) to be run using a cross-thread executor.
    ///
    /// This is required as the thread-local state in the proc_macro client does
    /// not handle being re-entered, and will invalidate all `Symbol`s when
    /// entering a nested macro.
    static ALREADY_RUNNING_SAME_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Keep `ALREADY_RUNNING_SAME_THREAD` (see also its documentation)
/// set to `true`, preventing same-thread reentrance.
struct RunningSameThreadGuard(());

impl RunningSameThreadGuard {
    fn new() -> Self {
        let already_running = ALREADY_RUNNING_SAME_THREAD.replace(true);
        assert!(
            !already_running,
            "same-thread nesting (\"reentrance\") of proc macro executions is not supported"
        );
        RunningSameThreadGuard(())
    }
}

impl Drop for RunningSameThreadGuard {
    fn drop(&mut self) {
        ALREADY_RUNNING_SAME_THREAD.set(false);
    }
}

pub struct MaybeCrossThread {
    pub cross_thread: bool,
}

pub const SAME_THREAD: MaybeCrossThread = MaybeCrossThread { cross_thread: false };
pub const CROSS_THREAD: MaybeCrossThread = MaybeCrossThread { cross_thread: true };

impl ExecutionStrategy for MaybeCrossThread {
    fn run_bridge_and_client(
        &self,
        dispatcher: &mut Dispatcher<impl Server>,
        input: Buffer,
        run_client: extern "C" fn(BridgeConfig<'_>) -> Buffer,
        force_show_panics: bool,
    ) -> Buffer {
        if self.cross_thread || ALREADY_RUNNING_SAME_THREAD.get() {
            let (mut server, mut client) = MessagePipe::new();

            let join_handle = thread::spawn(move || {
                let mut dispatch = |b: Buffer| -> Buffer {
                    client.send(b);
                    client.recv().expect("server died while client waiting for reply")
                };

                run_client(BridgeConfig {
                    input,
                    dispatch: (&mut dispatch).into(),
                    force_show_panics,
                })
            });

            while let Some(b) = server.recv() {
                server.send(dispatcher.dispatch(b));
            }

            join_handle.join().unwrap()
        } else {
            let _guard = RunningSameThreadGuard::new();

            let mut dispatch = |buf| dispatcher.dispatch(buf);

            run_client(BridgeConfig { input, dispatch: (&mut dispatch).into(), force_show_panics })
        }
    }
}

/// A message pipe used for communicating between server and client threads.
struct MessagePipe<T> {
    tx: mpsc::SyncSender<T>,
    rx: mpsc::Receiver<T>,
}

impl<T> MessagePipe<T> {
    /// Creates a new pair of endpoints for the message pipe.
    fn new() -> (Self, Self) {
        let (tx1, rx1) = mpsc::sync_channel(1);
        let (tx2, rx2) = mpsc::sync_channel(1);
        (MessagePipe { tx: tx1, rx: rx2 }, MessagePipe { tx: tx2, rx: rx1 })
    }

    /// Send a message to the other endpoint of this pipe.
    fn send(&mut self, value: T) {
        self.tx.send(value).unwrap();
    }

    /// Receive a message from the other endpoint of this pipe.
    ///
    /// Returns `None` if the other end of the pipe has been destroyed, and no
    /// message was received.
    fn recv(&mut self) -> Option<T> {
        self.rx.recv().ok()
    }
}

fn run_server<
    S: Server,
    I: Encode<HandleStore<S>>,
    O: for<'a, 's> Decode<'a, 's, HandleStore<S>>,
>(
    strategy: &impl ExecutionStrategy,
    server: S,
    input: I,
    run_client: extern "C" fn(BridgeConfig<'_>) -> Buffer,
    force_show_panics: bool,
) -> Result<O, PanicMessage> {
    let mut dispatcher = Dispatcher { handle_store: HandleStore::new(), server };

    let globals = dispatcher.server.globals();

    let mut buf = Buffer::new();
    (<ExpnGlobals<MarkedSpan<S>> as Mark>::mark(globals), input)
        .encode(&mut buf, &mut dispatcher.handle_store);

    buf = strategy.run_bridge_and_client(&mut dispatcher, buf, run_client, force_show_panics);

    Result::decode(&mut &buf[..], &mut dispatcher.handle_store)
}

impl client::Client {
    pub fn run1<S>(
        &self,
        strategy: &impl ExecutionStrategy,
        server: S,
        input: S::TokenStream,
        force_show_panics: bool,
    ) -> Result<S::TokenStream, PanicMessage>
    where
        S: Server,
    {
        let client::Client { run } = *self;
        run_server(strategy, server, <MarkedTokenStream<S>>::mark(input), run, force_show_panics)
            .map(|s| <Option<MarkedTokenStream<S>>>::unmark(s).unwrap_or_default())
    }

    pub fn run2<S>(
        &self,
        strategy: &impl ExecutionStrategy,
        server: S,
        input: S::TokenStream,
        input2: S::TokenStream,
        force_show_panics: bool,
    ) -> Result<S::TokenStream, PanicMessage>
    where
        S: Server,
    {
        let client::Client { run } = *self;
        run_server(
            strategy,
            server,
            (<MarkedTokenStream<S>>::mark(input), <MarkedTokenStream<S>>::mark(input2)),
            run,
            force_show_panics,
        )
        .map(|s| <Option<MarkedTokenStream<S>>>::unmark(s).unwrap_or_default())
    }
}
