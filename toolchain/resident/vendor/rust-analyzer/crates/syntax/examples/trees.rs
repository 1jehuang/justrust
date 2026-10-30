//! `cargo run --release -p syntax --example trees -- DIR...`: every `.rs` file under the
//! directories parsed into green trees and into flat ones, on one thread: the time to parse,
//! the bytes all the trees hold at once, and the time to walk every node and token of each
//! through the cursor's API, as lowering does, then every node alone, as an ast id map does,
//! and one node in 16 found again from a pointer, as a diagnostic's span is.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::{Path, PathBuf},
    sync::atomic::{AtomicIsize, Ordering},
    time::{Duration, Instant},
};

use syntax::{AstNode, Edition, SourceFile, SyntaxNodePtr, WalkEvent};

struct Counting;

static HELD: AtomicIsize = AtomicIsize::new(0);

// SAFETY: every call is passed on to the system's allocator, which is what holds the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: as the system allocator's own.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: as the system allocator's own.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|it| it == "rs") {
            out.push(path);
        }
    }
}

struct Measured {
    parsed: Duration,
    held: isize,
    walked: Duration,
    nodes: Duration,
    found: Duration,
    elements: usize,
}

fn measure(texts: &[String], green: bool) -> Measured {
    syntax::set_green_trees(green);
    let before = HELD.load(Ordering::Relaxed);
    let start = Instant::now();
    let parses: Vec<_> = texts.iter().map(|it| SourceFile::parse(it, Edition::CURRENT)).collect();
    let parsed = start.elapsed();
    let held = HELD.load(Ordering::Relaxed) - before;
    let start = Instant::now();
    let mut elements = 0;
    for parse in &parses {
        for event in parse.tree().syntax().preorder_with_tokens() {
            if let WalkEvent::Enter(it) = event {
                std::hint::black_box((it.kind(), it.text_range()));
                elements += 1;
            }
        }
    }
    let walked = start.elapsed();
    let start = Instant::now();
    let mut ptrs = Vec::new();
    for parse in &parses {
        let mut of_file = Vec::new();
        for (i, node) in parse.tree().syntax().descendants().enumerate() {
            std::hint::black_box((node.kind(), node.text_range()));
            if i % 16 == 0 {
                of_file.push(SyntaxNodePtr::new(&node));
            }
        }
        ptrs.push(of_file);
    }
    let nodes = start.elapsed();
    let start = Instant::now();
    for (parse, of_file) in parses.iter().zip(&ptrs) {
        let root = parse.tree().syntax().clone();
        for ptr in of_file {
            std::hint::black_box(ptr.to_node(&root));
        }
    }
    let found = start.elapsed();
    // A parse's tree is dropped on a thread of its own, so the next count waits for it.
    drop(parses);
    let mut last = HELD.load(Ordering::Relaxed);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now = HELD.load(Ordering::Relaxed);
        if now == last {
            break;
        }
        last = now;
    }
    Measured { parsed, held, walked, nodes, found, elements }
}

fn main() {
    let mut paths = Vec::new();
    for dir in std::env::args().skip(1) {
        files(Path::new(&dir), &mut paths);
    }
    let texts: Vec<String> =
        paths.iter().filter_map(|it| std::fs::read_to_string(it).ok()).collect();
    let bytes: usize = texts.iter().map(|it| it.len()).sum();
    let flat = measure(&texts, false);
    let green = measure(&texts, true);
    assert_eq!(flat.elements, green.elements, "the trees have the same elements");
    let ms = |it: Duration| it.as_secs_f64() * 1e3;
    let mb = |it: isize| it as f64 / (1 << 20) as f64;
    println!(
        "{} files, {:.1} MB of text, {} nodes and tokens",
        texts.len(),
        bytes as f64 / (1 << 20) as f64,
        flat.elements
    );
    for (name, it) in [("green", &green), ("flat", &flat)] {
        println!(
            "{name:5}: parsed in {:8.1} ms, holds {:8.1} MB, walked in {:8.1} ms, \
             its nodes in {:8.1} ms, one pointer in 16 found in {:8.1} ms",
            ms(it.parsed),
            mb(it.held),
            ms(it.walked),
            ms(it.nodes),
            ms(it.found)
        );
    }
}
