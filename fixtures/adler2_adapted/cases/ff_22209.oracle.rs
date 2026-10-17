extern crate adler2;
fn main() {
let p0: [u8; 22209] = [255; 22209];
let mut state = adler2::Adler32::new();
state.write_slice(&p0);
println!("{}", state.checksum());
}
