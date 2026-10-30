//! adler2's tests sum `&[0xff; 1024 * 1024]`, three of them in a function, on the two megabytes
//! of a test's thread. rustc points at read-only data; a megabyte of stack each overflows.

fn sum(bytes: &[u8]) -> u32 {
    bytes.iter().map(|&it| it as u32).sum()
}

fn sums() -> [u32; 3] {
    [sum(&[0xff; 1024 * 1024]), sum(&[0; 1024 * 1024]), sum(&[0xa5; 1024 * 1024 - 1024])]
}

pub fn run() -> [u32; 3] {
    let thread = std::thread::Builder::new().stack_size(2 * 1024 * 1024);
    thread.spawn(sums).unwrap().join().unwrap()
}
