struct Pair {
    a: u64,
    b: u64,
}

fn bump(p: *mut Pair) {
    unsafe {
        (*p).b = (*p).a + 10;
    }
}

fn read<T: Copy>(p: *const T) -> T {
    unsafe { *p }
}

struct Cell<T> {
    ptr: *mut T,
}

fn main() {
    let mut x: u64 = 3;
    let p = &mut x as *mut u64;
    let q = p as *const u64;
    let y = unsafe { *q };
    unsafe {
        *p = y + 4;
    }
    if x == 7 {
        println!("write through *mut");
    }
    let mut s = Pair { a: 1, b: 2 };
    bump(&mut s as *mut Pair);
    if s.b == 11 {
        println!("field through *mut");
    }
    let mut v = 5i32;
    let c = Cell { ptr: &mut v as *mut i32 };
    unsafe {
        *c.ptr = 9;
    }
    if read(c.ptr as *const i32) == 9 {
        println!("generic read");
    }
    let addr = p as usize;
    let back = addr as *const u64;
    if back == q && addr != 0 {
        println!("usize round trip");
    }
    let r = &v as *const i32;
    let r2 = r;
    if r == r2 {
        println!("pointer equality");
    }
}
