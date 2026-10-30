//! cpp_demangle's parser tries one production after another, each a call that returns a
//! `Result` of 168 bytes, and recurses up to 96 deep on the 2MB stack of a test thread: rustc
//! keeps one slot for each result and each binding, and a frame of twice that does not fit.
//! A local that is moved to be passed is not a second slot, nor is what is passed a third.

use std::hint::black_box;

pub struct Node {
    words: [usize; 17],
    child: Option<Box<Node>>,
}

type Parsed<'a> = Result<(Node, &'a [u8]), u8>;

#[inline(never)]
fn leaf<'a>(tag: u8, input: &'a [u8]) -> Parsed<'a> {
    match input.split_first() {
        Some((&first, tail)) if first == tag => {
            Ok((Node { words: [first as usize; 17], child: None }, tail))
        }
        _ => Err(tag),
    }
}

fn wrap(inner: Node) -> Node {
    Node { words: [inner.words[0] + 1; 17], child: Some(Box::new(inner)) }
}

#[inline(never)]
fn parse<'a>(input: &'a [u8], above: usize, frame: &mut usize) -> Parsed<'a> {
    let here = black_box(&above as *const usize as usize);
    *frame = above.wrapping_sub(here);
    if let Ok((node, tail)) = leaf(b'a', input) {
        return Ok((wrap(node), tail));
    }
    if let Ok((node, tail)) = leaf(b'b', input) {
        let node = wrap(node);
        return Ok((node, tail));
    }
    if let Ok((_, tail)) = leaf(b'P', input) {
        let (inner, tail) = parse(tail, here, frame)?;
        let node = wrap(inner);
        return Ok((node, tail));
    }
    if let Ok((_, tail)) = leaf(b'Q', input) {
        let (inner, tail) = parse(tail, here, frame)?;
        return Ok((wrap(wrap(inner)), tail));
    }
    let (node, tail) = leaf(b'c', input)?;
    let (other, tail) = leaf(b'd', tail)?;
    drop(other);
    Ok((node, tail))
}

/// What the parse gives, and whether a frame of `parse` is under 8k, as rustc's 6.3k is: a
/// smaller one fits too.
pub fn run() -> (Option<usize>, bool) {
    let mut frame = 0;
    let parsed = parse(b"PPQa", 0, &mut frame);
    (parsed.ok().map(|(node, _)| node.words[0]), frame < 8192)
}
