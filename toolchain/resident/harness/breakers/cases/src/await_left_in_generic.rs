//! Found in den-native: `Link::start(wake: impl Fn())` spawned a task that awaited
//! `Client::connect`, a function ra-jit left to rustc. Roots that hold a future of a function
//! left to rustc are left too, but `start` is generic, so its task was an instance of ours: it
//! polled rustc's future with our poll, read the `&self` rustc had put elsewhere as null, and
//! crashed. Here `once` is left to rustc for the constant `caller_in_constant` is about, so
//! `present`, whose future `once` holds, is left too, and `start` awaits `present`.
//!
//! Both compilers happen to lay `present`'s future out alike, so this printed the right answer
//! before the fix as well: which locals a future keeps comes from each compiler's MIR, and
//! den-native's differed where this one does not. It holds the path the fix takes: the poll
//! of `present`'s future is refused, and `start` calls rustc's copy.
//!
//! Both compilers happen to lay `present`'s future out alike, so this printed the right answer
//! before the fix as well: which locals a future keeps comes from each compiler's MIR, and
//! den-native's differed where this one does not. It holds the path the fix takes: the poll
//! of `present`'s future is refused, and `start` calls rustc's copy.

use std::future::Future;
use std::panic::Location;
use std::pin::{pin, Pin};
use std::task::{Context, Poll, Waker};

/// Pending once, then ready.
struct Once(bool);

impl Future for Once {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if std::mem::replace(&mut self.0, true) { Poll::Ready(()) } else { Poll::Pending }
    }
}

struct Connector {
    name: &'static str,
    port: u32,
}

fn read(file: &str) -> Result<String, ()> {
    if file.is_empty() { Err(()) } else { Ok(file.repeat(2)) }
}

impl Connector {
    async fn present(&self, conn: &mut Vec<u8>) -> u32 {
        let file = format!("{}/daemon.owner", self.name);
        let Ok(text) = read(&file) else { return 0 };
        let params = vec![text.len() as u32; 4];
        match rpc(conn, "daemon/auth", params).await {
            Ok(n) => n + self.port,
            Err(()) => 1,
        }
    }
}

async fn rpc(conn: &mut Vec<u8>, method: &str, params: Vec<u32>) -> Result<u32, ()> {
    let request = format!("{method} {params:?}");
    conn.extend_from_slice(request.as_bytes());
    let mut line = String::new();
    let scratch = [7u8; 256];
    Once(false).await;
    line.push_str(&request);
    let sum: u32 = scratch.iter().map(|&b| u32::from(b)).sum();
    Ok(line.len() as u32 + conn.len() as u32 + sum)
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

#[inline(never)]
fn once() -> u32 {
    const CALLER: &Location<'static> = Location::caller();
    let connector = Connector { name: "once", port: 1 };
    let mut conn = Vec::new();
    block_on(connector.present(&mut conn)) + u32::from(CALLER.line() > 0)
}

fn start(wake: impl Fn() -> u32) -> u32 {
    block_on(async move {
        let connector = Connector { name: "den", port: 1000 };
        let mut conn = Vec::new();
        connector.present(&mut conn).await + wake()
    })
}

pub fn run() -> (u32, u32, u32) {
    let base = 5;
    (start(|| base), start(move || base * 2), once())
}
