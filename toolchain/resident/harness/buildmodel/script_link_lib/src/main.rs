extern "C" {
    fn native_scale(n: u32) -> u32;
}

fn main() {
    println!("{}", unsafe { native_scale(std::env::args().count() as u32) });
}
