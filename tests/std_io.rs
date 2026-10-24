//! In-memory std::io: io::Result, Read and BufRead for byte slices, BufReader.
//! `&mut R` forwarding impls type-check but are not yet built natively. Stock rustc is the oracle; no file/socket/stdin I/O.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::{Case, Expect};

#[test]
fn buffered_byte_readers_match_stock() {
    closure_oracle::check(
        "std-io",
        &[
            Case {
                name: "slice_and_bufreader",
                source: r#"use std::io::{self, BufRead, BufReader, Read};
fn total<R: BufRead>(mut reader: R) -> io::Result<u32> {
    let mut sum = 0u32;
    loop {
        let len = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() { return Ok(sum); }
            for &b in buf.iter() { sum += u32::from(b); }
            buf.len()
        };
        reader.consume(len);
    }
}
fn main() {
    let data: [u8; 5] = [1, 2, 3, 4, 250];
    let mut slice: &[u8] = &data;
    let mut out = [0u8; 3];
    let n = slice.read(&mut out).unwrap();
    let reader = BufReader::with_capacity(2, slice);
    let rest = total::<BufReader<&[u8]>>(reader).unwrap();
    let whole = total::<BufReader<&[u8]>>(BufReader::new(&data[..])).unwrap();
    let direct = total::<&[u8]>(&data[..]).unwrap();
    println!("{} {} {} {} {}", n, out[2], rest, whole, direct);
}"#,
                expect: Expect::Run("3 3 254 260 260\n"),
            },
            Case {
                name: "io_result_is_not_a_plain_value",
                source: r#"use std::io::{self, BufRead};
fn first<R: BufRead>(mut r: R) -> io::Result<u8> { let b = r.fill_buf()?; Ok(b[0]) }
fn main() { let d: &[u8] = &[7]; let v: u8 = first::<&[u8]>(d); println!("{}", v); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
