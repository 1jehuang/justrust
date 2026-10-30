//! triomphe orders `HeaderSlice<H, [T]>`, a struct that ends in a slice, by a derive. The
//! derive asks for the discriminant behind each reference, which read the reference as one
//! scalar, and it is a pointer and a length.

#[derive(PartialEq, PartialOrd)]
#[repr(C)]
struct HeaderSlice<H, T: ?Sized> {
    header: H,
    slice: T,
}

pub fn run() -> [bool; 4] {
    let small: &HeaderSlice<u8, [u8]> = &HeaderSlice { header: 1, slice: [1, 2] };
    let large: &HeaderSlice<u8, [u8]> = &HeaderSlice { header: 1, slice: [1, 2, 3] };
    [small <= large, small >= large, small <= small, &small < &large]
}
