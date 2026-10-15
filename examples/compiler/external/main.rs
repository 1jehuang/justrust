mod math;
mod display;
fn main() {
    if math::sequence::sum(10) == 55 { display::success(); } else { println!("incorrect result"); }
}
