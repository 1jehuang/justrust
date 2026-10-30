//! dtor reads `__dso_handle` and secp256k1 its default hash function from statics of an
//! `extern` block, one of them under a `link_name`.

#[unsafe(no_mangle)]
pub static BREAKERS_EXTERN_WORD: u32 = 7;

#[unsafe(no_mangle)]
pub static BREAKERS_EXTERN_DOUBLE: fn(u32) -> u32 = double;

fn double(n: u32) -> u32 {
    n * 2
}

mod foreign {
    unsafe extern "C" {
        #[link_name = "BREAKERS_EXTERN_WORD"]
        pub static WORD: u32;
        pub static BREAKERS_EXTERN_DOUBLE: fn(u32) -> u32;
    }
}

pub fn run() -> (u32, u32, bool) {
    let (word, double) = unsafe { (foreign::WORD, foreign::BREAKERS_EXTERN_DOUBLE) };
    (word, double(word), std::ptr::eq(&raw const foreign::WORD, &BREAKERS_EXTERN_WORD))
}
