fn main() {
    println!("{} from {}", dep::scale(std::env::args().count() as u32), dep::origin());
}
