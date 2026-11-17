//! Compiler-owned proc-macro dispatch, separate from the unchanged dylib ABI.
//! Remote descriptors identify one request-owned helper. They are never query
//! cache keys or a reason to reuse a prior macro expansion.

use rustc_data_structures::AtomicRef;
use rustc_proc_macro::bridge::PanicMessage;
use rustc_proc_macro::bridge::client::Client;
use rustc_proc_macro::bridge::server::{self, ExecutionStrategy, Server};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteMacro {
    pub request: u64,
    pub library: u32,
    pub client: u32,
}

pub type RemoteDispatch = fn(
    RemoteMacro,
    bool,
    Vec<u8>,
    &mut dyn FnMut(Vec<u8>) -> Vec<u8>,
) -> Result<Vec<u8>, PanicMessage>;

/// The owning component installs a dispatcher that validates the active request
/// before sending any bytes. An uninstalled or retired dispatcher fails closed.
pub static REMOTE_DISPATCH: AtomicRef<RemoteDispatch> = AtomicRef::new(
    &((|_, _, _, _| Err(PanicMessage::StaticStr("no active request-owned proc-macro helper")))
        as RemoteDispatch),
);

#[derive(Clone, Copy)]
pub enum ProcMacroClient {
    Local(Client),
    Remote(RemoteMacro),
}

impl From<Client> for ProcMacroClient {
    fn from(client: Client) -> Self {
        Self::Local(client)
    }
}

impl ProcMacroClient {
    /// Only a local client may enter the existing cached-expansion callbacks.
    pub fn local(self) -> Option<Client> {
        match self {
            Self::Local(client) => Some(client),
            Self::Remote(_) => None,
        }
    }

    pub fn run1<S: Server>(
        self,
        strategy: &impl ExecutionStrategy,
        server: S,
        input: S::TokenStream,
        show_panics: bool,
    ) -> Result<S::TokenStream, PanicMessage> {
        match self {
            Self::Local(client) => client.run1(strategy, server, input, show_panics),
            Self::Remote(remote) => server::run1_external(server, input, |dispatcher, input| {
                (*REMOTE_DISPATCH)(remote, show_panics, input, &mut |request| {
                    dispatcher.dispatch_bytes(request)
                })
            }),
        }
    }

    pub fn run2<S: Server>(
        self,
        strategy: &impl ExecutionStrategy,
        server: S,
        input: S::TokenStream,
        input2: S::TokenStream,
        show_panics: bool,
    ) -> Result<S::TokenStream, PanicMessage> {
        match self {
            Self::Local(client) => client.run2(strategy, server, input, input2, show_panics),
            Self::Remote(remote) => {
                server::run2_external(server, input, input2, |dispatcher, input| {
                    (*REMOTE_DISPATCH)(remote, show_panics, input, &mut |request| {
                        dispatcher.dispatch_bytes(request)
                    })
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_descriptors_never_enter_local_query_cache() {
        let remote = RemoteMacro { request: 1, library: 2, client: 3 };
        assert!(ProcMacroClient::Remote(remote).local().is_none());
        let local = Client::expand1(|input| input);
        assert!(ProcMacroClient::Local(local).local().is_some());
    }
}
