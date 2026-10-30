// bun's `Bun__version` and `Bun__Node__ZeroFillBuffers`: private statics that C++ reads by
// name. One no Rust code reads, one Rust writes, and one exported under another name.
use core::ffi::c_char;
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(transparent)]
struct Version(*const c_char);
unsafe impl Sync for Version {}

#[unsafe(no_mangle)]
static EXPORTED_VERSION: Version = Version(c"v1.2.3".as_ptr());

#[unsafe(no_mangle)]
static EXPORTED_FLAG: AtomicBool = AtomicBool::new(false);

#[unsafe(export_name = "exported_count")]
static COUNT: u32 = 7;

pub fn set_flag() {
    EXPORTED_FLAG.store(true, Ordering::Relaxed);
}
