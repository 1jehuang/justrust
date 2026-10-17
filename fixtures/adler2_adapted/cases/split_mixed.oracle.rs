extern crate adler2;
fn main() {
let p0: [u8; 2] = [0, 255];
let p1: [u8; 0] = [];
let p2: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
let p3: [u8; 3] = [255, 128, 1];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
state.write_slice(&p1);
state.write_slice(&p2);
state.write_slice(&p3);
println!("{}", state.checksum());
}
