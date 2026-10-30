use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicBool, Ordering};

// What C++ does: the names alone, no path to the crate that defines them.
unsafe extern "C" {
    static EXPORTED_VERSION: *const c_char;
    static EXPORTED_FLAG: AtomicBool;
    static exported_count: u32;
}

fn main() {
    exported_statics::set_flag();
    unsafe {
        println!("{:?}", CStr::from_ptr(EXPORTED_VERSION));
        println!("{}", EXPORTED_FLAG.load(Ordering::Relaxed));
        println!("{exported_count}");
    }
}
