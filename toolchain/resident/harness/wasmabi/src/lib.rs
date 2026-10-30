//! `extern "C"` functions whose wasm signatures `wasmabi.py` compares with rustc's: an
//! aggregate that is one scalar goes as that scalar, anything else in memory.
#![allow(improper_ctypes, improper_ctypes_definitions)]

use std::marker::PhantomData;

#[repr(C)]
pub struct Word(u32);

#[repr(C)]
pub struct Byte(u8);

#[repr(C)]
pub struct Wide(u64);

/// As wasm-bindgen's `WasmRet<u32>`: one scalar and fields of no size.
#[repr(C)]
pub struct WithUnits(u32, (), PhantomData<u8>, ());

#[repr(C)]
pub struct Nested(Word);

#[repr(C)]
pub union Same {
    a: u32,
    b: i32,
}

#[repr(C)]
pub union Mixed {
    a: u32,
    b: f32,
}

#[repr(C)]
pub struct Two(u32, u32);

#[repr(C)]
pub struct Huge(u128);

#[repr(C, align(8))]
pub struct Padded(u32);

#[unsafe(no_mangle)]
pub extern "C" fn word(x: Word) -> Word {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn byte(x: Byte) -> Byte {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn wide(x: Wide) -> Wide {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn float(x: [f32; 1]) -> [f64; 1] {
    [x[0] as f64]
}

#[unsafe(no_mangle)]
pub extern "C" fn with_units(x: WithUnits) -> WithUnits {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn nested(x: Nested) -> (Nested, ()) {
    (x, ())
}

#[unsafe(no_mangle)]
pub extern "C" fn same(x: Same) -> Same {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn mixed(x: Mixed) -> Mixed {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn two(x: Two) -> Two {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn huge(x: Huge) -> Huge {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn plain(x: u128) -> u128 {
    x
}

#[unsafe(no_mangle)]
pub extern "C" fn padded(x: Padded) -> Padded {
    x
}

unsafe extern "C" {
    fn imported(x: WithUnits, y: Two) -> WithUnits;
}

#[unsafe(no_mangle)]
pub extern "C" fn calls_imported(x: WithUnits, y: Two) -> WithUnits {
    unsafe { imported(x, y) }
}
