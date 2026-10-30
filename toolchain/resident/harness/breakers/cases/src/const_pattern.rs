//! A constant of a struct around an enum, matched as a pattern: `http::Method::GET` is one, and
//! one variant of the enum holds a `Box<[u8]>` whose null is where the others are told apart.

#[derive(PartialEq, Eq)]
enum Inner {
    Get,
    Head,
    Post,
    Short([u8; 15], u8),
    Long(Box<[u8]>),
}

#[derive(PartialEq, Eq)]
pub struct Method(Inner);

impl Method {
    pub const GET: Method = Method(Inner::Get);
    pub const HEAD: Method = Method(Inner::Head);
}

#[inline(never)]
fn has_payload(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD)
}

pub fn run() -> Vec<bool> {
    let all = [
        Method::GET,
        Method::HEAD,
        Method(Inner::Post),
        Method(Inner::Short([7; 15], 3)),
        Method(Inner::Long(vec![1, 2, 3].into_boxed_slice())),
    ];
    all.iter().map(has_payload).collect()
}
