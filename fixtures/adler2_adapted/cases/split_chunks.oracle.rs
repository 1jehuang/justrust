extern crate adler2;
fn main() {
let p0: [u8; 5552] = [255; 5552];
let p1: [u8; 0] = [];
let p2: [u8; 16657] = [255; 16657];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
state.write_slice(&p1);
state.write_slice(&p2);
println!("{}", state.checksum());
}
