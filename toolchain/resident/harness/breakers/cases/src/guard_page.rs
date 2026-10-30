//! A frame larger than the stack it is on has to fault on the stack's guard page, where std
//! says the thread overflowed its stack and aborts. Cranelift's frames touched no page on
//! their way down except on Windows, so a large frame stepped over the guard page and its
//! first write landed in whatever was mapped below: rgb's game client died of a SIGILL on
//! CoreAudio's thread. rustc's code touches every page of such a frame on the way down
//! (`probe-stack=inline-asm`), and ra-jit's does now. The frame here writes only its lowest
//! byte: one that wrote all of it upwards would reach the guard page from below.

#[inline(never)]
fn large_frame() -> u8 {
    let mut buffer = std::mem::MaybeUninit::<[u8; 1024 * 1024]>::uninit();
    let lowest = buffer.as_mut_ptr().cast::<u8>();
    // SAFETY: the byte is the buffer's own.
    unsafe {
        lowest.write(7);
        std::hint::black_box(lowest).read()
    }
}

pub fn run() -> u8 {
    let thread = std::thread::Builder::new().name("small".to_owned()).stack_size(64 * 1024);
    thread.spawn(large_frame).unwrap().join().unwrap()
}
