//! A constant whose `None` is told from `Some` by a field deep inside the payload, promoted to a
//! static and compared with the same constant built at run time.

use std::collections::VecDeque;

#[derive(Debug, PartialEq)]
struct State(Option<VecDeque<(String, String)>>);

impl State {
    const NONE: State = State(None);
}

#[derive(Debug, PartialEq)]
struct Context {
    id: u128,
    span: u64,
    flags: u8,
    remote: bool,
    state: State,
}

impl Context {
    const NONE: Context = Context { id: 0, span: 0, flags: 0, remote: false, state: State::NONE };
}

struct Span {
    context: Context,
    inner: Option<Box<u8>>,
}

const NOOP: Span = Span { context: Context::NONE, inner: None };

fn noop() -> &'static Span {
    &NOOP
}

fn empty() -> Context {
    Context::NONE
}

pub fn run() -> (bool, bool, String) {
    let span = noop();
    (span.context == empty(), span.inner.is_none(), format!("{:?}", span.context))
}
